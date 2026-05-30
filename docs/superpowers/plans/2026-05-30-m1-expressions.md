# M1 表达式与整数运算 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 `bianyi` 能编译整数算术表达式（`+ - * / %`、一元 `+/-`、括号、优先级），如 `int main(){ return 1+2*3-(4/2); }` → 退出码 5。

**Architecture:** 在 M0 的七阶段管道上扩展：lexer 增加算术运算符 token；parser 用优先级爬升（precedence-climbing）解析表达式树；IR 从"单个常量"升级为**三地址码 + 临时变量**（每个子表达式结果占一个临时量 `Temp`）；codegen 给每个临时量分配一个栈槽，按指令顺序"加载操作数→运算→写回栈槽"地做**栈式求值**，函数带最小栈帧（prologue/epilogue）。

**Tech Stack:** Rust（无第三方 crate）、Cargo、系统 `clang`（汇编+链接）、macOS / AArch64。运行 cargo 前需 `source "$HOME/.cargo/env"`（shell 状态不跨命令保留）。

---

## 现状（M0 已合并到 main）

- `src/token.rs`：`TokenKind { KwInt, KwReturn, Ident(String), IntLit(i64), LParen, RParen, LBrace, RBrace, Semicolon, Eof }`，`Token{kind,span}`。
- `src/lexer.rs`：`lex(&str)->Result<Vec<Token>,CompileError>`，已能识别 `(){};`、数字、标识符、关键字。
- `src/ast.rs`：`Program{functions}`、`FuncDef{name,body}`、`enum Stmt{Return(Expr)}`、`enum Expr{IntLit(i64)}`。
- `src/parser.rs`：递归下降，`parse(&[Token])->Result<ast::Program,_>`；`parse_expr` 目前只接受单个整数字面量。
- `src/ir.rs`：`Program{functions}`、`Function{name,body}`、`enum Instr{Return(Value)}`、`enum Value{Const(i64)}`、`lower(&ast::Program)->ir::Program`。
- `src/codegen.rs`：`generate(&ir::Program)->String`，对 `Return(Const(v))` 发 `mov w0,#v` + `ret`。
- `src/lib.rs`：`compile_to_asm(&str)->Result<String,CompileError>` 串管道。
- `tests/integration.rs`：`compile_and_run(src,name)->i32` 用真实二进制编译并运行，校验退出码。

## 文件结构（M1 仅修改既有文件，不新增模块）

- Modify `src/token.rs` —— 新增 5 个运算符 token：`Plus Minus Star Slash Percent`。
- Modify `src/lexer.rs` —— 识别上述运算符字符。
- Modify `src/ast.rs` —— `Expr` 增加 `Unary`/`Binary`，新增 `UnaryOp`/`BinaryOp`。
- Modify `src/parser.rs` —— 优先级爬升表达式解析（一元、括号、`* / %` 高于 `+ -`，左结合）。
- Modify `src/ir.rs` —— 三地址 IR：`Temp`、`Instr{Const,Bin,Neg,Return}`、`BinOp`，`Function` 增加 `num_temps`，lowering 分配临时量。
- Modify `src/codegen.rs` —— 栈帧 + 栈槽求值 + 32 位常量物化（movz/movk）+ `sdiv`/`msub`（取模）。
- Modify `tests/integration.rs` —— 增加算术端到端用例。

## 设计要点（实现者必读）

