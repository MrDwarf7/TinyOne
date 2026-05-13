#[derive(Debug, Clone, Copy)]
pub(crate) struct BuiltinDef {
    pub(crate) name: &'static str,
    pub(crate) min_args: usize,
    pub(crate) max_args: usize,
    pub(crate) requires_unsafe: bool,
}

pub(crate) const BUILTINS: &[BuiltinDef] = &[
    BuiltinDef {
        name: "len",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "array",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "alloc",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "load",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "store",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "free",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "read",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "read_int",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "read_str",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "to_int",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr",
        min_args: 1,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "fieldptr",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_addr",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_at",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "ptr_add",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "ptr_load",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "ptr_store",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "ptr_type",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "buffer",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "is_null",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_eq",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_ne",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_base",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_offset",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_kind",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "ptr_field",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "read8",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "write8",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "read16",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "write16",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "read32",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "write32",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "cast_ptr",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "push",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "pop",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    // -------- Phase 2 stdlib bridge builtins --------
    // Step 1: Vec / Map
    BuiltinDef {
        name: "vec_new",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "vec_clear",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_new",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_set",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_get",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_has",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_del",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_len",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_keys",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "map_values",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    // Step 2: I/O
    BuiltinDef {
        name: "io_stdout",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_stderr",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_stdin",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_write",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_writeln",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_read_line",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_flush",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_capture_stdout",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "io_capture_stderr",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    // Step 3: String / Unicode
    BuiltinDef {
        name: "str_byte_len",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_char_len",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_byte_at",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_char_at",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_slice",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_concat",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_is_utf8",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "str_from_buffer",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    // Step 4: Threading / Sync
    BuiltinDef {
        name: "mutex_new",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "mutex_lock",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "mutex_unlock",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "atomic_new",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "atomic_load",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "atomic_store",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "atomic_add",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    // Step 5: Result / Option
    BuiltinDef {
        name: "result_ok",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "result_err",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "result_is_ok",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "result_is_err",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "result_unwrap",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "result_unwrap_err",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "option_some",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "option_none",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "option_is_some",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "option_is_none",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "option_unwrap",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    // Step 6: System introspection
    BuiltinDef {
        name: "sys_argc",
        min_args: 0,
        max_args: 0,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "sys_argv",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "sys_env_has",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "sys_env_get",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    // Step 7: Path / FS  (FS ops are unsafe per phase_2.md rule of thumb)
    BuiltinDef {
        name: "path_join",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "path_basename",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "path_dirname",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "fs_read",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "fs_write",
        min_args: 2,
        max_args: 2,
        requires_unsafe: true,
    },
    BuiltinDef {
        name: "fs_exists",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "fs_list_dir",
        min_args: 1,
        max_args: 1,
        requires_unsafe: true,
    },
    // Step 8: Math / Logic
    BuiltinDef {
        name: "math_const",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "math_abs",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "math_min",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "math_max",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "logic_and",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "logic_or",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "logic_not",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "logic_xor",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    // Typing system
    BuiltinDef {
        name: "type_of",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "type_id",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "smallest_fit",
        min_args: 1,
        max_args: 1,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "promote",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "check_int_range",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "typed_add",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "typed_sub",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "typed_mul",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "typed_div",
        min_args: 3,
        max_args: 3,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "typed_neg",
        min_args: 2,
        max_args: 2,
        requires_unsafe: false,
    },
    BuiltinDef {
        name: "assert",
        min_args: 1,
        max_args: 2,
        requires_unsafe: false,
    },
];

pub(crate) fn builtin_index(name: &str) -> Option<usize> {
    BUILTINS.iter().position(|item| item.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first 35 builtin slots are bytecode-stable: artifacts emitted in
    /// Phase 1 keep working. Any reordering of those slots is a breaking
    /// change. Stdlib bridge builtins (Phase 2) are appended after slot 34
    /// and must not change order without coordinated Python updates.
    #[test]
    fn builtin_phase1_indexes_remain_stable() {
        let names: Vec<&str> = BUILTINS.iter().take(35).map(|item| item.name).collect();
        assert_eq!(
            names,
            vec![
                "len",
                "array",
                "alloc",
                "load",
                "store",
                "free",
                "read",
                "read_int",
                "read_str",
                "to_int",
                "ptr",
                "fieldptr",
                "ptr_addr",
                "ptr_at",
                "ptr_add",
                "ptr_load",
                "ptr_store",
                "ptr_type",
                "buffer",
                "is_null",
                "ptr_eq",
                "ptr_ne",
                "ptr_base",
                "ptr_offset",
                "ptr_kind",
                "ptr_field",
                "read8",
                "write8",
                "read16",
                "write16",
                "read32",
                "write32",
                "cast_ptr",
                "push",
                "pop",
            ]
        );
    }

    #[test]
    fn all_builtin_names_are_unique() {
        let mut names: Vec<&str> = BUILTINS.iter().map(|item| item.name).collect();
        names.sort();
        let mut dedup = names.clone();
        dedup.dedup();
        assert_eq!(
            dedup, names,
            "builtin names must be unique: duplicates would shadow earlier slots"
        );
    }
}
