# M2 变量与语句 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `bianyi` 支持局部 `int` 变量（声明/初始化/赋值）、块作用域、`if/else`、`while`、`for`，以及比较/相等运算符（`< > <= >= == !=`），能编译出真正有控制流和状态的程序（如循环求和）。

**Architecture:** 在 M1 的三地址 IR 上扩展：引入**具名变量栈槽**（与匿名临时量统一编号为"槽位"），IR 增加 `Load/Store`（变量↔临时量）、`Label/Jump/JumpIfZero`（控制流）；parser 增加语句与声明文法；语义层用**作用域符号表**把变量名解析到槽位；codegen 把比较运算降为 `cmp + cset`，控制流降为 `b/cbz` + 标签。

**Tech Stack:** Rust（无第三方 crate）、Cargo、系统 `clang`、macOS / AArch64。运行 cargo 前需 `source "$HOME/.cargo/env"`。

---

## 范围与取舍（实现者必读）

**本计划包含**：局部 `int` 变量声明 `int x;` / `int x = expr;`、赋值表达式 `x = expr`（求值为所赋的值）、变量引用、`{}` 块与词法作用域、`if (c) s` / `if (c) s else s`、`while (c) s`、`for (init; cond; step) s`、比较 `< > <= >=`、相等 `== !=`、表达式语句 `expr;`、空语句。

**本计划不包含**（M2 之后增量补，独立性强）：短路逻辑 `&& || !`、位运算 `& | ^ ~ << >>`、`break`/`continue`、变量遮蔽以外的作用域细节、非 `int` 类型。这些不阻塞 M2 的可交付性。

**关键设计：统一"槽位（Slot）"模型**
M1 的"临时量"和 M2 的"具名变量"在栈上都是 4 字节槽位，codegen 已按 `slot(i)=i*4` 寻址。M2 把二者统一：一个函数有 `num_slots` 个槽位；前若干个分配给变量（由语义层在解析时按出现顺序分配下标），其余按需作为表达式临时量。`Temp` 类型沿用，语义上既可能是变量槽也可能是临时槽，codegen 不区分。

**控制流降级约定（AArch64）**：
- `Label(n)` → 发 `Ln:`（用函数级唯一标签名，如 `_main` 配 `.L<func>_<n>`，简化为 `L<n>`，但多函数会冲突——M2 仍只有 `main`，用 `L<n>` 即可；M3 引入多函数时再加函数前缀）。本计划只有 `main`，标签用 `L<n>`。
- `Jump(n)` → `b L<n>`
- `JumpIfZero(t, n)` → `ldr w9,[sp,#slot(t)]; cbz w9, L<n>`
- 比较：`Bin{op:Lt,..}` 等降为 `cmp w9,w10; cset w9, <cond>`（lt→`lt`, gt→`gt`, le→`le`, ge→`ge`, eq→`eq`, ne→`ne`），结果 0/1 存回 dst 槽。

---

## 现状（M1 已合并到 main）

- `src/token.rs`：`TokenKind` 含 `KwInt KwReturn Ident(String) IntLit(i64) LParen RParen LBrace RBrace Semicolon Plus Minus Star Slash Percent Eof`。
- `src/ast.rs`：`Program{functions}`、`FuncDef{name,body:Vec<Stmt>}`、`enum Stmt{Return(Expr)}`、`Expr{IntLit, Unary{op,operand}, Binary{op,lhs,rhs}}`、`UnaryOp{Neg,Plus}`、`BinaryOp{Add,Sub,Mul,Div,Mod}`。
- `src/parser.rs`：递归下降 + 优先级爬升 `parse_expr`/`parse_bin_expr`/`parse_unary`/`parse_primary`，自由函数 `binop_of`。
- `src/ir.rs`：`Temp=usize`、`Function{name,body,num_temps}`、`Instr{Const{dst,value}, Bin{dst,op,lhs,rhs}, Neg{dst,src}, Return{src}}`、`BinOp{Add,Sub,Mul,Div,Mod}`、`Lowerer{body,next_temp}` + `lower_expr`。
- `src/codegen.rs`：`generate`/`gen_func`/`gen_instr`/`materialize_const`/`slot`/`frame_size`，每槽 4 字节，sp 相对寻址。
- `tests/integration.rs`：`compile_and_run(src,name)->i32`。