1. **临时量与栈槽**：每个 `Temp` 是从 0 递增的下标。codegen 给临时量 `i` 分配栈偏移 `i*4`（每个 4 字节，存 `int`）。`num_temps==0` 时不分配栈帧。
2. **栈帧对齐**：AArch64 要求 sp 16 字节对齐。帧大小 `frame = align_up(num_temps*4, 16)`。prologue `sub sp, sp, #frame`，epilogue（在 Return 处）`add sp, sp, #frame` 后 `ret`。
3. **求值寄存器**：用临时寄存器 `w9`/`w10`/`w11`（AArch64 caller-saved，M1 无函数调用，安全）。二元运算：`ldr w9,[sp,#lhs]` `ldr w10,[sp,#rhs]` `<op> w9,w9,w10` `str w9,[sp,#dst]`。
4. **取模**：AArch64 无取模指令。`a % b` = `a - (a/b)*b`：`sdiv w11,w9,w10` 然后 `msub w9,w11,w10,w9`（`w9 = w9 - w11*w10`）。
5. **常量物化**：`int` 是 32 位。把常量低 32 位 `u = (value as i32) as u32` 装入 w 寄存器：`movz w9,#(u & 0xffff)`；若 `(u>>16)!=0` 再 `movk w9,#(u>>16),lsl #16`。负数靠二进制补码自然得到（如 `-1`→`0xffffffff`→两条指令）。
6. **一元 `+`** 是恒等运算，lowering 直接复用操作数的临时量，不生成指令。**一元 `-`** 生成 `Neg`（codegen 用 `neg w9,w9`）。
7. **整数范围**：M1 按 32 位 `int` 处理，超出 i32 的字面量取低 32 位（完整整型/64 位留待 M4）。退出码按 `mod 256`，测试用例结果取 0–255。
8. **向后兼容**：M0 的 `return 42` 现在 lower 为 `Const(t0,42); Return(t0)`，codegen 走栈槽路径仍输出退出码 42。M0 集成测试必须继续通过。

---

### Task 1: lexer 支持算术运算符

**Files:** Modify `src/token.rs`, `src/lexer.rs`

- [ ] **Step 1: 写失败测试.** 在 `src/lexer.rs` 的 `mod tests` 内新增：
```rust
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
```

- [ ] **Step 2: 运行确认失败.** `source "$HOME/.cargo/env" && cargo test --lib lex_arithmetic_operators`
Expected: 编译失败（`TokenKind::Plus` 等未定义）。

- [ ] **Step 3: 实现.** 在 `src/token.rs` 的 `TokenKind` 枚举里，`Semicolon,` 之后、`Eof,` 之前加入：
```rust
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
```
在 `src/lexer.rs` 的主 `match c` 中，把现有标点分支替换为同时处理这些运算符。将原来的
```rust
            '(' | ')' | '{' | '}' | ';' => {
                let kind = match c {
                    '(' => TokenKind::LParen,
                    ')' => TokenKind::RParen,
                    '{' => TokenKind::LBrace,
                    '}' => TokenKind::RBrace,
                    ';' => TokenKind::Semicolon,
                    _ => unreachable!(),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += 1;
                col += 1;
            }
```
改为：
```rust
            '(' | ')' | '{' | '}' | ';' | '+' | '-' | '*' | '/' | '%' => {
                let kind = match c {
                    '(' => TokenKind::LParen,
                    ')' => TokenKind::RParen,
                    '{' => TokenKind::LBrace,
                    '}' => TokenKind::RBrace,
                    ';' => TokenKind::Semicolon,
                    '+' => TokenKind::Plus,
                    '-' => TokenKind::Minus,
                    '*' => TokenKind::Star,
                    '/' => TokenKind::Slash,
                    '%' => TokenKind::Percent,
                    _ => unreachable!(),
                };
                tokens.push(Token { kind, span: Span::new(line, col) });
                i += 1;
                col += 1;
            }
```

- [ ] **Step 4: 运行确认通过.** `source "$HOME/.cargo/env" && cargo test --lib lexer` 然后 `source "$HOME/.cargo/env" && cargo build`
Expected: lexer 测试全部 PASS（含新增），build 干净。

- [ ] **Step 5: 提交.**
```bash
git add src/token.rs src/lexer.rs
git commit -m "feat(lexer): tokenize arithmetic operators + - * / %"
```

---

### Task 2: AST 与优先级解析器

**Files:** Modify `src/ast.rs`, `src/parser.rs`

