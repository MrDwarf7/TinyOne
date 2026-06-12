use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use crate::{
    BytecodeVerifier,
    Instr,
    MAX_CALL_DEPTH,
    Op,
    Program,
    Result,
    TinyHeapStats,
    TinyMemory,
    TinyOneError,
    TinyRuntimeContext,
    Value,
    checked_div,
    checked_non_negative_usize,
    pop_args,
    runtime_add,
    runtime_call_builtin,
    runtime_compare,
    runtime_get_field,
    runtime_index,
    runtime_is_false,
    runtime_make_array,
    runtime_make_struct,
    runtime_mul,
    runtime_neg,
    runtime_null,
    runtime_print,
    runtime_set_field,
    runtime_set_index,
    runtime_sub,
};

#[derive(Debug, Clone, PartialEq)]
pub struct TinyRunReport {
    pub memory:               TinyMemory,
    pub heap_before_shutdown: TinyHeapStats,
    pub heap_after_shutdown:  TinyHeapStats,
}

pub struct VM {
    pub(crate) program: Arc<Program>,
    memory:             TinyMemory,
    pub(crate) context: TinyRuntimeContext,
    call_depth:         usize,
}

fn vm_pop(stack: &mut Vec<Value>) -> Result<Value> {
    stack.pop().ok_or_else(|| TinyOneError::runtime("Stack underflow"))
}

fn vm_pop_pair(stack: &mut Vec<Value>) -> Result<(Value, Value)> {
    let rhs = vm_pop(stack)?;
    let lhs = vm_pop(stack)?;
    Ok((lhs, rhs))
}

fn lookup_field(fields: &[String], index: usize) -> Result<&str> {
    fields
        .get(index)
        .map(String::as_str)
        .ok_or_else(|| TinyOneError::runtime(format!("Invalid field index {index}")))
}

impl VM {
    pub fn new(program: Arc<Program>, memory: TinyMemory, inputs: Vec<String>) -> Result<Self> {
        BytecodeVerifier::verify(&program)?;
        Ok(Self::new_unchecked(program, memory, inputs))
    }

    /// Construct a VM without re-verifying. Only call this when the caller
    /// has already run `BytecodeVerifier::verify` on the same program.
    pub(crate) fn new_unchecked(program: Arc<Program>, memory: TinyMemory, inputs: Vec<String>) -> Self {
        Self {
            program,
            memory,
            context: TinyRuntimeContext::new(inputs),
            call_depth: 0,
        }
    }

    pub(crate) fn new_unchecked_with_context(
        program: Arc<Program>,
        memory: TinyMemory,
        context: TinyRuntimeContext,
    ) -> Self {
        Self {
            program,
            memory,
            context,
            call_depth: 0,
        }
    }

    pub fn set_sys_args(&mut self, args: Vec<String>) {
        self.context.set_sys_args(args);
    }

    pub fn set_sys_env(&mut self, env: HashMap<String, String>) {
        self.context.set_sys_env(env);
    }

    pub fn run(self, stdout: &mut dyn Write) -> Result<TinyMemory> {
        Ok(self.run_report(stdout)?.memory)
    }

    pub fn run_report(mut self, stdout: &mut dyn Write) -> Result<TinyRunReport> {
        let mut memory = std::mem::take(&mut self.memory);
        let code = self.program.code.clone();
        self.run_chunk(&code, &mut memory, stdout, "main", None)?;
        let heap_before_shutdown = self.context.heap_stats();
        let heap_after_shutdown = self.context.shutdown();
        Ok(TinyRunReport {
            memory,
            heap_before_shutdown,
            heap_after_shutdown,
        })
    }

    /// Run a single function by index. Used by thread_spawn.
    pub(crate) fn run_function_by_index(
        mut self,
        fn_index: usize,
        args: Vec<Value>,
        stdout: &mut dyn Write,
    ) -> Result<Value> {
        let function = self
            .program
            .functions
            .get(fn_index)
            .ok_or_else(|| TinyOneError::runtime(format!("thread_spawn: invalid function index {fn_index}")))?;
        let slot_count = function.slot_count;
        let fn_name = function.name.clone();
        let code = function.code.clone();
        let mut memory = TinyMemory::new(slot_count);
        for (i, v) in args.into_iter().enumerate() {
            memory.store(i, v)?;
        }
        let empty_globals = TinyMemory::new(0);
        let result = self.run_chunk(&code, &mut memory, stdout, &fn_name, Some(&empty_globals))?;
        result.ok_or_else(|| TinyOneError::runtime("thread function returned no value"))
    }

