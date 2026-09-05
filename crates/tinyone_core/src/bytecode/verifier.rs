use std::collections::{HashMap, HashSet};

use crate::{BUILTINS, EnumVariantDef, Function, Instr, Op, Program, Result, StructDef, TinyOneError};

const MAX_VERIFIER_STEPS: usize = 10_000_000;
const MAX_STACK_DEPTH: i64 = 65_536;
const MAX_VERIFIER_FUNCTIONS: usize = 4_096;
const MAX_VERIFIER_TOTAL_OPS: usize = 262_144;
const MAX_VERIFIER_SLOT_COUNT: usize = 65_536;
const MAX_VERIFIER_NAMES: usize = 65_536;
const MAX_VERIFIER_STRINGS: usize = 65_536;
const MAX_VERIFIER_FIELDS: usize = 65_536;
const MAX_VERIFIER_STRUCTS: usize = 4_096;
const MAX_VERIFIER_STRUCT_FIELDS: usize = 256;
const MAX_VERIFIER_ENUM_VARIANTS: usize = 4_096;
const MAX_VERIFIER_ENUM_VARIANT_FIELDS: usize = 256;
const MAX_VERIFIER_MODULES: usize = 256;
const MAX_VERIFIER_MODULE_IMPORTS: usize = 4_096;
const MAX_VERIFIER_MODULE_EXPORTS: usize = 4_096;
const MAX_VERIFIER_TEXT_BYTES: usize = 1024 * 1024;

pub struct BytecodeVerifier;

struct VerificationContext<'a> {
    functions:         &'a [Function],
    strings:           &'a [String],
    structs:           &'a [StructDef],
    fields:            &'a [String],
    enum_variants:     &'a [EnumVariantDef],
    global_slot_count: usize,
    modules:           ModuleGraph,
}

#[derive(Debug)]
struct ModuleGraph {
    function_owners:     Vec<Option<usize>>,
    struct_owners:       Vec<Option<usize>>,
    enum_variant_owners: Vec<Option<usize>>,
    exported_functions:  HashSet<usize>,
    exported_structs:    HashSet<usize>,
    imports:             Vec<HashSet<usize>>,
}