- [ ] **Step 1: 写失败测试.** 在 `src/parser.rs` 的 `mod tests` 内新增（保留已有的 `parse_return_42`、`parse_reports_missing_semicolon`）：
```rust
    use crate::ast::{BinaryOp, UnaryOp};

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
            Expr::Unary { op: UnaryOp::Neg, operand: Box::new(Expr::IntLit(5)) }
        );
    }

    #[test]
    fn parse_reports_missing_rparen() {
        let err = parse(&lex("int main(){ return (1+2; }").unwrap()).unwrap_err();
        assert!(err.message.contains("RParen") || err.message.contains(')'));
    }
```

- [ ] **Step 2: 运行确认失败.** `source "$HOME/.cargo/env" && cargo test --lib parser`
Expected: 编译失败（`Expr::Binary`、`BinaryOp`、`UnaryOp` 未定义）。

- [ ] **Step 3: 实现 AST.** 把 `src/ast.rs` 的 `Expr` 定义替换为，并在文件中新增两个运算符枚举：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    IntLit(i64),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}
```
（`Program`、`FuncDef`、`Stmt` 保持不变。）

- [ ] **Step 4: 实现 parser.** 在 `src/parser.rs` 顶部的 `use crate::ast::{...}` 中补上 `BinaryOp, UnaryOp`（变成 `use crate::ast::{BinaryOp, Expr, FuncDef, Program, Stmt, UnaryOp};`）。把现有的 `parse_expr` 方法整体替换为下面这组方法：
```rust
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
```
并在 `impl<'a> Parser<'a>` 块**之外**（文件中、测试模块之前）新增自由函数：
```rust
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
```

- [ ] **Step 5: 运行确认通过.** `source "$HOME/.cargo/env" && cargo test --lib parser` 然后 `source "$HOME/.cargo/env" && cargo build`
Expected: parser 测试全部 PASS（含 M0 旧测试与新增 5 个），build 干净。

- [ ] **Step 6: 提交.**
```bash
git add src/ast.rs src/parser.rs
git commit -m "feat(parser): precedence-climbing expressions (unary, binary, parens)"
```

---

### Task 3: 三地址 IR 与 lowering

**Files:** Modify `src/ir.rs`

- [ ] **Step 1: 写失败测试.** 把 `src/ir.rs` 的 `mod tests` 替换为（注意：M0 旧测试 `lower_return_42` 因 IR 结构变化需一并改写，这里给出新的等价版本）：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn lower_src(src: &str) -> Function {
        let ast = parse(&lex(src).unwrap()).unwrap();
        let ir = lower(&ast);
        ir.functions.into_iter().next().unwrap()
    }

    #[test]
    fn lower_const_return() {
        // return 42  =>  t0 = const 42 ; return t0
        let f = lower_src("int main(){ return 42; }");
        assert_eq!(f.name, "main");
        assert_eq!(f.num_temps, 1);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 42 },
                Instr::Return { src: 0 },
            ]
        );
    }

    #[test]
    fn lower_add() {
        // return 1+2  =>  t0=1; t1=2; t2=t0+t1; return t2
        let f = lower_src("int main(){ return 1+2; }");
        assert_eq!(f.num_temps, 3);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 1, value: 2 },
                Instr::Bin { dst: 2, op: BinOp::Add, lhs: 0, rhs: 1 },
                Instr::Return { src: 2 },
            ]
        );
    }

    #[test]
    fn lower_unary_plus_is_noop() {
        // return +7  =>  t0=7; return t0  (一元 + 不产生指令)
        let f = lower_src("int main(){ return +7; }");
        assert_eq!(f.num_temps, 1);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Return { src: 0 },
            ]
        );
    }

    #[test]
    fn lower_unary_neg() {
        // return -7  =>  t0=7; t1=neg t0; return t1
        let f = lower_src("int main(){ return -7; }");
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Neg { dst: 1, src: 0 },
                Instr::Return { src: 1 },
            ]
        );
    }
}
```

- [ ] **Step 2: 运行确认失败.** `source "$HOME/.cargo/env" && cargo test --lib ir`
Expected: 编译失败（`Instr::Const{dst,value}`、`num_temps`、`BinOp` 等新形态未定义）。

