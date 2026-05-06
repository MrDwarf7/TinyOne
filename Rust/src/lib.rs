use blake2::{Blake2b512, Digest};
use serde_json::{Value as JsonValue, json};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, TinyOneError>;

#[derive(Debug, Error)]
pub enum TinyOneError {
    #[error("{0}")]
    Compile(String),
    #[error("{0}")]
    Runtime(String),
}

impl TinyOneError {
    fn compile(message: impl Into<String>) -> Self {
        Self::Compile(message.into())
    }

    fn runtime(message: impl Into<String>) -> Self {
        Self::Runtime(message.into())
    }
}

#[derive(Debug, Clone)]
struct SourceMap {
    filename: String,
    source: String,
    line_starts: Vec<usize>,
}

impl SourceMap {
    fn new(source: impl Into<String>, filename: impl Into<String>) -> Self {
        let source = source.into();
        let mut line_starts = vec![0];
        for (index, byte) in source.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(index + 1);
            }
        }
        Self {
            filename: filename.into(),
            source,
            line_starts,
        }
    }

    fn line_col(&self, pos: usize) -> (usize, usize) {
        let pos = pos.min(self.source.len());
        let mut low = 0usize;
        let mut high = self.line_starts.len();
        while low + 1 < high {
            let mid = (low + high) / 2;
            if self.line_starts[mid] <= pos {
                low = mid;
            } else {
                high = mid;
            }
        }
        (low + 1, pos - self.line_starts[low] + 1)
    }

    fn format(&self, message: impl AsRef<str>, pos: usize, end: usize) -> String {
        let pos = pos.min(self.source.len());
        let (line, column) = self.line_col(pos);
        let line_start = self.line_starts[line - 1];
        let next_line_start = if line < self.line_starts.len() {
            self.line_starts[line]
        } else {
            self.source.len()
        };
        let line_text = self.source[line_start..next_line_start].trim_end_matches('\n');
        let span_end = end.max(pos + 1).min(next_line_start);
        let width = span_end.saturating_sub(pos).max(1);
        let caret = format!(
            "{}{}",
            " ".repeat(column.saturating_sub(1)),
            "^".repeat(width)
        );
        format!(
            "{}:{}:{}: {}\n{}\n{}",
            self.filename,
            line,
            column,
            message.as_ref(),
            line_text,
            caret
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum TokenKind {
    Int,
    Ident,
    String,
    Let,
    Print,
    Fn,
    Return,
    While,
    If,
    Else,
    Break,
    Continue,
    Struct,
    Import,
    Export,
    As,
    Set,
    Unsafe,
    Plus,
    Minus,
    Star,
    Slash,
    Equal,
    EqEq,
    BangEqual,
    Lt,
    Lte,
    Gt,
    Gte,
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Dot,
    Comma,
    Eof,
    Null,
}

impl TokenKind {
    fn name(self) -> &'static str {
        match self {
            TokenKind::Int => "INT",
            TokenKind::Ident => "IDENT",
            TokenKind::String => "STRING",
            TokenKind::Let => "LET",
            TokenKind::Print => "PRINT",
            TokenKind::Fn => "FN",
            TokenKind::Return => "RETURN",
            TokenKind::While => "WHILE",
            TokenKind::If => "IF",
            TokenKind::Else => "ELSE",
            TokenKind::Break => "BREAK",
            TokenKind::Continue => "CONTINUE",
            TokenKind::Struct => "STRUCT",
            TokenKind::Import => "IMPORT",
            TokenKind::Export => "EXPORT",
            TokenKind::As => "AS",
            TokenKind::Set => "SET",
            TokenKind::Unsafe => "UNSAFE",
            TokenKind::Plus => "PLUS",
            TokenKind::Minus => "MINUS",
            TokenKind::Star => "STAR",
            TokenKind::Slash => "SLASH",
            TokenKind::Equal => "EQUAL",
            TokenKind::EqEq => "EQEQ",
            TokenKind::BangEqual => "BANG_EQUAL",
            TokenKind::Lt => "LT",
            TokenKind::Lte => "LTE",
            TokenKind::Gt => "GT",
            TokenKind::Gte => "GTE",
            TokenKind::LParen => "LPAREN",
            TokenKind::RParen => "RPAREN",
            TokenKind::LBrace => "LBRACE",
            TokenKind::RBrace => "RBRACE",
            TokenKind::LBracket => "LBRACKET",
            TokenKind::RBracket => "RBRACKET",
            TokenKind::Dot => "DOT",
            TokenKind::Comma => "COMMA",
            TokenKind::Eof => "EOF",
            TokenKind::Null => "NULL",
        }
    }
}

#[derive(Debug, Clone)]
struct Token {
    kind: TokenKind,
    text: String,
    pos: usize,
    end: usize,
}

struct Lexer {
    source: String,
    source_map: SourceMap,
}

impl Lexer {
    fn new(source: impl Into<String>, filename: impl Into<String>) -> Self {
        let source = source.into();
        let filename = filename.into();
        Self {
            source: source.clone(),
            source_map: SourceMap::new(source, filename),
        }
    }

    fn tokenize(&self) -> Result<Vec<Token>> {
        let bytes = self.source.as_bytes();
        let mut pos = 0usize;
        let mut tokens = Vec::new();

        while pos < bytes.len() {
            let ch = bytes[pos];
            if ch.is_ascii_whitespace() {
                pos += 1;
                continue;
            }
            if ch == b'#' {
                pos += 1;
                while pos < bytes.len() && bytes[pos] != b'\n' {
                    pos += 1;
                }
                continue;
            }
            if ch.is_ascii_digit() {
                let start = pos;
                pos += 1;
                while pos < bytes.len() && bytes[pos].is_ascii_digit() {
                    pos += 1;
                }
                tokens.push(Token {
                    kind: TokenKind::Int,
                    text: self.source[start..pos].to_string(),
                    pos: start,
                    end: pos,
                });
                continue;
            }
            if ch == b'"' {
                let start = pos;
                pos += 1;
                let mut text = String::new();
                while pos < bytes.len() && bytes[pos] != b'"' {
                    if bytes[pos] == b'\n' {
                        return Err(self.error("Unterminated string literal", start, pos));
                    }
                    if bytes[pos] == b'\\' {
                        pos += 1;
                        if pos >= bytes.len() {
                            return Err(self.error("Unterminated string escape", start, pos));
                        }
                        match bytes[pos] {
                            b'n' => text.push('\n'),
                            b't' => text.push('\t'),
                            b'"' => text.push('"'),
                            b'\\' => text.push('\\'),
                            other => {
                                return Err(self.error(
                                    format!("Unknown string escape \\{}", other as char),
                                    pos,
                                    pos + 1,
                                ));
                            }
                        }
                    } else {
                        text.push(bytes[pos] as char);
                    }
                    pos += 1;
                }
                if pos >= bytes.len() {
                    return Err(self.error("Unterminated string literal", start, pos));
                }
                pos += 1;
                tokens.push(Token {
                    kind: TokenKind::String,
                    text,
                    pos: start,
                    end: pos,
                });
                continue;
            }
            if ch == b'_' || ch.is_ascii_alphabetic() {
                let start = pos;
                pos += 1;
                while pos < bytes.len()
                    && (bytes[pos] == b'_'
                        || bytes[pos].is_ascii_alphabetic()
                        || bytes[pos].is_ascii_digit())
                {
                    pos += 1;
                }
                let text = self.source[start..pos].to_string();
                let kind = keyword_kind(&text).unwrap_or(TokenKind::Ident);
                tokens.push(Token {
                    kind,
                    text,
                    pos: start,
                    end: pos,
                });
                continue;
            }
            if pos + 1 < bytes.len() {
                let pair = &self.source[pos..pos + 2];
                if let Some(kind) = two_char_token(pair) {
                    tokens.push(Token {
                        kind,
                        text: pair.to_string(),
                        pos,
                        end: pos + 2,
                    });
                    pos += 2;
                    continue;
                }
            }
            if let Some(kind) = single_char_token(ch) {
                tokens.push(Token {
                    kind,
                    text: (ch as char).to_string(),
                    pos,
                    end: pos + 1,
                });
                pos += 1;
                continue;
            }
            return Err(self.error(
                format!("Unexpected character {:?}", ch as char),
                pos,
                pos + 1,
            ));
        }

        tokens.push(Token {
            kind: TokenKind::Eof,
            text: String::new(),
            pos,
            end: pos,
        });
        Ok(tokens)
    }

    fn error(&self, message: impl AsRef<str>, pos: usize, end: usize) -> TinyOneError {
        TinyOneError::compile(self.source_map.format(message, pos, end))
    }
}

fn keyword_kind(text: &str) -> Option<TokenKind> {
    Some(match text {
        "let" => TokenKind::Let,
        "print" => TokenKind::Print,
        "fn" => TokenKind::Fn,
        "return" => TokenKind::Return,
        "while" => TokenKind::While,
        "if" => TokenKind::If,
        "else" => TokenKind::Else,
        "break" => TokenKind::Break,
        "continue" => TokenKind::Continue,
        "struct" => TokenKind::Struct,
        "import" => TokenKind::Import,
        "export" => TokenKind::Export,
        "as" => TokenKind::As,
        "set" => TokenKind::Set,
        "unsafe" => TokenKind::Unsafe,
        "null" => TokenKind::Null,
        _ => return None,
    })
}

fn two_char_token(text: &str) -> Option<TokenKind> {
    Some(match text {
        "==" => TokenKind::EqEq,
        "!=" => TokenKind::BangEqual,
        "<=" => TokenKind::Lte,
        ">=" => TokenKind::Gte,
        _ => return None,
    })
}

fn single_char_token(ch: u8) -> Option<TokenKind> {
    Some(match ch {
        b'+' => TokenKind::Plus,
        b'-' => TokenKind::Minus,
        b'*' => TokenKind::Star,
        b'/' => TokenKind::Slash,
        b'=' => TokenKind::Equal,
        b'<' => TokenKind::Lt,
        b'>' => TokenKind::Gt,
        b'(' => TokenKind::LParen,
        b')' => TokenKind::RParen,
        b'{' => TokenKind::LBrace,
        b'}' => TokenKind::RBrace,
        b'[' => TokenKind::LBracket,
        b']' => TokenKind::RBracket,
        b'.' => TokenKind::Dot,
        b',' => TokenKind::Comma,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Op {
    PushInt,
    Load,
    Store,
    Add,
    Sub,
    Mul,
    Div,
    Neg,
    Print,
    Lt,
    Lte,
    Gt,
    Gte,
    Eq,
    Ne,
    Jump,
    JumpIfZero,
    Call,
    Return,
    Halt,
    PushString,
    MakeArray,
    Index,
    SetIndex,
    MakeStruct,
    GetField,
    SetField,
    Builtin,
    PushNull,
}

impl Op {
    fn name(self) -> &'static str {
        match self {
            Op::PushInt => "PUSH_INT",
            Op::Load => "LOAD",
            Op::Store => "STORE",
            Op::Add => "ADD",
            Op::Sub => "SUB",
            Op::Mul => "MUL",
            Op::Div => "DIV",
            Op::Neg => "NEG",
            Op::Print => "PRINT",
            Op::Lt => "LT",
            Op::Lte => "LTE",
            Op::Gt => "GT",
            Op::Gte => "GTE",
            Op::Eq => "EQ",
            Op::Ne => "NE",
            Op::Jump => "JUMP",
            Op::JumpIfZero => "JUMP_IF_ZERO",
            Op::Call => "CALL",
            Op::Return => "RETURN",
            Op::Halt => "HALT",
            Op::PushString => "PUSH_STRING",
            Op::MakeArray => "MAKE_ARRAY",
            Op::Index => "INDEX",
            Op::SetIndex => "SET_INDEX",
            Op::MakeStruct => "MAKE_STRUCT",
            Op::GetField => "GET_FIELD",
            Op::SetField => "SET_FIELD",
            Op::Builtin => "BUILTIN",
            Op::PushNull => "PUSH_NULL",
        }
    }

    fn from_name(name: &str) -> Result<Self> {
        Ok(match name {
            "PUSH_INT" => Op::PushInt,
            "LOAD" => Op::Load,
            "STORE" => Op::Store,
            "ADD" => Op::Add,
            "SUB" => Op::Sub,
            "MUL" => Op::Mul,
            "DIV" => Op::Div,
            "NEG" => Op::Neg,
            "PRINT" => Op::Print,
            "LT" => Op::Lt,
            "LTE" => Op::Lte,
            "GT" => Op::Gt,
            "GTE" => Op::Gte,
            "EQ" => Op::Eq,
            "NE" => Op::Ne,
            "JUMP" => Op::Jump,
            "JUMP_IF_ZERO" => Op::JumpIfZero,
            "CALL" => Op::Call,
            "RETURN" => Op::Return,
            "HALT" => Op::Halt,
            "PUSH_STRING" => Op::PushString,
            "MAKE_ARRAY" => Op::MakeArray,
            "INDEX" => Op::Index,
            "SET_INDEX" => Op::SetIndex,
            "MAKE_STRUCT" => Op::MakeStruct,
            "GET_FIELD" => Op::GetField,
            "SET_FIELD" => Op::SetField,
            "BUILTIN" => Op::Builtin,
            "PUSH_NULL" => Op::PushNull,
            _ => return Err(TinyOneError::compile(format!("Unknown opcode {name:?}"))),
        })
    }

    fn ordinal(self) -> u16 {
        match self {
            Op::PushInt => 1,
            Op::Load => 2,
            Op::Store => 3,
            Op::Add => 4,
            Op::Sub => 5,
            Op::Mul => 6,
            Op::Div => 7,
            Op::Neg => 8,
            Op::Print => 9,
            Op::Lt => 10,
            Op::Lte => 11,
            Op::Gt => 12,
            Op::Gte => 13,
            Op::Eq => 14,
            Op::Ne => 15,
            Op::Jump => 16,
            Op::JumpIfZero => 17,
            Op::Call => 18,
            Op::Return => 19,
            Op::Halt => 20,
            Op::PushString => 21,
            Op::MakeArray => 22,
            Op::Index => 23,
            Op::SetIndex => 24,
            Op::MakeStruct => 25,
            Op::GetField => 26,
            Op::SetField => 27,
            Op::Builtin => 28,
            Op::PushNull => 29,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instr {
    pub op: Op,
    pub arg: i64,
    pub arg2: i64,
}

impl Instr {
    pub fn new(op: Op, arg: i64, arg2: i64) -> Self {
        Self { op, arg, arg2 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub param_count: usize,
    pub code: Vec<Instr>,
    pub slot_count: usize,
    pub names: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructDef {
    pub name: String,
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleImportDef {
    pub alias: String,
    pub path: String,
    pub module: String,
    pub resolved: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDef {
    pub name: String,
    pub path: String,
    pub imports: Vec<ModuleImportDef>,
    pub exported_functions: Vec<String>,
    pub exported_structs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub code: Vec<Instr>,
    pub slot_count: usize,
    pub names: Vec<String>,
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
    pub structs: Vec<StructDef>,
    pub fields: Vec<String>,
    pub modules: Vec<ModuleDef>,
}

impl Program {
    pub fn fingerprint(&self) -> String {
        let mut hasher = Blake2b512::new();
        self.hash_code(&mut hasher, &self.code);
        hasher.update((self.slot_count as u64).to_le_bytes());
        for name in &self.names {
            hash_string_u32(&mut hasher, name);
        }
        hasher.update((self.functions.len() as u64).to_le_bytes());
        for function in &self.functions {
            hash_string_u32(&mut hasher, &function.name);
            hasher.update((function.param_count as u64).to_le_bytes());
            hasher.update((function.slot_count as u64).to_le_bytes());
            self.hash_code(&mut hasher, &function.code);
        }
        for text in &self.strings {
            hash_string_u64(&mut hasher, text);
        }
        for item in &self.structs {
            hash_string_u32(&mut hasher, &item.name);
            hasher.update((item.fields.len() as u32).to_le_bytes());
            for field in &item.fields {
                hash_string_u32(&mut hasher, field);
            }
        }
        for field in &self.fields {
            hash_string_u32(&mut hasher, field);
        }
        for module in &self.modules {
            hash_string_u32(&mut hasher, &module.name);
            hash_string_u32(&mut hasher, &module.path);
            let lists: [&[String]; 6] = [
                &module
                    .imports
                    .iter()
                    .map(|item| item.alias.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.path.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.module.clone())
                    .collect::<Vec<_>>(),
                &module
                    .imports
                    .iter()
                    .map(|item| item.resolved.clone())
                    .collect::<Vec<_>>(),
                &module.exported_functions,
                &module.exported_structs,
            ];
            for list in lists {
                hasher.update((list.len() as u32).to_le_bytes());
                for item in list {
                    hash_string_u32(&mut hasher, item);
                }
            }
        }
        let digest = hasher.finalize();
        hex::encode(&digest[..16])
    }

    fn hash_code(&self, hasher: &mut Blake2b512, code: &[Instr]) {
        for instr in code {
            hasher.update(instr.op.ordinal().to_le_bytes());
            hasher.update((instr.arg as i128).to_le_bytes());
            hasher.update((instr.arg2 as i128).to_le_bytes());
        }
    }

    pub fn to_artifact(&self) -> JsonValue {
        json!({
            "format": "tinyone-bytecode",
            "version": 1,
            "code": encode_code(&self.code),
            "slot_count": self.slot_count,
            "names": self.names,
            "functions": self.functions.iter().map(|function| json!({
                "name": function.name,
                "param_count": function.param_count,
                "code": encode_code(&function.code),
                "slot_count": function.slot_count,
                "names": function.names,
            })).collect::<Vec<_>>(),
            "strings": self.strings,
            "structs": self.structs.iter().map(|item| json!({
                "name": item.name,
                "fields": item.fields,
            })).collect::<Vec<_>>(),
            "fields": self.fields,
            "modules": self.modules.iter().map(|module| json!({
                "name": module.name,
                "path": module.path,
                "imports": module.imports.iter().map(|item| json!({
                    "alias": item.alias,
                    "path": item.path,
                    "module": item.module,
                    "resolved": item.resolved,
                })).collect::<Vec<_>>(),
                "exported_functions": module.exported_functions,
                "exported_structs": module.exported_structs,
            })).collect::<Vec<_>>(),
        })
    }

    pub fn from_artifact(data: JsonValue) -> Result<Self> {
        let object = data
            .as_object()
            .ok_or_else(|| TinyOneError::compile("Artifact must be a JSON object"))?;
        if object.get("format").and_then(JsonValue::as_str) != Some("tinyone-bytecode")
            || object.get("version").and_then(JsonValue::as_i64) != Some(1)
        {
            return Err(TinyOneError::compile("Unsupported TinyOne artifact format"));
        }
        let functions = expect_array(object.get("functions"), "functions")?
            .iter()
            .map(|item| {
                let obj = item
                    .as_object()
                    .ok_or_else(|| TinyOneError::compile("Function artifact must be an object"))?;
                Ok(Function {
                    name: expect_str(obj.get("name"), "function name")?,
                    param_count: expect_usize(obj.get("param_count"), "param_count")?,
                    code: decode_code(obj.get("code"))?,
                    slot_count: expect_usize(obj.get("slot_count"), "slot_count")?,
                    names: expect_string_list(obj.get("names"), "names")?,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let program = Program {
            code: decode_code(object.get("code"))?,
            slot_count: expect_usize(object.get("slot_count"), "slot_count")?,
            names: expect_string_list(object.get("names"), "names")?,
            functions,
            strings: expect_string_list(object.get("strings"), "strings")?,
            structs: expect_array(object.get("structs"), "structs")?
                .iter()
                .map(|item| {
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Struct artifact must be an object")
                    })?;
                    Ok(StructDef {
                        name: expect_str(obj.get("name"), "struct name")?,
                        fields: expect_string_list(obj.get("fields"), "struct fields")?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
            fields: expect_string_list(object.get("fields"), "fields")?,
            modules: optional_array(object.get("modules"), "modules")?
                .iter()
                .map(|item| {
                    let obj = item.as_object().ok_or_else(|| {
                        TinyOneError::compile("Module artifact must be an object")
                    })?;
                    Ok(ModuleDef {
                        name: expect_str(obj.get("name"), "module name")?,
                        path: expect_str(obj.get("path"), "module path")?,
                        imports: optional_array(obj.get("imports"), "module imports")?
                            .iter()
                            .map(|item| {
                                let item = item.as_object().ok_or_else(|| {
                                    TinyOneError::compile("Module import must be an object")
                                })?;
                                Ok(ModuleImportDef {
                                    alias: expect_str(item.get("alias"), "import alias")?,
                                    path: expect_str(item.get("path"), "import path")?,
                                    module: expect_str(item.get("module"), "import module")?,
                                    resolved: expect_str(item.get("resolved"), "import resolved")?,
                                })
                            })
                            .collect::<Result<Vec<_>>>()?,
                        exported_functions: expect_string_list(
                            obj.get("exported_functions"),
                            "module function exports",
                        )?,
                        exported_structs: expect_string_list(
                            obj.get("exported_structs"),
                            "module struct exports",
                        )?,
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        BytecodeVerifier::verify(&program)?;
        Ok(program)
    }
}

fn hash_string_u32(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u32).to_le_bytes());
    hasher.update(bytes);
}

fn hash_string_u64(hasher: &mut Blake2b512, value: &str) {
    let bytes = value.as_bytes();
    hasher.update((bytes.len() as u64).to_le_bytes());
    hasher.update(bytes);
}

fn encode_code(code: &[Instr]) -> Vec<JsonValue> {
    code.iter()
        .map(|instr| json!({"op": instr.op.name(), "arg": instr.arg, "arg2": instr.arg2}))
        .collect()
}

fn decode_code(value: Option<&JsonValue>) -> Result<Vec<Instr>> {
    expect_array(value, "code")?
        .iter()
        .map(|item| {
            let obj = item
                .as_object()
                .ok_or_else(|| TinyOneError::compile("Instruction artifact must be an object"))?;
            let op = Op::from_name(
                obj.get("op")
                    .and_then(JsonValue::as_str)
                    .ok_or_else(|| TinyOneError::compile("Instruction op must be a string"))?,
            )?;
            Ok(Instr::new(
                op,
                obj.get("arg").and_then(JsonValue::as_i64).unwrap_or(0),
                obj.get("arg2").and_then(JsonValue::as_i64).unwrap_or(0),
            ))
        })
        .collect()
}

fn expect_array<'a>(value: Option<&'a JsonValue>, name: &str) -> Result<&'a Vec<JsonValue>> {
    value
        .and_then(JsonValue::as_array)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a list")))
}

fn optional_array<'a>(value: Option<&'a JsonValue>, name: &str) -> Result<&'a Vec<JsonValue>> {
    static EMPTY: Vec<JsonValue> = Vec::new();
    match value {
        Some(value) => value.as_array().ok_or_else(|| {
            TinyOneError::compile(format!("Artifact field {name:?} must be a list"))
        }),
        None => Ok(&EMPTY),
    }
}

fn expect_str(value: Option<&JsonValue>, name: &str) -> Result<String> {
    value
        .and_then(JsonValue::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be a string")))
}

fn expect_usize(value: Option<&JsonValue>, name: &str) -> Result<usize> {
    value
        .and_then(JsonValue::as_u64)
        .map(|value| value as usize)
        .ok_or_else(|| TinyOneError::compile(format!("Artifact field {name:?} must be an integer")))
}

fn expect_string_list(value: Option<&JsonValue>, name: &str) -> Result<Vec<String>> {
    expect_array(value, name)?
        .iter()
        .map(|item| {
            item.as_str().map(ToOwned::to_owned).ok_or_else(|| {
                TinyOneError::compile(format!("Artifact field {name:?} must contain strings"))
            })
        })
        .collect()
}

#[derive(Debug, Clone)]
struct ModuleInfo {
    name: String,
    path: String,
    function_exports: HashMap<String, usize>,
    struct_exports: HashMap<String, usize>,
    all_functions: HashSet<String>,
    all_structs: HashSet<String>,
    imports: Vec<ModuleImportDef>,
    finalized: bool,
}

#[derive(Debug, Default)]
struct CompilerSharedState {
    function_indexes: HashMap<String, usize>,
    functions: Vec<Function>,
    struct_indexes: HashMap<String, usize>,
    structs: Vec<StructDef>,
    field_indexes: HashMap<String, usize>,
    fields: Vec<String>,
    string_indexes: HashMap<String, usize>,
    strings: Vec<String>,
    modules: HashMap<String, ModuleInfo>,
    loading_modules: HashSet<String>,
    module_defs: Vec<ModuleDef>,
    module_name_owners: HashMap<String, String>,
}

#[derive(Debug, Clone, Copy)]
struct BuiltinDef {
    name: &'static str,
    min_args: usize,
    max_args: usize,
    requires_unsafe: bool,
}

const BUILTINS: &[BuiltinDef] = &[
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
        requires_unsafe: false,
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

fn builtin_index(name: &str) -> Option<usize> {
    BUILTINS.iter().position(|item| item.name == name)
}

#[derive(Debug, Default, Clone)]
struct SymbolTable {
    scopes: Vec<HashMap<String, usize>>,
    names: Vec<String>,
}

impl SymbolTable {
    fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
            names: Vec::new(),
        }
    }

    fn enter_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn exit_scope(&mut self) {
        if self.scopes.len() <= 1 {
            panic!("cannot exit root symbol scope");
        }
        self.scopes.pop();
    }

    fn define_or_get(&mut self, name: &str) -> usize {
        for scope in self.scopes.iter().rev() {
            if let Some(slot) = scope.get(name) {
                return *slot;
            }
        }
        let slot = self.names.len();
        self.scopes
            .last_mut()
            .expect("scope")
            .insert(name.to_string(), slot);
        self.names.push(name.to_string());
        slot
    }

    fn get(&self, name: &str) -> Option<usize> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(name).copied())
    }

    fn contains(&self, name: &str) -> bool {
        self.scopes.iter().any(|scope| scope.contains_key(name))
    }

    fn slot_count(&self) -> usize {
        self.names.len()
    }
}

type SharedState = Rc<RefCell<CompilerSharedState>>;
type Resolver = fn(&str, &str) -> Result<(String, String)>;

#[derive(Debug)]
struct LoopContext {
    start: usize,
    breaks: Vec<usize>,
}

struct Compiler {
    tokens: Vec<Token>,
    index: usize,
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
    loops: Vec<LoopContext>,
    in_function: bool,
    unsafe_depth: usize,
}

impl Compiler {
    fn new(
        source: impl Into<String>,
        filename: impl Into<String>,
        resolver: Option<Resolver>,
        module_mode: bool,
        module_name: impl Into<String>,
        shared: SharedState,
    ) -> Result<Self> {
        let source = source.into();
        let filename = filename.into();
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
                            path: filename.clone(),
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
            loops: Vec::new(),
            in_function: false,
            unsafe_depth: 0,
        })
    }

    fn compile(&mut self) -> Result<Program> {
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
        self.finalize_module();
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

    fn finalize_module(&mut self) {
        let Some(filename) = self.module_filename.clone() else {
            return;
        };
        let mut state = self.shared.borrow_mut();
        let def = {
            let info = state.modules.get_mut(&filename).expect("module info");
            if info.finalized {
                return;
            }
            info.imports = self.module_imports.clone();
            let mut exported_functions = info.function_exports.keys().cloned().collect::<Vec<_>>();
            let mut exported_structs = info.struct_exports.keys().cloned().collect::<Vec<_>>();
            exported_functions.sort();
            exported_structs.sort();
            info.finalized = true;
            ModuleDef {
                name: info.name.clone(),
                path: info.path.clone(),
                imports: info.imports.clone(),
                exported_functions,
                exported_structs,
            }
        };
        state.module_defs.push(def);
    }

    fn statement(&mut self) -> Result<()> {
        match self.current().kind {
            TokenKind::Let => self.let_statement(),
            TokenKind::Print => self.print_statement(),
            TokenKind::While => self.while_statement(),
            TokenKind::If => self.if_statement(),
            TokenKind::Break => self.break_statement(),
            TokenKind::Continue => self.continue_statement(),
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
            _ => Err(self.error("Expected statement", self.current().clone())),
        }
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
        let slot = self.symbols.define_or_get(&name_token.text);
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
        self.block()?;
        let loop_context = self.loops.pop().expect("loop context");
        self.emit(Op::Jump, loop_start as i64, 0);
        let loop_end = self.code.len() as i64;
        self.patch(exit_jump, loop_end);
        for break_jump in loop_context.breaks {
            self.patch(break_jump, loop_end);
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
            self.patch(false_jump, self.code.len() as i64);
            self.eat(TokenKind::Else)?;
            self.block()?;
            self.patch(end_jump, self.code.len() as i64);
        } else {
            self.patch(false_jump, self.code.len() as i64);
        }
        Ok(())
    }

    fn break_statement(&mut self) -> Result<()> {
        let token = self.eat(TokenKind::Break)?;
        if self.loops.is_empty() {
            return Err(self.error("Break outside loop", token));
        }
        let break_jump = self.emit_placeholder(Op::Jump);
        self.loops
            .last_mut()
            .expect("loop context")
            .breaks
            .push(break_jump);
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
        let slot = self.get_slot(&name_token)?;
        self.emit(Op::Load, slot as i64, 0);

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
            module: info.name,
            resolved: module_filename,
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
            let info = state.modules.get_mut(filename).expect("module info");
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
            let info = state.modules.get_mut(filename).expect("module info");
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
                let slot = function_symbols.define_or_get(&param.text);
                if slot != param_count {
                    return Err(self.error(format!("Duplicate parameter {:?}", param.text), param));
                }
                param_count += 1;
                if self.current().kind != TokenKind::Comma {
                    break;
                }
                self.eat(TokenKind::Comma)?;
            }
        }
        self.eat(TokenKind::RParen)?;

        let previous_symbols = std::mem::replace(&mut self.symbols, function_symbols);
        let previous_code = std::mem::take(&mut self.code);
        let previous_in_function = self.in_function;
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
        self.symbols.exit_scope();
        result
    }

    fn expression(&mut self) -> Result<()> {
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
                        let slot = self.get_slot(&name)?;
                        self.emit(Op::Load, slot as i64, 0);
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
            return Err(self.error_at(
                format!("Builtin {name:?} requires unsafe dereference syntax"),
                pos,
            ));
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
        &self.tokens[self.index]
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

    fn patch(&mut self, index: usize, arg: i64) {
        self.code[index].arg = arg;
    }

    fn get_slot(&self, token: &Token) -> Result<usize> {
        self.symbols.get(&token.text).ok_or_else(|| {
            self.error(
                format!("Undefined variable {:?}", token.text),
                token.clone(),
            )
        })
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

pub struct BytecodeVerifier;

impl BytecodeVerifier {
    pub fn verify(program: &Program) -> Result<()> {
        Self::verify_chunk(
            "main",
            &program.code,
            program.slot_count,
            &program.functions,
            &program.strings,
            &program.structs,
            &program.fields,
            Op::Halt,
        )?;
        for (index, function) in program.functions.iter().enumerate() {
            Self::verify_chunk(
                &format!("function {:?} (index {index})", function.name),
                &function.code,
                function.slot_count,
                &program.functions,
                &program.strings,
                &program.structs,
                &program.fields,
                Op::Return,
            )?;
        }
        Ok(())
    }

    fn verify_chunk(
        chunk_name: &str,
        code: &[Instr],
        slot_count: usize,
        functions: &[Function],
        strings: &[String],
        structs: &[StructDef],
        fields: &[String],
        final_op: Op,
    ) -> Result<()> {
        if code.last().map(|instr| instr.op) != Some(final_op) {
            let got = code
                .last()
                .map(|instr| instr.op.name())
                .unwrap_or("nothing");
            return Err(TinyOneError::compile(format!(
                "Verifier: {chunk_name} must end with {}, got {got}",
                final_op.name()
            )));
        }
        let mut seen: HashMap<usize, i64> = HashMap::new();
        let mut todo = Vec::new();
        visit(&mut seen, &mut todo, code, 0, 0, 0, chunk_name)?;
        while let Some((pc, depth)) = todo.pop() {
            let instr = code[pc];
            let op = instr.op;
            let arg = instr.arg;
            let arg2 = instr.arg2;
            if matches!(op, Op::Load | Op::Store) && !valid_index(arg, slot_count) {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid slot {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if op == Op::PushString && !valid_index(arg, strings.len()) {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid string index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            if matches!(op, Op::GetField | Op::SetField) && !valid_index(arg, fields.len()) {
                return Err(TinyOneError::compile(format!(
                    "Verifier: invalid field index {arg} at instruction {pc} in {chunk_name}"
                )));
            }
            match op {
                Op::Jump => visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?,
                Op::JumpIfZero => {
                    let depth = next_depth(pc, depth, -1, chunk_name)?;
                    visit(&mut seen, &mut todo, code, arg, depth, pc, chunk_name)?;
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        depth,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Call => {
                    if !valid_index(arg, functions.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid function index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let function = &functions[arg as usize];
                    if arg2 as usize != function.param_count {
                        return Err(TinyOneError::compile(format!(
                            "Function {:?} expects {} argument(s), got {arg2} at instruction {pc} in {chunk_name}",
                            function.name, function.param_count
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
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
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::MakeStruct => {
                    if !valid_index(arg, structs.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid struct index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let expected = structs[arg as usize].fields.len();
                    if arg2 as usize != expected {
                        return Err(TinyOneError::compile(format!(
                            "Struct {:?} expects {expected} field value(s), got {arg2} at instruction {pc} in {chunk_name}",
                            structs[arg as usize].name
                        )));
                    }
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
                Op::Builtin => {
                    if !valid_index(arg, BUILTINS.len()) {
                        return Err(TinyOneError::compile(format!(
                            "Verifier: invalid builtin index {arg} at instruction {pc} in {chunk_name}"
                        )));
                    }
                    let builtin = BUILTINS[arg as usize];
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
                        (pc + 1) as i64,
                        next_depth(pc, depth, 1 - arg2, chunk_name)?,
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
                        TinyOneError::compile(format!(
                            "Verifier: unknown opcode {op:?} at index {pc} in {chunk_name}"
                        ))
                    })?;
                    visit(
                        &mut seen,
                        &mut todo,
                        code,
                        (pc + 1) as i64,
                        next_depth(pc, depth, effect, chunk_name)?,
                        pc,
                        chunk_name,
                    )?;
                }
            }
        }
        Ok(())
    }
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
    if pc < 0 || pc as usize >= code.len() {
        return Err(TinyOneError::compile(format!(
            "Verifier: instruction {origin} in {chunk_name} targets {pc}"
        )));
    }
    let pc = pc as usize;
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
    let depth = depth + delta;
    if depth < 0 {
        return Err(TinyOneError::compile(format!(
            "Verifier: stack underflow at instruction {pc} in {chunk_name}"
        )));
    }
    Ok(depth)
}

fn valid_index(index: i64, len: usize) -> bool {
    index >= 0 && (index as usize) < len
}

fn stack_effect(op: Op) -> Option<i64> {
    Some(match op {
        Op::PushInt | Op::PushString | Op::PushNull | Op::Load => 1,
        Op::Store => -1,
        Op::Add
        | Op::Sub
        | Op::Mul
        | Op::Div
        | Op::Lt
        | Op::Lte
        | Op::Gt
        | Op::Gte
        | Op::Eq
        | Op::Ne
        | Op::Index => -1,
        Op::Neg | Op::GetField => 0,
        Op::Print => -1,
        Op::SetIndex => -3,
        Op::SetField => -2,
        _ => return None,
    })
}

pub struct PeepholeOptimizer;

impl PeepholeOptimizer {
    pub fn optimize(program: Program) -> Program {
        Program {
            code: Self::optimize_code(&program.code),
            functions: program
                .functions
                .into_iter()
                .map(|function| Function {
                    code: Self::optimize_code(&function.code),
                    ..function
                })
                .collect(),
            ..program
        }
    }

    fn optimize_code(original: &[Instr]) -> Vec<Instr> {
        if original
            .iter()
            .any(|instr| matches!(instr.op, Op::Jump | Op::JumpIfZero))
        {
            return original.to_vec();
        }
        let mut code = original.to_vec();
        let mut changed = true;
        while changed {
            changed = false;
            let mut out = Vec::with_capacity(code.len());
            let mut i = 0usize;
            while i < code.len() {
                if i + 1 < code.len() && code[i].op == Op::PushInt && code[i + 1].op == Op::Neg {
                    out.push(Instr::new(Op::PushInt, -code[i].arg, 0));
                    i += 2;
                    changed = true;
                    continue;
                }
                if i + 2 < code.len() && code[i].op == Op::PushInt && code[i + 1].op == Op::PushInt
                {
                    let a = code[i].arg;
                    let b = code[i + 1].arg;
                    if let Some(value) = fold_binop(code[i + 2].op, a, b) {
                        out.push(Instr::new(Op::PushInt, value, 0));
                        i += 3;
                        changed = true;
                        continue;
                    }
                }
                out.push(code[i]);
                i += 1;
            }
            code = out;
        }
        code
    }
}

fn fold_binop(op: Op, a: i64, b: i64) -> Option<i64> {
    Some(match op {
        Op::Add => a.checked_add(b)?,
        Op::Sub => a.checked_sub(b)?,
        Op::Mul => a.checked_mul(b)?,
        Op::Div if b != 0 => floor_div(a, b)?,
        Op::Lt => (a < b) as i64,
        Op::Lte => (a <= b) as i64,
        Op::Gt => (a > b) as i64,
        Op::Gte => (a >= b) as i64,
        Op::Eq => (a == b) as i64,
        Op::Ne => (a != b) as i64,
        _ => return None,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HeapRef {
    address: usize,
    generation: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawPointer {
    address: usize,
    kind: String,
    index: i64,
    field: String,
    generation: u64,
    cast: String,
}

impl RawPointer {
    fn new(
        address: usize,
        kind: impl Into<String>,
        index: i64,
        field: impl Into<String>,
        generation: u64,
        cast: impl Into<String>,
    ) -> Self {
        Self {
            address,
            kind: kind.into(),
            index,
            field: field.into(),
            generation,
            cast: cast.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeValue {
    Int(i64),
    Heap(HeapRef),
    Pointer(RawPointer),
}

impl Default for RuntimeValue {
    fn default() -> Self {
        Self::Int(0)
    }
}

type Value = RuntimeValue;

#[derive(Debug, Clone)]
enum HeapData {
    String(String),
    Array(Vec<Value>),
    Buffer(Vec<u8>),
    Struct(Vec<(String, Value)>),
    Cell(Value),
}

#[derive(Debug, Clone)]
struct HeapObject {
    data: HeapData,
    type_name: String,
}

impl HeapObject {
    fn kind(&self) -> &'static str {
        match self.data {
            HeapData::String(_) => "string",
            HeapData::Array(_) => "array",
            HeapData::Buffer(_) => "buffer",
            HeapData::Struct(_) => "struct",
            HeapData::Cell(_) => "cell",
        }
    }
}

#[derive(Debug, Default)]
pub struct TinyHeap {
    objects: Vec<Option<HeapObject>>,
    free: Vec<usize>,
    generations: Vec<u64>,
}

impl TinyHeap {
    fn new() -> Self {
        Self {
            objects: vec![None],
            free: Vec::new(),
            generations: vec![0],
        }
    }

    fn alloc(&mut self, object: HeapObject) -> HeapRef {
        if let Some(address) = self.free.pop() {
            self.generations[address] += 1;
            self.objects[address] = Some(object);
            HeapRef {
                address,
                generation: self.generations[address],
            }
        } else {
            let address = self.objects.len();
            self.objects.push(Some(object));
            self.generations.push(1);
            HeapRef {
                address,
                generation: 1,
            }
        }
    }

    fn alloc_string(&mut self, text: impl Into<String>) -> HeapRef {
        self.alloc(HeapObject {
            data: HeapData::String(text.into()),
            type_name: String::new(),
        })
    }

    fn alloc_array(&mut self, values: Vec<Value>) -> HeapRef {
        self.alloc(HeapObject {
            data: HeapData::Array(values),
            type_name: String::new(),
        })
    }

    fn alloc_buffer(&mut self, size: usize) -> HeapRef {
        self.alloc(HeapObject {
            data: HeapData::Buffer(vec![0; size]),
            type_name: String::new(),
        })
    }

    fn alloc_struct(
        &mut self,
        type_name: impl Into<String>,
        fields: Vec<(String, Value)>,
    ) -> HeapRef {
        self.alloc(HeapObject {
            data: HeapData::Struct(fields),
            type_name: type_name.into(),
        })
    }

    fn alloc_cell(&mut self, value: Value) -> HeapRef {
        self.alloc(HeapObject {
            data: HeapData::Cell(value),
            type_name: String::new(),
        })
    }

    fn get(&self, value: &Value) -> Result<&HeapObject> {
        let Value::Heap(reference) = value else {
            return Err(TinyOneError::runtime("Expected heap pointer"));
        };
        self.get_address(reference.address, reference.generation)
    }

    fn get_mut(&mut self, value: &Value) -> Result<&mut HeapObject> {
        let Value::Heap(reference) = value else {
            return Err(TinyOneError::runtime("Expected heap pointer"));
        };
        self.get_address_mut(reference.address, reference.generation)
    }

    fn ref_at(&self, address: usize) -> Result<HeapRef> {
        Ok(HeapRef {
            address,
            generation: self.current_generation(address)?,
        })
    }

    fn current_generation(&self, address: usize) -> Result<u64> {
        self.current_object(address)?;
        Ok(self.generations[address])
    }

    fn get_address(&self, address: usize, generation: u64) -> Result<&HeapObject> {
        let obj = self.current_object(address)?;
        if generation != 0 && self.generations[address] != generation {
            return Err(TinyOneError::runtime(format!(
                "Stale heap pointer {address}"
            )));
        }
        Ok(obj)
    }

    fn get_address_mut(&mut self, address: usize, generation: u64) -> Result<&mut HeapObject> {
        self.current_object(address)?;
        if generation != 0 && self.generations[address] != generation {
            return Err(TinyOneError::runtime(format!(
                "Stale heap pointer {address}"
            )));
        }
        Ok(self.objects[address].as_mut().expect("current object"))
    }

    fn current_object(&self, address: usize) -> Result<&HeapObject> {
        if address == 0 || address >= self.objects.len() {
            return Err(TinyOneError::runtime(format!(
                "Invalid heap pointer {address}"
            )));
        }
        self.objects[address].as_ref().ok_or_else(|| {
            TinyOneError::runtime(format!("Use after free for heap pointer {address}"))
        })
    }

    fn free(&mut self, value: &Value) -> Result<()> {
        let Value::Heap(reference) = value else {
            return Err(TinyOneError::runtime("Expected heap pointer"));
        };
        self.get_address(reference.address, reference.generation)?;
        self.objects[reference.address] = None;
        self.free.push(reference.address);
        Ok(())
    }
}

#[derive(Debug)]
pub struct TinyRuntimeContext {
    heap: TinyHeap,
    inputs: Vec<String>,
    input_index: usize,
}

impl TinyRuntimeContext {
    fn new(inputs: impl IntoIterator<Item = String>) -> Self {
        Self {
            heap: TinyHeap::new(),
            inputs: inputs.into_iter().collect(),
            input_index: 0,
        }
    }

    fn read_raw(&mut self) -> Result<String> {
        if self.input_index >= self.inputs.len() {
            return Err(TinyOneError::runtime("Input exhausted"));
        }
        let value = self.inputs[self.input_index].clone();
        self.input_index += 1;
        Ok(value)
    }
}

fn expect_int(value: &Value, operation: &str) -> Result<i64> {
    match value {
        Value::Int(value) => Ok(*value),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects integer operands"
        ))),
    }
}

fn expect_int_pair(lhs: Value, rhs: Value, operation: &str) -> Result<(i64, i64)> {
    let Value::Int(lhs) = lhs else {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects integer operands"
        )));
    };
    let Value::Int(rhs) = rhs else {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects integer operands"
        )));
    };
    Ok((lhs, rhs))
}

fn runtime_add_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Addition")?;
    Ok(Value::Int(lhs.checked_add(rhs).ok_or_else(|| {
        TinyOneError::runtime("Addition overflow")
    })?))
}

fn runtime_add(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_add_int(lhs, rhs)
}

fn runtime_sub_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Subtraction")?;
    Ok(Value::Int(lhs.checked_sub(rhs).ok_or_else(|| {
        TinyOneError::runtime("Subtraction overflow")
    })?))
}

fn runtime_sub(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_sub_int(lhs, rhs)
}

fn runtime_mul_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Multiplication")?;
    Ok(Value::Int(lhs.checked_mul(rhs).ok_or_else(|| {
        TinyOneError::runtime("Multiplication overflow")
    })?))
}

fn runtime_mul(lhs: Value, rhs: Value) -> Result<Value> {
    runtime_mul_int(lhs, rhs)
}

fn checked_div_int(lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, "Division")?;
    if rhs == 0 {
        return Err(TinyOneError::runtime("Division by zero"));
    }
    Ok(Value::Int(floor_div(lhs, rhs).ok_or_else(|| {
        TinyOneError::runtime("Division overflow")
    })?))
}

fn checked_div(lhs: Value, rhs: Value) -> Result<Value> {
    checked_div_int(lhs, rhs)
}

fn floor_div(lhs: i64, rhs: i64) -> Option<i64> {
    let quotient = lhs.checked_div(rhs)?;
    let remainder = lhs.checked_rem(rhs)?;
    if remainder != 0 && ((remainder > 0) != (rhs > 0)) {
        quotient.checked_sub(1)
    } else {
        Some(quotient)
    }
}

fn runtime_neg(value: Value) -> Result<Value> {
    Ok(Value::Int(
        expect_int(&value, "Negation")?
            .checked_neg()
            .ok_or_else(|| TinyOneError::runtime("Negation overflow"))?,
    ))
}

fn runtime_compare_int(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    let (lhs, rhs) = expect_int_pair(lhs, rhs, op.name())?;
    let result = match op {
        Op::Lt => lhs < rhs,
        Op::Lte => lhs <= rhs,
        Op::Gt => lhs > rhs,
        Op::Gte => lhs >= rhs,
        Op::Eq => lhs == rhs,
        Op::Ne => lhs != rhs,
        _ => {
            return Err(TinyOneError::runtime(format!(
                "Unsupported comparison opcode {op:?}"
            )));
        }
    };
    Ok(Value::Int(result as i64))
}

fn runtime_compare(op: Op, lhs: Value, rhs: Value) -> Result<Value> {
    runtime_compare_int(op, lhs, rhs)
}

fn runtime_is_false(value: &Value) -> bool {
    matches!(value, Value::Int(0)) || runtime_is_null(value)
}

fn runtime_is_null(value: &Value) -> bool {
    matches!(
        value,
        Value::Pointer(pointer) if pointer.kind == "null" && pointer.address == 0
    )
}

fn runtime_null() -> Value {
    Value::Pointer(RawPointer::new(0, "null", 0, "", 0, ""))
}

fn runtime_make_array(context: &mut TinyRuntimeContext, values: Vec<Value>) -> Value {
    Value::Heap(context.heap.alloc_array(values))
}

fn runtime_index(
    context: &mut TinyRuntimeContext,
    container: Value,
    index: Value,
) -> Result<Value> {
    let index = expect_int(&index, "Index")?;
    let object = context.heap.get(&container)?.clone();
    match object.data {
        HeapData::Array(values) => {
            if index < 0 || index as usize >= values.len() {
                return Err(TinyOneError::runtime(format!(
                    "Array index {index} out of bounds"
                )));
            }
            Ok(values[index as usize].clone())
        }
        HeapData::String(text) => {
            let ch = text.chars().nth(index as usize).ok_or_else(|| {
                TinyOneError::runtime(format!("String index {index} out of bounds"))
            })?;
            Ok(Value::Heap(context.heap.alloc_string(ch.to_string())))
        }
        _ => Err(TinyOneError::runtime(format!(
            "Cannot index {}",
            object.kind()
        ))),
    }
}

fn runtime_set_index(
    context: &mut TinyRuntimeContext,
    container: Value,
    index: Value,
    value: Value,
) -> Result<()> {
    let index = expect_int(&index, "Index")?;
    let object = context.heap.get_mut(&container)?;
    let kind = object.kind();
    let HeapData::Array(values) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot assign index on {kind}"
        )));
    };
    if index < 0 || index as usize >= values.len() {
        return Err(TinyOneError::runtime(format!(
            "Array index {index} out of bounds"
        )));
    }
    values[index as usize] = value;
    Ok(())
}

fn runtime_array_push(
    context: &mut TinyRuntimeContext,
    target: &Value,
    value: Value,
) -> Result<Value> {
    let object = context.heap.get_mut(target)?;
    let kind = object.kind();
    let HeapData::Array(values) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "push() expects an array, got {kind}"
        )));
    };
    values.push(value);
    Ok(Value::Int(values.len() as i64))
}

fn runtime_array_pop(context: &mut TinyRuntimeContext, target: &Value) -> Result<Value> {
    let object = context.heap.get_mut(target)?;
    let kind = object.kind();
    let HeapData::Array(values) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "pop() expects an array, got {kind}"
        )));
    };
    values
        .pop()
        .ok_or_else(|| TinyOneError::runtime("pop() cannot pop from an empty array"))
}

fn runtime_make_struct(
    context: &mut TinyRuntimeContext,
    type_name: &str,
    field_names: &[String],
    values: Vec<Value>,
) -> Value {
    let fields = field_names.iter().cloned().zip(values).collect();
    Value::Heap(context.heap.alloc_struct(type_name, fields))
}

fn runtime_get_field(context: &TinyRuntimeContext, target: Value, field: &str) -> Result<Value> {
    let object = context.heap.get(&target)?;
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot read field {field:?} from {}",
            object.kind()
        )));
    };
    fields
        .iter()
        .find(|(name, _)| name == field)
        .map(|(_, value)| value.clone())
        .ok_or_else(|| {
            TinyOneError::runtime(format!(
                "Unknown field {field:?} on struct {:?}",
                object.type_name
            ))
        })
}

fn runtime_set_field(
    context: &mut TinyRuntimeContext,
    target: Value,
    field: &str,
    value: Value,
) -> Result<()> {
    let object = context.heap.get_mut(&target)?;
    let type_name = object.type_name.clone();
    let kind = object.kind();
    let HeapData::Struct(fields) = &mut object.data else {
        return Err(TinyOneError::runtime(format!(
            "Cannot write field {field:?} on {kind}"
        )));
    };
    if let Some((_, field_value)) = fields.iter_mut().find(|(name, _)| name == field) {
        *field_value = value;
        Ok(())
    } else {
        Err(TinyOneError::runtime(format!(
            "Unknown field {field:?} on struct {type_name:?}"
        )))
    }
}

fn expect_string(context: &TinyRuntimeContext, value: &Value, operation: &str) -> Result<String> {
    let object = context.heap.get(value)?;
    match &object.data {
        HeapData::String(text) => Ok(text.clone()),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects a string"
        ))),
    }
}

