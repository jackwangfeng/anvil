use crate::ast::{BinaryOp, Expr, FuncDef, Global, LogOp, Program, Stmt, UnaryOp};
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
        globals: Vec::new(),
        global_types: HashMap::new(),
        anon_counter: 0,
        saw_extern: false,
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
    globals: Vec<Global>,
    global_types: HashMap<String, Type>,
    anon_counter: usize,
    /// 最近一次 parse_base_type 是否见到 `extern`（用于抑制 extern 全局的定义生成）。
    saw_extern: bool,
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
                TokenKind::KwStruct | TokenKind::KwUnion if self.is_aggregate_def() => {
                    self.parse_aggregate_def()?;
                    self.expect(&TokenKind::Semicolon)?;
                }
                TokenKind::KwEnum if self.is_aggregate_def() => {
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
            globals: std::mem::take(&mut self.globals),
        })
    }

    fn size_of(&self, ty: &Type) -> usize {
        crate::types::size_of(ty, &self.aggregates)
    }

    /// 当前 `struct`/`union` 是否引出一个聚合体定义（`struct [Tag] { ... }`），
    /// 而非把已知聚合体当作返回类型/变量类型用（`struct Tag name ...`）。
    fn is_aggregate_def(&self) -> bool {
        match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
            Some(TokenKind::LBrace) => true,
            Some(TokenKind::Ident(_)) => matches!(
                self.tokens.get(self.pos + 2).map(|t| &t.kind),
                Some(TokenKind::LBrace)
            ),
            _ => false,
        }
    }

    fn at_type_start(&self) -> bool {
        match self.peek_kind() {
            TokenKind::KwInt
            | TokenKind::KwChar
            | TokenKind::KwDouble
            | TokenKind::KwLong
            | TokenKind::KwShort
            | TokenKind::KwUnsigned
            | TokenKind::KwSigned
            | TokenKind::KwVoid
            | TokenKind::KwConst
            | TokenKind::KwStatic
            | TokenKind::KwExtern
            | TokenKind::KwRegister
            | TokenKind::KwAuto
            | TokenKind::KwStruct
            | TokenKind::KwUnion
            | TokenKind::KwEnum => true,
            TokenKind::Ident(name) => name == "va_list" || self.typedefs.contains_key(name),
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

    /// 消费一段整型说明符关键字组合，归一为(有/无符号的) Char / Long / Int。
    /// `short` 视作 int(无 16 位);`unsigned` 产出无符号变体。
    fn parse_integer_type(&mut self) -> Type {
        let mut is_char = false;
        let mut long_count = 0;
        let mut uns = false;
        loop {
            match self.peek_kind() {
                TokenKind::KwChar => {
                    is_char = true;
                    self.pos += 1;
                }
                TokenKind::KwLong => {
                    long_count += 1;
                    self.pos += 1;
                }
                TokenKind::KwUnsigned => {
                    uns = true;
                    self.pos += 1;
                }
                TokenKind::KwInt | TokenKind::KwShort | TokenKind::KwSigned => {
                    self.pos += 1;
                }
                _ => break,
            }
        }
        if is_char {
            if uns {
                Type::UChar
            } else {
                Type::Char
            }
        } else if long_count >= 1 {
            if uns {
                Type::ULong
            } else {
                Type::Long
            }
        } else if uns {
            Type::UInt
        } else {
            Type::Int
        }
    }

    /// 基础类型说明符（不含指针 `*`）：int/char/long/short/unsigned/signed/double/void、struct|union、enum、typedef 名。
    fn parse_base_type(&mut self) -> Option<Type> {
        // 跳过前导限定符 / 存储类（const、static、register、auto 语义忽略；extern 记下以抑制定义）
        self.saw_extern = false;
        while matches!(
            self.peek_kind(),
            TokenKind::KwConst
                | TokenKind::KwStatic
                | TokenKind::KwExtern
                | TokenKind::KwRegister
                | TokenKind::KwAuto
        ) {
            if *self.peek_kind() == TokenKind::KwExtern {
                self.saw_extern = true;
            }
            self.pos += 1;
        }
        // 整型说明符组合（int/char/long/short/unsigned/signed 任意搭配）
        if matches!(
            self.peek_kind(),
            TokenKind::KwInt
                | TokenKind::KwChar
                | TokenKind::KwLong
                | TokenKind::KwShort
                | TokenKind::KwUnsigned
                | TokenKind::KwSigned
        ) {
            return Some(self.parse_integer_type());
        }
        let ty = match self.peek_kind() {
            TokenKind::KwVoid => {
                self.pos += 1;
                Type::Void
            }
            TokenKind::KwDouble => {
                self.pos += 1;
                Type::Double
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
            TokenKind::Ident(name) if name == "va_list" => {
                // va_list 用一个指针表示（指向当前可变参数位置）
                self.pos += 1;
                Type::Pointer(Box::new(Type::Void))
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
        Some(ty)
    }

    /// 尝试解析函数指针声明符 `(*name)(参数表)`，成功则返回 (名字, FnPtr 类型)。
    /// 参数表按括号配平跳过（参数类型不参与代码生成）。`ret` 为返回类型。
    fn try_fnptr_declarator(
        &mut self,
        ret: &Type,
    ) -> Result<Option<(String, Type)>, CompileError> {
        let is_fnptr = *self.peek_kind() == TokenKind::LParen
            && matches!(
                self.tokens.get(self.pos + 1).map(|t| &t.kind),
                Some(TokenKind::Star)
            );
        if !is_fnptr {
            return Ok(None);
        }
        self.pos += 1; // (
        self.pos += 1; // *
        while *self.peek_kind() == TokenKind::Star {
            self.pos += 1; // 多级指针，仍按函数指针处理
        }
        let name = if let TokenKind::Ident(n) = self.peek_kind() {
            let n = n.clone();
            self.pos += 1;
            n
        } else {
            String::new()
        };
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LParen)?;
        let mut depth = 1;
        while depth > 0 {
            match self.peek_kind() {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => depth -= 1,
                TokenKind::Eof => {
                    return Err(CompileError::new(
                        self.tokens[self.pos].span,
                        "unterminated function-pointer parameter list".to_string(),
                    ))
                }
                _ => {}
            }
            self.pos += 1;
        }
        Ok(Some((name, Type::FnPtr(Box::new(ret.clone())))))
    }

    /// 类型说明符：基础类型后跟零个或多个指针 `*`。
    fn parse_type_specifier(&mut self) -> Option<Type> {
        let mut ty = self.parse_base_type()?;
        while *self.peek_kind() == TokenKind::Star {
            self.pos += 1;
            ty = Type::Pointer(Box::new(ty));
        }
        Some(ty)
    }

    /// 注册一个全局变量声明符（多维数组后缀 + 可选初始化器,含聚合 `{...}`；不消费分号）。
    fn parse_global_declarator(&mut self, mut ty: Type, name: String) -> Result<(), CompileError> {
        // 多维数组后缀,最外层可空(由初始化列表推断长度)
        let mut dims: Vec<Option<usize>> = Vec::new();
        while *self.peek_kind() == TokenKind::LBracket {
            self.pos += 1;
            if *self.peek_kind() == TokenKind::RBracket {
                dims.push(None);
                self.pos += 1;
            } else {
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
                dims.push(Some(n));
            }
        }
        let infer = dims.first().map(|d| d.is_none()).unwrap_or(false);
        for dim in dims.iter().rev() {
            ty = Type::Array(Box::new(ty), dim.unwrap_or(0));
        }
        let init = if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            let e = self.parse_initializer()?;
            if infer {
                if let Expr::InitList(items) = &e {
                    if let Type::Array(elem, _) = &ty {
                        ty = Type::Array(elem.clone(), items.len());
                    }
                }
            }
            Some(e)
        } else {
            None
        };
        self.global_types.insert(name.clone(), ty.clone());
        // `extern`(无初始化器)只引用外部符号,不生成存储 —— 由链接器解析(如 stdin/stdout/stderr)。
        let is_extern = self.saw_extern && init.is_none();
        self.globals.push(Global { name, ty, init, is_extern });
        Ok(())
    }

    /// 解析函数定义或原型声明。原型返回 None（仅注册签名）。
    fn parse_func_or_proto(&mut self) -> Result<Option<FuncDef>, CompileError> {
        // 基础类型在多个声明符间共享；指针 `*` 是每个声明符各自的。
        let base = self.parse_base_type().ok_or_else(|| {
            CompileError::new(self.tokens[self.pos].span, "expected return type".to_string())
        })?;
        let mut ty = base.clone();
        while *self.peek_kind() == TokenKind::Star {
            self.pos += 1;
            ty = Type::Pointer(Box::new(ty));
        }
        let name = self.expect_ident()?;
        // 非 '(' → 全局变量声明（可多声明符）
        if *self.peek_kind() != TokenKind::LParen {
            self.parse_global_declarator(ty, name)?;
            while *self.peek_kind() == TokenKind::Comma {
                self.pos += 1;
                let mut t = base.clone();
                while *self.peek_kind() == TokenKind::Star {
                    self.pos += 1;
                    t = Type::Pointer(Box::new(t));
                }
                let nm = self.expect_ident()?;
                self.parse_global_declarator(t, nm)?;
            }
            self.expect(&TokenKind::Semicolon)?;
            return Ok(None);
        }
        let ret = ty;
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
                // 函数指针参数 RET (*name)(...)
                if let Some((pname, fty)) = self.try_fnptr_declarator(&ty)? {
                    params.push((pname, fty));
                    if *self.peek_kind() == TokenKind::Comma {
                        self.pos += 1;
                        continue;
                    } else {
                        break;
                    }
                }
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
                params: params.iter().map(|(_, t)| t.clone()).collect(),
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
            variadic,
        }))
    }

    fn parse_stmt(&mut self) -> Result<Stmt, CompileError> {
        match self.peek_kind() {
            TokenKind::KwReturn => return self.parse_return(),
            TokenKind::KwIf => return self.parse_if(),
            TokenKind::KwWhile => return self.parse_while(),
            TokenKind::KwDo => return self.parse_do(),
            TokenKind::KwFor => return self.parse_for(),
            TokenKind::KwSwitch => return self.parse_switch(),
            TokenKind::LBrace => return self.parse_block(),
            TokenKind::KwBreak => {
                self.pos += 1;
                self.expect(&TokenKind::Semicolon)?;
                return Ok(Stmt::Break);
            }
            TokenKind::KwContinue => {
                self.pos += 1;
                self.expect(&TokenKind::Semicolon)?;
                return Ok(Stmt::Continue);
            }
            TokenKind::KwGoto => {
                self.pos += 1;
                let label = self.expect_ident()?;
                self.expect(&TokenKind::Semicolon)?;
                return Ok(Stmt::Goto(label));
            }
            // 标签：`ident :`（其后是冒号；与表达式语句区分）
            TokenKind::Ident(_)
                if self.tokens.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Colon) =>
            {
                let label = self.expect_ident()?;
                self.expect(&TokenKind::Colon)?;
                return Ok(Stmt::Label(label));
            }
            TokenKind::KwCase => {
                self.pos += 1;
                let e = self.parse_expr()?;
                self.expect(&TokenKind::Colon)?;
                match e {
                    Expr::IntLit(v) => return Ok(Stmt::Case(v)),
                    _ => {
                        return Err(CompileError::new(
                            self.tokens[self.pos].span,
                            "case label must be an integer constant".to_string(),
                        ))
                    }
                }
            }
            TokenKind::KwDefault => {
                self.pos += 1;
                self.expect(&TokenKind::Colon)?;
                return Ok(Stmt::Default);
            }
            TokenKind::Semicolon => {
                self.pos += 1;
                return Ok(Stmt::Empty);
            }
            _ => {}
        }
        if self.at_type_start() {
            return self.parse_declaration();
        }
        let e = self.parse_comma()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::ExprStmt(e))
    }

    fn parse_switch(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwSwitch)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_expr()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while *self.peek_kind() != TokenKind::RBrace {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(Stmt::Switch { cond, body })
    }

    fn parse_return(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwReturn)?;
        let expr = self.parse_comma()?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::Return(expr))
    }

    fn parse_declaration(&mut self) -> Result<Stmt, CompileError> {
        // 基础类型在多个声明符间共享；指针 `*` 与数组 `[N]` 是每个声明符各自的。
        let base = self
            .parse_base_type()
            .expect("parse_declaration called without a type");
        let mut decls = Vec::new();
        loop {
            // 该声明符自己的指针层级
            let mut ty = base.clone();
            while *self.peek_kind() == TokenKind::Star {
                self.pos += 1;
                ty = Type::Pointer(Box::new(ty));
            }
            // 函数指针声明符 RET (*name)(...)
            if let Some((name, fty)) = self.try_fnptr_declarator(&ty)? {
                let init = if *self.peek_kind() == TokenKind::Assign {
                    self.pos += 1;
                    Some(self.parse_assign()?)
                } else {
                    None
                };
                decls.push(Stmt::Declare { name, ty: fty, init });
                if *self.peek_kind() == TokenKind::Comma {
                    self.pos += 1;
                    continue;
                } else {
                    break;
                }
            }
            let name = self.expect_ident()?;
            // 数组后缀，可多维 name[N][M]…；最外层可空 name[]（长度由初始化列表推断）
            let mut dims: Vec<Option<usize>> = Vec::new();
            while *self.peek_kind() == TokenKind::LBracket {
                self.pos += 1;
                if *self.peek_kind() == TokenKind::RBracket {
                    dims.push(None); // 推断（仅最外层有意义）
                    self.pos += 1;
                } else {
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
                    dims.push(Some(n));
                }
            }
            let infer_size = dims.first().map(|d| d.is_none()).unwrap_or(false);
            // 由内向外嵌套：a[3][4] → Array(Array(elem,4),3)
            for dim in dims.iter().rev() {
                ty = Type::Array(Box::new(ty), dim.unwrap_or(0));
            }
            let init = if *self.peek_kind() == TokenKind::Assign {
                self.pos += 1;
                let e = self.parse_initializer()?;
                // int a[] = {...}：用初始化列表元素个数定数组长度
                if infer_size {
                    if let Expr::InitList(items) = &e {
                        if let Type::Array(elem, _) = &ty {
                            ty = Type::Array(elem.clone(), items.len());
                        }
                    }
                }
                Some(e)
            } else {
                None
            };
            decls.push(Stmt::Declare { name, ty, init });
            if *self.peek_kind() == TokenKind::Comma {
                self.pos += 1;
            } else {
                break;
            }
        }
        self.expect(&TokenKind::Semicolon)?;
        if decls.len() == 1 {
            Ok(decls.pop().unwrap())
        } else {
            Ok(Stmt::Decls(decls))
        }
    }

    /// 初始化器：花括号聚合列表 `{a, b, ...}`（可嵌套、允许尾逗号）或单个赋值级表达式。
    fn parse_initializer(&mut self) -> Result<Expr, CompileError> {
        if *self.peek_kind() == TokenKind::LBrace {
            self.pos += 1;
            let mut items = Vec::new();
            while *self.peek_kind() != TokenKind::RBrace {
                items.push(self.parse_initializer()?);
                if *self.peek_kind() == TokenKind::Comma {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace)?;
            Ok(Expr::InitList(items))
        } else {
            self.parse_assign()
        }
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

    fn parse_do(&mut self) -> Result<Stmt, CompileError> {
        self.expect(&TokenKind::KwDo)?;
        let body = Box::new(self.parse_stmt()?);
        self.expect(&TokenKind::KwWhile)?;
        self.expect(&TokenKind::LParen)?;
        let cond = self.parse_comma()?;
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::Semicolon)?;
        Ok(Stmt::DoWhile { body, cond })
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
            let e = self.parse_comma()?;
            self.expect(&TokenKind::Semicolon)?;
            Some(Box::new(Stmt::ExprStmt(e)))
        };
        let cond = if *self.peek_kind() == TokenKind::Semicolon {
            None
        } else {
            Some(self.parse_comma()?)
        };
        self.expect(&TokenKind::Semicolon)?;
        let step = if *self.peek_kind() == TokenKind::RParen {
            None
        } else {
            Some(self.parse_comma()?)
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

    /// 赋值级表达式（不含逗号运算符）——用于实参、初始化器、下标等以逗号分隔的语境。
    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_assign()
    }

    /// 完整表达式，含逗号运算符（最低优先级）——用于表达式语句、return、for 子句、括号内。
    fn parse_comma(&mut self) -> Result<Expr, CompileError> {
        let mut e = self.parse_assign()?;
        while *self.peek_kind() == TokenKind::Comma {
            self.pos += 1;
            let rhs = self.parse_assign()?;
            e = Expr::Comma {
                first: Box::new(e),
                second: Box::new(rhs),
            };
        }
        Ok(e)
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

    /// 当前在 `(`，其后是否为类型起始（用于区分强转 `(int)x` 与括号表达式 `(x)`）。
    fn is_cast_ahead(&self) -> bool {
        match self.tokens.get(self.pos + 1).map(|t| &t.kind) {
            Some(
                TokenKind::KwInt
                | TokenKind::KwChar
                | TokenKind::KwDouble
                | TokenKind::KwLong
                | TokenKind::KwShort
                | TokenKind::KwUnsigned
                | TokenKind::KwSigned
                | TokenKind::KwVoid
                | TokenKind::KwConst
                | TokenKind::KwStruct
                | TokenKind::KwUnion
                | TokenKind::KwEnum,
            ) => true,
            Some(TokenKind::Ident(n)) => self.typedefs.contains_key(n),
            _ => false,
        }
    }

    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        match self.peek_kind() {
            TokenKind::LParen if self.is_cast_ahead() => {
                self.pos += 1; // (
                let ty = self.parse_type_specifier().ok_or_else(|| {
                    CompileError::new(self.tokens[self.pos].span, "expected cast type".to_string())
                })?;
                self.expect(&TokenKind::RParen)?;
                let operand = self.parse_unary()?;
                Ok(Expr::Cast {
                    ty,
                    expr: Box::new(operand),
                })
            }
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
                // sizeof(type) 需要括号且括号后紧跟类型；否则按 sizeof 一元表达式（可不带括号）
                if *self.peek_kind() == TokenKind::LParen && self.is_cast_ahead() {
                    self.pos += 1; // (
                    let ty = self.parse_type_specifier().ok_or_else(|| {
                        CompileError::new(
                            self.tokens[self.pos].span,
                            "expected type in sizeof".to_string(),
                        )
                    })?;
                    self.expect(&TokenKind::RParen)?;
                    Ok(Expr::SizeofType(ty))
                } else {
                    let e = self.parse_unary()?;
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
                // 对任意表达式的调用 `expr(args)` → 经函数指针的间接调用（如 (*f)(x)）
                TokenKind::LParen => {
                    self.pos += 1;
                    let mut args = Vec::new();
                    while *self.peek_kind() != TokenKind::RParen {
                        args.push(self.parse_assign()?);
                        if *self.peek_kind() == TokenKind::Comma {
                            self.pos += 1;
                        } else {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen)?;
                    e = Expr::CallPtr {
                        func: Box::new(e),
                        args,
                    };
                }
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
            TokenKind::FloatLit(v) => {
                let v = *v;
                self.pos += 1;
                Ok(Expr::FloatLit(v))
            }
            TokenKind::StrLit(s) => {
                // 相邻字符串字面量拼接："foo" "bar" → "foobar"
                let mut s = s.clone();
                self.pos += 1;
                while let TokenKind::StrLit(next) = self.peek_kind() {
                    s.push_str(next);
                    self.pos += 1;
                }
                Ok(Expr::StrLit(s))
            }
            TokenKind::Ident(name)
                if matches!(name.as_str(), "va_start" | "va_arg" | "va_end")
                    && self.tokens.get(self.pos + 1).map(|t| &t.kind)
                        == Some(&TokenKind::LParen) =>
            {
                let which = name.clone();
                self.pos += 1; // 名字
                self.expect(&TokenKind::LParen)?;
                let ap = self.parse_assign()?; // va_list 变量
                let expr = match which.as_str() {
                    "va_arg" => {
                        self.expect(&TokenKind::Comma)?;
                        let ty = self.parse_type_specifier().ok_or_else(|| {
                            CompileError::new(
                                self.tokens[self.pos].span,
                                "expected type in va_arg".to_string(),
                            )
                        })?;
                        Expr::VaArg {
                            ap: Box::new(ap),
                            ty,
                        }
                    }
                    "va_start" => {
                        // 第二个参数（最后一个具名形参）解析后丢弃
                        if *self.peek_kind() == TokenKind::Comma {
                            self.pos += 1;
                            let _ = self.parse_assign()?;
                        }
                        Expr::VaStart { ap: Box::new(ap) }
                    }
                    // va_end(ap)：无操作，求值为 0
                    _ => Expr::IntLit(0),
                };
                self.expect(&TokenKind::RParen)?;
                Ok(expr)
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
                let e = self.parse_comma()?;
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