- [ ] **Step 3: 实现.** 把 `src/ir.rs` 主体（测试模块之上的全部内容）替换为：
```rust
use crate::ast::{BinaryOp, Expr, FuncDef, Program as AstProgram, Stmt, UnaryOp};

/// 临时量编号（从 0 递增）。codegen 据此分配栈槽。
pub type Temp = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Instr>,
    pub num_temps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    Const { dst: Temp, value: i64 },
    Bin { dst: Temp, op: BinOp, lhs: Temp, rhs: Temp },
    Neg { dst: Temp, src: Temp },
    Return { src: Temp },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

pub fn lower(ast: &AstProgram) -> Program {
    Program {
        functions: ast.functions.iter().map(lower_func).collect(),
    }
}

struct Lowerer {
    body: Vec<Instr>,
    next_temp: usize,
}

impl Lowerer {
    fn fresh(&mut self) -> Temp {
        let t = self.next_temp;
        self.next_temp += 1;
        t
    }

    /// 把表达式降到一串指令，返回存放其结果的临时量。
    fn lower_expr(&mut self, e: &Expr) -> Temp {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                dst
            }
            Expr::Unary { op, operand } => {
                let src = self.lower_expr(operand);
                match op {
                    UnaryOp::Plus => src, // 恒等：复用操作数临时量
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.body.push(Instr::Neg { dst, src });
                        dst
                    }
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let a = self.lower_expr(lhs);
                let b = self.lower_expr(rhs);
                let dst = self.fresh();
                self.body.push(Instr::Bin { dst, op: lower_binop(*op), lhs: a, rhs: b });
                dst
            }
        }
    }
}

fn lower_binop(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Mod,
    }
}

fn lower_func(f: &FuncDef) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_temp: 0,
    };
    for stmt in &f.body {
        match stmt {
            Stmt::Return(expr) => {
                let src = lw.lower_expr(expr);
                lw.body.push(Instr::Return { src });
            }
        }
    }
    Function {
        name: f.name.clone(),
        body: lw.body,
        num_temps: lw.next_temp,
    }
}
```

- [ ] **Step 4: 运行确认通过.** `source "$HOME/.cargo/env" && cargo test --lib ir` 然后 `source "$HOME/.cargo/env" && cargo build`
Expected: ir 测试全部 PASS，build 干净。

- [ ] **Step 5: 提交.**
```bash
git add src/ir.rs
git commit -m "feat(ir): three-address IR with temporaries and expression lowering"
```

---

### Task 4: codegen —— 栈帧 + 栈槽求值

**Files:** Modify `src/codegen.rs`

- [ ] **Step 1: 写失败测试.** 把 `src/codegen.rs` 的 `mod tests` 替换为：
```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, Function, Instr, Program};

    fn gen(func_body: Vec<Instr>, num_temps: usize) -> String {
        generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: func_body,
                num_temps,
            }],
        })
    }

    #[test]
    fn codegen_const_return() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 42 }, Instr::Return { src: 0 }],
            1,
        );
        assert!(asm.contains(".globl _main"));
        assert!(asm.contains("_main:"));
        assert!(asm.contains("movz w9, #42")); // 物化常量 42
        assert!(asm.contains("ret"));
    }

    #[test]
    fn codegen_add_uses_add_instr() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 1, value: 2 },
                Instr::Bin { dst: 2, op: BinOp::Add, lhs: 0, rhs: 1 },
                Instr::Return { src: 2 },
            ],
            3,
        );
        assert!(asm.contains("add w9, w9, w10"));
        // 栈帧：3 个临时量 *4 = 12，向上对齐到 16
        assert!(asm.contains("sub sp, sp, #16"));
        assert!(asm.contains("add sp, sp, #16"));
    }

    #[test]
    fn codegen_mod_uses_msub() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 17 },
                Instr::Const { dst: 1, value: 5 },
                Instr::Bin { dst: 2, op: BinOp::Mod, lhs: 0, rhs: 1 },
                Instr::Return { src: 2 },
            ],
            3,
        );
        assert!(asm.contains("sdiv w11, w9, w10"));
        assert!(asm.contains("msub w9, w11, w10, w9"));
    }
}
```