fn expect_pointer(value: &Value, operation: &str) -> Result<RawPointer> {
    match value {
        Value::Pointer(pointer) => Ok(pointer.clone()),
        _ => Err(TinyOneError::runtime(format!(
            "{operation} expects a raw pointer"
        ))),
    }
}

fn validate_pointer_base(
    context: &TinyRuntimeContext,
    pointer: &RawPointer,
    operation: &str,
) -> Result<()> {
    if pointer.kind == "null" && pointer.address == 0 {
        return Ok(());
    }
    match pointer.kind.as_str() {
        "object" | "array" | "buffer" | "field" => {
            context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            Ok(())
        }
        _ => Err(TinyOneError::runtime(format!(
            "{operation} got unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn pointer_identity(pointer: &RawPointer) -> (usize, u64, String, i64, String) {
    if pointer.kind == "null" && pointer.address == 0 {
        return (0, 0, "null".to_string(), 0, String::new());
    }
    (
        pointer.address,
        pointer.generation,
        pointer.kind.clone(),
        pointer.index,
        pointer.field.clone(),
    )
}

fn runtime_make_pointer(context: &TinyRuntimeContext, args: &[Value]) -> Result<Value> {
    if args.len() == 1 {
        match &args[0] {
            Value::Pointer(pointer) => return Ok(Value::Pointer(pointer.clone())),
            Value::Heap(reference) => {
                context.heap.get(&args[0])?;
                return Ok(Value::Pointer(RawPointer::new(
                    reference.address,
                    "object",
                    0,
                    "",
                    reference.generation,
                    "",
                )));
            }
            _ => {
                return Err(TinyOneError::runtime(
                    "ptr() expects a heap value or pointer",
                ));
            }
        }
    }
    let target = &args[0];
    let index = expect_int(&args[1], "ptr index")?;
    let Value::Heap(reference) = target else {
        return Err(TinyOneError::runtime(
            "ptr(value, index) expects an array or buffer heap value",
        ));
    };
    let object = context.heap.get(target)?;
    match object.kind() {
        "array" | "buffer" => Ok(Value::Pointer(RawPointer::new(
            reference.address,
            object.kind(),
            index,
            "",
            reference.generation,
            "",
        ))),
        _ => Err(TinyOneError::runtime(
            "ptr(value, index) expects an array or buffer heap value",
        )),
    }
}

fn runtime_make_field_pointer(
    context: &TinyRuntimeContext,
    target: &Value,
    field_value: &Value,
) -> Result<Value> {
    let Value::Heap(reference) = target else {
        return Err(TinyOneError::runtime(
            "fieldptr() expects a struct heap value",
        ));
    };
    let object = context.heap.get(target)?;
    let HeapData::Struct(fields) = &object.data else {
        return Err(TinyOneError::runtime(
            "fieldptr() expects a struct heap value",
        ));
    };
    let field = expect_string(context, field_value, "fieldptr")?;
    if !fields.iter().any(|(name, _)| name == &field) {
        return Err(TinyOneError::runtime(format!(
            "Unknown field {field:?} on struct {:?}",
            object.type_name
        )));
    }
    Ok(Value::Pointer(RawPointer::new(
        reference.address,
        "field",
        0,
        field,
        reference.generation,
        "",
    )))
}

fn runtime_pointer_address(context: &TinyRuntimeContext, value: &Value) -> Result<Value> {
    match value {
        Value::Pointer(pointer) => {
            validate_pointer_base(context, pointer, "ptr_addr")?;
            Ok(Value::Int(pointer.address as i64))
        }
        Value::Heap(reference) => {
            context.heap.get(value)?;
            Ok(Value::Int(reference.address as i64))
        }
        _ => Err(TinyOneError::runtime(
            "ptr_addr() expects a heap value or raw pointer",
        )),
    }
}

fn runtime_pointer_at(context: &TinyRuntimeContext, address: &Value) -> Result<Value> {
    let address = expect_int(address, "ptr_at")?;
    if address < 0 {
        return Err(TinyOneError::runtime(format!(
            "Invalid heap pointer {address}"
        )));
    }
    let generation = context.heap.current_generation(address as usize)?;
    Ok(Value::Pointer(RawPointer::new(
        address as usize,
        "object",
        0,
        "",
        generation,
        "",
    )))
}

fn runtime_pointer_add(
    context: &TinyRuntimeContext,
    pointer: &Value,
    offset: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_add")?;
    validate_pointer_base(context, &pointer, "ptr_add")?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to null",
        ));
    }
    let offset = expect_int(offset, "ptr_add")?;
    match pointer.kind.as_str() {
        "object" => {
            if offset != 0 {
                return Err(TinyOneError::runtime(
                    "Object pointer arithmetic requires an array or buffer pointer",
                ));
            }
            Ok(Value::Pointer(pointer))
        }
        "array" | "buffer" => Ok(Value::Pointer(RawPointer::new(
            pointer.address,
            pointer.kind,
            pointer.index + offset,
            pointer.field,
            pointer.generation,
            pointer.cast,
        ))),
        "field" => Err(TinyOneError::runtime(
            "Cannot apply pointer arithmetic to field pointers",
        )),
        _ => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn runtime_pointer_load(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_load")?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot load through null"));
    }
    match pointer.kind.as_str() {
        "object" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            if let HeapData::Cell(value) = &object.data {
                Ok(value.clone())
            } else {
                Ok(Value::Heap(context.heap.ref_at(pointer.address)?))
            }
        }
        "array" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            if pointer.index < 0 || pointer.index as usize >= values.len() {
                return Err(TinyOneError::runtime(format!(
                    "Array pointer index {} out of bounds",
                    pointer.index
                )));
            }
            Ok(values[pointer.index as usize].clone())
        }
        "buffer" => Err(TinyOneError::runtime(
            "Use read8/read16/read32 for buffer pointers",
        )),
        "field" => {
            let object = context
                .heap
                .get_address(pointer.address, pointer.generation)?;
            let HeapData::Struct(fields) = &object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            fields
                .iter()
                .find(|(name, _)| name == &pointer.field)
                .map(|(_, value)| value.clone())
                .ok_or_else(|| {
                    TinyOneError::runtime(format!(
                        "Unknown field {:?} on struct {:?}",
                        pointer.field, object.type_name
                    ))
                })
        }
        _ => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn runtime_pointer_store(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    value: Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_store")?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime("Cannot store through null"));
    }
    match pointer.kind.as_str() {
        "object" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Cell(cell) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Object raw pointers can only store through pointer cells; use array or field pointers for aggregates",
                ));
            };
            *cell = value.clone();
            Ok(value)
        }
        "array" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let HeapData::Array(values) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Array pointer no longer points at an array",
                ));
            };
            if pointer.index < 0 || pointer.index as usize >= values.len() {
                return Err(TinyOneError::runtime(format!(
                    "Array pointer index {} out of bounds",
                    pointer.index
                )));
            }
            values[pointer.index as usize] = value.clone();
            Ok(value)
        }
        "buffer" => Err(TinyOneError::runtime(
            "Use write8/write16/write32 for buffer pointers",
        )),
        "field" => {
            let object = context
                .heap
                .get_address_mut(pointer.address, pointer.generation)?;
            let type_name = object.type_name.clone();
            let HeapData::Struct(fields) = &mut object.data else {
                return Err(TinyOneError::runtime(
                    "Field pointer no longer points at a struct",
                ));
            };
            if let Some((_, field_value)) =
                fields.iter_mut().find(|(name, _)| name == &pointer.field)
            {
                *field_value = value.clone();
                Ok(value)
            } else {
                Err(TinyOneError::runtime(format!(
                    "Unknown field {:?} on struct {type_name:?}",
                    pointer.field
                )))
            }
        }
        _ => Err(TinyOneError::runtime(format!(
            "Unknown raw pointer kind {:?}",
            pointer.kind
        ))),
    }
}

