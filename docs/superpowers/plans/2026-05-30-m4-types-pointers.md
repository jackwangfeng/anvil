# M4 类型系统与指针 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]`.

**Goal:** 引入类型系统（`int`/`char`/`T*`/`T[N]`），支持取址 `&`、解引用 `*`、指针算术（按元素大小缩放）、数组声明与下标 `a[i]`、`sizeof`、`char` 类型、带类型的声明与参数；字符串字面量类型化为 `char*`。

**Architecture:** 新增 `src/types.rs` 定义 `Type` 与 `size()`。栈帧从"定长 8 字节槽 × 索引"升级为**按字节偏移分配器**：`Temp` 改义为"帧内字节偏移"，codegen `slot(t)=t`；临时量占 8 字节，变量占 `align8(sizeof(type))` 字节。Lowerer 的符号表记录 `name → (offset, Type)`；`lower_expr` 返回 `(Temp, Type)`，新增 `lower_lvalue` 返回左值地址。新增 IR 指令 `AddrOf/LoadInd/StoreInd/PtrAdd/PtrSub` 与带宽度的 `Copy`。codegen 用 `add x9,sp,#off` 取址、按宽度 `ldr/str`（含 `ldrsb/strb` 处理 char）、`add x9,x9,w10,sxtw #shift` 做 64 位指针算术。

**Tech Stack:** Rust（无第三方 crate）、Cargo、系统 `clang`、macOS/AArch64。cargo 前 `source "$HOME/.cargo/env"`。

---

## 范围与取舍（实现者必读）

**包含**：`int`、`char`、指针 `T*`（多级）、数组 `T[N]`（一维）；`&lvalue`、`*ptr`（左/右值）、`a[i]`（左/右值，= `*(a+i)`）；指针±整数（按 `sizeof(pointee)` 缩放，元素大小均为 2 的幂）；数组到指针的退化（decay）；`sizeof(类型名)` 与 `sizeof(表达式)`；带类型的局部声明 `T name;` / `T name = init;` / `T name[N];` 与函数参数 `T name`；字符串字面量类型 `char*`。

**不包含（后续/明确砍掉）**：指针−指针求差、指针比较、强制类型转换 `(T)x`、多维数组、全局变量、`unsigned`/`short`/`long`、结构体（M5）、函数指针。`char` 取有符号（`ldrsb`）。

## 关键设计

**1. `Type` 与大小/对齐**：`Int`=4、`Char`=1、`Pointer`=8、`Array(elem,n)`=`elem.size()*n`。帧内每个分配按 8 字节对齐（简单且正确，略浪费）。

**2. 帧分配器（Temp = 字节偏移）**：Lowerer 维护 `next_offset`（从 0 起，始终 8 对齐）。`fresh()` 临时量：返回当前 offset，`next_offset += 8`。`declare_var(name,ty)`：变量返回当前 offset，`next_offset += align8(ty.size())`。`Function` 字段 `num_temps` 改名为 `frame_bytes`（最终 `next_offset`）。codegen `slot(t) = t`（偏移即地址内偏移），`frame_size = align16(frame_bytes)`。

**3. 值模型与宽度**：临时量槽 8 字节。标量值在槽内：int/char 存低 32 位（`ldr w`/`str w`，宽度 4），指针存 64 位（`ldr x`/`str x`，宽度 8）。`copy_width(ty)`：`Pointer`→8，否则→4。**间接访问**（解引用/下标）宽度 = 标量 pointee 的 `size()`：`char`→1（`ldrsb`/`strb`）、`int`→4（`ldr w`/`str w`）、`Pointer`→8（`ldr x`/`str x`）。

**4. 左值/右值**：
- `lower_lvalue(e) -> (addr_temp, Type)`：返回左值**地址**所在临时量及其（被指）类型。支持 `Var`（`AddrOf` 取局部地址）、`*p`（地址 = p 的指针值）、`a[i]`（地址 = base + i*size）。
- `lower_expr(e) -> (val_temp, Type)`：右值。`Var` 标量→`Copy` 载值；`Var` 数组→**退化**为首元素地址（`AddrOf`，类型 `Pointer(elem)`）；`*p`→`LoadInd`；`a[i]`→等价 `*(a+i)`；`&lv`→`lower_lvalue` 的地址，类型 `Pointer(被指类型)`；`sizeof`→`Const`。
- 赋值 `lv = rhs`：`(addr,ty)=lower_lvalue(lv)`；`(v,_)=lower_expr(rhs)`；若 `lv` 是局部变量直接 `Copy`(按 copy_width)，若是 `*p`/`a[i]` 用 `StoreInd`(按间接宽度)。统一处理：`lower_lvalue` 对“具名变量”也返回其地址，赋值一律 `StoreInd`——更简单一致。**采用统一 StoreInd / LoadInd 经地址访问所有具名变量**，避免区分两类左值。

**5. 指针算术**：`p + n`（p 指针或数组退化、n 整数）→ `PtrAdd{dst,base,index,shift}`，`shift=log2(sizeof(pointee))`。`p - n` → `PtrSub`。`n + p` 同 `p + n`。两侧皆整数 → 普通 `Bin`。codegen：`ldr x9,[base]; ldr w10,[index]; add/sub x9,x9,w10,sxtw #shift; str x9,[dst]`。

**6. 统一变量访问**：为简化，**所有具名变量经地址访问**：`Var` 右值 = `AddrOf` 取地址 + `LoadInd`（标量）或仅 `AddrOf`（数组退化）；赋值 = `AddrOf` + `StoreInd`。这样 `&x` 自然就是那个 `AddrOf` 的地址。