impl ModuleGraph {
    fn build(program: &Program) -> Result<Self> {
        let mut modules_by_name = HashMap::new();
        for (module_index, module) in program.modules.iter().enumerate() {
            verify_identifier("module name", &module.name)?;
            if module.path != module.name {
                return Err(TinyOneError::compile(format!(
                    "Verifier: module {:?} has inconsistent logical path {:?}",
                    module.name, module.path
                )));
            }
            if modules_by_name.insert(module.name.as_str(), module_index).is_some() {
                return Err(TinyOneError::compile(format!("Verifier: duplicate module name {:?}", module.name)));
            }
        }

        let function_names =
            unique_named_indexes("function", program.functions.iter().map(|function| function.name.as_str()))?;
        let struct_names = unique_named_indexes("struct", program.structs.iter().map(|item| item.name.as_str()))?;
        let function_owners = program
            .functions
            .iter()
            .map(|function| owner_for_name("function", &function.name, &modules_by_name))
            .collect::<Result<Vec<_>>>()?;
        let struct_owners = program
            .structs
            .iter()
            .map(|item| owner_for_name("struct", &item.name, &modules_by_name))
            .collect::<Result<Vec<_>>>()?;
        let enum_variant_owners = program
            .enum_variants
            .iter()
            .map(|item| owner_for_name("enum", &item.enum_name, &modules_by_name))
            .collect::<Result<Vec<_>>>()?;

        let mut exported_functions = HashSet::new();
        let mut exported_structs = HashSet::new();
        let mut imports = vec![HashSet::new(); program.modules.len()];
        for (module_index, module) in program.modules.iter().enumerate() {
            let mut aliases = HashSet::new();
            for import in &module.imports {
                verify_identifier("module import alias", &import.alias)?;
                verify_identifier("module import target", &import.module)?;
                verify_identifier("resolved module import target", &import.resolved)?;
                if !aliases.insert(import.alias.as_str()) {
                    return Err(TinyOneError::compile(format!(
                        "Verifier: duplicate import alias {:?} in module {:?}",
                        import.alias, module.name
                    )));
                }
                if import.module != import.resolved {
                    return Err(TinyOneError::compile(format!(
                        "Verifier: import {:?} in module {:?} resolves inconsistently",
                        import.alias, module.name
                    )));
                }
                let target = modules_by_name.get(import.resolved.as_str()).copied().ok_or_else(|| {
                    TinyOneError::compile(format!(
                        "Verifier: import {:?} in module {:?} targets unknown module {:?}",
                        import.alias, module.name, import.resolved
                    ))
                })?;
                if target == module_index {
                    return Err(TinyOneError::compile(format!("Verifier: module {:?} imports itself", module.name)));
                }
                imports[module_index].insert(target);
            }

            let mut seen_exports = HashSet::new();
            for export in &module.exported_functions {
                verify_identifier("module function export", export)?;
                if !seen_exports.insert(export.as_str()) {
                    return Err(TinyOneError::compile(format!(
                        "Verifier: duplicate function export {:?} in module {:?}",
                        export, module.name
                    )));
                }
                let full_name = format!("{}.{}", module.name, export);
                let function_index = function_names.get(full_name.as_str()).copied().ok_or_else(|| {
                    TinyOneError::compile(format!(
                        "Verifier: module {:?} exports missing function {:?}",
                        module.name, export
                    ))
                })?;
                exported_functions.insert(function_index);
            }

            seen_exports.clear();
            for export in &module.exported_structs {
                verify_identifier("module struct export", export)?;
                if !seen_exports.insert(export.as_str()) {
                    return Err(TinyOneError::compile(format!(
                        "Verifier: duplicate struct export {:?} in module {:?}",
                        export, module.name
                    )));
                }
                let full_name = format!("{}.{}", module.name, export);
                let struct_index = struct_names.get(full_name.as_str()).copied().ok_or_else(|| {
                    TinyOneError::compile(format!(
                        "Verifier: module {:?} exports missing struct {:?}",
                        module.name, export
                    ))
                })?;
                exported_structs.insert(struct_index);
            }
        }
        reject_module_cycles(&program.modules, &imports)?;

        Ok(Self {
            function_owners,
            struct_owners,
            enum_variant_owners,
            exported_functions,
            exported_structs,
            imports,
        })
    }

    fn caller_owner(&self, caller_function: Option<usize>) -> Option<usize> {
        caller_function.and_then(|index| self.function_owners.get(index).copied().flatten())
    }

    fn can_access(&self, caller_function: Option<usize>, target_owner: Option<usize>, exported: bool) -> bool {
        let caller_owner = self.caller_owner(caller_function);
        match (caller_owner, target_owner) {
            (None, None) => true,
            (None, Some(_)) => exported,
            (Some(_), None) => false,
            (Some(caller), Some(target)) if caller == target => true,
            (Some(caller), Some(target)) => exported && self.imports[caller].contains(&target),
        }
    }

    fn can_call(&self, caller_function: Option<usize>, target: usize) -> bool {
        self.can_access(caller_function, self.function_owners[target], self.exported_functions.contains(&target))
    }

    fn can_construct(&self, caller_function: Option<usize>, target: usize) -> bool {
        self.can_access(caller_function, self.struct_owners[target], self.exported_structs.contains(&target))
    }

    fn can_construct_enum(&self, caller_function: Option<usize>, target: usize) -> bool {
        let target_owner = self.enum_variant_owners[target];
        let caller_owner = self.caller_owner(caller_function);
        (target_owner.is_none() && caller_owner.is_none()) || target_owner == caller_owner
    }
}

impl BytecodeVerifier {
    /// Verifies that `program` is a valid, safely executable bytecode program.
    ///
    /// # Errors
    ///
    /// Returns an error if the program violates any safety or structural
    /// invariant: a verification budget is exceeded, the module dependency
    /// graph cannot be built, or any chunk fails its static checks (e.g.
    /// invalid opcodes, stack/slot misuse, or capability ownership violations).
    pub fn verify(program: &Program) -> Result<()> {
        Self::verify_program_budget(program)?;
        let modules = ModuleGraph::build(program)?;
        let context = VerificationContext {
            functions: &program.functions,
            strings: &program.strings,
            structs: &program.structs,
            fields: &program.fields,
            enum_variants: &program.enum_variants,
            global_slot_count: program.slot_count,
            modules,
        };
        Self::verify_chunk("main", &program.code, program.slot_count, &context, Op::Halt, None)?;
        for (index, function) in program.functions.iter().enumerate() {
            Self::verify_chunk(
                &format!("function {:?} (index {index})", function.name),
                &function.code,
                function.slot_count,
                &context,
                Op::Return,
                Some(index),
            )?;
        }
        Ok(())
    }

