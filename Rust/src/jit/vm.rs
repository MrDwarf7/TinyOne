use std::collections::HashMap;
use std::io::Write;

use crate::{
    JitOp, JitProgram, MAX_CALL_DEPTH, Result, TinyMemory, TinyOneError, TinyRunReport,
    TinyRuntimeContext, Value, checked_div, checked_div_int, pop_args, runtime_add,
    runtime_add_int, runtime_call_builtin, runtime_compare, runtime_compare_int, runtime_get_field,
    runtime_index, runtime_is_false, runtime_make_array, runtime_make_struct, runtime_mul,
    runtime_mul_int, runtime_neg, runtime_null, runtime_print, runtime_set_field,
    runtime_set_index, runtime_sub, runtime_sub_int,
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

pub(crate) struct JitVm<'a> {
    program: &'a mut JitProgram,
    context: TinyRuntimeContext,
    call_depth: usize,
}

impl<'a> JitVm<'a> {
    pub(crate) fn new(program: &'a mut JitProgram, inputs: Vec<String>) -> Self {
        Self {
            program,
            context: TinyRuntimeContext::new(inputs),
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
            .ok_or_else(|| TinyOneError::runtime("JIT program has no main chunk"))?
            .slot_count;
        let mut memory = TinyMemory::new(slot_count);
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
        let stack_capacity = self
            .program
            .chunks
            .get(chunk_index)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid JIT chunk {chunk_index}")))?
            .ops
            .len()
            .min(32);
        let mut stack: Vec<Value> = Vec::with_capacity(stack_capacity);
        let mut pc = 0usize;
        loop {
            let instr = {
                let chunk = self.program.chunks.get(chunk_index).ok_or_else(|| {
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
            match instr {
                JitOp::PushInt(value) => stack.push(Value::I64(value)),
                JitOp::PushNull => stack.push(runtime_null()),
                JitOp::Pop => {
                    jit_pop(&mut stack)?;
                }
                JitOp::PushString(index) => {
                    let text = self
                        .program
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
                    let value = jit_pop(&mut stack)?;
                    memory.store(slot, value)?;
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
                JitOp::Add => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_add(lhs, rhs)?);
                }
                JitOp::AddInt => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_add_int(lhs, rhs)?);
                }
                JitOp::Sub => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_sub(lhs, rhs)?);
                }
                JitOp::SubInt => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_sub_int(lhs, rhs)?);
                }
                JitOp::Mul => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_mul(lhs, rhs)?);
                }
                JitOp::MulInt => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_mul_int(lhs, rhs)?);
                }
                JitOp::Div => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(checked_div(lhs, rhs)?);
                }
                JitOp::DivInt => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(checked_div_int(lhs, rhs)?);
                }
                JitOp::Neg => {
                    let value = jit_pop(&mut stack)?;
                    stack.push(runtime_neg(value)?);
                }
                JitOp::Compare(op) => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
                    stack.push(runtime_compare(op, lhs, rhs)?);
                }
                JitOp::CompareInt(op) => {
                    let (lhs, rhs) = jit_pop_pair(&mut stack)?;
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
                    let value = jit_pop(&mut stack)?;
                    if runtime_is_false(&value) {
                        if target < op_pc {
                            self.program.record_back_edge(chunk_index, op_pc, target);
                        }
                        pc = target;
                    }
                }
                JitOp::JumpIfZeroHot(target) => {
                    let value = jit_pop(&mut stack)?;
                    if runtime_is_false(&value) {
                        pc = target;
                    }
                }
                JitOp::Call(function_index, arg_count) => {
                    let globals = global_memory.unwrap_or(&*memory);
                    let result =
                        self.call_function(function_index, &mut stack, arg_count, stdout, globals)?;
                    stack.push(result);
                }
                JitOp::MakeArray(count) => {
                    let values = pop_args(&mut stack, count)?;
                    stack.push(runtime_make_array(&mut self.context, values)?);
                }
                JitOp::Index => {
                    let index = jit_pop(&mut stack)?;
                    let container = jit_pop(&mut stack)?;
                    stack.push(runtime_index(&mut self.context, container, index)?);
                }
                JitOp::SetIndex => {
                    let value = jit_pop(&mut stack)?;
                    let index = jit_pop(&mut stack)?;
                    let container = jit_pop(&mut stack)?;
                    runtime_set_index(&mut self.context, container, index, value)?;
                }
                JitOp::MakeStruct(struct_index, field_count) => {
                    let values = pop_args(&mut stack, field_count)?;
                    let struct_def = self.program.structs.get(struct_index).ok_or_else(|| {
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
                    let target = jit_pop(&mut stack)?;
                    let field = self.program.fields.get(field_index).ok_or_else(|| {
                        TinyOneError::runtime(format!("Invalid field index {field_index}"))
                    })?;
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                JitOp::SetField(field_index) => {
                    let value = jit_pop(&mut stack)?;
                    let target = jit_pop(&mut stack)?;
                    let field = self.program.fields.get(field_index).ok_or_else(|| {
                        TinyOneError::runtime(format!("Invalid field index {field_index}"))
                    })?;
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                JitOp::Builtin(builtin_index, arg_count) => {
                    let args = pop_args(&mut stack, arg_count)?;
                    stack.push(runtime_call_builtin(
                        &mut self.context,
                        builtin_index,
                        args,
                    )?);
                }
                JitOp::Return => return Ok(Some(jit_pop(&mut stack)?)),
                JitOp::Print => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout.write_all(&self.context.queued_stdout)
                            .map_err(|e| TinyOneError::runtime(format!("stdout flush error: {e}")))?;
                        self.context.queued_stdout.clear();
                    }
                    let value = jit_pop(&mut stack)?;
                    runtime_print(&self.context, stdout, &value)?;
                }
                JitOp::Halt => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout.write_all(&self.context.queued_stdout)
                            .map_err(|e| TinyOneError::runtime(format!("stdout flush error: {e}")))?;
                        self.context.queued_stdout.clear();
                    }
                    if !stack.is_empty() {
                        let chunk_name = self
                            .program
                            .chunks
                            .get(chunk_index)
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
        let (chunk_index, slot_count, param_count) = {
            let function = self.program.functions.get(function_index).ok_or_else(|| {
                TinyOneError::runtime(format!("Invalid function index {function_index}"))
            })?;
            (
                function.chunk_index,
                function.slot_count,
                function.param_count,
            )
        };
        if arg_count != param_count {
            return Err(TinyOneError::runtime(format!(
                "Function {:?} expects {param_count} argument(s), got {arg_count}",
                self.function_name(function_index)
            )));
        }
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(TinyOneError::runtime(format!(
                "Call stack overflow after {MAX_CALL_DEPTH} nested call(s)"
            )));
        }
        let args = pop_args(caller_stack, arg_count)?;
        let mut memory = TinyMemory::new(slot_count);
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
            .functions
            .get(function_index)
            .map(|function| function.name.as_str())
            .unwrap_or("<invalid>")
    }
}
