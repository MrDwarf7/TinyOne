use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use crate::{
    BUILTINS, Function, Instr, Lexer, ModuleDef, ModuleImportDef, ModuleInfo, Op, Program,
    Resolver, Result, SharedState, SourceMap, StructDef, SymbolTable, TinyOneError, Token,
    TokenKind, builtin_index, default_import_alias, module_name_from_import, unique_module_name,
};

#[derive(Debug)]
struct LoopContext {
    start: usize,
    breaks: Vec<usize>,
}

enum ReadSlot {
    Local(usize),
    Global(usize),
}

pub(crate) struct Compiler {
    tokens: Vec<Token>,
    index: usize,
    eof_token: Token,
    source_map: SourceMap,
    filename: String,
    resolver: Option<Resolver>,
    module_mode: bool,
    module_filename: Option<String>,
    module_qualified_name: Option<String>,
    module_imports: Vec<ModuleImportDef>,
    namespaces: HashMap<String, ModuleInfo>,
    accept_imports: bool,
    symbols: SymbolTable,
    code: Vec<Instr>,
    shared: SharedState,
    local_function_indexes: HashMap<String, usize>,
    local_struct_indexes: HashMap<String, usize>,
    function_globals: HashMap<String, usize>,
    loops: Vec<LoopContext>,
    in_function: bool,
    unsafe_depth: usize,
}

impl Compiler {
    pub(crate) fn new(
        source: impl Into<String>,
        filename: impl Into<String>,
        resolver: Option<Resolver>,
        module_mode: bool,
        module_name: impl Into<String>,
        shared: SharedState,
    ) -> Result<Self> {
        let source = source.into();
        let filename = filename.into();
        let source_len = source.len();
        let source_map = SourceMap::new(source.clone(), filename.clone());
        let tokens = Lexer::new(source, filename.clone()).tokenize()?;
        let mut module_filename = None;
        let mut module_qualified_name = None;
        if module_mode {
            let name = {
                let mut state = shared.borrow_mut();
                if let Some(info) = state.modules.get(&filename) {
                    info.name.clone()
                } else {
                    let base_name = module_name.into();
                    let name = unique_module_name(&mut state, &base_name, &filename);
                    state.modules.insert(
                        filename.clone(),
                        ModuleInfo {
                            name: name.clone(),
                            function_exports: HashMap::new(),
                            struct_exports: HashMap::new(),
                            all_functions: HashSet::new(),
                            all_structs: HashSet::new(),
                            imports: Vec::new(),
                            finalized: false,
                        },
                    );
                    name
                }
            };
            module_filename = Some(filename.clone());
            module_qualified_name = Some(name);
        }
        Ok(Self {
            tokens,
            index: 0,
            eof_token: Token {
                kind: TokenKind::Eof,
                text: String::new(),
                pos: source_len,
                end: source_len,
            },
            source_map,
            filename,
            resolver,
            module_mode,
            module_filename,
            module_qualified_name,
            module_imports: Vec::new(),
            namespaces: HashMap::new(),
            accept_imports: true,
            symbols: SymbolTable::new(),
            code: Vec::new(),
            shared,
            local_function_indexes: HashMap::new(),
            local_struct_indexes: HashMap::new(),
            function_globals: HashMap::new(),
            loops: Vec::new(),
            in_function: false,
            unsafe_depth: 0,
        })
    }

    pub(crate) fn compile(&mut self) -> Result<Program> {
        while self.current().kind != TokenKind::Eof {
            match self.current().kind {
                TokenKind::Import => self.import_statement()?,
                TokenKind::Export => {
                    self.accept_imports = false;
                    self.export_declaration()?;
                }
                TokenKind::Struct => {
                    self.accept_imports = false;
                    self.struct_definition(false)?;
                }
                TokenKind::Fn => {
                    self.accept_imports = false;
                    self.function_definition(false)?;
                }
                _ => {
                    if self.module_mode {
                        return Err(self.error(
                            "Imported modules may only contain import, struct, and fn declarations",
                            self.current().clone(),
                        ));
                    }
                    self.accept_imports = false;
                    self.statement()?;
                }
            }
        }
        self.emit(Op::Halt, 0, 0);
        self.finalize_module()?;
        let state = self.shared.borrow();
        Ok(Program {
            code: self.code.clone(),
            slot_count: self.symbols.slot_count(),
            names: self.symbols.names.clone(),
            functions: state.functions.clone(),
            strings: state.strings.clone(),
            structs: state.structs.clone(),
            fields: state.fields.clone(),
            modules: state.module_defs.clone(),
        })
    }