## 文件结构（M2 修改既有文件 + 视情况新增语义层）

- Modify `src/token.rs` —— 新增 token：`KwIf KwElse KwWhile KwFor Assign(=) Lt(<) Gt(>) Le(<=) Ge(>=) EqEq(==) NotEq(!=)`。
- Modify `src/lexer.rs` —— 识别 `= == < <= > >= !=` 双字符运算符、`if/else/while/for` 关键字。
- Modify `src/ast.rs` —— `Stmt` 增加 `Declare/ExprStmt/Block/If/While/For/Empty`；`Expr` 增加 `Var(String)`、`Assign{name,value}`；`BinaryOp` 增加 `Lt Gt Le Ge Eq Ne`。
- Modify `src/parser.rs` —— 语句解析 `parse_stmt` 扩展为分发；声明、块、if/else、while、for；表达式增加赋值（最低优先级、右结合）、变量引用、比较运算优先级层级。
- Modify `src/ir.rs` —— `Instr` 增加 `Load{dst,var} Store{var,src} Label(usize) Jump(usize) JumpIfZero{cond,target} Copy{dst,src}`；`BinOp` 增加 `Lt Gt Le Ge Eq Ne`；`Lowerer` 增加作用域符号表（`Vec<HashMap<String,Temp>>`）、`next_label`，实现语句降级。
- Modify `src/codegen.rs` —— `gen_instr` 处理新指令：`Load/Store`（同槽位 ldr/str，实质 `Copy`）、`Label/Jump/JumpIfZero`、比较 `cmp+cset`。
- Modify `tests/integration.rs` —— 控制流/变量端到端用例（循环求和、条件分支、for 阶乘等）。

> 说明：M2 不单独建语义层文件；变量名→槽位的解析在 `ir.rs` 的 `Lowerer` 内用作用域栈完成（轻量，避免过度设计）。若 `ir.rs` 超 500 行，再拆 `ir/lower.rs`。

---

### Task 1: lexer —— 关系/赋值运算符与控制流关键字

**Files:** Modify `src/token.rs`, `src/lexer.rs`

- [ ] **Step 1: 失败测试.** 在 `src/lexer.rs` 的 `mod tests` 内新增：
```rust
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
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib lex_m2_operators_and_keywords`（新 TokenKind 未定义）。

- [ ] **Step 3: 实现.** `src/token.rs` 的 `TokenKind` 在 `Percent,` 之后、`Eof,` 之前加入：
```rust
    Assign,
    Lt,
    Gt,
    Le,
    Ge,
    EqEq,
    NotEq,
    KwIf,
    KwElse,
    KwWhile,
    KwFor,
```
`src/lexer.rs`：处理双字符运算符需要前瞻。在主 `match c` 中，把 `=`/`<`/`>`/`!` 单独处理（不能并进单字符标点分支，因为可能是 `==`/`<=`/`>=`/`!=`）。在标点分支**之前**插入下列分支（注意 `!` 单独出现在 M2 不合法——只接受 `!=`，单独 `!` 报错，留给后续逻辑非）：
```rust
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
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::Le, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Lt, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            '>' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::Ge, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Gt, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
            '!' => {
                if i + 1 < chars.len() && chars[i + 1] == '=' {
                    tokens.push(Token { kind: TokenKind::NotEq, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    return Err(CompileError::new(
                        Span::new(line, col),
                        "unexpected character '!'".to_string(),
                    ));
                }
            }
```
在标识符分支的关键字匹配里补上控制流关键字：
```rust
                let kind = match ident.as_str() {
                    "int" => TokenKind::KwInt,
                    "return" => TokenKind::KwReturn,
                    "if" => TokenKind::KwIf,
                    "else" => TokenKind::KwElse,
                    "while" => TokenKind::KwWhile,
                    "for" => TokenKind::KwFor,
                    _ => TokenKind::Ident(ident),
                };
```