- [ ] **Step 2: 运行确认失败.** `source "$HOME/.cargo/env" && cargo test --lib codegen`
Expected: 失败 —— 旧 codegen 不发 `movz`/`sub sp`/`add w9` 等指令，断言不通过（且测试引用了 `Instr::Const{dst,value}` 等新形态，旧实现编译不过）。

- [ ] **Step 3: 实现.** 把 `src/codegen.rs` 主体（测试模块之上全部内容）替换为：
```rust
use crate::ir::{BinOp, Function, Instr, Program};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".section __TEXT,__text,regular,pure_instructions\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    out
}

/// 临时量 i 的栈槽：相对 sp 偏移 i*4 字节。
fn slot(t: usize) -> usize {
    t * 4
}

/// 栈帧大小：num_temps*4 向上对齐到 16；0 个临时量则无帧。
fn frame_size(num_temps: usize) -> usize {
    let bytes = num_temps * 4;
    (bytes + 15) / 16 * 16
}

fn gen_func(func: &Function, out: &mut String) {
    let frame = frame_size(func.num_temps);
    let _ = writeln!(out, ".globl _{}", func.name);
    out.push_str(".p2align 2\n");
    let _ = writeln!(out, "_{}:", func.name);
    if frame > 0 {
        let _ = writeln!(out, "    sub sp, sp, #{}", frame);
    }
    for instr in &func.body {
        gen_instr(instr, frame, out);
    }
}

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
        Instr::Bin { dst, op, lhs, rhs } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*lhs));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*rhs));
            match op {
                BinOp::Add => out.push_str("    add w9, w9, w10\n"),
                BinOp::Sub => out.push_str("    sub w9, w9, w10\n"),
                BinOp::Mul => out.push_str("    mul w9, w9, w10\n"),
                BinOp::Div => out.push_str("    sdiv w9, w9, w10\n"),
                BinOp::Mod => {
                    // w9 % w10 = w9 - (w9 / w10) * w10
                    out.push_str("    sdiv w11, w9, w10\n");
                    out.push_str("    msub w9, w11, w10, w9\n");
                }
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

/// 把 32 位常量装入 w9：movz 低半字，必要时 movk 高半字。
fn materialize_const(value: i64, out: &mut String) {
    let u = (value as i32) as u32;
    let lo = u & 0xffff;
    let hi = (u >> 16) & 0xffff;
    let _ = writeln!(out, "    movz w9, #{}", lo);
    if hi != 0 {
        let _ = writeln!(out, "    movk w9, #{}, lsl #16", hi);
    }
}
```

- [ ] **Step 4: 运行确认通过 + 汇编实跑.** `source "$HOME/.cargo/env" && cargo test --lib codegen` 然后 `source "$HOME/.cargo/env" && cargo build`。
再做一次真机端到端 sanity（验证生成的汇编对复杂表达式确实正确）：
```bash
source "$HOME/.cargo/env" && cargo build
echo 'int main(){ return 1+2*3-(4/2); }' > /tmp/m1.c
./target/debug/bianyi /tmp/m1.c -o /tmp/m1 && /tmp/m1; echo "exit=$?"
```
Expected: codegen 测试 PASS；`exit=5`（1+6-2=5）。

- [ ] **Step 5: 提交.**
```bash
git add src/codegen.rs
git commit -m "feat(codegen): stack-slot evaluation with frame, movz/movk, sdiv/msub"
```

---

### Task 5: 端到端集成测试（算术表达式）

**Files:** Modify `tests/integration.rs`

