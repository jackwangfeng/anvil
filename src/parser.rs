use crate::ast::{BinaryOp, Expr, FuncDef, LogOp, Program, Stmt, UnaryOp};
use crate::error::CompileError;
use crate::token::{Token, TokenKind};
use crate::types::{Aggregate, Aggregates, Field, Signature, Signatures, Type};
use std::collections::HashMap;

pub fn parse(tokens: &[Token]) -> Result<Program, CompileError> {
    let mut p = Parser {
        tokens,
        pos: 0,
        aggregates: HashMap::new(),
        typedefs: HashMap::new(),
        enum_consts: HashMap::new(),
        signatures: HashMap::new(),
        anon_counter: 0,
    };
    p.parse_program()
}

struct Parser<'a> {
    tokens: &'a [Token],
    pos: usize,
    aggregates: Aggregates,
    typedefs: HashMap<String, Type>,
    enum_consts: HashMap<String, i64>,
    signatures: Signatures,
    anon_counter: usize,
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
            match self.peek_kind() {
                TokenKind::KwStruct | TokenKind::KwUnion => {
                    self.parse_aggregate_def()?;
                    self.expect(&TokenKind::Semicolon)?;
                }
                TokenKind::KwEnum => {
                    self.parse_enum_def()?;
                    self.expect(&TokenKind::Semicolon)?;
                }
                TokenKind::KwTypedef => {
                    self.parse_typedef()?;
                }
                _ => {
                    if let Some(f) = self.parse_func_or_proto()? {
                        functions.push(f);
                    }
                }
            }
        }
        Ok(Program {
            functions,
            aggregates: self.aggregates.clone(),
            signatures: self.signatures.clone(),
        })
    }

    fn size_of(&self, ty: &Type) -> usize {
        crate::types::size_of(ty, &self.aggregates)
    }

    fn at_type_start(&self) -> bool {
        match self.peek_kind() {
            TokenKind::KwInt
            | TokenKind::KwChar
            | TokenKind::KwVoid
            | TokenKind::KwConst
            | TokenKind::KwStruct
            | TokenKind::KwUnion
            | TokenKind::KwEnum => true,
            TokenKind::Ident(name) => self.typedefs.contains_key(name),
            _ => false,
        }
    }

    /// 解析 struct/union 定义，计算布局并注册，返回 tag 名。
    fn parse_aggregate_def(&mut self) -> Result<String, CompileError> {
        let is_union = *self.peek_kind() == TokenKind::KwUnion;
        self.pos += 1; // struct/union
        let tag = if let TokenKind::Ident(name) = self.peek_kind() {
            let n = name.clone();
            self.pos += 1;
            n
        } else {
            let n = format!("__anon_{}", self.anon_counter);
            self.anon_counter += 1;
            n
        };
        self.expect(&TokenKind::LBrace)?;
        let mut fields = Vec::new();
        let mut offset = 0usize;
        let mut max = 0usize;
        while *self.peek_kind() != TokenKind::RBrace {
            let fty = self.parse_type_specifier().ok_or_else(|| {
                CompileError::new(self.tokens[self.pos].span, "expected field type".to_string())
            })?;
            let fname = self.expect_ident()?;
            self.expect(&TokenKind::Semicolon)?;
            let aligned = self.size_of(&fty).div_ceil(8) * 8;
            let foff = if is_union { 0 } else { offset };
            fields.push(Field {
                name: fname,
                ty: fty,
                offset: foff,
            });
            if is_union {
                max = max.max(aligned);
            } else {
                offset += aligned;
            }
        }
        self.expect(&TokenKind::RBrace)?;
        let size = if is_union { max } else { offset };
        self.aggregates.insert(
            tag.clone(),
            Aggregate {
                fields,
                size,
                is_union,
            },
        );
        Ok(tag)
    }

    fn parse_enum_def(&mut self) -> Result<(), CompileError> {
        self.expect(&TokenKind::KwEnum)?;
        if let TokenKind::Ident(_) = self.peek_kind() {
            self.pos += 1;
        }
        self.expect(&TokenKind::LBrace)?;
        let mut next = 0i64;
        while *self.peek_kind() != TokenKind::RBrace {
            let name = self.expect_ident()?;
            if *self.peek_kind() == TokenKind::Assign {
                self.pos += 1;
                if let TokenKind::IntLit(v) = self.peek_kind() {
                    next = *v;
                    self.pos += 1;
                } else {
                    return Err(CompileError::new(
                        self.tokens[self.pos].span,
                        "expected enum value".to_string(),
                    ));
                }
            }
            self.enum_consts.insert(name, next);
            next += 1;
            if *self.peek_kind() == TokenKind::Comma {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(())
    }

    fn parse_typedef(&mut self) -> Result<(), CompileError> {
        self.expect(&TokenKind::KwTypedef)?;
        let ty = self.parse_type_specifier().ok_or_else(|| {
            CompileError::new(
                self.tokens[self.pos].span,
                "expected type in typedef".to_string(),
            )
        })?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Semicolon)?;
        self.typedefs.insert(name, ty);
        Ok(())
    }

    /// 类型说明符：基础类型 / struct|union（含内联定义）/ enum / typedef 名，后跟 `*`。
    fn parse_type_specifier(&mut self) -> Option<Type> {
        // 跳过前导 const 限定符
        while *self.peek_kind() == TokenKind::KwConst {
            self.pos += 1;
        }
        let mut ty = match self.peek_kind() {
            TokenKind::KwVoid => {
                self.pos += 1;
                Type::Void
            }
            TokenKind::KwInt => {
                self.pos += 1;
                Type::Int
            }
            TokenKind::KwChar => {
                self.pos += 1;
                Type::Char
            }
            TokenKind::KwStruct | TokenKind::KwUnion => {
                let save = self.pos;
                let is_union = *self.peek_kind() == TokenKind::KwUnion;
                self.pos += 1;
                let tag = if let TokenKind::Ident(n) = self.peek_kind() {
                    let n = n.clone();
                    self.pos += 1;
                    n
                } else {
                    String::new()
                };
                if *self.peek_kind() == TokenKind::LBrace {
                    self.pos = save;
                    let t = self.parse_aggregate_def().ok()?;
                    if is_union {
                        Type::Union(t)
                    } else {
                        Type::Struct(t)
                    }
                } else if is_union {
                    Type::Union(tag)
                } else {
                    Type::Struct(tag)
                }
            }
            TokenKind::KwEnum => {
                self.pos += 1;
                if let TokenKind::Ident(_) = self.peek_kind() {
                    self.pos += 1;
                }
                Type::Int
            }
            TokenKind::Ident(name) => {
                if let Some(t) = self.typedefs.get(name) {
                    let t = t.clone();
                    self.pos += 1;
                    t
                } else {
                    return None;
                }
            }
            _ => return None,
        };
        while *self.peek_kind() == TokenKind::Star {
            self.pos += 1;
            ty = Type::Pointer(Box::new(ty));
        }
        Some(ty)
    }

    /// 解析函数定义或原型声明。原型返回 None（仅注册签名）。
    fn parse_func_or_proto(&mut self) -> Result<Option<FuncDef>, CompileError> {
        let ret = self.parse_type_specifier().ok_or_else(|| {
            CompileError::new(self.tokens[self.pos].span, "expected return type".to_string())
        })?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        let mut variadic = false;
        if *self.peek_kind() == TokenKind::KwVoid
            && self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::RParen)
        {
            self.pos += 1; // (void) = 无参数
        } else if *self.peek_kind() != TokenKind::RParen {
            loop {
                if *self.peek_kind() == TokenKind::Ellipsis {
                    self.pos += 1;
                    variadic = true;
                    break;
                }
                let ty = self.parse_type_specifier().ok_or_else(|| {
                    CompileError::new(
                        self.tokens[self.pos].span,
                        "expected parameter type".to_string(),
                    )
                })?;
                let pname = if let TokenKind::Ident(n) = self.peek_kind() {
                    let n = n.clone();
                    self.pos += 1;
                    n
                } else {
                    String::new()
                };
                params.push((pname, ty));
                if *self.peek_kind() == TokenKind::Comma {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen)?;
        self.signatures.insert(
            name.clone(),
            Signature {
                ret: ret.clone(),
                fixed: params.len(),
                variadic,
            },
        );
        if *self.peek_kind() == TokenKind::Semicolon {
            self.pos += 1;
            return Ok(None); // 原型
        }
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while *self.peek_kind() != TokenKind::RBrace {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Some(FuncDef {
            name,
            params,
            ret,
            body,
        }))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, CompileError> {
        match self.peek_kind() {
            TokenKind::KwReturn => return self.parse_return(),
            TokenKind::KwIf => return self.parse_if(),
            TokenKind::KwWhile => return self.parse_while(),
            TokenKind::KwFor => return self.parse_for(),
            TokenKind::LBrace => return self.parse_block(),
            TokenKind::Semicolon => {
                self.pos += 1;
                return Ok(Stmt::Empty);
            }
            _ => {}
        }
        if self.at_type_start() {
            return self.parse_declaration();
        }
        let e = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::ExprStmt(e))
    }

    fn parse_return(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwReturn)?;
        let expr = self.parse_expr()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Return(expr))
    }

    fn parse_declaration(&mut self) -> Result<Stmt, CompileError> {
        let mut ty = self
            .parse_type_specifier()
            .expect("parse_declaration called without a type");
        let name = self.expect_ident()?;
        // 数组后缀 name[N]
        if *self.peek_kind() == TokenKind::LBracket {
            self.pos += 1;
            let n = match self.peek_kind() {
                TokenKind::IntLit(v) => *v as usize,
                _ => {
                    return Err(CompileError::new(
                        self.tokens[self.pos].span,
                        "expected array size".to_string(),
                    ))
                }
            };
            self.pos += 1;
            self.expect(&TokenKind::RBracket)?;
            ty = Type::Array(Box::new(ty), n);
        }
        let init = if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            Some(self.parse_expr()?)
        } else {
            None
        };
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Declare { name, ty, init })
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
        } else if self.at_type_start() {
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
        let lhs = self.parse_ternary()?;
        let compound = match self.peek_kind() {
            TokenKind::Assign => None,
            TokenKind::PlusEq => Some(BinaryOp::Add),
            TokenKind::MinusEq => Some(BinaryOp::Sub),
            TokenKind::StarEq => Some(BinaryOp::Mul),
            TokenKind::SlashEq => Some(BinaryOp::Div),
            TokenKind::PercentEq => Some(BinaryOp::Mod),
            TokenKind::AmpEq => Some(BinaryOp::BitAnd),
            TokenKind::PipeEq => Some(BinaryOp::BitOr),
            TokenKind::CaretEq => Some(BinaryOp::BitXor),
            TokenKind::ShlEq => Some(BinaryOp::Shl),
            TokenKind::ShrEq => Some(BinaryOp::Shr),
            _ => return Ok(lhs),
        };
        if !matches!(
            lhs,
            Expr::Var(_) | Expr::Deref(_) | Expr::Index { .. } | Expr::Member { .. }
        ) {
            return Err(CompileError::new(
                self.tokens[self.pos].span,
                "invalid assignment target".to_string(),
            ));
        }
        self.pos += 1;
        let rhs = self.parse_assign()?; // 右结合
        let value = match compound {
            None => rhs,
            Some(op) => Expr::Binary {
                op,
                lhs: Box::new(lhs.clone()),
                rhs: Box::new(rhs),
            },
        };
        Ok(Expr::Assign {
            target: Box::new(lhs),
            value: Box::new(value),
        })
    }

    fn parse_ternary(&mut self) -> Result<Expr, CompileError> {
        let cond = self.parse_logical_or()?;
        if *self.peek_kind() == TokenKind::Question {
            self.pos += 1;
            let then_e = self.parse_expr()?;
            self.expect(&TokenKind::Colon)?;
            let else_e = self.parse_assign()?;
            Ok(Expr::Ternary {
                cond: Box::new(cond),
                then_e: Box::new(then_e),
                else_e: Box::new(else_e),
            })
        } else {
            Ok(cond)
        }
    }

    fn parse_logical_or(&mut self) -> Result<Expr, CompileError> {
        let mut l = self.parse_logical_and()?;
        while *self.peek_kind() == TokenKind::PipePipe {
            self.pos += 1;
            let r = self.parse_logical_and()?;
            l = Expr::Logical {
                op: LogOp::Or,
                lhs: Box::new(l),
                rhs: Box::new(r),
            };
        }
        Ok(l)
    }

    fn parse_logical_and(&mut self) -> Result<Expr, CompileError> {
        let mut l = self.parse_bin_expr(1)?;
        while *self.peek_kind() == TokenKind::AmpAmp {
            self.pos += 1;
            let r = self.parse_bin_expr(1)?;
            l = Expr::Logical {
                op: LogOp::And,
                lhs: Box::new(l),
                rhs: Box::new(r),
            };
        }
        Ok(l)
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
            TokenKind::Amp => {
                self.pos += 1;
                Ok(Expr::Addr(Box::new(self.parse_unary()?)))
            }
            TokenKind::Star => {
                self.pos += 1;
                Ok(Expr::Deref(Box::new(self.parse_unary()?)))
            }
            TokenKind::Bang => {
                // !x  =>  (x == 0)
                self.pos += 1;
                let e = self.parse_unary()?;
                Ok(Expr::Binary {
                    op: BinaryOp::Eq,
                    lhs: Box::new(e),
                    rhs: Box::new(Expr::IntLit(0)),
                })
            }
            TokenKind::Tilde => {
                // ~x  =>  x ^ -1
                self.pos += 1;
                let e = self.parse_unary()?;
                Ok(Expr::Binary {
                    op: BinaryOp::BitXor,
                    lhs: Box::new(e),
                    rhs: Box::new(Expr::Unary {
                        op: UnaryOp::Neg,
                        operand: Box::new(Expr::IntLit(1)),
                    }),
                })
            }
            TokenKind::PlusPlus => {
                self.pos += 1;
                let e = self.parse_unary()?;
                Ok(incdec_assign(e, BinaryOp::Add))
            }
            TokenKind::MinusMinus => {
                self.pos += 1;
                let e = self.parse_unary()?;
                Ok(incdec_assign(e, BinaryOp::Sub))
            }
            TokenKind::KwSizeof => {
                self.pos += 1;
                self.expect(&TokenKind::LParen)?;
                if let Some(ty) = self.parse_type_specifier() {
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::SizeofType(ty))
                } else {
                    let e = self.parse_expr()?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::SizeofExpr(Box::new(e)))
                }
            }
            _ => self.parse_postfix(),
        }
    }

    /// primary 后跟零个或多个后缀：下标 `[expr]`、成员 `.field`、`->field`。
    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LBracket => {
                    self.pos += 1;
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    e = Expr::Index {
                        base: Box::new(e),
                        index: Box::new(index),
                    };
                }
                TokenKind::Dot => {
                    self.pos += 1;
                    let field = self.expect_ident()?;
                    e = Expr::Member {
                        base: Box::new(e),
                        field,
                        arrow: false,
                    };
                }
                TokenKind::Arrow => {
                    self.pos += 1;
                    let field = self.expect_ident()?;
                    e = Expr::Member {
                        base: Box::new(e),
                        field,
                        arrow: true,
                    };
                }
                // 后缀 ++/--（M8 简化：求值为自增后的新值，非旧值）
                TokenKind::PlusPlus => {
                    self.pos += 1;
                    e = incdec_assign(e, BinaryOp::Add);
                }
                TokenKind::MinusMinus => {
                    self.pos += 1;
                    e = incdec_assign(e, BinaryOp::Sub);
                }
                _ => break,
            }
        }
        Ok(e)
    }

    fn parse_primary(&mut self) -> Result<Expr, CompileError> {
        let tok = &self.tokens[self.pos];
        match &tok.kind {
            TokenKind::IntLit(v) => {
                let v = *v;
                self.pos += 1;
                Ok(Expr::IntLit(v))
            }
            TokenKind::StrLit(s) => {
                let s = s.clone();
                self.pos += 1;
                Ok(Expr::StrLit(s))
            }
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                if *self.peek_kind() == TokenKind::LParen {
                    self.pos += 1; // 吃 '('
                    let mut args = Vec::new();
                    if *self.peek_kind() != TokenKind::RParen {
                        loop {
                            args.push(self.parse_expr()?);
                            if *self.peek_kind() == TokenKind::Comma {
                                self.pos += 1;
                            } else {
                                break;
                            }
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::Call { name, args })
                } else if let Some(v) = self.enum_consts.get(&name) {
                    Ok(Expr::IntLit(*v))
                } else {
                    Ok(Expr::Var(name))
                }
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
    // 优先级（越大越紧）：| < ^ < & < ==/!= < 比较 < <<>> < +- < */%
    match kind {
        TokenKind::Pipe => Some((BinaryOp::BitOr, 1)),
        TokenKind::Caret => Some((BinaryOp::BitXor, 2)),
        TokenKind::Amp => Some((BinaryOp::BitAnd, 3)),
        TokenKind::EqEq => Some((BinaryOp::Eq, 4)),
        TokenKind::NotEq => Some((BinaryOp::Ne, 4)),
        TokenKind::Lt => Some((BinaryOp::Lt, 5)),
        TokenKind::Gt => Some((BinaryOp::Gt, 5)),
        TokenKind::Le => Some((BinaryOp::Le, 5)),
        TokenKind::Ge => Some((BinaryOp::Ge, 5)),
        TokenKind::Shl => Some((BinaryOp::Shl, 6)),
        TokenKind::Shr => Some((BinaryOp::Shr, 6)),
        TokenKind::Plus => Some((BinaryOp::Add, 7)),
        TokenKind::Minus => Some((BinaryOp::Sub, 7)),
        TokenKind::Star => Some((BinaryOp::Mul, 8)),
        TokenKind::Slash => Some((BinaryOp::Div, 8)),
        TokenKind::Percent => Some((BinaryOp::Mod, 8)),
        _ => None,
    }
}

/// 把 `++e`/`--e`/`e++`/`e--` 脱糖为 `e = e <op> 1`（M8 取舍：求值为新值）。
fn incdec_assign(e: Expr, op: BinaryOp) -> Expr {
    Expr::Assign {
        target: Box::new(e.clone()),
        value: Box::new(Expr::Binary {
            op,
            lhs: Box::new(e),
            rhs: Box::new(Expr::IntLit(1)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{BinaryOp, Expr, Stmt, UnaryOp};
    use crate::lexer::lex;
    use crate::types::Type;

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
    fn parse_function_with_params() {
        let prog = parse(&lex("int add(int a, int b){ return a+b; }").unwrap()).unwrap();
        let f = &prog.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(
            f.params,
            vec![("a".to_string(), Type::Int), ("b".to_string(), Type::Int)]
        );
    }

    #[test]
    fn parse_struct_def_and_member() {
        let prog = parse(
            &lex("struct P { int x; int y; }; int main(){ struct P p; p.x = 3; return p.x; }")
                .unwrap(),
        )
        .unwrap();
        let agg = prog.aggregates.get("P").unwrap();
        assert_eq!(agg.fields.len(), 2);
        assert_eq!(agg.fields[0].name, "x");
        assert_eq!(agg.fields[1].offset, 8);
    }

    #[test]
    fn parse_arrow_member() {
        let e = parse_return_expr("int main(){ return p->x; }");
        assert_eq!(
            e,
            Expr::Member {
                base: Box::new(Expr::Var("p".into())),
                field: "x".into(),
                arrow: true
            }
        );
    }

    #[test]
    fn parse_enum_constants() {
        let e = parse_return_expr("enum E { A, B, C }; int main(){ return B; }");
        assert_eq!(e, Expr::IntLit(1));
    }

    #[test]
    fn parse_typedef_alias() {
        let prog = parse(&lex("typedef int MyInt; int main(){ MyInt x; x = 7; return x; }").unwrap())
            .unwrap();
        match &prog.functions[0].body[0] {
            Stmt::Declare { ty, .. } => assert_eq!(*ty, Type::Int),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_pointer_decl() {
        let body = parse_body("int main(){ int* p; return 0; }");
        match &body[0] {
            Stmt::Declare { name, ty, .. } => {
                assert_eq!(name, "p");
                assert_eq!(*ty, Type::Pointer(Box::new(Type::Int)));
            }
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_array_decl() {
        let body = parse_body("int main(){ int a[10]; return 0; }");
        match &body[0] {
            Stmt::Declare { ty, .. } => assert_eq!(*ty, Type::Array(Box::new(Type::Int), 10)),
            other => panic!("{:?}", other),
        }
    }

    #[test]
    fn parse_addr_deref_index() {
        assert_eq!(
            parse_return_expr("int main(){ return *p; }"),
            Expr::Deref(Box::new(Expr::Var("p".to_string())))
        );
        assert_eq!(
            parse_return_expr("int main(){ return &x; }"),
            Expr::Addr(Box::new(Expr::Var("x".to_string())))
        );
        assert_eq!(
            parse_return_expr("int main(){ return a[2]; }"),
            Expr::Index {
                base: Box::new(Expr::Var("a".to_string())),
                index: Box::new(Expr::IntLit(2)),
            }
        );
    }

    #[test]
    fn parse_sizeof() {
        assert_eq!(
            parse_return_expr("int main(){ return sizeof(int); }"),
            Expr::SizeofType(Type::Int)
        );
        assert_eq!(
            parse_return_expr("int main(){ return sizeof(x); }"),
            Expr::SizeofExpr(Box::new(Expr::Var("x".to_string())))
        );
    }

    #[test]
    fn parse_typed_params() {
        let prog = parse(&lex("int f(int* p, char c){ return 0; }").unwrap()).unwrap();
        assert_eq!(
            prog.functions[0].params,
            vec![
                ("p".to_string(), Type::Pointer(Box::new(Type::Int))),
                ("c".to_string(), Type::Char),
            ]
        );
    }

    #[test]
    fn parse_call_expr() {
        let e = parse_return_expr("int main(){ return add(1, 2); }");
        assert_eq!(
            e,
            Expr::Call {
                name: "add".to_string(),
                args: vec![Expr::IntLit(1), Expr::IntLit(2)],
            }
        );
    }

    #[test]
    fn parse_string_arg() {
        let body = parse_body("int main(){ puts(\"hi\"); return 0; }");
        match &body[0] {
            Stmt::ExprStmt(Expr::Call { name, args }) => {
                assert_eq!(name, "puts");
                assert_eq!(args, &vec![Expr::StrLit("hi".to_string())]);
            }
            other => panic!("expected call stmt, got {:?}", other),
        }
    }

    #[test]
    fn parse_declaration_with_init() {
        let body = parse_body("int main(){ int x = 5; return x; }");
        assert_eq!(
            body[0],
            Stmt::Declare {
                name: "x".to_string(),
                ty: Type::Int,
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
                ty: Type::Int,
                init: None
            }
        );
        assert_eq!(
            body[1],
            Stmt::ExprStmt(Expr::Assign {
                target: Box::new(Expr::Var("x".to_string())),
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