    fn run_chunk(
        &mut self,
        code: &[Instr],
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
        chunk_name: &str,
        global_memory: Option<&TinyMemory>,
    ) -> Result<Option<Value>> {
        let mut stack: Vec<Value> = Vec::with_capacity(code.len().min(32));
        let mut pc = 0usize;
        loop {
            let instr = code
                .get(pc)
                .copied()
                .ok_or_else(|| TinyOneError::runtime(format!("Invalid program counter in {chunk_name}")))?;
            pc += 1;
            match instr.op {
                Op::PushInt => stack.push(Value::I64(instr.arg)),
                Op::PushNull => stack.push(runtime_null()),
                Op::Pop => {
                    vm_pop(&mut stack)?;
                }
                Op::PushString => {
                    let string_index = checked_non_negative_usize(instr.arg, "string index")?;
                    let text = self
                        .program
                        .strings
                        .get(string_index)
                        .ok_or_else(|| TinyOneError::runtime(format!("Invalid string index {string_index}")))?
                        .clone();
                    stack.push(Value::Heap(self.context.heap().alloc_string(text)?));
                }
                Op::Load => {
                    let slot = checked_non_negative_usize(instr.arg, "memory slot")?;
                    stack.push(memory.load(slot)?);
                }
                Op::LoadGlobal => {
                    let slot = checked_non_negative_usize(instr.arg, "global memory slot")?;
                    let globals =
                        global_memory.ok_or_else(|| TinyOneError::runtime("Global load outside a function frame"))?;
                    stack.push(globals.load(slot)?);
                }
                Op::Store => {
                    let slot = checked_non_negative_usize(instr.arg, "memory slot")?;
                    memory.store(slot, vm_pop(&mut stack)?)?;
                }
                Op::Add => {
                    let (lhs, rhs) = vm_pop_pair(&mut stack)?;
                    stack.push(runtime_add(lhs, rhs)?);
                }
                Op::Sub => {
                    let (lhs, rhs) = vm_pop_pair(&mut stack)?;
                    stack.push(runtime_sub(lhs, rhs)?);
                }
                Op::Mul => {
                    let (lhs, rhs) = vm_pop_pair(&mut stack)?;
                    stack.push(runtime_mul(lhs, rhs)?);
                }
                Op::Div => {
                    let (lhs, rhs) = vm_pop_pair(&mut stack)?;
                    stack.push(checked_div(lhs, rhs)?);
                }
                Op::Neg => {
                    let value = vm_pop(&mut stack)?;
                    stack.push(runtime_neg(value)?);
                }
                Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne => {
                    let (lhs, rhs) = vm_pop_pair(&mut stack)?;
                    stack.push(runtime_compare(instr.op, lhs, rhs)?);
                }
                Op::Jump => pc = checked_non_negative_usize(instr.arg, "jump target")?,
                Op::JumpIfZero => {
                    if runtime_is_false(&vm_pop(&mut stack)?) {
                        pc = checked_non_negative_usize(instr.arg, "jump target")?;
                    }
                }
                Op::Call => {
                    let function_index = checked_non_negative_usize(instr.arg, "function index")?;
                    let arg_count = checked_non_negative_usize(instr.arg2, "function arity")?;
                    let globals = global_memory.unwrap_or(&*memory);
                    let result = self.call_function(function_index, &mut stack, arg_count, stdout, globals)?;
                    stack.push(result);
                }
                Op::MakeArray => {
                    let count = checked_non_negative_usize(instr.arg, "array arity")?;
                    let values = pop_args(&mut stack, count)?;
                    stack.push(runtime_make_array(&mut self.context, values)?);
                }
                Op::Index => {
                    let (container, index) = vm_pop_pair(&mut stack)?;
                    stack.push(runtime_index(&mut self.context, container, index)?);
                }
                Op::SetIndex => {
                    let value = vm_pop(&mut stack)?;
                    let index = vm_pop(&mut stack)?;
                    let container = vm_pop(&mut stack)?;
                    runtime_set_index(&mut self.context, container, index, value)?;
                }
                Op::MakeStruct => {
                    let field_count = checked_non_negative_usize(instr.arg2, "struct arity")?;
                    let values = pop_args(&mut stack, field_count)?;
                    let struct_index = checked_non_negative_usize(instr.arg, "struct index")?;
                    let struct_def = self
                        .program
                        .structs
                        .get(struct_index)
                        .ok_or_else(|| TinyOneError::runtime(format!("Invalid struct index {struct_index}")))?;
                    stack.push(runtime_make_struct(&mut self.context, &struct_def.name, &struct_def.fields, values)?);
                }
                Op::GetField => {
                    let target = vm_pop(&mut stack)?;
                    let field_index = checked_non_negative_usize(instr.arg, "field index")?;
                    let field = lookup_field(&self.program.fields, field_index)?;
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                Op::SetField => {
                    let value = vm_pop(&mut stack)?;
                    let target = vm_pop(&mut stack)?;
                    let field_index = checked_non_negative_usize(instr.arg, "field index")?;
                    let field = lookup_field(&self.program.fields, field_index)?;
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                Op::Builtin => {
                    let builtin_index = checked_non_negative_usize(instr.arg, "builtin index")?;
                    let arg_count = checked_non_negative_usize(instr.arg2, "builtin arity")?;
                    let args = pop_args(&mut stack, arg_count)?;
                    stack.push(runtime_call_builtin(&mut self.context, builtin_index, args)?);
                }
                Op::Return => return Ok(Some(vm_pop(&mut stack)?)),
                Op::Print => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout
                            .write_all(&self.context.queued_stdout)
                            .map_err(|e| TinyOneError::runtime(format!("stdout flush error: {e}")))?;
                        self.context.queued_stdout.clear();
                    }
                    let value = vm_pop(&mut stack)?;
                    runtime_print(&self.context, stdout, &value)?;
                }
                Op::Halt => {
                    if !self.context.queued_stdout.is_empty() {
                        stdout
                            .write_all(&self.context.queued_stdout)
                            .map_err(|e| TinyOneError::runtime(format!("stdout flush error: {e}")))?;
                        self.context.queued_stdout.clear();
                    }
                    if !stack.is_empty() {
                        return Err(TinyOneError::runtime(format!("Internal stack imbalance at halt in {chunk_name}")));
                    }
                    return Ok(None);
                }
            }
        }
    }