    fn export_declaration(&mut self) -> Result<()> {
        let token = self.eat(TokenKind::Export)?;
        match self.current().kind {
            TokenKind::Struct => self.struct_definition(true),
            TokenKind::Fn => self.function_definition(true),
            _ => Err(self.error(
                "Expected function or struct declaration after export",
                token,
            )),
        }
    }

    fn finalize_module(&mut self) -> Result<()> {
        let Some(filename) = self.module_filename.clone() else {
            return Ok(());
        };
        let mut state = self.shared.borrow_mut();
        let def = {
            let Some(info) = state.modules.get_mut(&filename) else {
                return Err(TinyOneError::compile("Internal module state error"));
            };
            if info.finalized {
                return Ok(());
            }
            info.imports = self.module_imports.clone();
            let mut exported_functions = info.function_exports.keys().cloned().collect::<Vec<_>>();
            let mut exported_structs = info.struct_exports.keys().cloned().collect::<Vec<_>>();
            exported_functions.sort();
            exported_structs.sort();
            info.finalized = true;
            ModuleDef {
                name: info.name.clone(),
                path: info.name.clone(),
                imports: info.imports.clone(),
                exported_functions,
                exported_structs,
            }
        };
        state.module_defs.push(def);
        Ok(())
    }

    fn statement(&mut self) -> Result<()> {
        match self.current().kind {
            TokenKind::Let => self.let_statement(),
            TokenKind::Print => self.print_statement(),
            TokenKind::While => self.while_statement(),
            TokenKind::If => self.if_statement(),
            TokenKind::Break => self.break_statement(),
            TokenKind::Continue => self.continue_statement(),
            TokenKind::Unsafe if self.peek_kind(1) == Some(TokenKind::LBrace) => {
                self.unsafe_block_statement()
            }
            TokenKind::Ident if self.peek_kind(1) == Some(TokenKind::Equal) => {
                self.assignment_statement()
            }
            TokenKind::Return => self.return_statement(),
            TokenKind::Set => self.set_statement(),
            TokenKind::Fn => Err(self.error(
                "Function definitions are only allowed at top level and before executable statements",
                self.current().clone(),
            )),
            TokenKind::Import | TokenKind::Struct | TokenKind::Export => Err(self.error(
                "Imports, exports, and struct definitions are only allowed at top level before statements",
                self.current().clone(),
            )),
            _ if self.is_expression_start() => self.expression_statement(),
            _ => Err(self.error("Expected statement", self.current().clone())),
        }
    }

    fn expression_statement(&mut self) -> Result<()> {
        self.expression()?;
        self.emit(Op::Pop, 0, 0);
        Ok(())
    }