- [ ] **Step 1: 写测试.** 在 `tests/integration.rs` 末尾追加（保留已有的 `m0_return_42`、`m0_return_0`）：
```rust
#[test]
fn m1_precedence() {
    // 1 + 2*3 - (4/2) = 1 + 6 - 2 = 5
    assert_eq!(compile_and_run("int main(){ return 1+2*3-(4/2); }", "m1_prec"), 5);
}

#[test]
fn m1_left_assoc() {
    // 20 - 5 - 3 = 12
    assert_eq!(compile_and_run("int main(){ return 20-5-3; }", "m1_lassoc"), 12);
}

#[test]
fn m1_modulo() {
    // 17 % 5 = 2
    assert_eq!(compile_and_run("int main(){ return 17%5; }", "m1_mod"), 2);
}

#[test]
fn m1_unary_neg_in_expr() {
    // 10 + -3 = 7
    assert_eq!(compile_and_run("int main(){ return 10 + -3; }", "m1_neg"), 7);
}

#[test]
fn m1_parens_nested() {
    // ((2+3)*4) % 7 = 20 % 7 = 6
    assert_eq!(compile_and_run("int main(){ return ((2+3)*4)%7; }", "m1_nested"), 6);
}
```

- [ ] **Step 2: 运行测试.** `source "$HOME/.cargo/env" && cargo test --test integration`
Expected: 7 passed（2 个 M0 + 5 个 M1）。

- [ ] **Step 3: 实现已在前序任务完成.** 无新增产品代码——本任务验证 Task 1–4 串起来对真实算术程序的正确性。

- [ ] **Step 4: 运行全套确认无回归.** `source "$HOME/.cargo/env" && cargo test`
Expected: 全部 PASS（lib 单测 + 7 个集成测试），0 失败。

- [ ] **Step 5: 提交.**
```bash
git add tests/integration.rs
git commit -m "test: end-to-end arithmetic expression cases for M1"
```

---

## 自查（Self-Review）

**Spec 覆盖（对照 spec §3 M1 行：整型常量、四则、取模、括号、一元正负；Pratt 优先级；IR 三地址码；栈式求值）：**
- 整型常量：Task 1（lex 数字已在 M0）、Task 4 `materialize_const`。
- 四则 `+ - * /`：Task 1（token）、Task 2（parse + 优先级）、Task 3（IR `Bin`）、Task 4（`add/sub/mul/sdiv`）。
- 取模 `%`：同上链路，Task 4 用 `sdiv`+`msub`。
- 括号：Task 2 `parse_primary` 的 `LParen` 分支。
- 一元 `+/-`：Task 2 `parse_unary`、Task 3（`Plus` 恒等 / `Neg` 指令）、Task 4（`neg`）。
- Pratt/优先级：Task 2 `parse_bin_expr` 优先级爬升（`* / %`=2 高于 `+ -`=1，左结合）。
- 三地址码 IR：Task 3（`Temp` + `Instr`）。
- 栈式求值：Task 4（每临时量一栈槽，加载→运算→写回）。
覆盖完整。

**占位符扫描：** 无 TBD/TODO；每个改代码步骤均给出完整代码与可运行命令及预期输出。

**类型一致性：**
- AST：`Expr::{IntLit, Unary{op,operand}, Binary{op,lhs,rhs}}`、`UnaryOp::{Neg,Plus}`、`BinaryOp::{Add,Sub,Mul,Div,Mod}` —— Task 2 定义，Task 3 lowering 引用一致（`ast::BinaryOp`→`ir::BinOp` 经 `lower_binop`）。
- IR：`Temp=usize`、`Function{name,body,num_temps}`、`Instr::{Const{dst,value}, Bin{dst,op,lhs,rhs}, Neg{dst,src}, Return{src}}`、`BinOp::{Add,Sub,Mul,Div,Mod}` —— Task 3 定义，Task 4 codegen 与测试引用一致。
- codegen 字段名 `dst/src/lhs/rhs/value/op/num_temps` 全程统一。
- 向后兼容：M0 集成测试 `m0_return_42/0` 在 Task 5 全套运行中必须仍通过（`return 42` → `Const(t0,42);Return(t0)` → `movz w9,#42;str;ldr w0;ret`，退出码 42）。

**已知边界（符合预期，留待后续里程碑）：** 仅 32 位 `int`（超 i32 字面量取低 32 位，完整类型在 M4）；无变量/语句/控制流（M2）；无函数调用与调用约定（M3）；除零依赖硬件行为（C UB）。
