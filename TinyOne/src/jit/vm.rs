use std::collections::HashMap;
use std::io::Write;

use crate::{
    HeapData, JitBuiltin, JitOp, JitProgram, Result, TinyMemory, TinyOneError, TinyRunReport,
    TinyRuntimeContext, TypeKind, Value, checked_div, checked_div_int, pop_args,
    require_builtin_capability, runtime_add, runtime_add_int, runtime_array_pop,
    runtime_array_push, runtime_call_builtin, runtime_compare, runtime_compare_int,
    runtime_get_field, runtime_index, runtime_is_false, runtime_make_array, runtime_make_enum,
    runtime_make_struct, runtime_mul, runtime_mul_int, runtime_neg, runtime_null, runtime_print,
    runtime_set_field, runtime_set_index, runtime_sub, runtime_sub_int,
};

fn jit_pop(stack: &mut Vec<Value>) -> Result<Value> {
    stack
        .pop()
        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))
}

fn jit_pop_pair(stack: &mut Vec<Value>) -> Result<(Value, Value)> {
    let rhs = jit_pop(stack)?;
    let lhs = jit_pop(stack)?;
    Ok((lhs, rhs))
}

fn run_direct_builtin(
    context: &mut TinyRuntimeContext,
    stack: &mut Vec<Value>,
    builtin: JitBuiltin,
    capabilities: crate::ModuleCapabilities,
) -> Result<()> {
    let result = match builtin {
        JitBuiltin::Len => {
            let target = jit_pop(stack)?;
            crate::runtime::builtins::runtime_len(context, &target)?
        }
        JitBuiltin::ArrayPush => {
            let value = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            runtime_array_push(context, &target, value)?
        }
        JitBuiltin::ArrayPop => {
            let target = jit_pop(stack)?;
            runtime_array_pop(context, &target)?
        }
        JitBuiltin::VecNew => crate::runtime::stdlib::b_vec_new(context)?,
        JitBuiltin::VecClear => {
            let target = jit_pop(stack)?;
            crate::runtime::stdlib::b_vec_clear(context, &target)?
        }
        JitBuiltin::MapNew => crate::runtime::stdlib::b_map_new(context)?,
        JitBuiltin::MapSet => {
            let value = jit_pop(stack)?;
            let key = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            match key {
                Value::I64(key) => {
                    crate::runtime::stdlib::b_map_set_i64(context, &target, key, value)?
                }
                key => crate::runtime::stdlib::b_map_set(context, &target, key, value)?,
            }
        }
        JitBuiltin::MapGet => {
            let key = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            match key {
                Value::I64(key) => crate::runtime::stdlib::b_map_get_i64(context, &target, key)?,
                key => crate::runtime::stdlib::b_map_get(context, &target, &key)?,
            }
        }
        JitBuiltin::MapHas => {
            let key = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            crate::runtime::stdlib::b_map_has(context, &target, &key)?
        }
        JitBuiltin::MapDel => {
            let key = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            crate::runtime::stdlib::b_map_del(context, &target, &key)?
        }
        JitBuiltin::Alloc => {
            let value = jit_pop(stack)?;
            Value::Heap(context.heap().alloc_cell(value)?)
        }
        JitBuiltin::Load => {
            let target = jit_pop(stack)?;
            let heap = context.heap();
            let object = heap.get(&target)?;
            let HeapData::Cell(bytes) = &object.data else {
                return Err(TinyOneError::runtime("load() expects a pointer cell"));
            };
            crate::runtime::value_codec::decode_value(bytes.as_slice().try_into().unwrap())
        }
        JitBuiltin::Store => {
            let value = jit_pop(stack)?;
            let target = jit_pop(stack)?;
            let mut heap = context.heap();
            let object = heap.get_mut(&target)?;
            let HeapData::Cell(bytes) = &mut object.data else {
                return Err(TinyOneError::runtime("store() expects a pointer cell"));
            };
            let encoded = crate::runtime::value_codec::encode_value(&value)?;
            bytes.as_mut_slice().copy_from_slice(&encoded);
            value
        }
        JitBuiltin::Free => {
            require_builtin_capability("free", true, capabilities)?;
            let target = jit_pop(stack)?;
            context.heap().free(&target)?;
            Value::I64(0)
        }
    };
    stack.push(result);
    Ok(())
}