**已知边界**：未声明变量/类型不匹配多以 panic 或最简报错处理（完整语义检查后置）；`char` 有符号；数组按值传参不支持（数组只在本函数内用或退化为指针传递）。

---

## 现状（M3 已合并到 main）

- token：`KwInt KwReturn KwIf KwElse KwWhile KwFor Ident IntLit StrLit ( ) { } ; , + - * / % = < > <= >= == != Eof`。
- ast：`FuncDef{name,params:Vec<String>,body}`、`Stmt{Return,Declare{name,init},ExprStmt,Block,If,While,For,Empty}`、`Expr{IntLit,Var,StrLit,Call{name,args},Assign{name,value},Unary,Binary}`。
- ir：`Temp=usize`、`Program{functions,strings}`、`Function{name,body,num_temps}`、`Instr{Const,Bin,Neg,Load{dst,var},Store{var,src},Copy{dst,src},Label,Jump,JumpIfZero,Call,StrLit,LoadArg,Return}`、`Lowerer<'a>{body,next_temp,scopes:Vec<HashMap<String,Temp>>,next_label,strings}`。
- codegen：`slot(t)=t*8`、`frame_size`、序言 `stp x29,x30`/尾声 `ldp`、`gen_instr(instr,func,frame,out)`、标签 `L<func>_<n>`、cstring 段。
- tests/integration.rs：`compile_and_run`、`compile_run_capture`。

## 文件结构

- Create `src/types.rs` —— `Type` 枚举 + `size()`/`decay()`/`pointee()`。
- Modify `src/lib.rs` —— `pub mod types;`。
- Modify `src/token.rs` —— `Amp(&) LBracket([) RBracket(]) KwChar KwSizeof`。
- Modify `src/lexer.rs` —— 识别 `& [ ]`、关键字 `char`/`sizeof`。
- Modify `src/ast.rs` —— `Declare{name,ty,init}`、`FuncDef.params: Vec<(String,Type)>`、`Expr` 增 `Addr(Box<Expr>) Deref(Box<Expr>) Index{base,index} SizeofType(Type) SizeofExpr(Box<Expr>)`；`Assign` 改为 `Assign{target:Box<Expr>,value}`（左值是表达式）。
- Modify `src/parser.rs` —— 类型说明符解析（基类型 + `*` + `[N]`）、`sizeof`、一元 `& *`、后缀 `[]`、赋值左值为表达式。
- Modify `src/ir.rs` —— 帧偏移分配、符号表带类型、`lower_expr`返回类型、`lower_lvalue`、新指令、退化与指针算术。
- Modify `src/codegen.rs` —— `slot(t)=t`、`AddrOf/LoadInd/StoreInd/PtrAdd/PtrSub`、宽度化 `Copy`、`frame_size=align16(frame_bytes)`。
- Modify `tests/integration.rs` —— 指针/数组/sizeof/char 用例。

---

### Task 1: types.rs —— 类型与大小

**Files:** Create `src/types.rs`; Modify `src/lib.rs`

- [ ] **Step 1: 失败测试.** 在 `src/types.rs` 末尾：
```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes() {
        assert_eq!(Type::Int.size(), 4);
        assert_eq!(Type::Char.size(), 1);
        assert_eq!(Type::Pointer(Box::new(Type::Int)).size(), 8);
        assert_eq!(Type::Array(Box::new(Type::Int), 10).size(), 40);
    }

    #[test]
    fn decay_and_pointee() {
        let arr = Type::Array(Box::new(Type::Char), 5);
        assert_eq!(arr.decay(), Type::Pointer(Box::new(Type::Char)));
        assert_eq!(Type::Pointer(Box::new(Type::Int)).pointee(), Some(&Type::Int));
        assert_eq!(Type::Int.pointee(), None);
    }
}
```
`src/lib.rs` 增加 `pub mod types;`。

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib types`。

- [ ] **Step 3: 实现.** `src/types.rs` 主体：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Type {
    Int,
    Char,
    Pointer(Box<Type>),
    Array(Box<Type>, usize),
}

impl Type {
    pub fn size(&self) -> usize {
        match self {
            Type::Int => 4,
            Type::Char => 1,
            Type::Pointer(_) => 8,
            Type::Array(elem, n) => elem.size() * n,
        }
    }

    /// 数组退化为指向元素的指针；其余类型不变。
    pub fn decay(&self) -> Type {
        match self {
            Type::Array(elem, _) => Type::Pointer(elem.clone()),
            other => other.clone(),
        }
    }

    /// 指针或数组的被指/元素类型。
    pub fn pointee(&self) -> Option<&Type> {
        match self {
            Type::Pointer(t) | Type::Array(t, _) => Some(t),
            _ => None,
        }
    }

    pub fn is_pointer_like(&self) -> bool {
        matches!(self, Type::Pointer(_) | Type::Array(..))
    }
}
```

- [ ] **Step 4: 通过.** `source "$HOME/.cargo/env" && cargo test --lib types` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/types.rs src/lib.rs
git commit -m "feat(types): Type enum with size/decay/pointee"
```

---

### Task 2: lexer —— `& [ ]`、`char`/`sizeof`

**Files:** Modify `src/token.rs`, `src/lexer.rs`

- [ ] **Step 1: 失败测试.** 在 `src/lexer.rs` `mod tests` 内：
```rust
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
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib lex_m4_tokens`。

- [ ] **Step 3: 实现.** `src/token.rs`：在 `Comma,` 附近加 `Amp, LBracket, RBracket,`，关键字区加 `KwChar, KwSizeof,`（放在 `KwFor,` 之后、`Eof` 之前）。
`src/lexer.rs`：把 `&` `[` `]` 并入单字符标点分支字符集与内层映射：在 `'(' | ')' | ... | ','` 集合加入 `| '&' | '[' | ']'`，内层 match 增加 `'&' => TokenKind::Amp, '[' => TokenKind::LBracket, ']' => TokenKind::RBracket,`。关键字匹配增加 `"char" => TokenKind::KwChar, "sizeof" => TokenKind::KwSizeof,`。
（注意：`&&` 不在 M4 范围；单 `&` 即取址。）

- [ ] **Step 4: 通过.** `source "$HOME/.cargo/env" && cargo test --lib lexer` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/token.rs src/lexer.rs
git commit -m "feat(lexer): & [ ] tokens and char/sizeof keywords"
```

