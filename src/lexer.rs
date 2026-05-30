use crate::error::CompileError;
use crate::span::Span;
use crate::token::{Token, TokenKind};

pub fn lex(src: &str) -> Result<Vec<Token>, CompileError> {
    let chars: Vec<char> = src.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    let mut line = 1u32;
    let mut col = 1u32;

    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' => {
                i += 1;
                col += 1;
            }
            '\n' => {
                i += 1;
                line += 1;
                col = 1;
            }
            '(' | ')' | '{' | '}' | ';' | '+' | '-' | '*' | '/' | '%' => {
                let kind = match c {
                    '(' => TokenKind::LParen,
                    ')' => TokenKind::RParen,
                    '{' => TokenKind::LBrace,
                    '}' => TokenKind::RBrace,
                    ';' => TokenKind::Semicolon,
                    '+' => TokenKind::Plus,
                    '-' => TokenKind::Minus,
                    '*' => TokenKind::Star,
                    '/' => TokenKind::Slash,
                    '%' => TokenKind::Percent,
                    _ => unreachable!(),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += 1;
                col += 1;
            }
            c if c.is_ascii_digit() => {
                let start_col = col;
                let mut num = String::new();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num.push(chars[i]);
                    i += 1;
                    col += 1;
                }
                let value: i64 = num.parse().map_err(|_| {
                    CompileError::new(
                        Span::new(line, start_col),
                        format!("invalid integer literal '{}'", num),
                    )
                })?;
                tokens.push(Token {
                    kind: TokenKind::IntLit(value),
                    span: Span::new(line, start_col),
                });
            }
            c if c.is_alphabetic() || c == '_' => {
                let start_col = col;
                let mut ident = String::new();
                while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                    ident.push(chars[i]);
                    i += 1;
                    col += 1;
                }
                let kind = match ident.as_str() {
                    "int" => TokenKind::KwInt,
                    "return" => TokenKind::KwReturn,
                    _ => TokenKind::Ident(ident),
                };
                tokens.push(Token { kind, span: Span::new(line, start_col) });
            }
            _ => {
                return Err(CompileError::new(
                    Span::new(line, col),
                    format!("unexpected character '{}'", c),
                ));
            }
        }
    }

    tokens.push(Token { kind: TokenKind::Eof, span: Span::new(line, col) });
    Ok(tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::TokenKind;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src).unwrap().into_iter().map(|t| t.kind).collect()
    }

    #[test]
    fn lex_return_42() {
        assert_eq!(
            kinds("int main(){ return 42; }"),
            vec![
                TokenKind::KwInt,
                TokenKind::Ident("main".to_string()),
                TokenKind::LParen,
                TokenKind::RParen,
                TokenKind::LBrace,
                TokenKind::KwReturn,
                TokenKind::IntLit(42),
                TokenKind::Semicolon,
                TokenKind::RBrace,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_tracks_line_and_col() {
        let toks = lex("int\n  main").unwrap();
        assert_eq!((toks[1].span.line, toks[1].span.col), (2, 3));
    }

    #[test]
    fn lex_rejects_unknown_char() {
        let err = lex("int @").unwrap_err();
        assert!(err.message.contains('@'));
    }

    #[test]
    fn lex_arithmetic_operators() {
        assert_eq!(
            kinds("+ - * / %"),
            vec![
                TokenKind::Plus,
                TokenKind::Minus,
                TokenKind::Star,
                TokenKind::Slash,
                TokenKind::Percent,
                TokenKind::Eof,
            ]
        );
    }
}