fn runtime_pointer_type(context: &mut TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_type")?;
    validate_pointer_base(context, &pointer, "ptr_type")?;
    let text = if pointer.cast.is_empty() {
        pointer.kind
    } else {
        pointer.cast
    };
    Ok(Value::Heap(context.heap.alloc_string(text)))
}

fn runtime_pointer_base(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_base")?;
    validate_pointer_base(context, &pointer, "ptr_base")?;
    Ok(Value::Int(pointer.address as i64))
}

fn runtime_pointer_offset(context: &TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_offset")?;
    validate_pointer_base(context, &pointer, "ptr_offset")?;
    Ok(Value::Int(
        if pointer.kind == "array" || pointer.kind == "buffer" {
            pointer.index
        } else {
            0
        },
    ))
}

fn runtime_pointer_kind(context: &mut TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_kind")?;
    validate_pointer_base(context, &pointer, "ptr_kind")?;
    Ok(Value::Heap(context.heap.alloc_string(pointer.kind)))
}

fn runtime_pointer_field(context: &mut TinyRuntimeContext, pointer: &Value) -> Result<Value> {
    let pointer = expect_pointer(pointer, "ptr_field")?;
    validate_pointer_base(context, &pointer, "ptr_field")?;
    let field = if pointer.kind == "field" {
        pointer.field
    } else {
        String::new()
    };
    Ok(Value::Heap(context.heap.alloc_string(field)))
}