---

### Task 3: AST —— 类型化声明/参数与指针表达式

**Files:** Modify `src/ast.rs`

- [ ] **Step 1: 实现（随 parser 验证）.** 顶部加 `use crate::types::Type;`。
`FuncDef` 改：
```rust
pub struct FuncDef {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub body: Vec<Stmt>,
}
```
`Stmt::Declare` 改为带类型：
```rust
    Declare {
        name: String,
        ty: Type,
        init: Option<Expr>,
    },
```
`Expr`：把 `Assign{name,value}` 改为左值为表达式，并新增指针相关变体：
```rust
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Addr(Box<Expr>),
    Deref(Box<Expr>),
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    SizeofType(Type),
    SizeofExpr(Box<Expr>),
```
（保留 `IntLit,Var,StrLit,Call,Unary,Binary`。）

- [ ] **Step 2: 确认红灯指向 parser/ir（预期）.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 提交.**
```bash
git add src/ast.rs
git commit -m "feat(ast): typed declarations/params, addr/deref/index/sizeof, lvalue assign"
```

---

### Task 4: parser —— 类型说明符、sizeof、& * 与下标

**Files:** Modify `src/parser.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 顶部 `use` 增 `use crate::types::Type;`，新增：
```rust
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
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 实现.** 顶部 `use` 增 `use crate::types::Type;`。新增类型说明符解析（基类型 + `*`*）：
```rust
    /// 解析基础类型 + 零个或多个 `*`。返回 None 表示当前不是类型起始。
    fn try_parse_base_type(&mut self) -> Option<Type> {
        let base = match self.peek_kind() {
            TokenKind::KwInt => Type::Int,
            TokenKind::KwChar => Type::Char,
            _ => return None,
        };
        self.pos += 1;
        let mut ty = base;
        while *self.peek_kind() == TokenKind::Star {
            self.pos += 1;
            ty = Type::Pointer(Box::new(ty));
        }
        Some(ty)
    }
```
替换 `parse_declaration`（支持指针/数组）：
```rust
    fn parse_declaration(&mut self) -> Result<Stmt, CompileError> {
        let mut ty = self
            .try_parse_base_type()
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
```
`parse_stmt` 的声明分发条件由 `KwInt` 改为 `KwInt | KwChar`：
```rust
            TokenKind::KwInt | TokenKind::KwChar => self.parse_declaration(),
```
（`parse_for` 内对 init 是否声明的判断同样改为 `KwInt | KwChar`。）
替换 `parse_func_def` 的参数解析为带类型：
```rust
    fn parse_func_def(&mut self) -> Result<FuncDef, CompileError> {
        self.expect(&TokenKind::KwInt)?; // 返回类型当前固定 int
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        if *self.peek_kind() != TokenKind::RParen {
            loop {
                let ty = self
                    .try_parse_base_type()
                    .ok_or_else(|| CompileError::new(self.tokens[self.pos].span, "expected parameter type".to_string()))?;
                let pname = self.expect_ident()?;
                params.push((pname, ty));
                if *self.peek_kind() == TokenKind::Comma {
                    self.pos += 1;
                } else {
                    break;
                }
            }
        }
        self.expect(&TokenKind::RParen)?;
        self.expect(&TokenKind::LBrace)?;
        let mut body = Vec::new();
        while *self.peek_kind() != TokenKind::RBrace {
            body.push(self.parse_stmt()?);
        }
        self.expect(&TokenKind::RBrace)?;
        Ok(FuncDef { name, params, body })
    }
```
一元运算符增加 `& *` 与 `sizeof`，替换 `parse_unary`：
```rust
    fn parse_unary(&mut self) -> Result<Expr, CompileError> {
        match self.peek_kind() {
            TokenKind::Minus => {
                self.pos += 1;
                Ok(Expr::Unary { op: UnaryOp::Neg, operand: Box::new(self.parse_unary()?) })
            }
            TokenKind::Plus => {
                self.pos += 1;
                Ok(Expr::Unary { op: UnaryOp::Plus, operand: Box::new(self.parse_unary()?) })
            }
            TokenKind::Amp => {
                self.pos += 1;
                Ok(Expr::Addr(Box::new(self.parse_unary()?)))
            }
            TokenKind::Star => {
                self.pos += 1;
                Ok(Expr::Deref(Box::new(self.parse_unary()?)))
            }
            TokenKind::KwSizeof => {
                self.pos += 1;
                self.expect(&TokenKind::LParen)?;
                // sizeof(类型) 或 sizeof(表达式)
                if let Some(ty) = self.try_parse_base_type() {
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

    /// primary 后跟零个或多个下标 `[expr]`。
    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut e = self.parse_primary()?;
        while *self.peek_kind() == TokenKind::LBracket {
            self.pos += 1;
            let index = self.parse_expr()?;
            self.expect(&TokenKind::RBracket)?;
            e = Expr::Index { base: Box::new(e), index: Box::new(index) };
        }
        Ok(e)
    }
```
赋值左值改为表达式（替换 `parse_assign`）：
```rust
    fn parse_assign(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_bin_expr(1)?;
        if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            let value = self.parse_assign()?;
            // 合法左值：Var / Deref / Index
            match &lhs {
                Expr::Var(_) | Expr::Deref(_) | Expr::Index { .. } => Ok(Expr::Assign {
                    target: Box::new(lhs),
                    value: Box::new(value),
                }),
                _ => Err(CompileError::new(
                    self.tokens[self.pos.saturating_sub(1)].span,
                    "invalid assignment target".to_string(),
                )),
            }
        } else {
            Ok(lhs)
        }
    }
```
（`parse_bin_expr`→`parse_unary`→`parse_postfix`→`parse_primary` 链路；`parse_primary` 不变，仍处理 IntLit/StrLit/Ident-or-Call/LParen。）

