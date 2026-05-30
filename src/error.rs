use crate::span::Span;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompileError {
    pub span: Span,
    pub message: String,
}

impl CompileError {
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        CompileError {
            span,
            message: message.into(),
        }
    }
}

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}: error: {}", self.span.line, self.span.col, self.message)
    }
}

impl std::error::Error for CompileError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::span::Span;

    #[test]
    fn error_displays_line_col_message() {
        let e = CompileError::new(Span::new(3, 7), "boom");
        assert_eq!(e.to_string(), "3:7: error: boom");
    }
}