fn runtime_pointer_eq(context: &TinyRuntimeContext, lhs: &Value, rhs: &Value) -> Result<Value> {
    let lhs = expect_pointer(lhs, "ptr_eq")?;
    let rhs = expect_pointer(rhs, "ptr_eq")?;
    validate_pointer_base(context, &lhs, "ptr_eq")?;
    validate_pointer_base(context, &rhs, "ptr_eq")?;
    Ok(Value::Int(
        (pointer_identity(&lhs) == pointer_identity(&rhs)) as i64,
    ))
}

fn runtime_cast_pointer(
    context: &TinyRuntimeContext,
    pointer: &Value,
    type_value: &Value,
) -> Result<Value> {
    let pointer = expect_pointer(pointer, "cast_ptr")?;
    validate_pointer_base(context, &pointer, "cast_ptr")?;
    let type_name = expect_string(context, type_value, "cast_ptr")?;
    match type_name.as_str() {
        "u8" | "u16" | "u32" | "i8" | "i16" | "i32" => Ok(Value::Pointer(RawPointer::new(
            pointer.address,
            pointer.kind,
            pointer.index,
            pointer.field,
            pointer.generation,
            type_name,
        ))),
        _ => Err(TinyOneError::runtime(format!(
            "Unsupported pointer cast {type_name:?}"
        ))),
    }
}