- [ ] **Step 4: 确认（ir 非穷尽，先跑 parser）.** `source "$HOME/.cargo/env" && cargo test --lib parser 2>&1 | head -30`，确认报错仅来自 ir.rs 则进入 Task 5。

- [ ] **Step 5: 提交.**
```bash
git add src/parser.rs
git commit -m "feat(parser): type specifiers, arrays, sizeof, &/*, postfix index, lvalue assign"
```

---

### Task 5: IR —— 帧偏移、类型化降级、左值与指针算术

**Files:** Modify `src/ir.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增（保留旧的；注意旧测试里硬编码的 `dst:0,1,2` 现在是字节偏移 `0,8,16`，需同步更新——见下条）。新增：
```rust
    use crate::types::Type;

    #[test]
    fn lower_addr_of_var() {
        // int x; int* p = &x;  —— 应出现 AddrOf
        let f = lower_src("int main(){ int x; int* p; p = &x; return 0; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::AddrOf { .. })));
    }

    #[test]
    fn lower_deref_loadind() {
        let f = lower_src("int main(){ int* p; return *p; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::LoadInd { .. })));
    }

    #[test]
    fn lower_index_uses_ptradd() {
        // a[2]  => PtrAdd(scale int=4 -> shift 2) + LoadInd
        let f = lower_src("int main(){ int a[4]; return a[2]; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::PtrAdd { shift: 2, .. })));
    }
```
**更新旧 IR 测试的偏移**：把 `lower_add`/`lower_unary_neg`/`lower_const_return` 等里硬编码的 `dst:0,1,2`、`src:0,1`、`num_temps` 改为字节偏移版本。具体：临时量第 k 个（从 0）偏移 = `k*8`。例如 `lower_add` 改为：
```rust
    #[test]
    fn lower_add() {
        let f = lower_src("int main(){ return 1+2; }");
        assert_eq!(f.frame_bytes, 24);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 8, value: 2 },
                Instr::Bin { dst: 16, op: BinOp::Add, lhs: 0, rhs: 8 },
                Instr::Return { src: 16 },
            ]
        );
    }
```
`lower_const_return` 改 `num_temps`→`frame_bytes`，值 `1`→`8`，`dst:0` 不变；`lower_unary_neg`：`Const{dst:0}`、`Neg{dst:8,src:0}`、`Return{src:8}`；`lower_unary_plus_is_noop`：`frame_bytes`=8。其余结构性断言（`lower_if_*`、`lower_while_*`、`lower_declare_*`、`lower_call_*`、`lower_params_*`）用的是 `any/filter/matches!`，多数不受偏移影响——但凡断言 `num_temps` 的改为 `frame_bytes`。

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head`。