    fn verify_program_budget(program: &Program) -> Result<()> {
        reject_over_limit("slot_count", program.slot_count, MAX_VERIFIER_SLOT_COUNT)?;
        verify_string_list("names", &program.names, MAX_VERIFIER_NAMES)?;
        verify_string_list("strings", &program.strings, MAX_VERIFIER_STRINGS)?;
        verify_string_list("fields", &program.fields, MAX_VERIFIER_FIELDS)?;
        reject_over_limit("struct count", program.structs.len(), MAX_VERIFIER_STRUCTS)?;
        reject_over_limit("module count", program.modules.len(), MAX_VERIFIER_MODULES)?;
        if program.functions.len() > MAX_VERIFIER_FUNCTIONS {
            return Err(TinyOneError::compile(format!(
                "Verifier: function count {} exceeds limit {MAX_VERIFIER_FUNCTIONS}",
                program.functions.len()
            )));
        }
        let mut total_ops = program.code.len();
        for function in &program.functions {
            reject_over_limit(
                &format!("function {:?} name bytes", function.name),
                function.name.len(),
                MAX_VERIFIER_TEXT_BYTES,
            )?;
            verify_string_list(
                &format!("function {:?} generic parameters", function.name),
                &function.generic_params,
                MAX_VERIFIER_NAMES,
            )?;
            reject_over_limit(
                &format!("function {:?} slot_count", function.name),
                function.slot_count,
                MAX_VERIFIER_SLOT_COUNT,
            )?;
            verify_string_list(&format!("function {:?} names", function.name), &function.names, MAX_VERIFIER_NAMES)?;
            if function.param_count > function.slot_count {
                return Err(TinyOneError::compile(format!(
                    "Verifier: function {:?} has {} parameter(s) but only {} slot(s)",
                    function.name, function.param_count, function.slot_count
                )));
            }
            total_ops = total_ops
                .checked_add(function.code.len())
                .ok_or_else(|| TinyOneError::compile("Verifier: total instruction count overflow"))?;
        }
        for item in &program.structs {
            reject_over_limit(&format!("struct {:?} name bytes", item.name), item.name.len(), MAX_VERIFIER_TEXT_BYTES)?;
            verify_string_list(&format!("struct {:?} fields", item.name), &item.fields, MAX_VERIFIER_STRUCT_FIELDS)?;
        }
        reject_over_limit("enum variant count", program.enum_variants.len(), MAX_VERIFIER_ENUM_VARIANTS)?;
        for item in &program.enum_variants {
            reject_over_limit(
                &format!("enum {:?} name bytes", item.enum_name),
                item.enum_name.len(),
                MAX_VERIFIER_TEXT_BYTES,
            )?;
            reject_over_limit(
                &format!("enum variant {:?} name bytes", item.variant_name),
                item.variant_name.len(),
                MAX_VERIFIER_TEXT_BYTES,
            )?;
            verify_string_list(
                &format!("enum {:?} variant {:?} fields", item.enum_name, item.variant_name),
                &item.fields,
                MAX_VERIFIER_ENUM_VARIANT_FIELDS,
            )?;
        }
        for module in &program.modules {
            reject_over_limit(
                &format!("module {:?} name bytes", module.name),
                module.name.len(),
                MAX_VERIFIER_TEXT_BYTES,
            )?;
            reject_over_limit(
                &format!("module {:?} path bytes", module.name),
                module.path.len(),
                MAX_VERIFIER_TEXT_BYTES,
            )?;
            reject_over_limit(
                &format!("module {:?} imports", module.name),
                module.imports.len(),
                MAX_VERIFIER_MODULE_IMPORTS,
            )?;
            for import in &module.imports {
                for (label, value) in [
                    ("alias", &import.alias),
                    ("path", &import.path),
                    ("module", &import.module),
                    ("resolved module", &import.resolved),
                ] {
                    reject_over_limit(
                        &format!("module {:?} import {label} bytes", module.name),
                        value.len(),
                        MAX_VERIFIER_TEXT_BYTES,
                    )?;
                }
            }
            verify_string_list(
                &format!("module {:?} function exports", module.name),
                &module.exported_functions,
                MAX_VERIFIER_MODULE_EXPORTS,
            )?;
            verify_string_list(
                &format!("module {:?} struct exports", module.name),
                &module.exported_structs,
                MAX_VERIFIER_MODULE_EXPORTS,
            )?;
        }
        if total_ops > MAX_VERIFIER_TOTAL_OPS {
            return Err(TinyOneError::compile(format!(
                "Verifier: total instruction count {total_ops} exceeds limit {MAX_VERIFIER_TOTAL_OPS}"
            )));
        }
        Ok(())
    }