fn runtime_make_buffer(context: &mut TinyRuntimeContext, size: &Value) -> Result<Value> {
    let size = expect_int(size, "buffer")?;
    if size < 0 {
        return Err(TinyOneError::runtime("buffer() size must be non-negative"));
    }
    Ok(Value::Heap(context.heap.alloc_buffer(size as usize)))
}

fn buffer_pointer<'a>(
    context: &'a mut TinyRuntimeContext,
    pointer: &Value,
    operation: &str,
) -> Result<(&'a mut Vec<u8>, i64)> {
    let pointer = expect_pointer(pointer, operation)?;
    if pointer.kind == "null" && pointer.address == 0 {
        return Err(TinyOneError::runtime(format!(
            "{operation} cannot use null"
        )));
    }
    if pointer.kind != "buffer" {
        return Err(TinyOneError::runtime(format!(
            "{operation} expects a buffer pointer"
        )));
    }
    let object = context
        .heap
        .get_address_mut(pointer.address, pointer.generation)?;
    let HeapData::Buffer(data) = &mut object.data else {
        return Err(TinyOneError::runtime(
            "Buffer pointer no longer points at a buffer",
        ));
    };
    Ok((data, pointer.index))
}

fn runtime_read_uint(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    width: usize,
    operation: &str,
) -> Result<Value> {
    let (data, offset) = buffer_pointer(context, pointer, operation)?;
    if offset < 0 || offset as usize + width > data.len() {
        return Err(TinyOneError::runtime(format!(
            "{operation} out of bounds at byte offset {offset}"
        )));
    }
    let mut value = 0u32;
    for i in 0..width {
        value |= (data[offset as usize + i] as u32) << (i * 8);
    }
    Ok(Value::Int(value as i64))
}

fn runtime_write_uint(
    context: &mut TinyRuntimeContext,
    pointer: &Value,
    value: &Value,
    width: usize,
    operation: &str,
) -> Result<Value> {
    let value_int = expect_int(value, operation)?;
    let max_value = (1i64 << (width * 8)) - 1;
    if value_int < 0 || value_int > max_value {
        return Err(TinyOneError::runtime(format!(
            "{operation} value must be in range 0..{max_value}"
        )));
    }
    let (data, offset) = buffer_pointer(context, pointer, operation)?;
    if offset < 0 || offset as usize + width > data.len() {
        return Err(TinyOneError::runtime(format!(
            "{operation} out of bounds at byte offset {offset}"
        )));
    }
    for i in 0..width {
        data[offset as usize + i] = ((value_int >> (i * 8)) & 0xff) as u8;
    }
    Ok(Value::Int(value_int))
}