- [ ] **Step 3: 实现.** 顶部 `use` 增 `use crate::types::Type;`。`Function` 字段改名：
```rust
pub struct Function {
    pub name: String,
    pub body: Vec<Instr>,
    pub frame_bytes: usize,
}
```
`Instr` 增加（保留旧的，但 `Load/Store/Copy` 语义改为带宽度——见下）：
```rust
    AddrOf { dst: Temp, off: usize },
    LoadInd { dst: Temp, addr: Temp, width: usize, signed: bool },
    StoreInd { addr: Temp, src: Temp, width: usize },
    PtrAdd { dst: Temp, base: Temp, index: Temp, shift: u32 },
    PtrSub { dst: Temp, base: Temp, index: Temp, shift: u32 },
```
把 `Copy{dst,src}` 改为带宽度 `Copy{dst,src,width}`，并**移除 `Load/Store`**（统一用 AddrOf+LoadInd/StoreInd 访问具名变量；M2 引入的 Load/Store 不再需要）。即 `Instr` 删除 `Load`、`Store` 两个变体，`Copy` 变体改签名。
> 注意：删除 Load/Store 后，旧 M2 的 lower_stmt 里对它们的使用要改。
`Lowerer` 符号表改为存 `(offset, Type)`，加 `next_offset`：
```rust
struct Lowerer<'a> {
    body: Vec<Instr>,
    next_offset: usize,
    scopes: Vec<HashMap<String, (usize, Type)>>,
    next_label: usize,
    strings: &'a mut Vec<String>,
}
```
分配/查找：
```rust
    fn fresh(&mut self) -> Temp {
        let off = self.next_offset;
        self.next_offset += 8;
        off
    }

    fn declare_var(&mut self, name: &str, ty: Type) -> usize {
        let size = ty.size();
        let aligned = size.div_ceil(8) * 8;
        let off = self.next_offset;
        self.next_offset += aligned;
        self.scopes.last_mut().unwrap().insert(name.to_string(), (off, ty));
        off
    }

    fn lookup_var(&self, name: &str) -> Option<(usize, Type)> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }
```
宽度辅助（自由函数）：
```rust
/// 标量在槽内/局部访问的宽度：指针 8，其余 4。
fn copy_width(ty: &Type) -> usize {
    match ty {
        Type::Pointer(_) => 8,
        _ => 4,
    }
}

/// 解引用/下标的间接访问宽度 = 标量被指类型大小。
fn access_width(ty: &Type) -> usize {
    ty.size()
}
```
`lower_expr` 改为返回 `(Temp, Type)`，并实现退化与各表达式。完整替换 `lower_expr`：
```rust
    /// 返回 (存放右值结果的临时量, 类型)。
    fn lower_expr(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                (dst, Type::Int)
            }
            Expr::StrLit(s) => {
                let index = self.strings.len();
                self.strings.push(s.clone());
                let dst = self.fresh();
                self.body.push(Instr::StrLit { dst, index });
                (dst, Type::Pointer(Box::new(Type::Char)))
            }
            Expr::Var(name) => {
                let (off, ty) = self.lookup_var(name).expect("undeclared variable");
                match ty {
                    Type::Array(elem, _) => {
                        // 数组退化为首元素地址
                        let dst = self.fresh();
                        self.body.push(Instr::AddrOf { dst, off });
                        (dst, Type::Pointer(elem))
                    }
                    scalar => {
                        let addr = self.fresh();
                        self.body.push(Instr::AddrOf { dst: addr, off });
                        let dst = self.fresh();
                        self.body.push(Instr::LoadInd {
                            dst,
                            addr,
                            width: copy_width(&scalar),
                            signed: matches!(scalar, Type::Char),
                        });
                        (dst, scalar)
                    }
                }
            }
            Expr::Addr(inner) => {
                let (addr, ty) = self.lower_lvalue(inner);
                (addr, Type::Pointer(Box::new(ty)))
            }
            Expr::Deref(inner) => {
                let (ptr, ty) = self.lower_expr(inner);
                let pointee = ty.decay().pointee().expect("deref of non-pointer").clone();
                let dst = self.fresh();
                self.body.push(Instr::LoadInd {
                    dst,
                    addr: ptr,
                    width: access_width(&pointee),
                    signed: matches!(pointee, Type::Char),
                });
                (dst, pointee)
            }
            Expr::Index { base, index } => {
                // a[i] == *(a + i)
                let addr = self.lower_index_addr(base, index);
                let (ptr, pointee) = addr;
                let dst = self.fresh();
                self.body.push(Instr::LoadInd {
                    dst,
                    addr: ptr,
                    width: access_width(&pointee),
                    signed: matches!(pointee, Type::Char),
                });
                (dst, pointee)
            }
            Expr::SizeofType(ty) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: ty.size() as i64 });
                (dst, Type::Int)
            }
            Expr::SizeofExpr(inner) => {
                // 不求值，仅取类型大小（M4 简化：仍降级以取类型，但丢弃指令影响——这里直接计算类型）
                let ty = self.type_of(inner);
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: ty.size() as i64 });
                (dst, Type::Int)
            }
            Expr::Unary { op, operand } => {
                let (src, _ty) = self.lower_expr(operand);
                match op {
                    UnaryOp::Plus => (src, Type::Int),
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.body.push(Instr::Neg { dst, src });
                        (dst, Type::Int)
                    }
                }
            }
            Expr::Binary { op, lhs, rhs } => self.lower_binary(*op, lhs, rhs),
            Expr::Assign { target, value } => {
                let (v, vty) = self.lower_expr(value);
                let (addr, ty) = self.lower_lvalue(target);
                self.body.push(Instr::StoreInd {
                    addr,
                    src: v,
                    width: access_width(&ty),
                });
                (v, vty)
            }
            Expr::Call { name, args } => {
                let arg_temps: Vec<Temp> = args.iter().map(|a| self.lower_expr(a).0).collect();
                let dst = self.fresh();
                self.body.push(Instr::Call {
                    dst,
                    name: name.clone(),
                    args: arg_temps,
                });
                (dst, Type::Int)
            }
        }
    }
```
辅助：左值地址、下标地址、二元（含指针算术）、类型推断、binop 缩放：
```rust
    /// 返回 (左值地址临时量, 被指类型)。
    fn lower_lvalue(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::Var(name) => {
                let (off, ty) = self.lookup_var(name).expect("undeclared variable");
                let dst = self.fresh();
                self.body.push(Instr::AddrOf { dst, off });
                (dst, ty)
            }
            Expr::Deref(inner) => {
                let (ptr, ty) = self.lower_expr(inner);
                let pointee = ty.decay().pointee().expect("deref of non-pointer").clone();
                (ptr, pointee)
            }
            Expr::Index { base, index } => self.lower_index_addr(base, index),
            other => panic!("not an lvalue: {:?}", other),
        }
    }

    /// a[i] 的地址：base 退化为指针，addr = base + i*sizeof(elem)。返回 (地址临时量, 元素类型)。
    fn lower_index_addr(&mut self, base: &Expr, index: &Expr) -> (Temp, Type) {
        let (ptr, pty) = self.lower_expr(base); // 数组在 lower_expr 已退化为指针
        let elem = pty.decay().pointee().expect("index of non-pointer").clone();
        let (idx, _) = self.lower_expr(index);
        let dst = self.fresh();
        self.body.push(Instr::PtrAdd {
            dst,
            base: ptr,
            index: idx,
            shift: shift_of(elem.size()),
        });
        (dst, elem)
    }

    fn lower_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> (Temp, Type) {
        let (l, lty) = self.lower_expr(lhs);
        let (r, rty) = self.lower_expr(rhs);
        // 指针 ± 整数
        let l_ptr = lty.is_pointer_like();
        let r_ptr = rty.is_pointer_like();
        if (op == BinaryOp::Add || op == BinaryOp::Sub) && (l_ptr ^ r_ptr) {
            let (ptr, pty, idx) = if l_ptr { (l, lty.clone(), r) } else { (r, rty.clone(), l) };
            let elem = pty.decay().pointee().unwrap().clone();
            let dst = self.fresh();
            let shift = shift_of(elem.size());
            if op == BinaryOp::Add {
                self.body.push(Instr::PtrAdd { dst, base: ptr, index: idx, shift });
            } else {
                self.body.push(Instr::PtrSub { dst, base: ptr, index: idx, shift });
            }
            return (dst, pty.decay());
        }
        // 普通整数运算
        let dst = self.fresh();
        self.body.push(Instr::Bin { dst, op: lower_binop(op), lhs: l, rhs: r });
        (dst, Type::Int)
    }

    /// 仅推断表达式类型（用于 sizeof(expr)，不产生有意义副作用时仍复用 lower 的逻辑过于复杂，简化：递归推断）。
    fn type_of(&self, e: &Expr) -> Type {
        match e {
            Expr::IntLit(_) | Expr::Unary { .. } | Expr::Binary { .. } | Expr::Call { .. } | Expr::SizeofType(_) | Expr::SizeofExpr(_) => Type::Int,
            Expr::StrLit(_) => Type::Pointer(Box::new(Type::Char)),
            Expr::Var(name) => self.lookup_var(name).map(|(_, t)| t.decay()).unwrap_or(Type::Int),
            Expr::Addr(inner) => Type::Pointer(Box::new(self.type_of_lvalue(inner))),
            Expr::Deref(inner) => self.type_of(inner).decay().pointee().cloned().unwrap_or(Type::Int),
            Expr::Index { base, .. } => self.type_of(base).decay().pointee().cloned().unwrap_or(Type::Int),
            Expr::Assign { value, .. } => self.type_of(value),
        }
    }

    fn type_of_lvalue(&self, e: &Expr) -> Type {
        match e {
            Expr::Var(name) => self.lookup_var(name).map(|(_, t)| t).unwrap_or(Type::Int),
            Expr::Deref(inner) => self.type_of(inner).decay().pointee().cloned().unwrap_or(Type::Int),
            Expr::Index { base, .. } => self.type_of(base).decay().pointee().cloned().unwrap_or(Type::Int),
            _ => Type::Int,
        }
    }
```
自由函数 `shift_of`（元素大小→log2）：
```rust
fn shift_of(size: usize) -> u32 {
    match size {
        1 => 0,
        2 => 1,
        4 => 2,
        8 => 3,
        _ => 0, // M4 元素大小均为 2 的幂；其它退化为字节寻址
    }
}
```
`lower_stmt`：`Declare` 改为用类型声明并按 copy_width 存初值；用 StoreInd 写入。替换 `Declare` 与 `Return`/`ExprStmt` 中对变量的处理为统一 lvalue 风格。具体把 `lower_stmt` 的 `Declare` 分支改为：
```rust
            Stmt::Declare { name, ty, init } => {
                let off = self.declare_var(name, ty.clone());
                if let Some(e) = init {
                    let (v, _) = self.lower_expr(e);
                    let addr = self.fresh();
                    self.body.push(Instr::AddrOf { dst: addr, off });
                    self.body.push(Instr::StoreInd {
                        addr,
                        src: v,
                        width: copy_width(ty),
                    });
                }
            }
```
（`If/While/For/Block/Return/ExprStmt/Empty` 逻辑不变，但其中 `lower_expr` 现返回元组，凡用到处取 `.0`：`Return(e)` → `let (src,_) = self.lower_expr(e);`；`JumpIfZero` 的 cond、`For` 的 cond/step、`ExprStmt` 均改取 `.0`。）
`lower_binop` 不变。`lower_func` 改用类型化参数（参数按 copy_width 从入参寄存器存入其槽）：
```rust
fn lower_func(f: &FuncDef, strings: &mut Vec<String>) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_offset: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
        strings,
    };
    for (index, (pname, pty)) in f.params.iter().enumerate() {
        let off = lw.declare_var(pname, pty.clone());
        let addr = lw.fresh();
        lw.body.push(Instr::AddrOf { dst: addr, off });
        // 入参在寄存器 index；先落到一个临时量再 StoreInd
        let tmp = lw.fresh();
        lw.body.push(Instr::LoadArg { dst: tmp, index });
        lw.body.push(Instr::StoreInd { addr, src: tmp, width: copy_width(pty) });
    }
    for stmt in &f.body {
        lw.lower_stmt(stmt);
    }
    Function {
        name: f.name.clone(),
        body: lw.body,
        frame_bytes: lw.next_offset,
    }
}
```
删除不再使用的旧 `Instr::Load/Store` 相关代码（lower_stmt 里 M2 的 Declare 用过 Store；已被上面替换）。