    fn unsafe_block_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::Unsafe)?;
        self.unsafe_depth += 1;
        let result = self.block();
        self.unsafe_depth -= 1;
        result
    }

    fn let_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::Let)?;
        let name_token = self.eat(TokenKind::Ident)?;
        if self.namespaces.contains_key(&name_token.text) {
            return Err(self.error_at(
                format!(
                    "Variable {:?} conflicts with an imported namespace",
                    name_token.text
                ),
                name_token.pos,
            ));
        }
        self.eat(TokenKind::Equal)?;
        self.expression()?;
        let slot = self
            .symbols
            .define_current(&name_token.text)
            .ok_or_else(|| {
                self.error(
                    format!(
                        "Variable {:?} is already defined in this scope",
                        name_token.text
                    ),
                    name_token.clone(),
                )
            })?;
        self.emit(Op::Store, slot as i64, 0);
        Ok(())
    }

    fn assignment_statement(&mut self) -> Result<()> {
        let name_token = self.eat(TokenKind::Ident)?;
        if self.namespaces.contains_key(&name_token.text) {
            return Err(self.error(
                format!("Cannot assign to import namespace {:?}", name_token.text),
                name_token,
            ));
        }
        let slot = self.get_assignment_slot(&name_token)?;
        self.eat(TokenKind::Equal)?;
        self.expression()?;
        self.emit(Op::Store, slot as i64, 0);
        Ok(())
    }

    fn print_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::Print)?;
        self.expression()?;
        self.emit(Op::Print, 0, 0);
        Ok(())
    }

    fn while_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::While)?;
        let loop_start = self.code.len();
        self.expression()?;
        let exit_jump = self.emit_placeholder(Op::JumpIfZero);
        self.loops.push(LoopContext {
            start: loop_start,
            breaks: Vec::new(),
        });
        let result = self.block();
        result?;
        let loop_context = self
            .loops
            .pop()
            .ok_or_else(|| TinyOneError::compile("Internal loop state error"))?;
        self.emit(Op::Jump, loop_start as i64, 0);
        let loop_end = self.code.len() as i64;
        self.patch(exit_jump, loop_end)?;
        for break_jump in loop_context.breaks {
            self.patch(break_jump, loop_end)?;
        }
        Ok(())
    }

    fn if_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::If)?;
        self.expression()?;
        let false_jump = self.emit_placeholder(Op::JumpIfZero);
        self.block()?;
        if self.current().kind == TokenKind::Else {
            let end_jump = self.emit_placeholder(Op::Jump);
            self.patch(false_jump, self.code.len() as i64)?;
            self.eat(TokenKind::Else)?;
            if self.current().kind == TokenKind::If {
                self.if_statement()?;
            } else {
                self.block()?;
            }
            self.patch(end_jump, self.code.len() as i64)?;
        } else {
            self.patch(false_jump, self.code.len() as i64)?;
        }
        Ok(())
    }

    fn break_statement(&mut self) -> Result<()> {
        let token = self.eat(TokenKind::Break)?;
        if self.loops.is_empty() {
            return Err(self.error("Break outside loop", token));
        }
        let break_jump = self.emit_placeholder(Op::Jump);
        let Some(loop_context) = self.loops.last_mut() else {
            return Err(TinyOneError::compile("Internal loop state error"));
        };
        loop_context.breaks.push(break_jump);
        Ok(())
    }

    fn continue_statement(&mut self) -> Result<()> {
        let token = self.eat(TokenKind::Continue)?;
        let loop_start = self
            .loops
            .last()
            .map(|item| item.start)
            .ok_or_else(|| self.error("Continue outside loop", token))?;
        self.emit(Op::Jump, loop_start as i64, 0);
        Ok(())
    }

    fn return_statement(&mut self) -> Result<()> {
        if !self.in_function {
            return Err(self.error("Return outside function", self.current().clone()));
        }
        self.eat(TokenKind::Return)?;
        self.expression()?;
        self.emit(Op::Return, 0, 0);
        Ok(())
    }

    fn set_statement(&mut self) -> Result<()> {
        self.eat(TokenKind::Set)?;
        let name_token = self.eat(TokenKind::Ident)?;
        self.emit_load_name(&name_token)?;

        if self.current().kind == TokenKind::LBracket {
            self.eat(TokenKind::LBracket)?;
            self.expression()?;
            self.eat(TokenKind::RBracket)?;
            self.eat(TokenKind::Equal)?;
            self.expression()?;
            self.emit(Op::SetIndex, 0, 0);
            return Ok(());
        }

        if self.current().kind == TokenKind::Dot {
            self.eat(TokenKind::Dot)?;
            let field = self.eat(TokenKind::Ident)?.text;
            let field_index = self.intern_field(&field);
            self.eat(TokenKind::Equal)?;
            self.expression()?;
            self.emit(Op::SetField, field_index as i64, 0);
            return Ok(());
        }

        Err(self.error(
            "Expected indexed or field assignment target after set",
            self.current().clone(),
        ))
    }

    fn import_statement(&mut self) -> Result<()> {
        let token = self.current().clone();
        if !self.accept_imports {
            return Err(self.error(
                "Imports must appear before declarations and statements",
                token,
            ));
        }
        self.eat(TokenKind::Import)?;
        let path_token = self.eat(TokenKind::String)?;
        let (alias, alias_pos) = if self.current().kind == TokenKind::As {
            self.eat(TokenKind::As)?;
            let alias_token = self.eat(TokenKind::Ident)?;
            (alias_token.text.clone(), alias_token.pos)
        } else {
            (default_import_alias(&path_token.text), path_token.pos)
        };
        let resolver = self.resolver.ok_or_else(|| {
            self.error(
                "Imports require compiling from a source file",
                path_token.clone(),
            )
        })?;
        let (module_filename, module_source) = resolver(&self.filename, &path_token.text)?;
        if self.namespaces.contains_key(&alias) || self.symbols.contains(&alias) {
            return Err(self.error_at(
                format!("Import namespace {alias:?} is already defined"),
                alias_pos,
            ));
        }
        if builtin_index(&alias).is_some() {
            return Err(self.error_at(
                format!("Import namespace {alias:?} conflicts with a builtin"),
                alias_pos,
            ));
        }

        let needs_load = {
            let state = self.shared.borrow();
            state
                .modules
                .get(&module_filename)
                .map(|info| !info.finalized)
                .unwrap_or(true)
        };
        if needs_load {
            {
                let mut state = self.shared.borrow_mut();
                if state.loading_modules.contains(&module_filename) {
                    return Err(self.error_at(
                        format!("Import cycle involving {module_filename}"),
                        path_token.pos,
                    ));
                }
                state.loading_modules.insert(module_filename.clone());
            }
            let compile_result = (|| {
                let mut compiler = Compiler::new(
                    module_source,
                    module_filename.clone(),
                    self.resolver,
                    true,
                    module_name_from_import(&path_token.text, &module_filename),
                    Rc::clone(&self.shared),
                )?;
                compiler.compile().map(|_| ())
            })();
            self.shared
                .borrow_mut()
                .loading_modules
                .remove(&module_filename);
            compile_result?;
        }

        let info = self
            .shared
            .borrow()
            .modules
            .get(&module_filename)
            .cloned()
            .ok_or_else(|| TinyOneError::compile("Internal import state error"))?;
        self.namespaces.insert(alias.clone(), info.clone());
        self.module_imports.push(ModuleImportDef {
            alias,
            path: path_token.text,
            module: info.name.clone(),
            resolved: info.name,
        });
        Ok(())
    }

    fn struct_definition(&mut self, exported: bool) -> Result<()> {
        self.eat(TokenKind::Struct)?;
        let name_token = self.eat(TokenKind::Ident)?;
        let name = name_token.text.clone();
        if self.namespaces.contains_key(&name) {
            return Err(self.error(
                format!("Struct {name:?} conflicts with an imported namespace"),
                name_token,
            ));
        }
        if self.local_struct_indexes.contains_key(&name) {
            return Err(self.error(format!("Struct {name:?} is already defined"), name_token));
        }
        if self.local_function_indexes.contains_key(&name) || builtin_index(&name).is_some() {
            return Err(self.error(
                format!("Struct {name:?} conflicts with an existing callable"),
                name_token,
            ));
        }
        let mut fields = Vec::new();
        let mut seen = HashSet::new();
        self.eat(TokenKind::LBrace)?;
        if self.current().kind != TokenKind::RBrace {
            loop {
                let field_token = self.eat(TokenKind::Ident)?;
                if !seen.insert(field_token.text.clone()) {
                    return Err(self.error(
                        format!("Duplicate struct field {:?}", field_token.text),
                        field_token,
                    ));
                }
                self.intern_field(&field_token.text);
                fields.push(field_token.text);
                if self.current().kind != TokenKind::Comma {
                    break;
                }
                self.eat(TokenKind::Comma)?;
            }
        }
        self.eat(TokenKind::RBrace)?;

        let full_name = self.qualified_declaration_name(&name);
        let struct_index = {
            let mut state = self.shared.borrow_mut();
            if state.struct_indexes.contains_key(&full_name) {
                return Err(self.error(
                    format!("Struct {full_name:?} is already defined"),
                    Token {
                        kind: TokenKind::Ident,
                        text: name,
                        pos: 0,
                        end: 0,
                    },
                ));
            }
            let index = state.structs.len();
            state.struct_indexes.insert(full_name.clone(), index);
            state.structs.push(StructDef {
                name: full_name,
                fields,
            });
            index
        };
        self.local_struct_indexes.insert(name.clone(), struct_index);
        if let Some(filename) = &self.module_filename {
            let mut state = self.shared.borrow_mut();
            let Some(info) = state.modules.get_mut(filename) else {
                return Err(TinyOneError::compile("Internal module state error"));
            };
            info.all_structs.insert(name.clone());
            if exported {
                info.struct_exports.insert(name, struct_index);
            }
        }
        Ok(())
    }

    fn function_definition(&mut self, exported: bool) -> Result<()> {
        self.eat(TokenKind::Fn)?;
        let name_token = self.eat(TokenKind::Ident)?;
        let name = name_token.text.clone();
        if self.namespaces.contains_key(&name) {
            return Err(self.error(
                format!("Function {name:?} conflicts with an imported namespace"),
                name_token,
            ));
        }
        if self.local_function_indexes.contains_key(&name) {
            return Err(self.error(format!("Function {name:?} is already defined"), name_token));
        }
        if self.local_struct_indexes.contains_key(&name) || builtin_index(&name).is_some() {
            return Err(self.error(
                format!("Function {name:?} conflicts with an existing callable"),
                name_token,
            ));
        }
        let full_name = self.qualified_declaration_name(&name);
        let function_index = {
            let mut state = self.shared.borrow_mut();
            if state.function_indexes.contains_key(&full_name) {
                return Err(self.error(
                    format!("Function {full_name:?} is already defined"),
                    Token {
                        kind: TokenKind::Ident,
                        text: name.clone(),
                        pos: 0,
                        end: 0,
                    },
                ));
            }
            let index = state.functions.len();
            state.function_indexes.insert(full_name.clone(), index);
            index
        };
        self.local_function_indexes
            .insert(name.clone(), function_index);
        if let Some(filename) = &self.module_filename {
            let mut state = self.shared.borrow_mut();
            let Some(info) = state.modules.get_mut(filename) else {
                return Err(TinyOneError::compile("Internal module state error"));
            };
            info.all_functions.insert(name.clone());
            if exported {
                info.function_exports.insert(name.clone(), function_index);
            }
        }

        let mut function_symbols = SymbolTable::new();
        self.eat(TokenKind::LParen)?;
        let mut param_count = 0usize;
        if self.current().kind != TokenKind::RParen {
            loop {
                let param = self.eat(TokenKind::Ident)?;
                let Some(slot) = function_symbols.define_current(&param.text) else {
                    return Err(self.error(format!("Duplicate parameter {:?}", param.text), param));
                };
                debug_assert_eq!(slot, param_count);
                param_count += 1;
                if self.current().kind != TokenKind::Comma {
                    break;
                }
                self.eat(TokenKind::Comma)?;
            }
        }
        self.eat(TokenKind::RParen)?;

        let global_slots = self.symbols.top_level_slots();
        let previous_symbols = std::mem::replace(&mut self.symbols, function_symbols);
        let previous_code = std::mem::take(&mut self.code);
        let previous_in_function = self.in_function;
        let previous_function_globals = std::mem::replace(&mut self.function_globals, global_slots);
        self.in_function = true;
        let result = (|| {
            self.block()?;
            self.emit(Op::PushInt, 0, 0);
            self.emit(Op::Return, 0, 0);
            Ok(Function {
                name: full_name,
                param_count,
                code: self.code.clone(),
                slot_count: self.symbols.slot_count(),
                names: self.symbols.names.clone(),
            })
        })();
        let function_symbols = std::mem::replace(&mut self.symbols, previous_symbols);
        self.code = previous_code;
        self.in_function = previous_in_function;
        self.function_globals = previous_function_globals;
        drop(function_symbols);
        let function = result?;
        self.shared.borrow_mut().functions.push(function);
        Ok(())
    }

    fn qualified_declaration_name(&self, name: &str) -> String {
        match &self.module_qualified_name {
            Some(module_name) => format!("{module_name}.{name}"),
            None => name.to_string(),
        }
    }

    fn block(&mut self) -> Result<()> {
        self.eat(TokenKind::LBrace)?;
        self.symbols.enter_scope();
        let result = (|| {
            while self.current().kind != TokenKind::RBrace {
                if self.current().kind == TokenKind::Eof {
                    return Err(self.error("Unterminated block", self.current().clone()));
                }
                self.statement()?;
            }
            self.eat(TokenKind::RBrace)?;
            Ok(())
        })();
        self.symbols.exit_scope()?;
        result
    }

    fn expression(&mut self) -> Result<()> {
        self.logical_or()
    }

    fn logical_or(&mut self) -> Result<()> {
        self.logical_and()?;
        while self.current().kind == TokenKind::PipePipe {
            self.eat(TokenKind::PipePipe)?;
            let rhs_jump = self.emit_placeholder(Op::JumpIfZero);
            self.emit(Op::PushInt, 1, 0);
            let end_jump = self.emit_placeholder(Op::Jump);
            self.patch(rhs_jump, self.code.len() as i64)?;
            self.logical_and()?;
            let false_jump = self.emit_placeholder(Op::JumpIfZero);
            self.emit(Op::PushInt, 1, 0);
            let rhs_end_jump = self.emit_placeholder(Op::Jump);
            self.patch(false_jump, self.code.len() as i64)?;
            self.emit(Op::PushInt, 0, 0);
            self.patch(rhs_end_jump, self.code.len() as i64)?;
            self.patch(end_jump, self.code.len() as i64)?;
        }
        Ok(())
    }

    fn logical_and(&mut self) -> Result<()> {
        self.comparison()?;
        while self.current().kind == TokenKind::AmpAmp {
            self.eat(TokenKind::AmpAmp)?;
            let lhs_false_jump = self.emit_placeholder(Op::JumpIfZero);
            self.comparison()?;
            let rhs_false_jump = self.emit_placeholder(Op::JumpIfZero);
            self.emit(Op::PushInt, 1, 0);
            let end_jump = self.emit_placeholder(Op::Jump);
            self.patch(lhs_false_jump, self.code.len() as i64)?;
            self.patch(rhs_false_jump, self.code.len() as i64)?;
            self.emit(Op::PushInt, 0, 0);
            self.patch(end_jump, self.code.len() as i64)?;
        }
        Ok(())
    }

    fn comparison(&mut self) -> Result<()> {
        self.binary_level(Self::additive, comparison_op)
    }

    fn additive(&mut self) -> Result<()> {
        self.binary_level(Self::term, additive_op)
    }

    fn term(&mut self) -> Result<()> {
        self.binary_level(Self::factor, term_op)
    }

    fn binary_level(
        &mut self,
        parse_operand: fn(&mut Self) -> Result<()>,
        op_lookup: fn(TokenKind) -> Option<Op>,
    ) -> Result<()> {
        parse_operand(self)?;
        while let Some(op) = op_lookup(self.current().kind) {
            let kind = self.current().kind;
            self.eat(kind)?;
            parse_operand(self)?;
            self.emit(op, 0, 0);
        }
        Ok(())
    }

    fn factor(&mut self) -> Result<()> {
        self.primary()?;
        loop {
            match self.current().kind {
                TokenKind::LBracket => {
                    self.eat(TokenKind::LBracket)?;
                    self.expression()?;
                    self.eat(TokenKind::RBracket)?;
                    self.emit(Op::Index, 0, 0);
                }
                TokenKind::Dot => {
                    self.eat(TokenKind::Dot)?;
                    let field = self.eat(TokenKind::Ident)?.text;
                    let field_index = self.intern_field(&field);
                    self.emit(Op::GetField, field_index as i64, 0);
                }
                _ => break,
            }
        }
        Ok(())
    }

    fn primary(&mut self) -> Result<()> {
        let token = self.current().clone();
        match token.kind {
            TokenKind::Int => {
                self.eat(TokenKind::Int)?;
                let value = token.text.parse::<i64>().map_err(|_| {
                    self.error(
                        format!("Integer literal {:?} is out of range", token.text),
                        token,
                    )
                })?;
                self.emit(Op::PushInt, value, 0);
            }
            TokenKind::String => {
                self.eat(TokenKind::String)?;
                let index = self.intern_string(&token.text);
                self.emit(Op::PushString, index as i64, 0);
            }
            TokenKind::Null => {
                self.eat(TokenKind::Null)?;
                self.emit(Op::PushNull, 0, 0);
            }
            TokenKind::LBracket => {
                self.eat(TokenKind::LBracket)?;
                let mut count = 0i64;
                if self.current().kind != TokenKind::RBracket {
                    loop {
                        self.expression()?;
                        count += 1;
                        if self.current().kind != TokenKind::Comma {
                            break;
                        }
                        self.eat(TokenKind::Comma)?;
                    }
                }
                self.eat(TokenKind::RBracket)?;
                self.emit(Op::MakeArray, count, 0);
            }
            TokenKind::Ident => {
                if self.is_qualified_call() {
                    let namespace = self.eat(TokenKind::Ident)?;
                    self.eat(TokenKind::Dot)?;
                    let member = self.eat(TokenKind::Ident)?;
                    self.qualified_call_expression(&namespace, &member)?;
                } else {
                    let name = self.eat(TokenKind::Ident)?;
                    if self.current().kind == TokenKind::LParen {
                        self.call_expression(&name)?;
                    } else {
                        self.emit_load_name(&name)?;
                    }
                }
            }
            TokenKind::LParen => {
                self.eat(TokenKind::LParen)?;
                self.expression()?;
                self.eat(TokenKind::RParen)?;
            }
            TokenKind::Minus => {
                self.eat(TokenKind::Minus)?;
                self.factor()?;
                self.emit(Op::Neg, 0, 0);
            }
            TokenKind::Bang => {
                self.eat(TokenKind::Bang)?;
                self.factor()?;
                let true_jump = self.emit_placeholder(Op::JumpIfZero);
                self.emit(Op::PushInt, 0, 0);
                let end_jump = self.emit_placeholder(Op::Jump);
                self.patch(true_jump, self.code.len() as i64)?;
                self.emit(Op::PushInt, 1, 0);
                self.patch(end_jump, self.code.len() as i64)?;
            }
            TokenKind::Unsafe => {
                self.eat(TokenKind::Unsafe)?;
                self.unsafe_depth += 1;
                let result = self.factor();
                self.unsafe_depth -= 1;
                result?;
            }
            _ => return Err(self.error("Expected expression", token)),
        }
        Ok(())
    }

    fn is_qualified_call(&self) -> bool {
        self.current().kind == TokenKind::Ident
            && self.index + 3 < self.tokens.len()
            && self.tokens[self.index + 1].kind == TokenKind::Dot
            && self.tokens[self.index + 2].kind == TokenKind::Ident
            && self.tokens[self.index + 3].kind == TokenKind::LParen
    }

    fn is_expression_start(&self) -> bool {
        matches!(
            self.current().kind,
            TokenKind::Int
                | TokenKind::String
                | TokenKind::Null
                | TokenKind::Ident
                | TokenKind::LBracket
                | TokenKind::LParen
                | TokenKind::Minus
                | TokenKind::Bang
                | TokenKind::Unsafe
        )
    }

    fn call_expression(&mut self, name: &Token) -> Result<()> {
        if let Some(struct_index) = self.local_struct_indexes.get(&name.text).copied() {
            return self.constructor_call(&name.text, struct_index);
        }
        if let Some(builtin_index) = builtin_index(&name.text) {
            return self.builtin_call(&name.text, builtin_index, name.pos);
        }
        let function_index = self
            .local_function_indexes
            .get(&name.text)
            .copied()
            .ok_or_else(|| {
                self.error_at(
                    format!("Undefined function or constructor {:?}", name.text),
                    name.pos,
                )
            })?;
        self.eat(TokenKind::LParen)?;
        let arg_count = self.argument_list()?;
        self.emit(Op::Call, function_index as i64, arg_count as i64);
        Ok(())
    }

    fn qualified_call_expression(&mut self, namespace: &Token, member: &Token) -> Result<()> {
        let info = self
            .namespaces
            .get(&namespace.text)
            .cloned()
            .ok_or_else(|| {
                self.error_at(
                    format!("Unknown module namespace {:?}", namespace.text),
                    namespace.pos,
                )
            })?;
        if let Some(struct_index) = info.struct_exports.get(&member.text).copied() {
            return self
                .constructor_call(&format!("{}.{}", namespace.text, member.text), struct_index);
        }
        if let Some(function_index) = info.function_exports.get(&member.text).copied() {
            self.eat(TokenKind::LParen)?;
            let arg_count = self.argument_list()?;
            self.emit(Op::Call, function_index as i64, arg_count as i64);
            return Ok(());
        }
        if info.all_functions.contains(&member.text) || info.all_structs.contains(&member.text) {
            return Err(self.error_at(
                format!(
                    "Module member {}.{} is not exported",
                    namespace.text, member.text
                ),
                member.pos,
            ));
        }
        Err(self.error_at(
            format!(
                "Module {:?} has no exported member {:?}",
                namespace.text, member.text
            ),
            member.pos,
        ))
    }

    fn constructor_call(&mut self, name: &str, struct_index: usize) -> Result<()> {
        let field_count = self.shared.borrow().structs[struct_index].fields.len();
        self.eat(TokenKind::LParen)?;
        let arg_count = self.argument_list()?;
        if arg_count != field_count {
            return Err(self.error_at(
                format!("Struct {name:?} expects {field_count} field value(s), got {arg_count}"),
                self.current().pos,
            ));
        }
        self.emit(Op::MakeStruct, struct_index as i64, arg_count as i64);
        Ok(())
    }

    fn builtin_call(&mut self, name: &str, builtin_index: usize, pos: usize) -> Result<()> {
        let builtin = BUILTINS[builtin_index];
        self.eat(TokenKind::LParen)?;
        let arg_count = self.argument_list()?;
        if arg_count < builtin.min_args || arg_count > builtin.max_args {
            let expected = if builtin.min_args == builtin.max_args {
                builtin.min_args.to_string()
            } else {
                format!("{}..{}", builtin.min_args, builtin.max_args)
            };
            return Err(self.error_at(
                format!("Builtin {name:?} expects {expected} argument(s), got {arg_count}"),
                self.current().pos,
            ));
        }
        if builtin.requires_unsafe && self.unsafe_depth == 0 {
            return Err(self.error_at(format!("Builtin {name:?} requires unsafe syntax"), pos));
        }
        self.emit(Op::Builtin, builtin_index as i64, arg_count as i64);
        Ok(())
    }

    fn argument_list(&mut self) -> Result<usize> {
        let mut count = 0usize;
        if self.current().kind != TokenKind::RParen {
            loop {
                self.expression()?;
                count += 1;
                if self.current().kind != TokenKind::Comma {
                    break;
                }
                self.eat(TokenKind::Comma)?;
            }
        }
        self.eat(TokenKind::RParen)?;
        Ok(count)
    }

    fn current(&self) -> &Token {
        self.tokens.get(self.index).unwrap_or(&self.eof_token)
    }

    fn peek_kind(&self, offset: usize) -> Option<TokenKind> {
        self.tokens.get(self.index + offset).map(|token| token.kind)
    }

    fn eat(&mut self, kind: TokenKind) -> Result<Token> {
        let token = self.current().clone();
        if token.kind != kind {
            return Err(self.error(
                format!("Expected {}, got {}", kind.name(), token.kind.name()),
                token,
            ));
        }
        self.index += 1;
        Ok(token)
    }

    fn emit(&mut self, op: Op, arg: i64, arg2: i64) {
        self.code.push(Instr::new(op, arg, arg2));
    }

    fn emit_placeholder(&mut self, op: Op) -> usize {
        let index = self.code.len();
        self.code.push(Instr::new(op, -1, 0));
        index
    }

    fn patch(&mut self, index: usize, arg: i64) -> Result<()> {
        let Some(instr) = self.code.get_mut(index) else {
            return Err(TinyOneError::compile(
                "Internal compiler patch target error",
            ));
        };
        instr.arg = arg;
        Ok(())
    }

    fn get_assignment_slot(&self, token: &Token) -> Result<usize> {
        self.symbols.get(&token.text).ok_or_else(|| {
            if self.in_function && self.function_globals.contains_key(&token.text) {
                return self.error(
                    format!(
                        "Cannot assign to top-level variable {:?} inside a function",
                        token.text
                    ),
                    token.clone(),
                );
            }
            self.error(
                format!("Undefined variable {:?}", token.text),
                token.clone(),
            )
        })
    }

    fn get_read_slot(&self, token: &Token) -> Result<ReadSlot> {
        if let Some(slot) = self.symbols.get(&token.text) {
            return Ok(ReadSlot::Local(slot));
        }
        if self.in_function {
            if let Some(slot) = self.function_globals.get(&token.text).copied() {
                return Ok(ReadSlot::Global(slot));
            }
        }
        Err(self.error(
            format!("Undefined variable {:?}", token.text),
            token.clone(),
        ))
    }

    fn emit_load_name(&mut self, token: &Token) -> Result<()> {
        match self.get_read_slot(token)? {
            ReadSlot::Local(slot) => self.emit(Op::Load, slot as i64, 0),
            ReadSlot::Global(slot) => self.emit(Op::LoadGlobal, slot as i64, 0),
        }
        Ok(())
    }

    fn intern_string(&self, text: &str) -> usize {
        let mut state = self.shared.borrow_mut();
        if let Some(index) = state.string_indexes.get(text) {
            return *index;
        }
        let index = state.strings.len();
        state.string_indexes.insert(text.to_string(), index);
        state.strings.push(text.to_string());
        index
    }

    fn intern_field(&self, text: &str) -> usize {
        let mut state = self.shared.borrow_mut();
        if let Some(index) = state.field_indexes.get(text) {
            return *index;
        }
        let index = state.fields.len();
        state.field_indexes.insert(text.to_string(), index);
        state.fields.push(text.to_string());
        index
    }

    fn error(&self, message: impl AsRef<str>, token: Token) -> TinyOneError {
        TinyOneError::compile(self.source_map.format(message, token.pos, token.end))
    }

    fn error_at(&self, message: impl AsRef<str>, pos: usize) -> TinyOneError {
        TinyOneError::compile(self.source_map.format(message, pos, pos + 1))
    }
}

fn comparison_op(kind: TokenKind) -> Option<Op> {
    Some(match kind {
        TokenKind::Lt => Op::Lt,
        TokenKind::Lte => Op::Lte,
        TokenKind::Gt => Op::Gt,
        TokenKind::Gte => Op::Gte,
        TokenKind::EqEq => Op::Eq,
        TokenKind::BangEqual => Op::Ne,
        _ => return None,
    })
}

fn additive_op(kind: TokenKind) -> Option<Op> {
    Some(match kind {
        TokenKind::Plus => Op::Add,
        TokenKind::Minus => Op::Sub,
        _ => return None,
    })
}

fn term_op(kind: TokenKind) -> Option<Op> {
    Some(match kind {
        TokenKind::Star => Op::Mul,
        TokenKind::Slash => Op::Div,
        _ => return None,
    })
}
