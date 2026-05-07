use std::collections::HashMap;

#[derive(Debug, Default, Clone)]
pub(crate) struct SymbolTable {
    pub(crate) scopes: Vec<HashMap<String, usize>>,
    pub(crate) names: Vec<String>,
}

impl SymbolTable {
    pub(crate) fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            names: Vec::new(),
        }
    }

    pub(crate) fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn exit_scope(&mut self) {
        if self.scopes.len() <= 1 {
            panic!("cannot exit root symbol scope");
        }
        self.scopes.pop();
    }

    pub(crate) fn define_current(&mut self, name: &str) -> Option<usize> {
        if self
            .scopes
            .last()
            .is_some_and(|scope| scope.contains_key(name))
        {
            return None;
        }
        let slot = self.names.len();
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.to_string(), slot);
        self.names.push(name.to_string());
        Some(slot)
    }

    pub(crate) fn get(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains_key(name))
    }

    pub(crate) fn slot_count(&self) -> usize {
        self.names.len()
    }
}