- [ ] **Step 4: 确认（codegen 非穷尽，先跑 ir）.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head -40`，确认其余报错仅来自 codegen.rs，进入 Task 6。

- [ ] **Step 5: 提交.**
```bash
git add src/ir.rs
git commit -m "feat(ir): byte-offset frame, typed lowering, lvalues, pointer arithmetic"
```

---

### Task 6: codegen —— 偏移寻址、间接读写、取址、指针算术

**Files:** Modify `src/codegen.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 的 `gen` 辅助：把 `num_temps` 改为 `frame_bytes`。并更新旧测试中涉及帧大小/偏移的断言：
  - `codegen_add_uses_add_instr`：原 `num_temps:3`→`frame_bytes:24`（仍 `sub sp, sp, #32`）；`dst`/`lhs`/`rhs` 由索引 0,1,2 改为偏移 0,8,16，且断言里 `[sp, #...]` 随之；`add w9, w9, w10` 不变。
  - `codegen_const_return`/`codegen_compare_uses_cset`/`codegen_mod_uses_msub`/`codegen_control_flow`/`codegen_load_store_roundtrip`/`codegen_prologue_saves_fp_lr`/`codegen_call_and_loadarg`/`codegen_strlit_section`/`codegen_labels_prefixed_by_func`：把构造里的 `num_temps` 改 `frame_bytes`（数值 = 槽数×8），`dst/src/lhs/rhs/cond/...` 索引改为字节偏移（×8）；`[sp,#0]` 等断言中偏移随之（slot(0)=0 不变，其余 ×8）。`codegen_load_store_roundtrip` 现无 Load/Store 指令——改为测试 `Copy{dst,src,width}` 或删除并由新指令测试覆盖（见下，建议删除该测试，新增 AddrOf/LoadInd/StoreInd 测试）。