- [ ] **Step 4: 确认通过.** `source "$HOME/.cargo/env" && cargo test --lib lexer` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/token.rs src/lexer.rs
git commit -m "feat(lexer): relational/assign operators and if/else/while/for keywords"
```

---

### Task 2: AST —— 语句与变量/赋值/比较表达式

**Files:** Modify `src/ast.rs`

- [ ] **Step 1: 实现（AST 是纯数据，测试随 parser 在 Task 3 一起验证）.** 把 `src/ast.rs` 的 `Stmt`、`Expr`、`BinaryOp` 替换/扩展为：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stmt {
    Return(Expr),
    /// int <name>;  或  int <name> = <init>;
    Declare { name: String, init: Option<Expr> },
    ExprStmt(Expr),
    Block(Vec<Stmt>),
    If { cond: Expr, then_branch: Box<Stmt>, else_branch: Option<Box<Stmt>> },
    While { cond: Expr, body: Box<Stmt> },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
    },
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    IntLit(i64),
    Var(String),
    Assign { name: String, value: Box<Expr> },
    Unary { op: UnaryOp, operand: Box<Expr> },
    Binary { op: BinaryOp, lhs: Box<Expr>, rhs: Box<Expr> },
}
```
`BinaryOp` 增加比较/相等变体（`Mod,` 之后）：
```rust
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
```
（`UnaryOp`、`Program`、`FuncDef` 不变。）

- [ ] **Step 2: 确认编译（会引发 parser/ir 非穷尽 match 报错——预期，由后续任务修复）.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。
Expected: 因为 `Stmt` 现在有新变体、`Expr` 多了 `Var/Assign`，`parser.rs`/`ir.rs` 的 match 变成非穷尽——这是预期的红灯，Task 3、Task 4 会修复。本步骤不要求 build 通过；确认错误指向 parser/ir 即可。

- [ ] **Step 3: 提交.**
```bash
git add src/ast.rs
git commit -m "feat(ast): statements (decl/if/while/for/block) and var/assign/compare exprs"
```

---

### Task 3: parser —— 语句与扩展表达式文法

**Files:** Modify `src/parser.rs`

- [ ] **Step 1: 失败测试.** 在 `src/parser.rs` 的 `mod tests` 内，`use` 行改为 `use crate::ast::{BinaryOp, Expr, Stmt, UnaryOp};` 并新增：
```rust
    fn parse_body(src: &str) -> Vec<Stmt> {
        parse(&lex(src).unwrap()).unwrap().functions.into_iter().next().unwrap().body
    }

    #[test]
    fn parse_declaration_with_init() {
        let body = parse_body("int main(){ int x = 5; return x; }");
        assert_eq!(
            body[0],
            Stmt::Declare { name: "x".to_string(), init: Some(Expr::IntLit(5)) }
        );
        assert_eq!(body[1], Stmt::Return(Expr::Var("x".to_string())));
    }

    #[test]
    fn parse_assignment_expr() {
        let body = parse_body("int main(){ int x; x = 3; return x; }");
        assert_eq!(body[0], Stmt::Declare { name: "x".to_string(), init: None });
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
            Stmt::If { cond, then_branch, else_branch } => {
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
            Stmt::For { init, cond, step, .. } => {
                assert!(init.is_some());
                assert!(cond.is_some());
                assert!(step.is_some());
            }
            other => panic!("expected For, got {:?}", other),
        }
    }
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`（parser 仍只懂旧文法）。