    fn call_function(
        &mut self,
        function_index: usize,
        caller_stack: &mut Vec<Value>,
        arg_count: usize,
        stdout: &mut dyn Write,
        global_memory: &TinyMemory,
    ) -> Result<Value> {
        let (fn_name, fn_slot_count, fn_code, fn_param_count) = {
            let function = self
                .program
                .functions
                .get(function_index)
                .ok_or_else(|| TinyOneError::runtime(format!("Invalid function index {function_index}")))?;
            (function.name.clone(), function.slot_count, function.code.clone(), function.param_count)
        };
        if arg_count != fn_param_count {
            return Err(TinyOneError::runtime(format!(
                "Function {:?} expects {} argument(s), got {arg_count}",
                fn_name, fn_param_count
            )));
        }
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(TinyOneError::runtime(format!("Call stack overflow after {MAX_CALL_DEPTH} nested call(s)")));
        }
        let args = pop_args(caller_stack, arg_count)?;
        let mut memory = TinyMemory::new(fn_slot_count);
        for (slot, value) in args.into_iter().enumerate() {
            memory.store(slot, value)?;
        }
        self.call_depth += 1;
        let result = self.run_chunk(&fn_code, &mut memory, stdout, &fn_name, Some(global_memory));
        self.call_depth -= 1;
        result?.ok_or_else(|| TinyOneError::runtime(format!("Function {:?} returned no value", fn_name)))
    }
}