fn runtime_call_builtin(
    context: &mut TinyRuntimeContext,
    builtin_index: usize,
    args: Vec<Value>,
) -> Result<Value> {
    let builtin = BUILTINS
        .get(builtin_index)
        .ok_or_else(|| TinyOneError::runtime(format!("Invalid builtin index {builtin_index}")))?;
    if args.len() < builtin.min_args || args.len() > builtin.max_args {
        return Err(TinyOneError::runtime(format!(
            "Builtin {:?} expects {}..{} argument(s), got {}",
            builtin.name,
            builtin.min_args,
            builtin.max_args,
            args.len()
        )));
    }
    match builtin.name {
        "len" => {
            let object = context.heap.get(&args[0])?;
            let len = match &object.data {
                HeapData::Array(values) => values.len(),
                HeapData::String(text) => text.chars().count(),
                HeapData::Buffer(data) => data.len(),
                HeapData::Struct(fields) => fields.len(),
                HeapData::Cell(_) => {
                    return Err(TinyOneError::runtime("len() does not support cell"));
                }
            };
            Ok(Value::Int(len as i64))
        }
        "array" => {
            let count = expect_int(&args[0], "array")?;
            if count < 0 {
                return Err(TinyOneError::runtime("array() length must be non-negative"));
            }
            Ok(Value::Heap(
                context
                    .heap
                    .alloc_array(vec![args[1].clone(); count as usize]),
            ))
        }
        "alloc" => Ok(Value::Heap(context.heap.alloc_cell(args[0].clone()))),
        "load" => {
            let object = context.heap.get(&args[0])?;
            let HeapData::Cell(value) = &object.data else {
                return Err(TinyOneError::runtime("load() expects a pointer cell"));
            };
            Ok(value.clone())
        }
        "store" => {
            let object = context.heap.get_mut(&args[0])?;
            let HeapData::Cell(value) = &mut object.data else {
                return Err(TinyOneError::runtime("store() expects a pointer cell"));
            };
            *value = args[1].clone();
            Ok(args[1].clone())
        }
        "free" => {
            context.heap.free(&args[0])?;
            Ok(Value::Int(0))
        }
        "read" => {
            let raw = context.read_raw()?;
            if looks_like_int(&raw) {
                Ok(Value::Int(raw.parse().map_err(|_| {
                    TinyOneError::runtime("read() integer input is out of range")
                })?))
            } else {
                Ok(Value::Heap(context.heap.alloc_string(raw)))
            }
        }
        "read_int" => {
            let raw = context.read_raw()?;
            if !looks_like_int(&raw) {
                return Err(TinyOneError::runtime(format!(
                    "read_int() expected integer input, got {raw:?}"
                )));
            }
            Ok(Value::Int(raw.parse().map_err(|_| {
                TinyOneError::runtime("read_int() integer input is out of range")
            })?))
        }
        "read_str" => {
            let raw = context.read_raw()?;
            Ok(Value::Heap(context.heap.alloc_string(raw)))
        }
        "to_int" => match &args[0] {
            Value::Int(value) => Ok(Value::Int(*value)),
            _ => {
                let text = expect_string(context, &args[0], "to_int")?;
                if !looks_like_int(&text) {
                    return Err(TinyOneError::runtime(
                        "to_int() expects an integer or numeric string",
                    ));
                }
                Ok(Value::Int(text.parse().map_err(|_| {
                    TinyOneError::runtime("to_int() integer input is out of range")
                })?))
            }
        },
        "ptr" => runtime_make_pointer(context, &args),
        "fieldptr" => runtime_make_field_pointer(context, &args[0], &args[1]),
        "ptr_addr" => runtime_pointer_address(context, &args[0]),
        "ptr_at" => runtime_pointer_at(context, &args[0]),
        "ptr_add" => runtime_pointer_add(context, &args[0], &args[1]),
        "ptr_load" => runtime_pointer_load(context, &args[0]),
        "ptr_store" => runtime_pointer_store(context, &args[0], args[1].clone()),
        "ptr_type" => runtime_pointer_type(context, &args[0]),
        "buffer" => runtime_make_buffer(context, &args[0]),
        "is_null" => {
            let pointer = expect_pointer(&args[0], "is_null")?;
            validate_pointer_base(context, &pointer, "is_null")?;
            Ok(Value::Int(
                (pointer.kind == "null" && pointer.address == 0) as i64,
            ))
        }
        "ptr_eq" => runtime_pointer_eq(context, &args[0], &args[1]),
        "ptr_ne" => match runtime_pointer_eq(context, &args[0], &args[1])? {
            Value::Int(0) => Ok(Value::Int(1)),
            _ => Ok(Value::Int(0)),
        },
        "ptr_base" => runtime_pointer_base(context, &args[0]),
        "ptr_offset" => runtime_pointer_offset(context, &args[0]),
        "ptr_kind" => runtime_pointer_kind(context, &args[0]),
        "ptr_field" => runtime_pointer_field(context, &args[0]),
        "read8" => runtime_read_uint(context, &args[0], 1, "read8"),
        "write8" => runtime_write_uint(context, &args[0], &args[1], 1, "write8"),
        "read16" => runtime_read_uint(context, &args[0], 2, "read16"),
        "write16" => runtime_write_uint(context, &args[0], &args[1], 2, "write16"),
        "read32" => runtime_read_uint(context, &args[0], 4, "read32"),
        "write32" => runtime_write_uint(context, &args[0], &args[1], 4, "write32"),
        "cast_ptr" => runtime_cast_pointer(context, &args[0], &args[1]),
        "push" => runtime_array_push(context, &args[0], args[1].clone()),
        "pop" => runtime_array_pop(context, &args[0]),
        _ => Err(TinyOneError::runtime(format!(
            "Missing builtin handler {:?}",
            builtin.name
        ))),
    }
}