- [ ] **Step 3: 实现 parser.** 顶部 `use` 改为 `use crate::ast::{BinaryOp, Expr, FuncDef, Program, Stmt, UnaryOp};`（已含）。把 `parse_func_def` 中读取函数体的循环改为收集语句（不变，仍是 `parse_stmt`）。将原 `parse_stmt` 整体替换为分发器，并新增各语句解析：
```rust
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
        Ok(Stmt::If { cond, then_branch, else_branch })
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
        // init: 声明 或 表达式语句 或 空
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
        // cond
        let cond = if *self.peek_kind() == TokenKind::Semicolon {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::Semicolon)?;
        // step
        let step = if *self.peek_kind() == TokenKind::RParen {
            None
        } else {
            Some(self.parse_expr()?)
        };
        self.expect(&TokenKind::RParen)?;
        let body = Box::new(self.parse_stmt()?);
        Ok(Stmt::For { init, cond, step, body })
    }
```
表达式层：赋值是**最低优先级、右结合**，且只允许左值是变量名。把 `parse_expr` 改为先解析赋值：
```rust
    fn parse_expr(&mut self) -> Result<Expr, CompileError> {
        self.parse_assign()
    }

    fn parse_assign(&mut self) -> Result<Expr, CompileError> {
        let lhs = self.parse_bin_expr(1)?;
        if *self.peek_kind() == TokenKind::Assign {
            self.pos += 1;
            let value = self.parse_assign()?; // 右结合
            if let Expr::Var(name) = lhs {
                Ok(Expr::Assign { name, value: Box::new(value) })
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
```
`parse_primary` 增加变量引用分支（在 `IntLit` 与 `LParen` 之间）：
```rust
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                Ok(Expr::Var(name))
            }
```
比较/相等运算符并入 `binop_of`，优先级低于加减（加减=3、乘除模=4，比较=2、相等=1，赋值已在 parse_assign 之外更低）。把 `binop_of` 替换为：
```rust
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
```
注意：`parse_bin_expr` 的起始 `min_prec` 现在应为 `1`（已是 1，对应最低的相等优先级），但赋值由 `parse_assign` 在更外层处理，故 `parse_assign` 调用 `parse_bin_expr(1)` 正确。

- [ ] **Step 4: 确认通过.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head` —— 注意：此时 `ir.rs` 的 match 仍非穷尽，build 会因 ir.rs 失败。**因此本步骤先只跑 parser 测试**：`source "$HOME/.cargo/env" && cargo test --lib parser 2>&1 | head -30`。若 ir.rs 阻止编译，确认报错仅来自 ir.rs（parser 自身代码无误），并进入 Task 4 修复 ir 后再统一验证。
> 实务建议：Task 3 与 Task 4 连续完成后再跑全量测试。Task 3 结束时 parser 代码应自洽，仅因下游 ir 未更新而无法整体编译。

- [ ] **Step 5: 提交.**
```bash
git add src/parser.rs
git commit -m "feat(parser): statements, declarations, control flow, assignment and comparisons"
```

---

### Task 4: IR —— 变量槽位、控制流指令与语句降级

**Files:** Modify `src/ir.rs`

- [ ] **Step 1: 失败测试.** 把 `src/ir.rs` 的 `mod tests` 扩展为（保留 M1 的 `lower_const_return`/`lower_add` 等，新增）：
```rust
    #[test]
    fn lower_declare_and_return_var() {
        // int x = 5; return x;
        let f = lower_src("int main(){ int x = 5; return x; }");
        // 变量 x 占槽 0；其后是临时量
        // 期望指令序列包含：Const 到某临时量、Store{var:0,..}、Load{..,var:0}、Return
        let has_store_to_var0 = f.body.iter().any(|i| matches!(i, Instr::Store { var: 0, .. }));
        let has_load_var0 = f.body.iter().any(|i| matches!(i, Instr::Load { var: 0, .. }));
        assert!(has_store_to_var0, "expected a Store to var slot 0");
        assert!(has_load_var0, "expected a Load from var slot 0");
    }

    #[test]
    fn lower_if_emits_labels_and_branch() {
        let f = lower_src("int main(){ if (1) return 2; return 3; }");
        let labels = f.body.iter().filter(|i| matches!(i, Instr::Label(_))).count();
        let branches = f.body.iter().filter(|i| matches!(i, Instr::JumpIfZero { .. })).count();
        assert!(labels >= 1, "if should emit at least one label");
        assert!(branches >= 1, "if should emit a conditional branch");
    }

    #[test]
    fn lower_while_emits_loop() {
        let f = lower_src("int main(){ int x = 0; while (x < 3) x = x + 1; return x; }");
        let jumps = f.body.iter().filter(|i| matches!(i, Instr::Jump(_))).count();
        let cond_jumps = f.body.iter().filter(|i| matches!(i, Instr::JumpIfZero { .. })).count();
        assert!(jumps >= 1 && cond_jumps >= 1, "while should emit back-edge jump and exit branch");
    }
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head`（新 Instr 变体未定义）。

- [ ] **Step 3: 实现.** 在 `src/ir.rs` 顶部 `use` 改为：
```rust
use crate::ast::{BinaryOp, Expr, FuncDef, Program as AstProgram, Stmt, UnaryOp};
use std::collections::HashMap;
```
`Instr` 枚举增加变体（与现有并列）：
```rust
    Load { dst: Temp, var: Temp },
    Store { var: Temp, src: Temp },
    Copy { dst: Temp, src: Temp },
    Label(usize),
    Jump(usize),
    JumpIfZero { cond: Temp, target: usize },