    fn verify_chunk(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        context: &VerificationContext<'_>,
        final_op: Op,
        caller_function: Option<usize>,
    ) -> Result<()> {
        if code.last().map(|instr| instr.op) != Some(final_op) {
            let got = code.last().map_or("nothing", |instr| instr.op.name());
            return Err(TinyOneError::compile(format!(
                "Verifier: {chunk_name} must end with {}, got {got}",
                final_op.name()
            )));
        }
        for (pc, instr) in code.iter().copied().enumerate() {
            Self::verify_instruction_operands(chunk_name, code, slot_count, context, caller_function, pc, instr)?;
        }
        let mut seen: HashMap<usize, i64> = HashMap::new();
        let mut todo = Vec::new();
        visit(&mut seen, &mut todo, code, 0, 0, 0, chunk_name)?;
        let mut steps: usize = 0;
        while let Some((pc, depth)) = todo.pop() {
            steps += 1;
            if steps > MAX_VERIFIER_STEPS {
                return Err(TinyOneError::compile(format!(
                    "Verifier: {chunk_name} exceeded step limit ({MAX_VERIFIER_STEPS})"
                )));
            }
            let instr = code.get(pc).copied().ok_or_else(|| {
                TinyOneError::compile(format!("Verifier: internal invalid instruction {pc} in {chunk_name}"))
            })?;
            let op = instr.op;
            let arg = instr.arg;
            let arg2 = instr.arg2;
            if matches!(op, Op::Load | Op::Store) && checked_index(arg, slot_count).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::LoadGlobal && checked_index(arg, context.global_slot_count).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid global slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::PushString && checked_index(arg, context.strings.len()).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::PushFunction && checked_index(arg, context.functions.len()).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if matches!(op, Op::GetField | Op::SetField) && checked_index(arg, context.fields.len()).is_err() {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            match op {
                Op::Jump => visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?,
                Op::JumpIfZero => {
                    let depth = next_depth(pc, depth, -1, chunk_name)?;
                    visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?;
                    visit(&mut seen, &mut todo, code, next_pc(pc)?, depth, pc, chunk_name)?;
                }
                Op::Call => {
                    let function_index = checked_index(arg, context.functions.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let arg_count = usize::try_from(arg2).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid function arity {arg2} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let function = &context.functions[function_index];
                    if arg_count != function.param_count {
                        return Err(TinyOneError::compile(format!(
                            "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            function.name, function.param_count
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::CallValue => {
                    let arg_count = usize::try_from(arg).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid call arity {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    // The callee occupies one stack slot in addition to the
                    // arguments; the call result replaces the whole group.
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(
                            pc,
                            depth,
                            i64::try_from(arg_count + 1)
                                .map_err(|_| TinyOneError::compile("Verifier: call arity overflow"))?,
                            chunk_name,
                        )?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeArray => {
                    if arg < 0 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: negative array arity {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeStruct => {
                    let struct_index = checked_index(arg, context.structs.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let field_count = usize::try_from(arg2).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid struct arity {arg2} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let struct_def = &context.structs[struct_index];
                    let expected = struct_def.fields.len();
                    if field_count != expected {
                        return Err(TinyOneError::compile(format!(
                            "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                            struct_def.name
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeEnum => {
                    let variant_index = checked_index(arg, context.enum_variants.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid enum variant index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let field_count = usize::try_from(arg2).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid enum variant arity {arg2} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let variant_def = &context.enum_variants[variant_index];
                    let expected = variant_def.fields.len();
                    if field_count != expected {
                        return Err(TinyOneError::compile(format!(
                            "Enum variant {:?}.{:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                            variant_def.enum_name, variant_def.variant_name
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Builtin => {
                    let builtin_index = checked_index(arg, BUILTINS.len()).map_err(|_| {
                        TinyOneError::compile(format!(
                            "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                        ))
                    })?;
                    let builtin = BUILTINS[builtin_index];
                    if arg2 < builtin.min_args as i64 || arg2 > builtin.max_args as i64 {
                        return Err(TinyOneError::compile(format!(
                            "Builtin {:?} expects {}..{} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            builtin.name, builtin.min_args, builtin.max_args
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth_after_popping_to_one(pc, depth, arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Return => {
                    if depth != 1 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: RETURN in {chunk_name} requires one value, got {depth}"
                        )));
                    }
                }
                Op::Halt => {
                    if depth != 0 {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: HALT in {chunk_name} requires empty stack, got {depth}"
                        )));
                    }
                }
                _ => {
                    let effect = stack_effect(op).ok_or_else(|| {
                        TinyOneError::compile(format!("Verifier: unknown opcode {op:?} at index {pc} in {chunk_name}"))
                    })?;
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        next_pc(pc)?,
                        next_depth(pc, depth, effect, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn verify_instruction_operands(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        context: &VerificationContext<'_>,
        caller_function: Option<usize>,
        pc: usize,
        instr: Instr,
    ) -> Result<()> {
        let op = instr.op;
        let arg = instr.arg;
        let arg2 = instr.arg2;
        if matches!(op, Op::Load | Op::Store) && checked_index(arg, slot_count).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::LoadGlobal && checked_index(arg, context.global_slot_count).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid global slot {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::LoadGlobal && context.modules.caller_owner(caller_function).is_some() {
            return Err(TinyOneError::compile(format!(
                "Verifier: module function in {chunk_name} cannot access root global slots"
            )));
        }
        if op == Op::PushString && checked_index(arg, context.strings.len()).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::PushFunction {
            let function_index = checked_index(arg, context.functions.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            if !context.modules.can_call(caller_function, function_index) {
                return Err(module_access_error("function", &context.functions[function_index].name, chunk_name, pc));
            }
        }
        if op == Op::CallValue && arg < 0 {
            return Err(TinyOneError::compile(format!(
                "Verifier: negative call arity {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if matches!(op, Op::GetField | Op::SetField) && checked_index(arg, context.fields.len()).is_err() {
            return Err(TinyOneError::compile(format!(
                "Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if matches!(op, Op::Jump | Op::JumpIfZero) && checked_index(arg, code.len()).is_err() {
            return Err(TinyOneError::compile(format!("Verifier: instruction {pc} in {chunk_name} targets {arg}")));
        }
        if op == Op::Call {
            let function_index = checked_index(arg, context.functions.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let arg_count = usize::try_from(arg2).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid function arity {arg2} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let function = &context.functions[function_index];
            if !context.modules.can_call(caller_function, function_index) {
                return Err(module_access_error("function", &function.name, chunk_name, pc));
            }
            if arg_count != function.param_count {
                return Err(TinyOneError::compile(format!(
                    "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                    function.name, function.param_count
                )));
            }
        }
        if op == Op::MakeArray && arg < 0 {
            return Err(TinyOneError::compile(format!(
                "Verifier: negative array arity {arg} at instruction {pc} in {chunk_name}"
            )));
        }
        if op == Op::MakeStruct {
            let struct_index = checked_index(arg, context.structs.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let field_count = usize::try_from(arg2).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid struct arity {arg2} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let struct_def = &context.structs[struct_index];
            if !context.modules.can_construct(caller_function, struct_index) {
                return Err(module_access_error("struct", &struct_def.name, chunk_name, pc));
            }
            let expected = struct_def.fields.len();
            if field_count != expected {
                return Err(TinyOneError::compile(format!(
                    "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                    struct_def.name
                )));
            }
        }
        if op == Op::MakeEnum {
            let variant_index = checked_index(arg, context.enum_variants.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid enum variant index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            if !context.modules.can_construct_enum(caller_function, variant_index) {
                let item = &context.enum_variants[variant_index];
                return Err(module_access_error("enum", &item.enum_name, chunk_name, pc));
            }
        }
        if op == Op::Builtin {
            let builtin_index = checked_index(arg, BUILTINS.len()).map_err(|_| {
                TinyOneError::compile(format!(
                    "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                ))
            })?;
            let builtin = BUILTINS[builtin_index];
            if arg2 < builtin.min_args as i64 || arg2 > builtin.max_args as i64 {
                return Err(TinyOneError::compile(format!(
                    "Builtin {:?} expects {}..{} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                    builtin.name, builtin.min_args, builtin.max_args
                )));
            }
        }
        Ok(())
    }
}

fn verify_identifier(kind: &str, value: &str) -> Result<()> {
    reject_over_limit(&format!("{kind} text bytes"), value.len(), MAX_VERIFIER_TEXT_BYTES)?;
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(TinyOneError::compile(format!("Verifier: {kind} must not be empty")));
    };
    if first.is_ascii_digit()
        || !(first == '_' || first.is_alphanumeric())
        || chars.any(|ch| ch != '_' && !ch.is_alphanumeric())
    {
        return Err(TinyOneError::compile(format!("Verifier: invalid {kind} {value:?}")));
    }
    Ok(())
}

fn unique_named_indexes<'a>(kind: &str, names: impl IntoIterator<Item = &'a str>) -> Result<HashMap<&'a str, usize>> {
    let mut indexes = HashMap::new();
    for (index, name) in names.into_iter().enumerate() {
        reject_over_limit(&format!("{kind} name bytes"), name.len(), MAX_VERIFIER_TEXT_BYTES)?;
        if indexes.insert(name, index).is_some() {
            return Err(TinyOneError::compile(format!("Verifier: duplicate {kind} name {name:?}")));
        }
    }
    Ok(indexes)
}

fn owner_for_name(kind: &str, name: &str, modules_by_name: &HashMap<&str, usize>) -> Result<Option<usize>> {
    let Some((module_name, local_name)) = name.split_once('.') else {
        verify_identifier(&format!("{kind} name"), name)?;
        return Ok(None);
    };
    if local_name.contains('.') {
        return Err(TinyOneError::compile(format!("Verifier: invalid qualified {kind} name {name:?}")));
    }
    verify_identifier(&format!("{kind} module qualifier"), module_name)?;
    verify_identifier(&format!("{kind} local name"), local_name)?;
    modules_by_name.get(module_name).copied().map(Some).ok_or_else(|| {
        TinyOneError::compile(format!("Verifier: {kind} {name:?} belongs to unknown module {module_name:?}"))
    })
}

fn reject_module_cycles(modules: &[crate::ModuleDef], imports: &[HashSet<usize>]) -> Result<()> {
    fn visit_module(
        module_index: usize,
        modules: &[crate::ModuleDef],
        imports: &[HashSet<usize>],
        states: &mut [u8],
    ) -> Result<()> {
        match states[module_index] {
            2 => return Ok(()),
            1 => {
                return Err(TinyOneError::compile(format!(
                    "Verifier: cyclic module dependency involving {:?}",
                    modules[module_index].name
                )));
            }
            _ => {}
        }
        states[module_index] = 1;
        for dependency in &imports[module_index] {
            visit_module(*dependency, modules, imports, states)?;
        }
        states[module_index] = 2;
        Ok(())
    }

    let mut states = vec![0; modules.len()];
    for module_index in 0..modules.len() {
        visit_module(module_index, modules, imports, &mut states)?;
    }
    Ok(())
}

fn module_access_error(target_kind: &str, target_name: &str, chunk_name: &str, pc: usize) -> TinyOneError {
    TinyOneError::compile(format!(
        "Verifier: {target_kind} {target_name:?} is not visible from {chunk_name} at instruction {pc}"
    ))
}

fn visit(
    seen: &mut HashMap<usize, i64>,
    todo: &mut Vec<(usize, i64)>,
    code: &[Instr],
    pc: i64,
    depth: i64,
    origin: usize,
    chunk_name: &str,
) -> Result<()> {
    let Ok(pc_usize) = usize::try_from(pc) else {
        return Err(TinyOneError::compile(format!("Verifier: instruction {origin} in {chunk_name} targets {pc}")));
    };
    if pc_usize >= code.len() {
        return Err(TinyOneError::compile(format!("Verifier: instruction {origin} in {chunk_name} targets {pc}")));
    }
    let pc = pc_usize;
    if let Some(old_depth) = seen.get(&pc) {
        if *old_depth != depth {
            return Err(TinyOneError::compile(format!(
                "Verifier: stack depth mismatch at instruction {pc} in {chunk_name}: {old_depth} vs {depth}"
            )));
        }
        return Ok(());
    }
    seen.insert(pc, depth);
    todo.push((pc, depth));
    Ok(())
}

fn next_depth(pc: usize, depth: i64, delta: i64, chunk_name: &str) -> Result<i64> {
    let depth = depth.checked_add(delta).ok_or_else(|| {
        TinyOneError::compile(format!("Verifier: stack depth overflow at instruction {pc} in {chunk_name}"))
    })?;
    if depth < 0 {
        return Err(TinyOneError::compile(format!("Verifier: stack underflow at instruction {pc} in {chunk_name}")));
    }
    if depth > MAX_STACK_DEPTH {
        return Err(TinyOneError::compile(format!(
            "Verifier: stack depth {depth} exceeds limit in {chunk_name} at instruction {pc}"
        )));
    }
    Ok(depth)
}

fn next_depth_after_popping_to_one(pc: usize, depth: i64, count: i64, chunk_name: &str) -> Result<i64> {
    let delta = 1i64.checked_sub(count).ok_or_else(|| {
        TinyOneError::compile(format!("Verifier: stack effect overflow at instruction {pc} in {chunk_name}"))
    })?;
    next_depth(pc, depth, delta, chunk_name)
}

fn next_pc(pc: usize) -> Result<i64> {
    let next = pc
        .checked_add(1)
        .ok_or_else(|| TinyOneError::compile("Verifier: instruction index overflow"))?;
    i64::try_from(next).map_err(|_| TinyOneError::compile("Verifier: instruction index too large"))
}

fn checked_index(index: i64, len: usize) -> Result<usize> {
    if index < 0 {
        return Err(TinyOneError::compile("negative index"));
    }
    let index = usize::try_from(index).map_err(|_| TinyOneError::compile("index is too large for this platform"))?;
    if index >= len {
        return Err(TinyOneError::compile("index out of bounds"));
    }
    Ok(index)
}

fn reject_over_limit(name: &str, got: usize, max: usize) -> Result<()> {
    if got > max {
        return Err(TinyOneError::compile(format!("Verifier: {name} {got} exceeds limit {max}")));
    }
    Ok(())
}

fn verify_string_list(name: &str, values: &[String], max_count: usize) -> Result<()> {
    reject_over_limit(name, values.len(), max_count)?;
    let mut bytes = 0usize;
    for value in values {
        bytes = bytes
            .checked_add(value.len())
            .ok_or_else(|| TinyOneError::compile(format!("Verifier: {name} text overflow")))?;
        reject_over_limit(&format!("{name} text bytes"), bytes, MAX_VERIFIER_TEXT_BYTES)?;
    }
    Ok(())
}

fn stack_effect(op: Op) -> Option<i64> {
    Some(match op {
        Op::PushInt
        | Op::PushString
        | Op::PushNull
        | Op::PushBool
        | Op::PushFloat
        | Op::Load
        | Op::LoadGlobal
        | Op::PushFunction => 1,
        Op::Store | Op::Pop => -1,
        Op::Add | Op::Sub | Op::Mul | Op::Div | Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne | Op::Index => -1,
        Op::Neg | Op::GetField => 0,
        Op::Print => -1,
        Op::SetIndex => -3,
        Op::SetField => -2,
        _ => return None,
    })
}
