use crate::ast::{Expr, FuncDef, Program, Stmt};
use crate::error::CompileError;
use crate::token::{Token, TokenKind};

pub fn parse(tokens: &[Token]) -> Result<Program, CompileError> {
    let mut p = Parser { tokens, pos: 0 };
    p.parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek_kind(&self) -> &TokenKind {
        &self.tokens[self.pos].kind
    }

    fn expect(&mut self, kind: &TokenKind) -> Result<(), CompileError> {
        let tok = &self.tokens[self.pos];
        if &tok.kind == kind {
            self.pos += 1;
            Ok(())
        } else {
            Err(CompileError::new(
                tok.span,
                format!("expected {:?}, found {:?}", kind, tok.kind),
            ))
        }
    }

    fn expect_ident(&mut self) -> Result<String, CompileError> {
        let tok = &self.tokens[self.pos];
        if let TokenKind::Ident(name) = &tok.kind {
            let name = name.clone();
            self.pos += 1;
            Ok(name)
        } else {
            Err(CompileError::new(
                tok.span,
                format!("expected identifier, found {:?}", tok.kind),
            ))
        }
    }

    fn parse_program(&mut self) -> Result<Program, CompileError> {
        let mut functions = Vec::new();
        while *self.peek_kind() != TokenKind::Eof {
            functions.push(self.parse_func_def()?);
        }
        Ok(Program { functions })
    }

    fn parse_func_def(&mut self) -> Result<FuncDef, CompileError> {
        self.expect(&TokenKind::KwInt)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while *self.peek_kind() != TokenKind::RBrace {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(FuncDef { name, body })
    }

    fn parse_stmt(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwReturn)?;
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Return(expr))
    }

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        let tok = &self.tokens[self.pos];
        if let TokenKind::IntLit(v) = tok.kind {
            self.pos += 1;
            Ok(Expr::IntLit(v))
        } else {
            Err(CompileError::new(
                tok.span,
                format!("expected integer literal, found {:?}", tok.kind),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{Expr, Stmt};
    use crate::lexer::lex;

    #[test]
    fn parse_return_42() {
        let toks = lex("int main(){ return 42; }").unwrap();
        let prog = parse(&toks).unwrap();
        assert_eq!(prog.functions.len(), 1);
        let f = &prog.functions[0];
        assert_eq!(f.name, "main");
        assert_eq!(f.body, vec![Stmt::Return(Expr::IntLit(42))]);
    }

    #[test]
    fn parse_reports_missing_semicolon() {
        let toks = lex("int main(){ return 42 }").unwrap();
        let err = parse(&toks).unwrap_err();
        assert!(err.message.contains("Semicolon") || err.message.contains(';'));
    }
}