```
`BinOp` 增加：
```rust
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
```
`lower_binop` 增加对应分支：
```rust
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::Ne => BinOp::Ne,
```
`Function` 字段 `num_temps` 含义不变（=总槽位数），但语义上现在既含变量槽也含临时槽。`Lowerer` 扩展为带作用域符号表与标签计数：
```rust
struct Lowerer {
    body: Vec<Instr>,
    next_temp: usize,
    scopes: Vec<HashMap<String, Temp>>,
    next_label: usize,
}

impl Lowerer {
    fn fresh(&mut self) -> Temp {
        let t = self.next_temp;
        self.next_temp += 1;
        t
    }

    fn new_label(&mut self) -> usize {
        let l = self.next_label;
        self.next_label += 1;
        l
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// 在当前作用域声明变量，分配一个槽位。
    fn declare_var(&mut self, name: &str) -> Temp {
        let slot = self.fresh();
        self.scopes.last_mut().unwrap().insert(name.to_string(), slot);
        slot
    }

    /// 由内向外查找变量槽位。
    fn lookup_var(&self, name: &str) -> Option<Temp> {
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    fn lower_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Return(e) => {
                let src = self.lower_expr(e);
                self.body.push(Instr::Return { src });
            }
            Stmt::Declare { name, init } => {
                let slot = self.declare_var(name);
                if let Some(e) = init {
                    let v = self.lower_expr(e);
                    self.body.push(Instr::Store { var: slot, src: v });
                }
            }
            Stmt::ExprStmt(e) => {
                let _ = self.lower_expr(e);
            }
            Stmt::Empty => {}
            Stmt::Block(stmts) => {
                self.push_scope();
                for st in stmts {
                    self.lower_stmt(st);
                }
                self.pop_scope();
            }
            Stmt::If { cond, then_branch, else_branch } => {
                let c = self.lower_expr(cond);
                let else_label = self.new_label();
                self.body.push(Instr::JumpIfZero { cond: c, target: else_label });
                self.lower_stmt(then_branch);
                if let Some(else_s) = else_branch {
                    let end_label = self.new_label();
                    self.body.push(Instr::Jump(end_label));
                    self.body.push(Instr::Label(else_label));
                    self.lower_stmt(else_s);
                    self.body.push(Instr::Label(end_label));
                } else {
                    self.body.push(Instr::Label(else_label));
                }
            }
            Stmt::While { cond, body } => {
                let start = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                let c = self.lower_expr(cond);
                self.body.push(Instr::JumpIfZero { cond: c, target: end });
                self.lower_stmt(body);
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
            }
            Stmt::For { init, cond, step, body } => {
                self.push_scope(); // for 的 init 声明作用域限于循环
                if let Some(init_s) = init {
                    self.lower_stmt(init_s);
                }
                let start = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                if let Some(c) = cond {
                    let cv = self.lower_expr(c);
                    self.body.push(Instr::JumpIfZero { cond: cv, target: end });
                }
                self.lower_stmt(body);
                if let Some(st) = step {
                    let _ = self.lower_expr(st);
                }
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
                self.pop_scope();
            }
        }
    }
```
`lower_expr` 增加 `Var`/`Assign`（在现有 `match e` 中加分支）：
```rust
            Expr::Var(name) => {
                let slot = self.lookup_var(name).expect("undeclared variable");
                let dst = self.fresh();
                self.body.push(Instr::Load { dst, var: slot });
                dst
            }
            Expr::Assign { name, value } => {
                let v = self.lower_expr(value);
                let slot = self.lookup_var(name).expect("undeclared variable");
                self.body.push(Instr::Store { var: slot, src: v });
                // 赋值表达式求值为所赋的值：复用 v 作为结果临时量
                v
            }
```
> 说明：`lookup_var` 失败时 M2 用 `expect` panic（语义检查留到后续；spec 把完整语义分析排在更后）。这是 M2 的已知边界。
`lower_func` 改为用作用域栈，并把整个函数体当作一个顶层作用域：
```rust
fn lower_func(f: &FuncDef) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_temp: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
    };
    for stmt in &f.body {
        lw.lower_stmt(stmt);
    }
    Function {
        name: f.name.clone(),
        body: lw.body,
        num_temps: lw.next_temp,
    }
}
```

- [ ] **Step 4: 确认通过.** `source "$HOME/.cargo/env" && cargo test --lib 2>&1 | grep "test result"` —— lib 全部单测（lexer/parser/ir/codegen/error/lib）应在 codegen 更新前先确保 ir 与 parser 编译通过；但 codegen 仍未处理新 Instr，会导致 codegen 非穷尽 match 报错。**因此 Task 4 结束时**只验证 `cargo test --lib ir`（ir 自身测试）能跑：
`source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head -30`。若因 codegen 非穷尽阻止编译，确认报错仅来自 codegen.rs，进入 Task 5。

- [ ] **Step 5: 提交.**
```bash
git add src/ir.rs
git commit -m "feat(ir): variable slots, scopes, control-flow instrs and statement lowering"
```

---

### Task 5: codegen —— 新指令降级（Load/Store/Copy/Label/Jump/比较）

**Files:** Modify `src/codegen.rs`

- [ ] **Step 1: 失败测试.** 在 `src/codegen.rs` 的 `mod tests`，`use` 改为 `use crate::ir::{BinOp, Function, Instr, Program};` 并新增：
```rust
    #[test]
    fn codegen_compare_uses_cset() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 1, value: 2 },
                Instr::Bin { dst: 2, op: BinOp::Lt, lhs: 0, rhs: 1 },
                Instr::Return { src: 2 },
            ],
            3,
        );
        assert!(asm.contains("cmp w9, w10"));
        assert!(asm.contains("cset w9, lt"));
    }

    #[test]
    fn codegen_control_flow() {
        let asm = gen(
            vec![
                Instr::Label(0),
                Instr::Const { dst: 0, value: 0 },
                Instr::JumpIfZero { cond: 0, target: 1 },
                Instr::Jump(0),
                Instr::Label(1),
                Instr::Const { dst: 1, value: 7 },
                Instr::Return { src: 1 },
            ],
            2,
        );
        assert!(asm.contains("L0:"));
        assert!(asm.contains("L1:"));
        assert!(asm.contains("b L0"));
        assert!(asm.contains("cbz w9, L1"));
    }

    #[test]
    fn codegen_load_store_roundtrip() {
        // Store var0 = t0; Load t1 = var0; return t1
        let asm = gen(
            vec![
                Instr::Const { dst: 1, value: 9 },
                Instr::Store { var: 0, src: 1 },
                Instr::Load { dst: 2, var: 0 },
                Instr::Return { src: 2 },
            ],
            3,
        );
        // Store/Load 在统一槽位模型下就是 ldr/str 同槽搬运
        assert!(asm.contains("str w9, [sp, #0]"));
        assert!(asm.contains("ldr w9, [sp, #0]"));
    }
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib codegen 2>&1 | head`。

- [ ] **Step 3: 实现.** 在 `src/codegen.rs` 的 `gen_instr` 的 `match instr` 中，为新指令增加分支，并在 `Bin` 的 `match op` 中增加比较运算。完整替换 `gen_instr` 为：
```rust
fn gen_instr(instr: &Instr, frame: usize, out: &mut String) {
    match instr {
        Instr::Const { dst, value } => {
            materialize_const(*value, out);
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Neg { dst, src } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            out.push_str("    neg w9, w9\n");
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Load { dst, var } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*var));
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Store { var, src } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*var));
        }
        Instr::Copy { dst, src } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Label(n) => {
            let _ = writeln!(out, "L{}:", n);
        }
        Instr::Jump(n) => {
            let _ = writeln!(out, "    b L{}", n);
        }
        Instr::JumpIfZero { cond, target } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*cond));
            let _ = writeln!(out, "    cbz w9, L{}", target);
        }
        Instr::Bin { dst, op, lhs, rhs } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*lhs));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*rhs));
            match op {
                BinOp::Add => out.push_str("    add w9, w9, w10\n"),
                BinOp::Sub => out.push_str("    sub w9, w9, w10\n"),
                BinOp::Mul => out.push_str("    mul w9, w9, w10\n"),
                BinOp::Div => out.push_str("    sdiv w9, w9, w10\n"),
                BinOp::Mod => {
                    out.push_str("    sdiv w11, w9, w10\n");
                    out.push_str("    msub w9, w11, w10, w9\n");
                }
                BinOp::Lt => out.push_str("    cmp w9, w10\n    cset w9, lt\n"),
                BinOp::Gt => out.push_str("    cmp w9, w10\n    cset w9, gt\n"),
                BinOp::Le => out.push_str("    cmp w9, w10\n    cset w9, le\n"),
                BinOp::Ge => out.push_str("    cmp w9, w10\n    cset w9, ge\n"),
                BinOp::Eq => out.push_str("    cmp w9, w10\n    cset w9, eq\n"),
                BinOp::Ne => out.push_str("    cmp w9, w10\n    cset w9, ne\n"),
            }
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Return { src } => {
            let _ = writeln!(out, "    ldr w0, [sp, #{}]", slot(*src));
            if frame > 0 {
                let _ = writeln!(out, "    add sp, sp, #{}", frame);
            }
            out.push_str("    ret\n");
        }
    }
}
```
> 注意 frame_size 现在覆盖所有槽位（变量+临时），因 `num_temps` 即总槽数，逻辑不变。

- [ ] **Step 4: 确认通过 + 真机端到端.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"` 然后 `cargo clippy --all-targets 2>&1 | grep -E "warning|error"`（应无输出）。
再做真机 sanity：
```bash
source "$HOME/.cargo/env" && cargo build -q
echo 'int main(){ int s=0; int i=1; while (i<=10) { s = s + i; i = i + 1; } return s; }' > /tmp/m2.c
./target/debug/bianyi /tmp/m2.c -o /tmp/m2 && /tmp/m2; echo "sum1..10 exit=$? (expect 55)"
echo 'int main(){ int r=1; for (int i=1; i<=5; i=i+1) r = r*i; return r; }' > /tmp/m2b.c
./target/debug/bianyi /tmp/m2b.c -o /tmp/m2b && /tmp/m2b; echo "5! exit=$? (expect 120)"
```
Expected: 55 与 120。