pub(crate) struct JitVm<'a> {
    program: &'a mut JitProgram,
    context: TinyRuntimeContext,
    call_depth: usize,
}

impl<'a> JitVm<'a> {
    pub(crate) fn new(program: &'a mut JitProgram, inputs: Vec<String>) -> Self {
        // The JIT retains the verification token for its entire lifetime;
        // obtain the execution metadata from that token rather than from an
        // independently mutable program handle.
        let source_program = program.verified_program.program_arc();
        let mut context = TinyRuntimeContext::new(inputs);
        context.program_arc = Some(source_program);
        context.verified_program = Some(program.verified_program.clone());
        Self {
            program,
            context,
            call_depth: 0,
        }
    }

    pub(crate) fn set_sys_args(&mut self, args: Vec<String>) {
        self.context.set_sys_args(args);
    }

    pub(crate) fn set_sys_env(&mut self, env: HashMap<String, String>) {
        self.context.set_sys_env(env);
    }

    pub(crate) fn run(self, stdout: &mut dyn Write) -> Result<TinyMemory> {
        Ok(self.run_report(stdout)?.memory)
    }

    pub(crate) fn run_report(mut self, stdout: &mut dyn Write) -> Result<TinyRunReport> {
        let slot_count = self
            .program
            .chunks
            .first()
            .and_then(Option::as_ref)
            .ok_or_else(|| TinyOneError::runtime("JIT program has no main chunk"))?
            .slot_count;
        let mut memory = TinyMemory::try_new(slot_count)?;
        self.run_chunk(0, &mut memory, stdout, None)?;
        let heap_before_shutdown = self.context.heap_stats();
        let heap_after_shutdown = self.context.shutdown();
        Ok(TinyRunReport {
            memory,
            heap_before_shutdown,
            heap_after_shutdown,
        })
    }

    pub(crate) fn run_chunk(
        &mut self,
        chunk_index: usize,
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
        global_memory: Option<&TinyMemory>,
    ) -> Result<Option<Value>> {
        self.program.ensure_chunk(chunk_index)?;
        let stack_capacity = self
            .program
            .chunks
            .get(chunk_index)
            .and_then(Option::as_ref)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid JIT chunk {chunk_index}")))?
            .ops
            .len()
            .min(32);
        let mut stack = self.program.take_operand_stack(stack_capacity);
        let result =
            self.run_chunk_with_stack(chunk_index, memory, stdout, global_memory, &mut stack);
        self.program.recycle_operand_stack(stack);
        result
    }

