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
];

pub(crate) fn builtin_index(name: &str) -> Option<usize> {
    BUILTINS.iter().position(|item| item.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_indexes_remain_stable() {
        let names = BUILTINS.iter().map(|item| item.name).collect::<Vec<_>>();
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
}