新增：
```rust
    #[test]
    fn codegen_addr_of() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![Instr::AddrOf { dst: 8, off: 0 }, Instr::Return { src: 8 }],
                frame_bytes: 16,
            }],
            strings: vec![],
        });
        assert!(asm.contains("add x9, sp, #0"));
        assert!(asm.contains("str x9, [sp, #8]"));
    }

    #[test]
    fn codegen_load_store_ind_widths() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::LoadInd { dst: 8, addr: 0, width: 4, signed: false },
                    Instr::LoadInd { dst: 16, addr: 0, width: 1, signed: true },
                    Instr::StoreInd { addr: 0, src: 8, width: 8 },
                    Instr::Return { src: 8 },
                ],
                frame_bytes: 24,
            }],
            strings: vec![],
        });
        assert!(asm.contains("ldr w10, [x9]"));   // width 4 间接读
        assert!(asm.contains("ldrsb w10, [x9]")); // width 1 有符号
        assert!(asm.contains("str x10, [x9]"));   // width 8 间接写
    }

    #[test]
    fn codegen_ptradd_scales() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::PtrAdd { dst: 16, base: 0, index: 8, shift: 2 },
                    Instr::Return { src: 16 },
                ],
                frame_bytes: 24,
            }],
            strings: vec![],
        });
        assert!(asm.contains("add x9, x9, w10, sxtw #2"));
    }
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib codegen 2>&1 | head`。

- [ ] **Step 3: 实现.** `slot` 改为恒等、`frame_size` 改名语义为按字节：
```rust
/// 槽位即帧内字节偏移。
fn slot(t: usize) -> usize {
    t
}

/// 栈帧大小：frame_bytes 向上对齐到 16。
fn frame_size(frame_bytes: usize) -> usize {
    frame_bytes.div_ceil(16) * 16
}
```
`gen_func` 用 `func.frame_bytes`：`let frame = frame_size(func.frame_bytes);`。
`gen_instr` 删除 `Load/Store` 分支（已不存在该 Instr），`Copy` 改为带宽度，并新增 `AddrOf/LoadInd/StoreInd/PtrAdd/PtrSub`：
```rust
        Instr::Copy { dst, src, width } => {
            if *width == 8 {
                let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*src));
                let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
            }
        }
        Instr::AddrOf { dst, off } => {
            let _ = writeln!(out, "    add x9, sp, #{}", off);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::LoadInd { dst, addr, width, signed } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*addr));
            match (*width, *signed) {
                (1, true) => out.push_str("    ldrsb w10, [x9]\n"),
                (1, false) => out.push_str("    ldrb w10, [x9]\n"),
                (8, _) => out.push_str("    ldr x10, [x9]\n"),
                _ => out.push_str("    ldr w10, [x9]\n"),
            }
            if *width == 8 {
                let _ = writeln!(out, "    str x10, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    str w10, [sp, #{}]", slot(*dst));
            }
        }
        Instr::StoreInd { addr, src, width } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*addr));
            if *width == 8 {
                let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*src));
                out.push_str("    str x10, [x9]\n");
            } else {
                let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*src));
                match *width {
                    1 => out.push_str("    strb w10, [x9]\n"),
                    _ => out.push_str("    str w10, [x9]\n"),
                }
            }
        }
        Instr::PtrAdd { dst, base, index, shift } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*index));
            let _ = writeln!(out, "    add x9, x9, w10, sxtw #{}", shift);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::PtrSub { dst, base, index, shift } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*index));
            let _ = writeln!(out, "    sub x9, x9, w10, sxtw #{}", shift);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
```
（`Const/Neg/Label/Jump/JumpIfZero/LoadArg/StrLit/Call/Bin/Return` 不变。`materialize_const` 不变。）

