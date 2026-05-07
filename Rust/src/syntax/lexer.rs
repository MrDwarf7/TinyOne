use crate::syntax::token::{Token, TokenKind};
use crate::{Result, SourceMap, TinyOneError};

pub(crate) struct Lexer {
    source: String,
    source_map: SourceMap,
}

impl Lexer {
    pub(crate) fn new(source: impl Into<String>, filename: impl Into<String>) -> Self {
        let source = source.into();
        let filename = filename.into();
        Self {
            source: source.clone(),
            source_map: SourceMap::new(source, filename),
        }
    }

    pub(crate) fn tokenize(&self) -> Result<Vec<Token>> {
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