- [ ] **Step 5: 提交.**
```bash
git add src/codegen.rs
git commit -m "feat(codegen): load/store/copy, labels/branches, comparison via cmp+cset"
```

---

### Task 6: 端到端集成测试（变量与控制流）

**Files:** Modify `tests/integration.rs`

- [ ] **Step 1: 写测试.** 在 `tests/integration.rs` 末尾追加：
```rust
#[test]
fn m2_local_var() {
    assert_eq!(compile_and_run("int main(){ int x = 7; int y = 6; return x*y; }", "m2_var"), 42);
}

#[test]
fn m2_if_else() {
    assert_eq!(compile_and_run("int main(){ int x = 5; if (x > 3) return 1; else return 0; }", "m2_if"), 1);
}

#[test]
fn m2_while_sum() {
    // 1+2+...+10 = 55
    assert_eq!(
        compile_and_run("int main(){ int s=0; int i=1; while (i<=10) { s=s+i; i=i+1; } return s; }", "m2_while"),
        55
    );
}

#[test]
fn m2_for_factorial() {
    // 5! = 120
    assert_eq!(
        compile_and_run("int main(){ int r=1; for (int i=1; i<=5; i=i+1) r=r*i; return r; }", "m2_for"),
        120
    );
}

#[test]
fn m2_assignment_value() {
    // 赋值表达式求值为所赋值：int x; int y = (x = 9); return y;
    assert_eq!(compile_and_run("int main(){ int x; int y = (x = 9); return y; }", "m2_assign_val"), 9);
}

#[test]
fn m2_equality() {
    assert_eq!(compile_and_run("int main(){ int x = 4; return x == 4; }", "m2_eq"), 1);
    assert_eq!(compile_and_run("int main(){ int x = 4; return x != 4; }", "m2_ne"), 0);
}
```