- [ ] **Step 4: 通过 + 真机端到端（关键）.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`，`cargo clippy --all-targets 2>&1 | grep -E "warning|error"`（应空）。真机验证：
```bash
source "$HOME/.cargo/env" && cargo build -q
# 指针读写
echo 'int main(){ int x=5; int* p=&x; *p=9; return x; }' > /tmp/p.c
./target/debug/bianyi /tmp/p.c -o /tmp/p && /tmp/p; echo "ptr store: $? (expect 9)"
# 数组求和
echo 'int main(){ int a[3]; a[0]=10; a[1]=20; a[2]=12; int s=0; for(int i=0;i<3;i=i+1) s=s+a[i]; return s; }' > /tmp/arr.c
./target/debug/bianyi /tmp/arr.c -o /tmp/arr && /tmp/arr; echo "array sum: $? (expect 42)"
# sizeof
echo 'int main(){ return sizeof(int) + sizeof(char) + sizeof(int*); }' > /tmp/sz.c
./target/debug/bianyi /tmp/sz.c -o /tmp/sz && /tmp/sz; echo "sizeof: $? (expect 13)"
# 指针算术遍历字符串并数长度（char*）
echo 'int main(){ char* s="abcd"; int n=0; while(*s != 0){ n=n+1; s=s+1; } return n; }' > /tmp/str.c
./target/debug/bianyi /tmp/str.c -o /tmp/str && /tmp/str; echo "strlen: $? (expect 4)"
```
Expected: 9、42、13、4。

- [ ] **Step 5: 提交.**
```bash
git add src/codegen.rs
git commit -m "feat(codegen): byte-offset slots, addr-of, indirect load/store, pointer arith"
```

---

### Task 7: 端到端集成测试（指针/数组/sizeof/char）

**Files:** Modify `tests/integration.rs`

- [ ] **Step 1: 写测试.** 末尾追加：
```rust
#[test]
fn m4_pointer_store() {
    assert_eq!(
        compile_and_run("int main(){ int x=5; int* p=&x; *p=9; return x; }", "m4_ptr"),
        9
    );
}

#[test]
fn m4_array_sum() {
    assert_eq!(
        compile_and_run(
            "int main(){ int a[3]; a[0]=10; a[1]=20; a[2]=12; int s=0; for(int i=0;i<3;i=i+1) s=s+a[i]; return s; }",
            "m4_arr"
        ),
        42
    );
}

#[test]
fn m4_sizeof() {
    assert_eq!(
        compile_and_run("int main(){ return sizeof(int)+sizeof(char)+sizeof(int*); }", "m4_sizeof"),
        13
    );
}

#[test]
fn m4_strlen_via_pointer() {
    assert_eq!(
        compile_and_run(
            "int main(){ char* s=\"abcd\"; int n=0; while(*s != 0){ n=n+1; s=s+1; } return n; }",
            "m4_strlen"
        ),
        4
    );
}

#[test]
fn m4_pointer_param() {
    // 通过指针参数修改调用方变量
    assert_eq!(
        compile_and_run(
            "int set9(int* p){ *p = 9; return 0; } int main(){ int x = 1; set9(&x); return x; }",
            "m4_ptr_param"
        ),
        9
    );
}
```

- [ ] **Step 2: 运行.** `source "$HOME/.cargo/env" && cargo test --test integration 2>&1 | grep "test result"`（22 passed = 17 旧 + 5 新）。

- [ ] **Step 3: 无新增产品代码.**

- [ ] **Step 4: 全量.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`（全绿）。

- [ ] **Step 5: 提交.**
```bash
git add tests/integration.rs
git commit -m "test: pointers, arrays, sizeof, char-pointer strlen, pointer params for M4"
```

---

## 自查（Self-Review）

**Spec 覆盖（spec §3 M4：int*、char、数组、&/*、指针运算、sizeof、隐式转换、字符串字面量；完整标量类型、左值规则、大小与对齐）：**
- 类型系统：Task 1 `types.rs`。
- char：Task 2（关键字）、Task 4（声明/参数）、Task 5/6（间接 1 字节 `ldrsb/strb`）。
- 指针 `T*`、`&`/`*`：Task 2/4（token/parse）、Task 5（AddrOf/LoadInd/StoreInd、lvalue）、Task 6（汇编）。
- 数组与下标、decay：Task 4（`[N]`/`a[i]`）、Task 5（`lower_index_addr`、Var 数组退化）、Task 6（PtrAdd）。
- 指针算术缩放：Task 5（`lower_binary` 指针±整数 + `shift_of`）、Task 6（`add/sub x9,x9,w10,sxtw #shift`）。
- sizeof：Task 4/5（`SizeofType/SizeofExpr` → Const）。
- 字符串字面量类型化为 char*：Task 5（StrLit→`Pointer(Char)`）。
- 左值规则：Task 5（`lower_lvalue` 限定 Var/Deref/Index；parser 也校验赋值目标）。
- 大小与对齐：Task 1 size()；Task 5 帧分配 8 字节对齐。

**占位符扫描：** 无 TBD；跨任务"非穷尽 match 中间态"在 Task 3/4/5 验证步骤说明（连续做 3→4→5→6）。

**类型一致性：**
- AST：`FuncDef.params: Vec<(String,Type)>`、`Declare{name,ty,init}`、`Assign{target,value}`、新增 `Addr/Deref/Index/SizeofType/SizeofExpr`。
- IR：`Function.frame_bytes`、`Temp`=字节偏移、`Copy{dst,src,width}`、新增 `AddrOf{dst,off}/LoadInd{dst,addr,width,signed}/StoreInd{addr,src,width}/PtrAdd{dst,base,index,shift}/PtrSub{...}`；**删除 `Load/Store`**。`lower_expr -> (Temp,Type)`，`lower_lvalue -> (Temp,Type)`。
- codegen：`slot(t)=t`、`frame_size(frame_bytes)`、新指令汇编；旧单测的 `num_temps`→`frame_bytes`、索引→字节偏移（×8）。

**已知边界：** 无指针相减/比较/强转/多维数组/全局/无符号；`char` 有符号；未声明/类型错多为 panic；数组不按值传参（退化为指针）。这些为有意取舍，spec 的"隐式转换"在 M4 仅作最小处理（int/char 经 4 字节槽自然容纳，指针/整数混用按上面规则）。
