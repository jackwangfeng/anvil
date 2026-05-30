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
        match self.peek_kind() {
            TokenKind::KwReturn => self.parse_return(),
            TokenKind::KwInt => self.parse_declaration(),
            TokenKind::KwIf => self.parse_if(),
            TokenKind::KwWhile => self.parse_while(),
            TokenKind::KwFor => self.parse_for(),
            TokenKind::LBrace => self.parse_block(),
            TokenKind::Semicolon => {
                self.pos += 1;
                Ok(Stmt::Empty)
            }
            _ => {
                let e = self.parse_expr()?;
                self.expect(&TokenKind::Semicolon)?;
                Ok(Stmt::ExprStmt(e))
            }
        }
    }

    fn parse_return(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwReturn)?;
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Return(expr))
    }

    fn parse_declaration(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwInt)?;
        let name = self.expect_ident()?;
        let init = if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Declare { name, init })
    }

    fn parse_block(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::LBrace)?;
        let mut stmts = Vec::new();
        while *self.peek_kind() != TokenKind::RBrace {
            stmts.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Block(stmts))
    }

    fn parse_if(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwIf)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let then_branch = Box::new(self.parse_stmt()?);
        let else_branch = if *self.peek_kind() == TokenKind::KwElse {
            self.pos += 1;
            Some(Box::new(self.parse_stmt()?))
        } else {
            None
        };
        Ok(Stmt::If {
            cond,
            then_branch,
            else_branch,
        })
    }

    fn parse_while(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwWhile)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::While { cond, body })
    }

    fn parse_for(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwFor)?;
        self.expect(&TokenKind::LParen)?;
        let init = if *self.peek_kind() == TokenKind::Semicolon {
            self.pos += 1;
            None
        } else if *self.peek_kind() == TokenKind::KwInt {
            Some(Box::new(self.parse_declaration()?)) // 自带分号消费
        } else {
            let e = self.parse_expr()?;
            self.expect(&TokenKind::Semicolon)?;
            Some(Box::new(Stmt::ExprStmt(e)))
        };
        let cond = if *self.peek_kind() == TokenKind::Semicolon {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::Semicolon)?;
        let step = if *self.peek_kind() == TokenKind::RParen {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::For {
            init,
            cond,
            step,
            body,
        })
    }

    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_bin_expr(1)?;
        if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            let value = self.parse_assign()?; // 右结合
            if let Expr::Var(name) = lhs {
                Ok(Expr::Assign {
                    name,
                    value: Box::new(value),
                })
            } else {
                Err(CompileError::new(
                    self.tokens[self.pos.saturating_sub(1)].span,
                    "invalid assignment target".to_string(),
                ))
            }
        } else {
            Ok(lhs)
        }
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
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                Ok(Expr::Var(name))
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
        TokenKind::EqEq => Some((BinaryOp::Eq, 1)),
        TokenKind::NotEq => Some((BinaryOp::Ne, 1)),
        TokenKind::Lt => Some((BinaryOp::Lt, 2)),
        TokenKind::Gt => Some((BinaryOp::Gt, 2)),
        TokenKind::Le => Some((BinaryOp::Le, 2)),
        TokenKind::Ge => Some((BinaryOp::Ge, 2)),
        TokenKind::Plus => Some((BinaryOp::Add, 3)),
        TokenKind::Minus => Some((BinaryOp::Sub, 3)),
        TokenKind::Star => Some((BinaryOp::Mul, 4)),
        TokenKind::Slash => Some((BinaryOp::Div, 4)),
        TokenKind::Percent => Some((BinaryOp::Mod, 4)),
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
            other => panic!("expected Return, got {:?}", other),
        }
    }

    fn parse_body(src: &str) -> Vec<Stmt> {
        parse(&lex(src).unwrap())
            .unwrap()
            .functions
            .into_iter()
            .next()
            .unwrap()
            .body
    }

    #[test]
    fn parse_declaration_with_init() {
        let body = parse_body("int main(){ int x = 5; return x; }");
        assert_eq!(
            body[0],
            Stmt::Declare {
                name: "x".to_string(),
                init: Some(Expr::IntLit(5))
            }
        );
        assert_eq!(body[1], Stmt::Return(Expr::Var("x".to_string())));
    }

    #[test]
    fn parse_assignment_expr() {
        let body = parse_body("int main(){ int x; x = 3; return x; }");
        assert_eq!(
            body[0],
            Stmt::Declare {
                name: "x".to_string(),
                init: None
            }
        );
        assert_eq!(
            body[1],
            Stmt::ExprStmt(Expr::Assign {
                name: "x".to_string(),
                value: Box::new(Expr::IntLit(3)),
            })
        );
    }

    #[test]
    fn parse_if_else() {
        let body = parse_body("int main(){ if (1) return 2; else return 3; }");
        match &body[0] {
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                assert_eq!(*cond, Expr::IntLit(1));
                assert_eq!(**then_branch, Stmt::Return(Expr::IntLit(2)));
                assert_eq!(**else_branch.as_ref().unwrap(), Stmt::Return(Expr::IntLit(3)));
            }
            other => panic!("expected If, got {:?}", other),
        }
    }

    #[test]
    fn parse_while_and_compare() {
        let body = parse_body("int main(){ while (x < 10) x = x + 1; return x; }");
        match &body[0] {
            Stmt::While { cond, .. } => {
                assert_eq!(
                    *cond,
                    Expr::Binary {
                        op: BinaryOp::Lt,
                        lhs: Box::new(Expr::Var("x".to_string())),
                        rhs: Box::new(Expr::IntLit(10)),
                    }
                );
            }
            other => panic!("expected While, got {:?}", other),
        }
    }

    #[test]
    fn parse_for_loop() {
        let body = parse_body("int main(){ for (int i = 0; i < 3; i = i + 1) {} return 0; }");
        match &body[0] {
            Stmt::For {
                init, cond, step, ..
            } => {
                assert!(init.is_some());
                assert!(cond.is_some());
                assert!(step.is_some());
            }
            other => panic!("expected For, got {:?}", other),
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