fn looks_like_int(text: &str) -> bool {
    if text.is_empty() {
        return false;
    }
    let rest = text.strip_prefix(['+', '-']).unwrap_or(text);
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

fn runtime_format(context: &TinyRuntimeContext, value: &Value) -> Result<String> {
    runtime_format_inner(context, value, &mut HashSet::new())
}

fn runtime_format_inner(
    context: &TinyRuntimeContext,
    value: &Value,
    seen: &mut HashSet<usize>,
) -> Result<String> {
    match value {
        Value::Int(value) => Ok(value.to_string()),
        Value::Pointer(pointer) => {
            let suffix = if pointer.cast.is_empty() {
                String::new()
            } else {
                format!(":{}", pointer.cast)
            };
            if pointer.kind == "null" && pointer.address == 0 {
                Ok("null".to_string())
            } else if pointer.kind == "array" {
                Ok(format!(
                    "ptr(array@{}[{}]{suffix})",
                    pointer.address, pointer.index
                ))
            } else if pointer.kind == "buffer" {
                Ok(format!(
                    "ptr(buffer@{}+{}{suffix})",
                    pointer.address, pointer.index
                ))
            } else if pointer.kind == "field" {
                Ok(format!(
                    "ptr(field@{}.{}{suffix})",
                    pointer.address, pointer.field
                ))
            } else {
                Ok(format!("ptr({}@{}{suffix})", pointer.kind, pointer.address))
            }
        }
        Value::Heap(reference) => {
            let object = context.heap.get(value)?.clone();
            if seen.contains(&reference.address) {
                return Ok(format!("&{}<cycle>", reference.address));
            }
            seen.insert(reference.address);
            let rendered = match object.data {
                HeapData::String(text) => Ok(text),
                HeapData::Array(values) => {
                    let mut parts = Vec::with_capacity(values.len());
                    for item in &values {
                        parts.push(runtime_format_inner(context, item, seen)?);
                    }
                    Ok(format!("[{}]", parts.join(", ")))
                }
                HeapData::Buffer(data) => Ok(format!(
                    "buffer[{}]",
                    data.iter()
                        .map(|byte| format!("{byte:02x}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                )),
                HeapData::Struct(fields) => {
                    let mut parts = Vec::with_capacity(fields.len());
                    for (name, value) in &fields {
                        parts.push(format!(
                            "{name}: {}",
                            runtime_format_inner(context, value, seen)?
                        ));
                    }
                    Ok(format!("{}{{{}}}", object.type_name, parts.join(", ")))
                }
                HeapData::Cell(value) => Ok(format!(
                    "&{}({})",
                    reference.address,
                    runtime_format_inner(context, &value, seen)?
                )),
            };
            seen.remove(&reference.address);
            rendered
        }
    }
}

fn runtime_print(
    context: &TinyRuntimeContext,
    stdout: &mut dyn Write,
    value: &Value,
) -> Result<()> {
    writeln!(stdout, "{}", runtime_format(context, value)?)
        .map_err(|error| TinyOneError::runtime(format!("Write error: {error}")))
}

#[derive(Debug, Clone)]
pub struct TinyMemory {
    values: Vec<Value>,
}

impl TinyMemory {
    pub fn new(slot_count: usize) -> Self {
        Self {
            values: vec![Value::default(); slot_count],
        }
    }

    pub fn reset(&mut self) {
        self.values.fill(Value::default());
    }

    pub fn load(&self, slot: usize) -> Result<Value> {
        self.values
            .get(slot)
            .cloned()
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))
    }

    pub fn store(&mut self, slot: usize, value: Value) -> Result<()> {
        let target = self
            .values
            .get_mut(slot)
            .ok_or_else(|| TinyOneError::runtime(format!("Invalid memory slot {slot}")))?;
        *target = value;
        Ok(())
    }

    pub fn snapshot(&self) -> Vec<Value> {
        self.values.clone()
    }
}

pub struct VM<'a> {
    program: &'a Program,
    memory: TinyMemory,
    context: TinyRuntimeContext,
}

impl<'a> VM<'a> {
    pub fn new(program: &'a Program, memory: TinyMemory, inputs: Vec<String>) -> Self {
        Self {
            program,
            memory,
            context: TinyRuntimeContext::new(inputs),
        }
    }

    pub fn run(mut self, stdout: &mut dyn Write) -> Result<TinyMemory> {
        let mut memory = self.memory.clone();
        self.run_chunk(&self.program.code, &mut memory, stdout, "main")?;
        Ok(memory)
    }

    fn run_chunk(
        &mut self,
        code: &[Instr],
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
        chunk_name: &str,
    ) -> Result<Option<Value>> {
        let mut stack: Vec<Value> = Vec::new();
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
                    let text = self.program.strings[instr.arg as usize].clone();
                    stack.push(Value::Heap(self.context.heap.alloc_string(text)));
                }
                Op::Load => stack.push(memory.load(instr.arg as usize)?),
                Op::Store => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    memory.store(instr.arg as usize, value)?;
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
                Op::Jump => pc = instr.arg as usize,
                Op::JumpIfZero => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    if runtime_is_false(&value) {
                        pc = instr.arg as usize;
                    }
                }
                Op::Call => {
                    let result = self.call_function(
                        instr.arg as usize,
                        &mut stack,
                        instr.arg2 as usize,
                        stdout,
                    )?;
                    stack.push(result);
                }
                Op::MakeArray => {
                    let mut values = Vec::with_capacity(instr.arg as usize);
                    for _ in 0..instr.arg {
                        values.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    values.reverse();
                    stack.push(runtime_make_array(&mut self.context, values));
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
                    let mut values = Vec::with_capacity(instr.arg2 as usize);
                    for _ in 0..instr.arg2 {
                        values.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    values.reverse();
                    let struct_def = &self.program.structs[instr.arg as usize];
                    stack.push(runtime_make_struct(
                        &mut self.context,
                        &struct_def.name,
                        &struct_def.fields,
                        values,
                    ));
                }
                Op::GetField => {
                    let target = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let field = &self.program.fields[instr.arg as usize];
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                Op::SetField => {
                    let value = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let target = stack
                        .pop()
                        .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?;
                    let field = &self.program.fields[instr.arg as usize];
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                Op::Builtin => {
                    let mut args = Vec::with_capacity(instr.arg2 as usize);
                    for _ in 0..instr.arg2 {
                        args.push(
                            stack
                                .pop()
                                .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
                        );
                    }
                    args.reverse();
                    stack.push(runtime_call_builtin(
                        &mut self.context,
                        instr.arg as usize,
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
        let function = &self.program.functions[function_index];
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(
                caller_stack
                    .pop()
                    .ok_or_else(|| TinyOneError::runtime("Stack underflow"))?,
            );
        }
        args.reverse();
        let mut memory = TinyMemory::new(function.slot_count);
        for (slot, value) in args.into_iter().enumerate() {
            memory.store(slot, value)?;
        }
        self.run_chunk(&function.code, &mut memory, stdout, &function.name)?
            .ok_or_else(|| {
                TinyOneError::runtime(format!("Function {:?} returned no value", function.name))
            })
    }
}

pub fn compile_source(source: &str) -> Result<Program> {
    compile_source_with_filename(source, "<source>")
}

pub fn lex_source(source: &str) -> Result<usize> {
    Ok(Lexer::new(source, "<source>").tokenize()?.len())
}

pub fn compile_source_unoptimized(source: &str) -> Result<Program> {
    compile_source_unoptimized_with_filename(source, "<source>")
}

pub fn compile_source_unoptimized_with_filename(source: &str, filename: &str) -> Result<Program> {
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let mut compiler = Compiler::new(source, filename, None, false, "", shared)?;
    compiler.compile()
}

pub fn optimize_program(program: Program) -> Program {
    PeepholeOptimizer::optimize(program)
}

pub fn compile_source_with_filename(source: &str, filename: &str) -> Result<Program> {
    let program = compile_source_unoptimized_with_filename(source, filename)?;
    let program = PeepholeOptimizer::optimize(program);
    BytecodeVerifier::verify(&program)?;
    Ok(program)
}

pub fn compile_file(path: impl AsRef<Path>) -> Result<Program> {
    let path = path
        .as_ref()
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| TinyOneError::compile(format!("File error: {error}")))?;
    let shared = Rc::new(RefCell::new(CompilerSharedState::default()));
    let mut compiler = Compiler::new(
        source,
        path.to_string_lossy().to_string(),
        Some(resolve_import),
        false,
        "",
        shared,
    )?;
    let program = compiler.compile()?;
    let program = PeepholeOptimizer::optimize(program);
    BytecodeVerifier::verify(&program)?;
    Ok(program)
}

pub fn load_artifact(path: impl AsRef<Path>) -> Result<Program> {
    let text = fs::read_to_string(path)
        .map_err(|error| TinyOneError::compile(format!("Artifact read error: {error}")))?;
    let data = serde_json::from_str(&text)
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    Program::from_artifact(data)
}

pub fn write_artifact(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let text = serde_json::to_string_pretty(&program.to_artifact())
        .map_err(|error| TinyOneError::compile(format!("Artifact JSON error: {error}")))?;
    fs::write(path, format!("{text}\n"))
        .map_err(|error| TinyOneError::compile(format!("Artifact write error: {error}")))
}

const HOT_BACK_EDGE_THRESHOLD: u16 = 8;

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitStats {
    pub compiled_chunks: usize,
    pub compiled_ops: usize,
    pub hot_back_edges: u64,
    pub hot_ranges: usize,
    pub quickened_ops: usize,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct JitCacheStats {
    pub programs: usize,
    pub compiled_chunks: usize,
    pub compiled_ops: usize,
    pub hot_back_edges: u64,
    pub hot_ranges: usize,
    pub quickened_ops: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JitOp {
    PushInt(i64),
    PushNull,
    PushString(usize),
    Load(usize),
    Store(usize),
    Add,
    AddInt,
    Sub,
    SubInt,
    Mul,
    MulInt,
    Div,
    DivInt,
    Neg,
    Compare(Op),
    CompareInt(Op),
    Jump(usize),
    JumpHot(usize),
    JumpIfZero(usize),
    JumpIfZeroHot(usize),
    Call(usize, usize),
    MakeArray(usize),
    Index,
    SetIndex,
    MakeStruct(usize, usize),
    GetField(usize),
    SetField(usize),
    Builtin(usize, usize),
    Return,
    Print,
    Halt,
}

impl JitOp {
    fn from_instr(instr: Instr) -> Self {
        match instr.op {
            Op::PushInt => Self::PushInt(instr.arg),
            Op::PushNull => Self::PushNull,
            Op::PushString => Self::PushString(instr.arg as usize),
            Op::Load => Self::Load(instr.arg as usize),
            Op::Store => Self::Store(instr.arg as usize),
            Op::Add => Self::Add,
            Op::Sub => Self::Sub,
            Op::Mul => Self::Mul,
            Op::Div => Self::Div,
            Op::Neg => Self::Neg,
            Op::Lt | Op::Lte | Op::Gt | Op::Gte | Op::Eq | Op::Ne => Self::Compare(instr.op),
            Op::Jump => Self::Jump(instr.arg as usize),
            Op::JumpIfZero => Self::JumpIfZero(instr.arg as usize),
            Op::Call => Self::Call(instr.arg as usize, instr.arg2 as usize),
            Op::MakeArray => Self::MakeArray(instr.arg as usize),
            Op::Index => Self::Index,
            Op::SetIndex => Self::SetIndex,
            Op::MakeStruct => Self::MakeStruct(instr.arg as usize, instr.arg2 as usize),
            Op::GetField => Self::GetField(instr.arg as usize),
            Op::SetField => Self::SetField(instr.arg as usize),
            Op::Builtin => Self::Builtin(instr.arg as usize, instr.arg2 as usize),
            Op::Return => Self::Return,
            Op::Print => Self::Print,
            Op::Halt => Self::Halt,
        }
    }

    fn quickened(self) -> Self {
        match self {
            Self::Add => Self::AddInt,
            Self::Sub => Self::SubInt,
            Self::Mul => Self::MulInt,
            Self::Div => Self::DivInt,
            Self::Compare(op) => Self::CompareInt(op),
            Self::Jump(target) => Self::JumpHot(target),
            Self::JumpIfZero(target) => Self::JumpIfZeroHot(target),
            _ => self,
        }
    }

    fn listing(self) -> String {
        match self {
            Self::PushInt(value) => format!("push.i {value}"),
            Self::PushNull => "push.null".to_string(),
            Self::PushString(index) => format!("push.str {index}"),
            Self::Load(slot) => format!("load {slot}"),
            Self::Store(slot) => format!("store {slot}"),
            Self::Add => "add".to_string(),
            Self::AddInt => "add.int".to_string(),
            Self::Sub => "sub".to_string(),
            Self::SubInt => "sub.int".to_string(),
            Self::Mul => "mul".to_string(),
            Self::MulInt => "mul.int".to_string(),
            Self::Div => "div".to_string(),
            Self::DivInt => "div.int".to_string(),
            Self::Neg => "neg".to_string(),
            Self::Compare(op) => format!("cmp.{}", op.name().to_ascii_lowercase()),
            Self::CompareInt(op) => format!("cmp.int.{}", op.name().to_ascii_lowercase()),
            Self::Jump(target) => format!("jmp {target}"),
            Self::JumpHot(target) => format!("jmp.hot {target}"),
            Self::JumpIfZero(target) => format!("jz {target}"),
            Self::JumpIfZeroHot(target) => format!("jz.hot {target}"),
            Self::Call(function, arg_count) => format!("call f{function} argc={arg_count}"),
            Self::MakeArray(count) => format!("array {count}"),
            Self::Index => "index".to_string(),
            Self::SetIndex => "set.index".to_string(),
            Self::MakeStruct(index, field_count) => format!("struct s{index} fields={field_count}"),
            Self::GetField(field) => format!("get.field {field}"),
            Self::SetField(field) => format!("set.field {field}"),
            Self::Builtin(index, arg_count) => format!("builtin b{index} argc={arg_count}"),
            Self::Return => "return".to_string(),
            Self::Print => "print".to_string(),
            Self::Halt => "halt".to_string(),
        }
    }
}

#[derive(Debug, Clone)]
struct JitChunk {
    name: String,
    slot_count: usize,
    ops: Vec<JitOp>,
    edge_counts: Vec<u16>,
}

impl JitChunk {
    fn compile(name: impl Into<String>, slot_count: usize, code: &[Instr]) -> Self {
        let ops = code
            .iter()
            .copied()
            .map(JitOp::from_instr)
            .collect::<Vec<_>>();
        Self {
            name: name.into(),
            slot_count,
            edge_counts: vec![0; ops.len()],
            ops,
        }
    }

    fn promote_range(&mut self, start: usize, end: usize) -> usize {
        let start = start.min(self.ops.len());
        let end = end.min(self.ops.len());
        let mut changed = 0usize;
        for op in &mut self.ops[start..end] {
            let quickened = op.quickened();
            if quickened != *op {
                *op = quickened;
                changed += 1;
            }
        }
        changed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct JitFunction {
    name: String,
    param_count: usize,
    slot_count: usize,
    chunk_index: usize,
}

#[derive(Debug, Clone)]
pub struct JitProgram {
    fingerprint: String,
    chunks: Vec<JitChunk>,
    functions: Vec<JitFunction>,
    strings: Vec<String>,
    structs: Vec<StructDef>,
    fields: Vec<String>,
    stats: JitStats,
}

impl JitProgram {
    pub fn compile(program: &Program) -> Self {
        Self::compile_with_fingerprint(program, program.fingerprint())
    }

    fn compile_with_fingerprint(program: &Program, fingerprint: String) -> Self {
        let mut chunks = vec![JitChunk::compile("main", program.slot_count, &program.code)];
        let mut functions = Vec::with_capacity(program.functions.len());
        for function in &program.functions {
            let chunk_index = chunks.len();
            chunks.push(JitChunk::compile(
                function.name.clone(),
                function.slot_count,
                &function.code,
            ));
            functions.push(JitFunction {
                name: function.name.clone(),
                param_count: function.param_count,
                slot_count: function.slot_count,
                chunk_index,
            });
        }
        let compiled_ops = chunks.iter().map(|chunk| chunk.ops.len()).sum();
        let compiled_chunks = chunks.len();
        Self {
            fingerprint,
            chunks,
            functions,
            strings: program.strings.clone(),
            structs: program.structs.clone(),
            fields: program.fields.clone(),
            stats: JitStats {
                compiled_chunks,
                compiled_ops,
                ..JitStats::default()
            },
        }
    }

    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    pub fn stats(&self) -> JitStats {
        self.stats
    }

    pub fn listing(&self) -> String {
        use std::fmt::Write as _;

        let mut out = String::new();
        writeln!(&mut out, "; tinyone adaptive-jit {}", self.fingerprint).expect("write string");
        writeln!(
            &mut out,
            "; chunks={} ops={} hot_back_edges={} hot_ranges={} quickened_ops={}",
            self.stats.compiled_chunks,
            self.stats.compiled_ops,
            self.stats.hot_back_edges,
            self.stats.hot_ranges,
            self.stats.quickened_ops
        )
        .expect("write string");
        for (chunk_index, chunk) in self.chunks.iter().enumerate() {
            writeln!(
                &mut out,
                ".chunk {chunk_index} {} slots={} ops={}",
                chunk.name,
                chunk.slot_count,
                chunk.ops.len()
            )
            .expect("write string");
            for (pc, op) in chunk.ops.iter().enumerate() {
                writeln!(&mut out, "  {pc:04} {}", op.listing()).expect("write string");
            }
        }
        out
    }

    pub fn run(&mut self, stdout: &mut dyn Write, inputs: Vec<String>) -> Result<TinyMemory> {
        JitVm::new(self, inputs).run(stdout)
    }

    fn record_back_edge(&mut self, chunk_index: usize, op_pc: usize, target: usize) {
        if target >= op_pc {
            return;
        }
        self.stats.hot_back_edges += 1;
        let changed = {
            let Some(chunk) = self.chunks.get_mut(chunk_index) else {
                return;
            };
            let Some(counter) = chunk.edge_counts.get_mut(op_pc) else {
                return;
            };
            *counter = counter.saturating_add(1);
            if *counter == HOT_BACK_EDGE_THRESHOLD {
                chunk.promote_range(target, op_pc + 1)
            } else {
                0
            }
        };
        if changed > 0 {
            self.stats.hot_ranges += 1;
            self.stats.quickened_ops += changed;
        }
    }
}

pub fn write_jit_listing(program: &Program, path: impl AsRef<Path>) -> Result<()> {
    let compiled = JitProgram::compile(program);
    fs::write(path, compiled.listing())
        .map_err(|error| TinyOneError::compile(format!("JIT listing write error: {error}")))
}

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

struct JitVm<'a> {
    program: &'a mut JitProgram,
    context: TinyRuntimeContext,
}

impl<'a> JitVm<'a> {
    fn new(program: &'a mut JitProgram, inputs: Vec<String>) -> Self {
        Self {
            program,
            context: TinyRuntimeContext::new(inputs),
        }
    }

    fn run(mut self, stdout: &mut dyn Write) -> Result<TinyMemory> {
        let slot_count = self.program.chunks[0].slot_count;
        let mut memory = TinyMemory::new(slot_count);
        self.run_chunk(0, &mut memory, stdout)?;
        Ok(memory)
    }

    fn run_chunk(
        &mut self,
        chunk_index: usize,
        memory: &mut TinyMemory,
        stdout: &mut dyn Write,
    ) -> Result<Option<Value>> {
        let mut stack: Vec<Value> = Vec::new();
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
                JitOp::PushInt(value) => stack.push(Value::Int(value)),
                JitOp::PushNull => stack.push(runtime_null()),
                JitOp::PushString(index) => {
                    let text = self.program.strings[index].clone();
                    stack.push(Value::Heap(self.context.heap.alloc_string(text)));
                }
                JitOp::Load(slot) => stack.push(memory.load(slot)?),
                JitOp::Store(slot) => {
                    let value = jit_pop(&mut stack)?;
                    memory.store(slot, value)?;
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
                    let result =
                        self.call_function(function_index, &mut stack, arg_count, stdout)?;
                    stack.push(result);
                }
                JitOp::MakeArray(count) => {
                    let mut values = Vec::with_capacity(count);
                    for _ in 0..count {
                        values.push(jit_pop(&mut stack)?);
                    }
                    values.reverse();
                    stack.push(runtime_make_array(&mut self.context, values));
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
                    let mut values = Vec::with_capacity(field_count);
                    for _ in 0..field_count {
                        values.push(jit_pop(&mut stack)?);
                    }
                    values.reverse();
                    let struct_def = &self.program.structs[struct_index];
                    stack.push(runtime_make_struct(
                        &mut self.context,
                        &struct_def.name,
                        &struct_def.fields,
                        values,
                    ));
                }
                JitOp::GetField(field_index) => {
                    let target = jit_pop(&mut stack)?;
                    let field = &self.program.fields[field_index];
                    stack.push(runtime_get_field(&self.context, target, field)?);
                }
                JitOp::SetField(field_index) => {
                    let value = jit_pop(&mut stack)?;
                    let target = jit_pop(&mut stack)?;
                    let field = &self.program.fields[field_index];
                    runtime_set_field(&mut self.context, target, field, value)?;
                }
                JitOp::Builtin(builtin_index, arg_count) => {
                    let mut args = Vec::with_capacity(arg_count);
                    for _ in 0..arg_count {
                        args.push(jit_pop(&mut stack)?);
                    }
                    args.reverse();
                    stack.push(runtime_call_builtin(
                        &mut self.context,
                        builtin_index,
                        args,
                    )?);
                }
                JitOp::Return => return Ok(Some(jit_pop(&mut stack)?)),
                JitOp::Print => {
                    let value = jit_pop(&mut stack)?;
                    runtime_print(&self.context, stdout, &value)?;
                }
                JitOp::Halt => {
                    if !stack.is_empty() {
                        let chunk_name = &self.program.chunks[chunk_index].name;
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
        let (chunk_index, slot_count, param_count, name) = {
            let function = self.program.functions.get(function_index).ok_or_else(|| {
                TinyOneError::runtime(format!("Invalid function index {function_index}"))
            })?;
            (
                function.chunk_index,
                function.slot_count,
                function.param_count,
                function.name.clone(),
            )
        };
        if arg_count != param_count {
            return Err(TinyOneError::runtime(format!(
                "Function {name:?} expects {param_count} argument(s), got {arg_count}"
            )));
        }
        let mut args = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            args.push(jit_pop(caller_stack)?);
        }
        args.reverse();
        let mut memory = TinyMemory::new(slot_count);
        for (slot, value) in args.into_iter().enumerate() {
            memory.store(slot, value)?;
        }
        self.run_chunk(chunk_index, &mut memory, stdout)?
            .ok_or_else(|| TinyOneError::runtime(format!("Function {name:?} returned no value")))
    }
}

#[derive(Debug, Default, Clone)]
pub struct JitCache {
    cache: HashMap<String, JitProgram>,
}

impl JitCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.cache.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cache.is_empty()
    }

    pub fn compile(&mut self, program: &Program) -> &JitProgram {
        &*self.compile_mut(program)
    }

    fn compile_mut(&mut self, program: &Program) -> &mut JitProgram {
        let key = program.fingerprint();
        self.cache
            .entry(key.clone())
            .or_insert_with(|| JitProgram::compile_with_fingerprint(program, key))
    }

    pub fn stats(&self) -> JitCacheStats {
        self.cache
            .values()
            .fold(JitCacheStats::default(), |mut stats, program| {
                let program_stats = program.stats();
                stats.programs += 1;
                stats.compiled_chunks += program_stats.compiled_chunks;
                stats.compiled_ops += program_stats.compiled_ops;
                stats.hot_back_edges += program_stats.hot_back_edges;
                stats.hot_ranges += program_stats.hot_ranges;
                stats.quickened_ops += program_stats.quickened_ops;
                stats
            })
    }

    pub fn run_program(
        &mut self,
        program: &Program,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        let compiled = self.compile_mut(program);
        compiled.run(stdout, inputs)
    }

    pub fn run_source(
        &mut self,
        source: &str,
        stdout: &mut dyn Write,
        inputs: Vec<String>,
    ) -> Result<TinyMemory> {
        let program = compile_source(source)?;
        self.run_program(&program, stdout, inputs)
    }
}

pub fn run_program(
    program: &Program,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    match mode {
        "vm" => VM::new(program, TinyMemory::new(program.slot_count), inputs).run(stdout),
        "jit" => {
            let mut cache = JitCache::new();
            cache.run_program(program, stdout, inputs)
        }
        _ => Err(TinyOneError::runtime(format!("Unsupported mode {mode:?}"))),
    }
}

pub fn run_source(
    source: &str,
    mode: &str,
    stdout: &mut dyn Write,
    inputs: Vec<String>,
) -> Result<TinyMemory> {
    let program = compile_source(source)?;
    run_program(&program, mode, stdout, inputs)
}

fn resolve_import(from_filename: &str, import_path: &str) -> Result<(String, String)> {
    let base = Path::new(from_filename)
        .canonicalize()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."));
    let path = resolve_manifest_import(&base, import_path)?
        .unwrap_or_else(|| base.join(import_path))
        .canonicalize()
        .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
    let source = fs::read_to_string(&path)
        .map_err(|error| TinyOneError::compile(format!("Import error: {error}")))?;
    Ok((path.to_string_lossy().to_string(), source))
}

fn resolve_manifest_import(base: &Path, import_path: &str) -> Result<Option<PathBuf>> {
    if !looks_like_module_key(import_path) {
        return Ok(None);
    }
    for directory in base.ancestors() {
        let manifest_path = directory.join("tinyone.json");
        if !manifest_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&manifest_path).map_err(|error| {
            TinyOneError::compile(format!("Package manifest read error: {error}"))
        })?;
        let data: JsonValue = serde_json::from_str(&text).map_err(|error| {
            TinyOneError::compile(format!("Package manifest JSON error: {error}"))
        })?;
        let modules = data
            .get("modules")
            .and_then(JsonValue::as_object)
            .ok_or_else(|| {
                TinyOneError::compile(format!(
                    "Package manifest {} must contain a modules object",
                    manifest_path.display()
                ))
            })?;
        let Some(target) = modules.get(import_path) else {
            continue;
        };
        let target = target.as_str().ok_or_else(|| {
            TinyOneError::compile(format!(
                "Package manifest module {import_path:?} in {} must be a string",
                manifest_path.display()
            ))
        })?;
        return Ok(Some(directory.join(target)));
    }
    Ok(None)
}

fn module_name_from_filename(filename: &str) -> String {
    sanitize_identifier(
        Path::new(filename)
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("module"),
    )
}

fn module_name_from_import(import_path: &str, filename: &str) -> String {
    if looks_like_module_key(import_path) {
        sanitize_identifier(import_path)
    } else {
        module_name_from_filename(filename)
    }
}

fn unique_module_name(state: &mut CompilerSharedState, base_name: &str, filename: &str) -> String {
    if state
        .module_name_owners
        .get(base_name)
        .map(|owner| owner == filename)
        .unwrap_or(true)
    {
        state
            .module_name_owners
            .insert(base_name.to_string(), filename.to_string());
        return base_name.to_string();
    }
    let digest = Blake2b512::digest(filename.as_bytes());
    let suffix = hex::encode(&digest[..4]);
    let mut name = format!("{base_name}_{suffix}");
    while state
        .module_name_owners
        .get(&name)
        .map(|owner| owner != filename)
        .unwrap_or(false)
    {
        let digest = Blake2b512::digest(format!("{filename}:{suffix}").as_bytes());
        name = format!("{}_{}", base_name, hex::encode(&digest[..4]));
    }
    state
        .module_name_owners
        .insert(name.clone(), filename.to_string());
    name
}

fn default_import_alias(import_path: &str) -> String {
    if looks_like_module_key(import_path) {
        sanitize_identifier(import_path)
    } else {
        sanitize_identifier(
            Path::new(import_path)
                .file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or("module"),
        )
    }
}

fn looks_like_module_key(import_path: &str) -> bool {
    !import_path.contains('/')
        && !import_path.contains('\\')
        && !import_path.starts_with('.')
        && !import_path.contains('.')
}

fn sanitize_identifier(text: &str) -> String {
    let mut out = text
        .chars()
        .map(|ch| {
            if ch == '_' || ch.is_alphanumeric() {
                ch
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string();
    if out.is_empty() || out.chars().next().is_some_and(|ch| ch.is_ascii_digit()) {
        out = format!("module_{out}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str, mode: &str) -> String {
        let mut out = Vec::new();
        run_source(source, mode, &mut out, Vec::new()).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn arithmetic_loops_and_functions() {
        let source = r#"
        fn mul_by_count(value, count) {
          let acc = 0
          while count > 0 {
            let acc = acc + value
            let count = count - 1
          }
          return acc
        }
        let i = 1
        let total = 0
        while i <= 8 {
          let total = total + mul_by_count(i, 3)
          let i = i + 1
        }
        print total
        "#;
        assert_eq!(run(source, "vm"), "108\n");
        assert_eq!(run(source, "jit"), "108\n");
    }

    #[test]
    fn heap_pointers_and_buffers() {
        let source = r#"
        struct Pair { left, right }
        let values = [10, 20, 30]
        let second = ptr(values, 1)
        print unsafe ptr_load(second)
        print unsafe ptr_store(unsafe ptr_add(second, 1), 77)
        print values[2]
        let pair = Pair(4, 5)
        let field = fieldptr(pair, "right")
        print unsafe ptr_load(field)
        print unsafe ptr_store(field, 99)
        print pair.right
        let mem = buffer(8)
        let p = ptr(mem, 0)
        print unsafe write16(unsafe ptr_add(p, 2), 4660)
        print unsafe read8(unsafe ptr_add(p, 2))
        print unsafe read8(unsafe ptr_add(p, 3))
        "#;
        assert_eq!(run(source, "vm"), "20\n77\n77\n5\n99\n99\n4660\n52\n18\n");
    }

    #[test]
    fn imports_and_artifact_roundtrip() {
        let root =
            std::env::temp_dir().join(format!("tinyone-rust-import-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("pairs.to"),
            r#"
            fn hidden(p) {
              return p.left + p.right + 1000
            }
            export struct Pair { left, right }
            export fn sum_pair(p) {
              return p.left + p.right
            }
            "#,
        )
        .unwrap();
        let main_path = root.join("main.to");
        fs::write(
            &main_path,
            r#"
            import "pairs.to" as pairs
            let pair = pairs.Pair(18, 24)
            print pairs.sum_pair(pair)
            "#,
        )
        .unwrap();

        let program = compile_file(&main_path).unwrap();
        assert_eq!(program.modules.len(), 1);
        assert_eq!(program.modules[0].exported_functions, vec!["sum_pair"]);
        assert_eq!(program.modules[0].exported_structs, vec!["Pair"]);

        let loaded = Program::from_artifact(program.to_artifact()).unwrap();
        assert_eq!(program.fingerprint(), loaded.fingerprint());

        let mut out = Vec::new();
        run_program(&loaded, "jit", &mut out, Vec::new()).unwrap();
        assert_eq!(String::from_utf8(out).unwrap(), "42\n");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn conditionals_break_and_continue() {
        let source = r#"
        let i = 0
        let total = 0
        while i < 10 {
          let i = i + 1
          if i == 3 {
            continue
          }
          if i == 8 {
            break
          } else {
            let total = total + i
          }
        }
        print total
        if total == 25 {
          print 1
        } else {
          print 0
        }
        "#;
        assert_eq!(run(source, "vm"), "25\n1\n");
        assert_eq!(run(source, "jit"), "25\n1\n");
    }

    #[test]
    fn dynamic_array_push_and_pop_storage() {
        let source = r#"
        let values = []
        let i = 0
        while i < 4 {
          let ignored = push(values, i * 2)
          let i = i + 1
        }
        print len(values)
        print values[2]
        print pop(values)
        print len(values)
        "#;
        assert_eq!(run(source, "vm"), "4\n4\n6\n3\n");
    }

    #[test]
    fn loop_control_requires_loop_context() {
        let break_err = compile_source("break").unwrap_err().to_string();
        assert!(break_err.contains("Break outside loop"));
        let continue_err = compile_source("continue").unwrap_err().to_string();
        assert!(continue_err.contains("Continue outside loop"));
    }

    #[test]
    fn pop_rejects_empty_arrays() {
        let err = run_source(
            "let values = [] print pop(values)",
            "vm",
            &mut Vec::new(),
            Vec::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("empty array"));
    }

    #[test]
    fn unsafe_gate_is_compile_time() {
        let err = compile_source("let values = [1] let p = ptr(values, 0) print ptr_load(p)")
            .unwrap_err()
            .to_string();
        assert!(err.contains("requires unsafe"));
    }
}