    fn run_chunk_with_stack(
        &mut self,
        chunk_index: usize,
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
        global_memory: Option<&TinyMemory>,
        stack: &mut Vec<Value>,
    ) -> Result<Option<Value>> {
        let capabilities = self
            .program
            .verified_program
            .program()
            .capabilities_for_function(chunk_index.checked_sub(1));
        let mut pc = 0usize;
        loop {
            let instr = {
                let chunk = self
                    .program
                    .chunks
                    .get(chunk_index)
                    .and_then(Option::as_ref)
                    .ok_or_else(|| {
                        TinyOneError::runtime(format!("Invalid JIT chunk {chunk_index}"))
                    })?;
                let Some(instr) = chunk.ops.get(pc).copied() else {
                    return Err(TinyOneError::runtime(format!(
                        "Invalid program counter in {}",
                        chunk.name
                    )));
                };
                instr
            };
            let op_pc = pc;
            pc += 1;
            self.program.record_execution_op(instr, stack.len());
            match instr {
                JitOp::PushInt(value) => stack.push(Value::I64(value)),
                JitOp::PushNull => stack.push(runtime_null()),
                JitOp::PushBool(value) => stack.push(Value::Bool(value)),
                JitOp::PushFloat(bits) => stack.push(Value::Float {
                    kind: TypeKind::Fp64,
                    bits: f64::from_bits(bits),
                }),
                JitOp::PushFunction(function_index) => {
                    if function_index >= self.program.verified_program.program().functions.len() {
                        return Err(TinyOneError::runtime(format!(
                            "Invalid function index {function_index}"
                        )));
                    }
                    stack.push(Value::Function(function_index as u32));
                }
                JitOp::Pop => {
                    jit_pop(stack)?;
                }
                JitOp::PushString(index) => {
                    let text = self
                        .program
                        .verified_program
                        .program()
                        .strings
                        .get(index)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!("Invalid string index {index}"))
                        })?
                        .clone();
                    stack.push(Value::Heap(self.context.heap().alloc_string(text)?));
                }
                JitOp::Load(slot) => stack.push(memory.load(slot)?),
                JitOp::LoadGlobal(slot) => {
                    let globals = global_memory.ok_or_else(|| {
                        TinyOneError::runtime("Global load outside a function frame")
                    })?;
                    stack.push(globals.load(slot)?);
                }
                JitOp::Store(slot) => {
                    let value = jit_pop(stack)?;
                    memory.store(slot, value)?;
                }
                JitOp::MoveSlot(source, destination) => {
                    memory.store(destination, memory.load(source)?)?;
                }
                JitOp::StoreInt(slot, value) => {
                    memory.store_int(slot, value)?;
                }
                JitOp::AddSlotInt(slot, value) => {
                    memory.add_int(slot, value)?;
                }
                JitOp::SubSlotInt(slot, value) => {
                    memory.sub_int(slot, value)?;
                }
                JitOp::MulSlotInt(slot, value) => {
                    let result = runtime_mul(memory.load(slot)?, Value::I64(value))?;
                    memory.store(slot, result)?;
                }
                JitOp::MulSlotIntHot(slot, value) => {
                    memory.mul_int_assign(slot, value)?;
                }
                JitOp::DivSlotInt(slot, value) => {
                    let result = checked_div(memory.load(slot)?, Value::I64(value))?;
                    memory.store(slot, result)?;
                }
                JitOp::DivSlotIntHot(slot, value) => {
                    memory.div_int_assign(slot, value)?;
                }
                JitOp::CompareSlotIntJumpIfZero(operands) => {
                    let slot = operands.slot();
                    let value = operands.value();
                    let op = operands.comparison();
                    let target = operands.target();
                    let condition = runtime_compare(op, memory.load(slot)?, Value::I64(value))?;
                    if runtime_is_false(&condition) {
                        if target < op_pc {
                            self.program.record_back_edge(chunk_index, op_pc, target);
                        }
                        pc = target;
                    }
                }
                JitOp::CompareSlotIntJumpIfZeroHot(operands) => {
                    let slot = operands.slot();
                    let value = operands.value();
                    let op = operands.comparison();
                    let target = operands.target();
                    let is_false = match memory.compare_int(slot, value, op)? {
                        Some(condition) => !condition,
                        None => runtime_is_false(&runtime_compare_int(
                            op,
                            memory.load(slot)?,
                            Value::I64(value),
                        )?),
                    };
                    if is_false {
                        pc = target;
                    }
                }
                JitOp::JumpIfZeroSlot(slot, target) => {
                    if runtime_is_false(&memory.load(slot)?) {
                        if target < op_pc {
                            self.program.record_back_edge(chunk_index, op_pc, target);
                        }
                        pc = target;
                    }
                }
                JitOp::JumpIfZeroSlotHot(slot, target) => {
                    let is_false = match memory.is_int_zero(slot)? {
                        Some(is_zero) => is_zero,
                        None => runtime_is_false(&memory.load(slot)?),
                    };
                    if is_false {
                        pc = target;
                    }
                }
                JitOp::ArrayLenSlotJumpIfZero(slot, target) => {
                    let target_value = memory.load(slot)?;
                    let length =
                        crate::runtime::builtins::runtime_len(&self.context, &target_value)?;
                    if matches!(length, Value::I64(0)) {
                        if target < op_pc {
                            self.program.record_back_edge(chunk_index, op_pc, target);
                        }
                        pc = target;
                    }
                }
                JitOp::MapGetAddSlots(slots) => {
                    let total = memory.load(slots.total_slot())?;
                    let target = memory.load(slots.map_slot())?;
                    let key = memory.load(slots.key_slot())?;
                    let value = match key {
                        Value::I64(key) => {
                            crate::runtime::stdlib::b_map_get_i64(&mut self.context, &target, key)?
                        }
                        key => crate::runtime::stdlib::b_map_get(&mut self.context, &target, &key)?,
                    };
                    memory.store(slots.total_slot(), runtime_add(total, value)?)?;
                }
                JitOp::MapSetMulSlots(slots) => {
                    let target = memory.load(slots.map_slot())?;
                    let key = memory.load(slots.key_slot())?;
                    let value = match memory.mul_int(slots.value_slot(), slots.multiplier())? {
                        Some(value) => Value::I64(value),
                        None => runtime_mul_int(
                            memory.load(slots.value_slot())?,
                            Value::I64(slots.multiplier()),
                        )?,
                    };
                    let result = match key {
                        Value::I64(key) => crate::runtime::stdlib::b_map_set_i64(
                            &mut self.context,
                            &target,
                            key,
                            value,
                        )?,
                        key => crate::runtime::stdlib::b_map_set(
                            &mut self.context,
                            &target,
                            key,
                            value,
                        )?,
                    };
                    memory.store(slots.destination_slot(), result)?;
                }
                JitOp::PushMulSlotInt(slot, value) => {
                    stack.push(runtime_mul(memory.load(slot)?, Value::I64(value))?);
                }
                JitOp::PushMulSlotIntHot(slot, value) => {
                    let result = match memory.mul_int(slot, value)? {
                        Some(result) => Value::I64(result),
                        None => runtime_mul_int(memory.load(slot)?, Value::I64(value))?,
                    };
                    stack.push(result);
                }
                JitOp::PushDivSlotInt(slot, value) => {
                    stack.push(checked_div(memory.load(slot)?, Value::I64(value))?);
                }
                JitOp::PushDivSlotIntHot(slot, value) => {
                    let result = match memory.div_int(slot, value)? {
                        Some(result) => Value::I64(result),
                        None => checked_div_int(memory.load(slot)?, Value::I64(value))?,
                    };
                    stack.push(result);
                }
                JitOp::Add => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_add(lhs, rhs)?);
                }
                JitOp::AddInt => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_add_int(lhs, rhs)?);
                }
                JitOp::Sub => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_sub(lhs, rhs)?);
                }
                JitOp::SubInt => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_sub_int(lhs, rhs)?);
                }
                JitOp::Mul => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_mul(lhs, rhs)?);
                }
                JitOp::MulInt => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_mul_int(lhs, rhs)?);
                }
                JitOp::Div => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(checked_div(lhs, rhs)?);
                }
                JitOp::DivInt => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(checked_div_int(lhs, rhs)?);
                }
                JitOp::Neg => {
                    let value = jit_pop(stack)?;
                    stack.push(runtime_neg(value)?);
                }
                JitOp::Compare(op) => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_compare(op, lhs, rhs)?);
                }
                JitOp::CompareInt(op) => {
                    let (lhs, rhs) = jit_pop_pair(stack)?;
                    stack.push(runtime_compare_int(op, lhs, rhs)?);
                }
                JitOp::Jump(target) => {
                    if target < op_pc {
                        self.program.record_back_edge(chunk_index, op_pc, target);
                    }
                    pc = target;
                }
                JitOp::JumpHot(target) => {
                    pc = target;
                }
                JitOp::JumpIfZero(target) => {
                    let value = jit_pop(stack)?;
                    if runtime_is_false(&value) {
                        if target < op_pc {
                            self.program.record_back_edge(chunk_index, op_pc, target);
                        }
                        pc = target;
                    }
                }
                JitOp::JumpIfZeroHot(target) => {
                    let value = jit_pop(stack)?;
                    if runtime_is_false(&value) {
                        pc = target;
                    }
                }
                JitOp::Call(function_index, arg_count) => {
                    let globals = global_memory.unwrap_or(&*memory);
                    let result =
                        self.call_function(function_index, stack, arg_count, stdout, globals)?;
                    stack.push(result);
                }
                JitOp::CallValue(arg_count) => {
                    let args = pop_args(stack, arg_count)?;
                    let callable = jit_pop(stack)?;
                    let (function_index, call_args) = match callable {
                        Value::Function(index) => (index as usize, args),
                        Value::Heap(reference) => {
                            let captures = {
                                let heap = self.context.heap();
                                let object = heap.get(&Value::Heap(reference))?;
                                let HeapData::Closure {
                                    function_id,
                                    captures,
                                } = &object.data
                                else {
                                    return Err(TinyOneError::runtime(
                                        "CallValue expects a function or Closure",
                                    ));
                                };
                                (
                                    *function_id as usize,
                                    crate::runtime::heap::decode_array_values(captures),
                                )
                            };
                            let (function_index, mut captures) = captures;
                            captures.extend(args);
                            (function_index, captures)
                        }
                        _ => {
                            return Err(TinyOneError::runtime(
                                "CallValue expects a function or Closure",
                            ));
                        }
                    };
                    let globals = global_memory.unwrap_or(&*memory);
                    stack.push(self.call_function_with_args(
                        function_index,
                        call_args,
                        stdout,
                        globals,
                    )?);
                }
                JitOp::MakeArray(count) => {
                    let values = pop_args(stack, count)?;
                    stack.push(runtime_make_array(&mut self.context, values)?);
                }
                JitOp::Index => {
                    let index = jit_pop(stack)?;
                    let container = jit_pop(stack)?;
                    stack.push(runtime_index(&mut self.context, container, index)?);
                }
                JitOp::SetIndex => {
                    let value = jit_pop(stack)?;
                    let index = jit_pop(stack)?;
                    let container = jit_pop(stack)?;
                    runtime_set_index(&mut self.context, container, index, value)?;
                }
                JitOp::MakeStruct(struct_index, field_count) => {
                    let values = pop_args(stack, field_count)?;
                    let struct_def = self
                        .program
                        .verified_program
                        .program()
                        .structs
                        .get(struct_index)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!("Invalid struct index {struct_index}"))
                        })?;
                    stack.push(runtime_make_struct(
                        &mut self.context,
                        &struct_def.name,
                        &struct_def.fields,
                        values,
                    )?);
                }
                JitOp::GetField(field_index) => {
                    let target = jit_pop(stack)?;
                    let field = self
                        .program
                        .verified_program
                        .program()
                        .fields
                        .get(field_index)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!("Invalid field index {field_index}"))
                        })?;
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                JitOp::SetField(field_index) => {
                    let value = jit_pop(stack)?;
                    let target = jit_pop(stack)?;
                    let field = self
                        .program
                        .verified_program
                        .program()
                        .fields
                        .get(field_index)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!("Invalid field index {field_index}"))
                        })?;
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                JitOp::MakeEnum(variant_id, field_count) => {
                    let values = pop_args(stack, field_count)?;
                    let variant_def = self
                        .program
                        .verified_program
                        .program()
                        .enum_variants
                        .get(variant_id)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!(
                                "Invalid enum variant index {variant_id}"
                            ))
                        })?;
                    stack.push(runtime_make_enum(
                        &mut self.context,
                        &variant_def.enum_name,
                        &variant_def.variant_name,
                        variant_def.tag,
                        &variant_def.fields,
                        values,
                    )?);
                }
                JitOp::BuiltinDirect(builtin) => {
                    run_direct_builtin(&mut self.context, stack, builtin, capabilities)?;
                }
                JitOp::Builtin(builtin_index, arg_count) => {
                    let args_start = stack
                        .len()
                        .checked_sub(arg_count)
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let globals = global_memory.unwrap_or(&*memory);
                    let caller_function = chunk_index.checked_sub(1);
                    let result = runtime_call_builtin(
                        &mut self.context,
                        globals,
                        builtin_index,
                        caller_function,
                        capabilities,
                        &stack[args_start..],
                    )?;
                    stack.truncate(args_start);
                    stack.push(result);
                }
                JitOp::Return => return Ok(Some(jit_pop(stack)?)),
                JitOp::Print => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout.write_all(&self.context.queued_stdout).map_err(|e| {
                            TinyOneError::runtime(format!("stdout flush error: {e}"))
                        })?;
                        self.context.queued_stdout.clear();
                    }
                    let value = jit_pop(stack)?;
                    runtime_print(&self.context, stdout, &value)?;
                }
                JitOp::Halt => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout.write_all(&self.context.queued_stdout).map_err(|e| {
                            TinyOneError::runtime(format!("stdout flush error: {e}"))
                        })?;
                        self.context.queued_stdout.clear();
                    }
                    if !stack.is_empty() {
                        let chunk_name = self
                            .program
                            .chunks
                            .get(chunk_index)
                            .and_then(Option::as_ref)
                            .map(|chunk| chunk.name.as_str())
                            .unwrap_or("<invalid>");
                        return Err(TinyOneError::runtime(format!(
                            "Internal stack imbalance at halt in {chunk_name}"
                        )));
                    }
                    return Ok(None);
                }
            }
        }
    }

    pub(crate) fn call_function(
        &mut self,
        function_index: usize,
        caller_stack: &mut Vec<Value>,
        arg_count: usize,
        stdout: &mut dyn Write,
        global_memory: &TinyMemory,
    ) -> Result<Value> {
        let args = pop_args(caller_stack, arg_count)?;
        self.call_function_with_args(function_index, args, stdout, global_memory)
    }

    fn call_function_with_args(
        &mut self,
        function_index: usize,
        args: Vec<Value>,
        stdout: &mut dyn Write,
        global_memory: &TinyMemory,
    ) -> Result<Value> {
        let (slot_count, param_count) = {
            let function = self
                .program
                .verified_program
                .program()
                .functions
                .get(function_index)
                .ok_or_else(|| {
                    TinyOneError::runtime(format!("Invalid function index {function_index}"))
                })?;
            (function.slot_count, function.param_count)
        };
        let chunk_index = function_index + 1;
        if args.len() != param_count {
            return Err(TinyOneError::runtime(format!(
                "Function {:?} expects {} argument(s), got {}",
                self.function_name(function_index),
                param_count,
                args.len()
            )));
        }
        let max_call_depth = self.program.verified_program.program().max_call_depth();
        if self.call_depth >= max_call_depth {
            return Err(TinyOneError::runtime(format!(
                "Call stack overflow after {max_call_depth} nested call(s)"
            )));
        }
        let mut memory = TinyMemory::try_new(slot_count)?;
        for (slot, value) in args.into_iter().enumerate() {
            memory.store(slot, value)?;
        }
        self.call_depth += 1;
        let result = self.run_chunk(chunk_index, &mut memory, stdout, Some(global_memory));
        self.call_depth -= 1;
        result?.ok_or_else(|| {
            TinyOneError::runtime(format!(
                "Function {:?} returned no value",
                self.function_name(function_index)
            ))
        })
    }

    fn function_name(&self, function_index: usize) -> &str {
        self.program
            .verified_program
            .program()
            .functions
            .get(function_index)
            .map(|function| function.name.as_str())
            .unwrap_or("<invalid>")
    }
}
