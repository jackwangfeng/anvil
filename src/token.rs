use crate::span::Span;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenKind {
    KwInt,
    KwReturn,
    Ident(String),
    IntLit(i64),
    LParen,
    RParen,
    LBrace,
    RBrace,
    Semicolon,
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eof,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}
