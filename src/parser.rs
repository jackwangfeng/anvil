use crate::ast::{BinaryOp, Expr, FuncDef, Program, Stmt, UnaryOp};
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
        self.parse_bin_expr(1)
    }

    /// 优先级爬升：min_prec 为当前可接受的最低运算符优先级。
    fn parse_bin_expr(&mut self, min_prec: u8) -> Result<Expr, CompileError> {
        let mut lhs = self.parse_unary()?;
        while let Some((op, prec)) = binop_of(self.peek_kind()) {
            if prec < min_prec {
                break;
            }
            self.pos += 1; // 消费运算符
            let rhs = self.parse_bin_expr(prec + 1)?; // 左结合：右侧要求更高优先级
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
            };
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                self.pos += 1;
                Ok(Expr::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(self.parse_unary()?),
                })
            }
            TokenKind::Plus => {
                self.pos += 1;
                Ok(Expr::Unary {
                    op: UnaryOp::Plus,
                    operand: Box::new(self.parse_unary()?),
                })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        let tok = &self.tokens[self.pos];
        match &tok.kind {
            TokenKind::IntLit(v) => {
                let v = *v;
                self.pos += 1;
                Ok(Expr::IntLit(v))
            }
            TokenKind::LParen => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect(&TokenKind::RParen)?;
                Ok(e)
            }
            _ => Err(CompileError::new(
                tok.span,
                format!("expected expression, found {:?}", tok.kind),
            )),
        }
    }
}

fn binop_of(kind: &TokenKind) -> Option<(BinaryOp, u8)> {
    match kind {
        TokenKind::Plus => Some((BinaryOp::Add, 1)),
        TokenKind::Minus => Some((BinaryOp::Sub, 1)),
        TokenKind::Star => Some((BinaryOp::Mul, 2)),
        TokenKind::Slash => Some((BinaryOp::Div, 2)),
        TokenKind::Percent => Some((BinaryOp::Mod, 2)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, Expr, Stmt, UnaryOp};
    use crate::lexer::lex;

    fn parse_return_expr(src: &str) -> Expr {
        let prog = parse(&lex(src).unwrap()).unwrap();
        match &prog.functions[0].body[0] {
            Stmt::Return(e) => e.clone(),
        }
    }

    #[test]
    fn parse_precedence_mul_over_add() {
        // 1 + 2 * 3  =>  Add(1, Mul(2, 3))
        let e = parse_return_expr("int main(){ return 1+2*3; }");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinaryOp::Add,
                lhs: Box::new(Expr::IntLit(1)),
                rhs: Box::new(Expr::Binary {
                    op: BinaryOp::Mul,
                    lhs: Box::new(Expr::IntLit(2)),
                    rhs: Box::new(Expr::IntLit(3)),
                }),
            }
        );
    }

    #[test]
    fn parse_left_assoc_sub() {
        // 10 - 3 - 2  =>  Sub(Sub(10,3), 2)
        let e = parse_return_expr("int main(){ return 10-3-2; }");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinaryOp::Sub,
                lhs: Box::new(Expr::Binary {
                    op: BinaryOp::Sub,
                    lhs: Box::new(Expr::IntLit(10)),
                    rhs: Box::new(Expr::IntLit(3)),
                }),
                rhs: Box::new(Expr::IntLit(2)),
            }
        );
    }

    #[test]
    fn parse_parens_override_precedence() {
        // (1 + 2) * 3
        let e = parse_return_expr("int main(){ return (1+2)*3; }");
        assert_eq!(
            e,
            Expr::Binary {
                op: BinaryOp::Mul,
                lhs: Box::new(Expr::Binary {
                    op: BinaryOp::Add,
                    lhs: Box::new(Expr::IntLit(1)),
                    rhs: Box::new(Expr::IntLit(2)),
                }),
                rhs: Box::new(Expr::IntLit(3)),
            }
        );
    }

    #[test]
    fn parse_unary_neg() {
        // -5  =>  Neg(5)
        let e = parse_return_expr("int main(){ return -5; }");
        assert_eq!(
            e,
            Expr::Unary {
                op: UnaryOp::Neg,
                operand: Box::new(Expr::IntLit(5))
            }
        );
    }

    #[test]
    fn parse_reports_missing_rparen() {
        let err = parse(&lex("int main(){ return (1+2; }").unwrap()).unwrap_err();
        assert!(err.message.contains("RParen") || err.message.contains(')'));
    }

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
