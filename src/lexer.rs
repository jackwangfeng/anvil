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
            '(' | ')' | '{' | '}' | ';' | ',' | '[' | ']' | '~' | '?' | ':' => {
                let kind = match c {
                    '(' => TokenKind::LParen,
                    ')' => TokenKind::RParen,
                    '{' => TokenKind::LBrace,
                    '}' => TokenKind::RBrace,
                    ';' => TokenKind::Semicolon,
                    ',' => TokenKind::Comma,
                    '[' => TokenKind::LBracket,
                    ']' => TokenKind::RBracket,
                    '~' => TokenKind::Tilde,
                    '?' => TokenKind::Question,
                    ':' => TokenKind::Colon,
                    _ => unreachable!(),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += 1;
                col += 1;
            }
            '+' | '*' | '%' | '^' | '|' | '&' => {
                // 一/二字符运算符：X、X=、（&&/||/++）
                let next = chars.get(i + 1).copied();
                let (kind, len) = match (c, next) {
                    ('+', Some('+')) => (TokenKind::PlusPlus, 2),
                    ('+', Some('=')) => (TokenKind::PlusEq, 2),
                    ('+', _) => (TokenKind::Plus, 1),
                    ('*', Some('=')) => (TokenKind::StarEq, 2),
                    ('*', _) => (TokenKind::Star, 1),
                    ('%', Some('=')) => (TokenKind::PercentEq, 2),
                    ('%', _) => (TokenKind::Percent, 1),
                    ('^', Some('=')) => (TokenKind::CaretEq, 2),
                    ('^', _) => (TokenKind::Caret, 1),
                    ('|', Some('|')) => (TokenKind::PipePipe, 2),
                    ('|', Some('=')) => (TokenKind::PipeEq, 2),
                    ('|', _) => (TokenKind::Pipe, 1),
                    ('&', Some('&')) => (TokenKind::AmpAmp, 2),
                    ('&', Some('=')) => (TokenKind::AmpEq, 2),
                    ('&', _) => (TokenKind::Amp, 1),
                    _ => unreachable!(),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += len;
                col += len as u32;
            }
            '/' => {
                // 注释 // 与 /* */，以及 / 和 /=
                if chars.get(i + 1) == Some(&'/') {
                    while i < chars.len() && chars[i] != '\n' {
                        i += 1;
                        col += 1;
                    }
                } else if chars.get(i + 1) == Some(&'*') {
                    i += 2;
                    col += 2;
                    while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                        if chars[i] == '\n' {
                            line += 1;
                            col = 1;
                        } else {
                            col += 1;
                        }
                        i += 1;
                    }
                    i += 2; // 跳过 */
                    col += 2;
                } else if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token { kind: TokenKind::SlashEq, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Slash, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            '.' => {
                if i + 2 < chars.len() && chars[i + 1] == '.' && chars[i + 2] == '.' {
                    tokens.push(Token { kind: TokenKind::Ellipsis, span: Span::new(line, col) });
                    i += 3;
                    col += 3;
                } else {
                    tokens.push(Token { kind: TokenKind::Dot, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            '-' => {
                let next = chars.get(i + 1).copied();
                let (kind, len) = match next {
                    Some('>') => (TokenKind::Arrow, 2),
                    Some('-') => (TokenKind::MinusMinus, 2),
                    Some('=') => (TokenKind::MinusEq, 2),
                    _ => (TokenKind::Minus, 1),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += len;
                col += len as u32;
            }
            '\'' => {
                // 字符字面量 'x' / '\n' 等 → 整型常量（其字符码）
                let start_col = col;
                i += 1;
                col += 1;
                if i >= chars.len() {
                    return Err(CompileError::new(
                        Span::new(line, start_col),
                        "unterminated char literal".to_string(),
                    ));
                }
                let value: i64 = if chars[i] == '\\' {
                    i += 1;
                    col += 1;
                    let v = match chars.get(i) {
                        Some('n') => 10,
                        Some('t') => 9,
                        Some('r') => 13,
                        Some('0') => 0,
                        Some('\\') => 92,
                        Some('\'') => 39,
                        Some('"') => 34,
                        Some(&other) => other as i64,
                        None => {
                            return Err(CompileError::new(
                                Span::new(line, start_col),
                                "unterminated char literal".to_string(),
                            ))
                        }
                    };
                    i += 1;
                    col += 1;
                    v
                } else {
                    let v = chars[i] as i64;
                    i += 1;
                    col += 1;
                    v
                };
                if chars.get(i) != Some(&'\'') {
                    return Err(CompileError::new(
                        Span::new(line, start_col),
                        "unterminated char literal".to_string(),
                    ));
                }
                i += 1;
                col += 1;
                tokens.push(Token {
                    kind: TokenKind::IntLit(value),
                    span: Span::new(line, start_col),
                });
            }
            '"' => {
                let start_col = col;
                i += 1; // 跳过开引号
                col += 1;
                let mut s = String::new();
                loop {
                    if i >= chars.len() {
                        return Err(CompileError::new(
                            Span::new(line, start_col),
                            "unterminated string literal".to_string(),
                        ));
                    }
                    let ch = chars[i];
                    if ch == '"' {
                        i += 1;
                        col += 1;
                        break;
                    } else if ch == '\\' {
                        i += 1;
                        col += 1;
                        if i >= chars.len() {
                            return Err(CompileError::new(
                                Span::new(line, start_col),
                                "unterminated string literal".to_string(),
                            ));
                        }
                        let esc = chars[i];
                        let mapped = match esc {
                            'n' => '\n',
                            't' => '\t',
                            '\\' => '\\',
                            '"' => '"',
                            '0' => '\0',
                            other => {
                                return Err(CompileError::new(
                                    Span::new(line, col),
                                    format!("unknown escape '\\{}'", other),
                                ))
                            }
                        };
                        s.push(mapped);
                        i += 1;
                        col += 1;
                    } else {
                        s.push(ch);
                        i += 1;
                        col += 1;
                    }
                }
                tokens.push(Token {
                    kind: TokenKind::StrLit(s),
                    span: Span::new(line, start_col),
                });
            }
            '=' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::EqEq, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Assign, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            '<' => {
                let (kind, len) = if chars.get(i + 1) == Some(&'<') {
                    if chars.get(i + 2) == Some(&'=') {
                        (TokenKind::ShlEq, 3)
                    } else {
                        (TokenKind::Shl, 2)
                    }
                } else if chars.get(i + 1) == Some(&'=') {
                    (TokenKind::Le, 2)
                } else {
                    (TokenKind::Lt, 1)
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += len;
                col += len as u32;
            }
            '>' => {
                let (kind, len) = if chars.get(i + 1) == Some(&'>') {
                    if chars.get(i + 2) == Some(&'=') {
                        (TokenKind::ShrEq, 3)
                    } else {
                        (TokenKind::Shr, 2)
                    }
                } else if chars.get(i + 1) == Some(&'=') {
                    (TokenKind::Ge, 2)
                } else {
                    (TokenKind::Gt, 1)
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += len;
                col += len as u32;
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token { kind: TokenKind::NotEq, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Bang, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            c if c.is_ascii_digit() => {
                let start_col = col;
                // 十六进制 (0x..) 与二进制 (0b..) 字面量
                if c == '0'
                    && i + 1 < chars.len()
                    && matches!(chars[i + 1], 'x' | 'X' | 'b' | 'B')
                {
                    let radix = if matches!(chars[i + 1], 'x' | 'X') { 16 } else { 2 };
                    i += 2;
                    col += 2;
                    let mut digits = String::new();
                    while i < chars.len() && chars[i].is_ascii_hexdigit() {
                        digits.push(chars[i]);
                        i += 1;
                        col += 1;
                    }
                    let value = i64::from_str_radix(&digits, radix).map_err(|_| {
                        CompileError::new(
                            Span::new(line, start_col),
                            format!("invalid base-{} literal '{}'", radix, digits),
                        )
                    })?;
                    while i < chars.len() && matches!(chars[i], 'l' | 'L' | 'u' | 'U') {
                        i += 1;
                        col += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::IntLit(value),
                        span: Span::new(line, start_col),
                    });
                    continue;
                }
                let mut num = String::new();
                while i < chars.len() && chars[i].is_ascii_digit() {
                    num.push(chars[i]);
                    i += 1;
                    col += 1;
                }
                // 浮点字面量：小数点后跟数字（1.5），或指数（1e10 / 1.5e-3 / 2E8）
                let has_frac =
                    i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit();
                let has_exp = i < chars.len()
                    && matches!(chars[i], 'e' | 'E')
                    && {
                        let mut j = i + 1;
                        if j < chars.len() && matches!(chars[j], '+' | '-') {
                            j += 1;
                        }
                        j < chars.len() && chars[j].is_ascii_digit()
                    };
                if has_frac || has_exp {
                    // 小数部分
                    if has_frac {
                        num.push('.');
                        i += 1;
                        col += 1;
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            num.push(chars[i]);
                            i += 1;
                            col += 1;
                        }
                    }
                    // 指数部分 e[+/-]digits
                    if i < chars.len() && matches!(chars[i], 'e' | 'E') {
                        num.push('e');
                        i += 1;
                        col += 1;
                        if i < chars.len() && matches!(chars[i], '+' | '-') {
                            num.push(chars[i]);
                            i += 1;
                            col += 1;
                        }
                        while i < chars.len() && chars[i].is_ascii_digit() {
                            num.push(chars[i]);
                            i += 1;
                            col += 1;
                        }
                    }
                    let value: f64 = num.parse().map_err(|_| {
                        CompileError::new(
                            Span::new(line, start_col),
                            format!("invalid float literal '{}'", num),
                        )
                    })?;
                    tokens.push(Token {
                        kind: TokenKind::FloatLit(value),
                        span: Span::new(line, start_col),
                    });
                } else {
                    // 前导 0 且不止一位 → 八进制（如 0777）；否则十进制
                    let value: i64 = if num.len() > 1 && num.starts_with('0') {
                        i64::from_str_radix(&num[1..], 8).map_err(|_| {
                            CompileError::new(
                                Span::new(line, start_col),
                                format!("invalid octal literal '{}'", num),
                            )
                        })?
                    } else {
                        num.parse().map_err(|_| {
                            CompileError::new(
                                Span::new(line, start_col),
                                format!("invalid integer literal '{}'", num),
                            )
                        })?
                    };
                    // 消费整数后缀 L/U（不影响数值，仅避免被当作标识符）
                    while i < chars.len() && matches!(chars[i], 'l' | 'L' | 'u' | 'U') {
                        i += 1;
                        col += 1;
                    }
                    tokens.push(Token {
                        kind: TokenKind::IntLit(value),
                        span: Span::new(line, start_col),
                    });
                }
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
                    "if" => TokenKind::KwIf,
                    "else" => TokenKind::KwElse,
                    "while" => TokenKind::KwWhile,
                    "do" => TokenKind::KwDo,
                    "goto" => TokenKind::KwGoto,
                    "static" => TokenKind::KwStatic,
                    "extern" => TokenKind::KwExtern,
                    "register" => TokenKind::KwRegister,
                    "auto" => TokenKind::KwAuto,
                    "for" => TokenKind::KwFor,
                    "char" => TokenKind::KwChar,
                    "sizeof" => TokenKind::KwSizeof,
                    "struct" => TokenKind::KwStruct,
                    "union" => TokenKind::KwUnion,
                    "enum" => TokenKind::KwEnum,
                    "typedef" => TokenKind::KwTypedef,
                    "void" => TokenKind::KwVoid,
                    "const" => TokenKind::KwConst,
                    "break" => TokenKind::KwBreak,
                    "continue" => TokenKind::KwContinue,
                    "switch" => TokenKind::KwSwitch,
                    "case" => TokenKind::KwCase,
                    "default" => TokenKind::KwDefault,
                    "double" | "float" => TokenKind::KwDouble,
                    "long" => TokenKind::KwLong,
                    "short" => TokenKind::KwShort,
                    "unsigned" => TokenKind::KwUnsigned,
                    "signed" => TokenKind::KwSigned,
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

    #[test]
    fn lex_hex_and_binary_literals() {
        assert_eq!(
            kinds("0xF 0x10 0xff 0b101"),
            vec![
                TokenKind::IntLit(15),
                TokenKind::IntLit(16),
                TokenKind::IntLit(255),
                TokenKind::IntLit(5),
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_m8_operators() {
        assert_eq!(
            kinds("&& || ! ~ | ^ << >> ++ -- += <<= ? :"),
            vec![
                TokenKind::AmpAmp,
                TokenKind::PipePipe,
                TokenKind::Bang,
                TokenKind::Tilde,
                TokenKind::Pipe,
                TokenKind::Caret,
                TokenKind::Shl,
                TokenKind::Shr,
                TokenKind::PlusPlus,
                TokenKind::MinusMinus,
                TokenKind::PlusEq,
                TokenKind::ShlEq,
                TokenKind::Question,
                TokenKind::Colon,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_comments() {
        assert_eq!(kinds("1 // line\n+ 2"), vec![TokenKind::IntLit(1), TokenKind::Plus, TokenKind::IntLit(2), TokenKind::Eof]);
        assert_eq!(kinds("1 /* block */ + 2"), vec![TokenKind::IntLit(1), TokenKind::Plus, TokenKind::IntLit(2), TokenKind::Eof]);
    }

    #[test]
    fn lex_m5_tokens() {
        assert_eq!(
            kinds(". -> struct union enum typedef"),
            vec![
                TokenKind::Dot,
                TokenKind::Arrow,
                TokenKind::KwStruct,
                TokenKind::KwUnion,
                TokenKind::KwEnum,
                TokenKind::KwTypedef,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_m4_tokens() {
        assert_eq!(
            kinds("& [ ] char sizeof *"),
            vec![
                TokenKind::Amp,
                TokenKind::LBracket,
                TokenKind::RBracket,
                TokenKind::KwChar,
                TokenKind::KwSizeof,
                TokenKind::Star,
                TokenKind::Eof,
            ]
        );
    }

    #[test]
    fn lex_comma_and_string() {
        let toks = lex("foo(\"hi\\n\", 1)").unwrap();
        let ks: Vec<TokenKind> = toks.into_iter().map(|t| t.kind).collect();
        assert_eq!(ks[0], TokenKind::Ident("foo".to_string()));
        assert_eq!(ks[1], TokenKind::LParen);
        assert_eq!(ks[2], TokenKind::StrLit("hi\n".to_string()));
        assert_eq!(ks[3], TokenKind::Comma);
        assert_eq!(ks[4], TokenKind::IntLit(1));
        assert_eq!(ks[5], TokenKind::RParen);
    }

    #[test]
    fn lex_m2_operators_and_keywords() {
        assert_eq!(
            kinds("= == != < <= > >= if else while for"),
            vec![
                TokenKind::Assign,
                TokenKind::EqEq,
                TokenKind::NotEq,
                TokenKind::Lt,
                TokenKind::Le,
                TokenKind::Gt,
                TokenKind::Ge,
                TokenKind::KwIf,
                TokenKind::KwElse,
                TokenKind::KwWhile,
                TokenKind::KwFor,
                TokenKind::Eof,
            ]
        );
    }
}