- [ ] **Step 2: 运行.** `source "$HOME/.cargo/env" && cargo test --test integration 2>&1 | grep "test result"`
Expected: 13 passed（2 M0 + 5 M1 + 6 M2）。

- [ ] **Step 3: 无新增产品代码.** 验证 Task 1–5 的整体正确性。

- [ ] **Step 4: 全量确认.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`（全绿）。

- [ ] **Step 5: 提交.**
```bash
git add tests/integration.rs
git commit -m "test: end-to-end variables and control-flow cases for M2"
```

---

## 自查（Self-Review）

**Spec 覆盖（spec §3 M2 行：局部变量、赋值、if/else、while、for、{}块、比较/逻辑/位运算；符号表、作用域、栈帧布局、控制流）：**
- 局部变量/赋值：Task 2（AST `Declare`/`Assign`/`Var`）、Task 3（parser）、Task 4（IR `Load`/`Store` + 作用域符号表）、Task 5（codegen ldr/str）。
- if/else、while、for、块：Task 2/3（AST+parser）、Task 4（标签+分支降级）、Task 5（`b`/`cbz`/标签）。
- 比较/相等：Task 1（token）、Task 3（优先级）、Task 4（`BinOp::Lt..Ne`）、Task 5（`cmp`+`cset`）。
- 符号表/作用域：Task 4 `Lowerer.scopes`（`Vec<HashMap>` 嵌套作用域，`push/pop_scope`，`lookup_var` 由内向外）。
- 栈帧布局：沿用 M1 槽位模型，变量与临时量统一编号，`frame_size` 覆盖全部槽位。
- 控制流：Task 4 降级 + Task 5 汇编。
- **逻辑 `&&/||/!` 与位运算**：本里程碑**显式不含**（见"范围与取舍"），作为 M2 后续增量任务。这是有意取舍，已在计划开头声明。

**占位符扫描：** 无 TBD/TODO；每个改代码步骤均给出完整代码与命令、预期输出。跨任务的"非穷尽 match 暂时编译不过"是依赖链导致的预期中间态，已在 Task 2/3/4 的验证步骤中明确说明（连续完成 Task 2→3→4→5 后再跑全量）。

**类型一致性：**
- AST：`Stmt::{Return,Declare{name,init},ExprStmt,Block,If{cond,then_branch,else_branch},While{cond,body},For{init,cond,step,body},Empty}`、`Expr::{IntLit,Var(String),Assign{name,value},Unary,Binary}`、`BinaryOp` 含 `Lt Gt Le Ge Eq Ne` —— Task 2 定义，Task 3 parser、Task 4 lowering 引用一致。
- IR：`Instr` 增 `Load{dst,var} Store{var,src} Copy{dst,src} Label(usize) Jump(usize) JumpIfZero{cond,target}`、`BinOp` 增 `Lt..Ne` —— Task 4 定义，Task 5 codegen 与测试引用一致（字段名 `dst/var/src/cond/target` 全程统一）。
- `Function.num_temps` 复用为"总槽位数"，codegen `frame_size(num_temps)` 不变。
- 优先级数值：赋值（parse_assign 层）< 相等(1) < 比较(2) < 加减(3) < 乘除模(4) < 一元 < primary，左结合（赋值右结合）—— Task 3 内自洽。

**已知边界（符合预期，留待后续）：** 未声明变量用 `expect` panic（完整语义分析后置）；无 `&& || ! & | ^ ~ << >>`、无 `break/continue`、仅 `int`、控制流标签用全局 `L<n>`（M2 只有单函数 `main`，多函数标签前缀在 M3 处理）。
