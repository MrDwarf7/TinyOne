use std::io::Write;

use crate::{
    Instr, MAX_CALL_DEPTH, Op, Program, Result, TinyHeapStats, TinyMemory, TinyOneError,
    TinyRuntimeContext, Value, checked_div, checked_non_negative_usize, checked_stack_count,
    runtime_add, runtime_call_builtin, runtime_compare, runtime_get_field, runtime_index,
    runtime_is_false, runtime_make_array, runtime_make_struct, runtime_mul, runtime_neg,
    runtime_null, runtime_print, runtime_set_field, runtime_set_index, runtime_sub,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TinyRunReport {
    pub memory: TinyMemory,
    pub heap_before_shutdown: TinyHeapStats,
    pub heap_after_shutdown: TinyHeapStats,
}

pub struct VM<'a> {
    program: &'a Program,
    memory: TinyMemory,
    context: TinyRuntimeContext,
    call_depth: usize,
}

impl<'a> VM<'a> {
    pub fn new(program: &'a Program, memory: TinyMemory, inputs: Vec<String>) -> Self {
        Self {
            program,
            memory,
            context: TinyRuntimeContext::new(inputs),
            call_depth: 0,
        }
    }

    pub fn run(self, stdout: &mut dyn Write) -> Result<TinyMemory> {
        Ok(self.run_report(stdout)?.memory)
    }

    pub fn run_report(mut self, stdout: &mut dyn Write) -> Result<TinyRunReport> {
        let mut memory = std::mem::take(&mut self.memory);
        self.run_chunk(&self.program.code, &mut memory, stdout, "main")?;
        let heap_before_shutdown = self.context.heap_stats();
        let heap_after_shutdown = self.context.shutdown();
        Ok(TinyRunReport {
            memory,
            heap_before_shutdown,
            heap_after_shutdown,
        })
    }

    fn run_chunk(
        &mut self,
        code: &[Instr],
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
        chunk_name: &str,
    ) -> Result<Option<Value>> {
        let mut stack: Vec<Value> = Vec::with_capacity(code.len().min(32));
        let mut pc = 0usize;
        loop {
            let instr = code.get(pc).copied().ok_or_else(|| {
                TinyOneError::runtime(format!("Invalid program counter in {chunk_name}"))
            })?;
            pc += 1;
            match instr.op {
                Op::PushInt => stack.push(Value::Int(instr.arg)),
                Op::PushNull => stack.push(runtime_null()),
                Op::PushString => {
                    let string_index = checked_non_negative_usize(instr.arg, "string index")?;
                    let text = self
                        .program
                        .strings
                        .get(string_index)
                        .ok_or_else(|| {
                            TinyOneError::runtime(format!("Invalid string index {string_index}"))
                        })?
                        .clone();
                    stack.push(Value::Heap(self.context.heap.alloc_string(text)?));
                }
                Op::Load => {
                    let slot = checked_non_negative_usize(instr.arg, "memory slot")?;
                    stack.push(memory.load(slot)?);
                }
                Op::Store => {
                    let slot = checked_non_negative_usize(instr.arg, "memory slot")?;
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    memory.store(slot, value)?;
                }
                Op::Add => {
                    let rhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let lhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_add(lhs, rhs)?);
                }
                Op::Sub => {
                    let rhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let lhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_sub(lhs, rhs)?);
                }
                Op::Mul => {
                    let rhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let lhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_mul(lhs, rhs)?);
                }
                Op::Div => {
                    let rhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let lhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(checked_div(lhs, rhs)?);
                }
                Op::Neg => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_neg(value)?);
                }
                Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne => {
                    let rhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let lhs = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_compare(instr.op, lhs, rhs)?);
                }
                Op::Jump => pc = checked_non_negative_usize(instr.arg, "jump target")?,
                Op::JumpIfZero => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    if runtime_is_false(&value) {
                        pc = checked_non_negative_usize(instr.arg, "jump target")?;
                    }
                }
                Op::Call => {
                    let function_index = checked_non_negative_usize(instr.arg, "function index")?;
                    let arg_count = checked_non_negative_usize(instr.arg2, "function arity")?;
                    let result =
                        self.call_function(function_index, &mut stack, arg_count, stdout)?;
                    stack.push(result);
                }
                Op::MakeArray => {
                    let count = checked_non_negative_usize(instr.arg, "array arity")?;
                    checked_stack_count(stack.len(), count)?;
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        values.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    values.reverse();
                    stack.push(runtime_make_array(&mut self.context, values)?);
                }
                Op::Index => {
                    let index = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let container = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    stack.push(runtime_index(&mut self.context, container, index)?);
                }
                Op::SetIndex => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let index = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let container = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    runtime_set_index(&mut self.context, container, index, value)?;
                }
                Op::MakeStruct => {
                    let field_count = checked_non_negative_usize(instr.arg2, "struct arity")?;
                    checked_stack_count(stack.len(), field_count)?;
                    let mut values = Vec::with_capacity(field_count);
                    for _ in 0..field_count {
                        values.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    values.reverse();
                    let struct_index = checked_non_negative_usize(instr.arg, "struct index")?;
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
                Op::GetField => {
                    let target = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let field_index = checked_non_negative_usize(instr.arg, "field index")?;
                    let field = self.program.fields.get(field_index).ok_or_else(|| {
                        TinyOneError::runtime(format!("Invalid field index {field_index}"))
                    })?;
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                Op::SetField => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let target = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let field_index = checked_non_negative_usize(instr.arg, "field index")?;
                    let field = self.program.fields.get(field_index).ok_or_else(|| {
                        TinyOneError::runtime(format!("Invalid field index {field_index}"))
                    })?;
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                Op::Builtin => {
                    let builtin_index = checked_non_negative_usize(instr.arg, "builtin index")?;
                    let arg_count = checked_non_negative_usize(instr.arg2, "builtin arity")?;
                    checked_stack_count(stack.len(), arg_count)?;
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    args.reverse();
                    stack.push(runtime_call_builtin(
                        &mut self.context,
                        builtin_index,
                        args,
                    )?);
                }
                Op::Return => {
                    return Ok(Some(
                        stack
                            .pop()
                            .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                    ));
                }
                Op::Print => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    runtime_print(&self.context, stdout, &value)?;
                }
                Op::Halt => {
                    if !stack.is_empty() {
                        return Err(TinyOneError::runtime(format!(
                            "Internal stack imbalance at halt in {chunk_name}"
                        )));
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
    ) -> Result<Value> {
        let function = self.program.functions.get(function_index).ok_or_else(|| {
            TinyOneError::runtime(format!("Invalid function index {function_index}"))
        })?;
        if arg_count != function.param_count {
            return Err(TinyOneError::runtime(format!(
                "Function {:?} expects {} argument(s), got {arg_count}",
                function.name, function.param_count
            )));
        }
        if self.call_depth >= MAX_CALL_DEPTH {
            return Err(TinyOneError::runtime(format!(
                "Call stack overflow after {MAX_CALL_DEPTH} nested call(s)"
            )));
        }
        checked_stack_count(caller_stack.len(), arg_count)?;
        let mut memory = TinyMemory::new(function.slot_count);
        for slot in (0..arg_count).rev() {
            let value = caller_stack
                .pop()
                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
            memory.store(slot, value)?;
        }
        self.call_depth += 1;
        let result = self.run_chunk(&function.code, &mut memory, stdout, &function.name);
        self.call_depth -= 1;
        result?.ok_or_else(|| {
            TinyOneError::runtime(format!("Function {:?} returned no value", function.name))
        })
    }
}
