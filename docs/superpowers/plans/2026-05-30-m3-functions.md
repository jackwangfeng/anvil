# M3 函数与调用约定（含 Hello World）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Steps use `- [ ]` checkboxes.

**Goal:** 支持多函数定义、`int` 参数传递、返回值、**递归**，遵循 AArch64 调用约定；并支持字符串字面量 + 调用外部 `puts` 打印（Hello World）。

**Architecture:** 在 M2 基础上：parser/AST 增加带参函数定义与函数调用表达式、字符串字面量；IR 增加 `Call/StrLit/LoadArg` 指令与程序级字符串表；codegen 改用标准 AArch64 函数序言/尾声（`stp x29,x30` 保存 fp/lr、设 fp、分配帧），槽位**加宽到 8 字节**以容纳指针，参数经 x0–x7 传递，字符串经 `adrp/add`（PIC）取址并放入 `__cstring` 段，函数内标签按函数名加前缀避免冲突。

**Tech Stack:** Rust（无第三方 crate）、Cargo、系统 `clang`（默认链接 libSystem，故 `puts` 可直接调用）、macOS / AArch64。cargo 前需 `source "$HOME/.cargo/env"`。

---

## 关键设计（实现者必读）

**1. 槽位加宽到 8 字节**：`slot(i)=i*8`，`frame_size=align16(num_slots*8)`。`int` 仍用 `w` 寄存器以 `ldr w`/`str w` 读写（32 位语义不变）；指针（字符串地址）用 `x` 寄存器 `ldr x`/`str x`。

**2. 标准序言/尾声**（每个函数都做，安全支持调用与递归）：
```
_<func>:
    stp x29, x30, [sp, #-16]!   ; 保存 fp, lr，sp -= 16
    mov x29, sp                 ; 设帧指针
    sub sp, sp, #FRAME          ; 分配局部（FRAME>0 时）
    ... 局部槽位寻址 [sp, #i*8] ...
```
尾声（在每个 `Return` 处）：
```
    ldr w0, [sp, #slot(src)]    ; 返回值（int → w0）
    add sp, sp, #FRAME          ; 回收局部（FRAME>0 时）
    ldp x29, x30, [sp], #16     ; 恢复 fp, lr，sp += 16
    ret
```
因参数 ≤8 个全部走寄存器、函数内不改 sp，故局部仍可 `[sp,#off]` 寻址；sp 始终 16 对齐（FRAME 16 对齐、初始 stp -16）。

**3. 调用约定（`Call`）**：参数 i（i<8）`ldr x{i}, [sp,#slot(args[i])]`（`ldr x` 对 int 取低 32 位由被调用方按 w 使用、对指针取全 64 位，均正确）；`bl _<name>`；结果 `str w0, [sp,#slot(dst)]`（按 int 接收，如 `puts` 返回 int）。**M3 仅支持 ≤8 个参数**（超出需栈传参，后续里程碑）。调用前所有实参已在槽位，无寄存器跨调用存活，故 caller-saved 被破坏无影响。

**4. 参数接收（`LoadArg`）**：函数序言后，把入参寄存器存入参数槽：参数 i → `str w{i}, [sp,#slot(param_i)]`。参数按出现顺序占据槽位 0..n。

**5. 字符串字面量**：程序级字符串表 `Program.strings: Vec<String>`。`StrLit{dst,index}` → `adrp x9, L_.str.{index}@PAGE; add x9, x9, L_.str.{index}@PAGEOFF; str x9, [sp,#slot(dst)]`。`generate` 末尾输出：
```
.section __TEXT,__cstring,cstring_literals
L_.str.{i}:
    .byte b0, b1, ..., 0
```
用 `.byte`（含末尾 0）避免汇编字符串转义问题。

**6. 标签按函数名加前缀**：`Label(n)/Jump(n)/JumpIfZero` 在 codegen 发为 `L<func>_<n>`，避免多函数标签冲突（取代 M2 的全局 `L<n>`）。

**已知边界（留待后续）**：仅 `int` 参数/返回；>8 参数不支持；无函数原型检查（实参个数/类型不校验，依赖 libc 符号链接）；字符串仅作 `char*` 传给函数用，无字符串运算；未声明函数按隐式调用处理。

---

## 现状（M2 已合并到 main）

- token：含 `KwInt KwReturn KwIf KwElse KwWhile KwFor Ident IntLit ( ) { } ; + - * / % = < > <= >= == != Eof`。
- ast：`FuncDef{name,body}`（**无参数字段**）、`Stmt{Return,Declare,ExprStmt,Block,If,While,For,Empty}`、`Expr{IntLit,Var,Assign,Unary,Binary}`、`BinaryOp{Add..Ne}`。
- parser：`parse_func_def` 固定解析 `int name(){...}`（吃掉 `()`，无参数）；`parse_primary` 处理 IntLit/Var/`( )`。
- ir：`Temp=usize`、`Function{name,body,num_temps}`、`Instr{Const,Bin,Neg,Load,Store,Copy,Label,Jump,JumpIfZero,Return}`、`BinOp{Add..Ne}`、`Lowerer{body,next_temp,scopes,next_label}`。
- codegen：`slot(t)=t*4`、`frame_size`、`gen_func`（`.globl/_name:/sub sp`）、`gen_instr`、`materialize_const`。
- tests/integration.rs：`compile_and_run(src,name)->i32`。

## 文件结构（M3 修改既有 + 新增 stdout 测试辅助）

- Modify `src/token.rs` —— 新增 `Comma`、`StrLit(String)`。
- Modify `src/lexer.rs` —— 识别 `,` 与 `"..."`（基本转义 `\n \t \\ \" \0`）。
- Modify `src/ast.rs` —— `FuncDef` 增 `params: Vec<String>`；`Expr` 增 `Call{name,args}`、`StrLit(String)`。
- Modify `src/parser.rs` —— 函数参数表、调用表达式（后缀 `(`）、字符串字面量 primary。
- Modify `src/ir.rs` —— `Program` 增 `strings`；`Instr` 增 `Call{dst,name,args}`、`StrLit{dst,index}`、`LoadArg{dst,index}`；`Function` 增 `params` 计数无需（参数即前 N 个槽）；`lower` 维护字符串表与参数槽、调用降级。
- Modify `src/codegen.rs` —— 8 字节槽、新序言/尾声、`Call/StrLit/LoadArg`、标签前缀、cstring 段输出。
- Modify `tests/integration.rs` —— 递归/多函数 + 捕获 stdout 的 Hello World 测试。

---

### Task 1: lexer —— 逗号与字符串字面量

**Files:** Modify `src/token.rs`, `src/lexer.rs`

- [ ] **Step 1: 失败测试.** 在 `src/lexer.rs` 的 `mod tests` 内新增：
```rust
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
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib lex_comma_and_string`。

- [ ] **Step 3: 实现.** `src/token.rs`：在 `Percent,` 之后加 `Comma,`，在 `IntLit(i64),` 之后加 `StrLit(String),`。
`src/lexer.rs`：在主 `match c` 的单字符标点分支字符集加入 `,`，并在 `match c` 内映射 `',' => TokenKind::Comma`（即把 `,` 并入 `'(' | ')' | ... | '%'` 那一组，并在内层 match 增加 `',' => TokenKind::Comma`）。再在 `match c` 中新增字符串分支（放在数字分支之前）：
```rust
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
                tokens.push(Token { kind: TokenKind::StrLit(s), span: Span::new(line, start_col) });
            }
```

- [ ] **Step 4: 确认通过.** `source "$HOME/.cargo/env" && cargo test --lib lexer` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/token.rs src/lexer.rs
git commit -m "feat(lexer): comma and string literals with basic escapes"
```

---

### Task 2: AST —— 函数参数、调用、字符串

**Files:** Modify `src/ast.rs`

- [ ] **Step 1: 实现（AST 纯数据，随 parser 验证）.**
`FuncDef` 增加参数字段：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<String>,
    pub body: Vec<Stmt>,
}
```
`Expr` 增加两个变体（在 `Var(String),` 之后）：
```rust
    StrLit(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
```

- [ ] **Step 2: 确认编译红灯指向 parser/ir（预期）.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 提交.**
```bash
git add src/ast.rs
git commit -m "feat(ast): function params, call and string-literal expressions"
```

---

### Task 3: parser —— 参数表、调用表达式、字符串 primary

**Files:** Modify `src/parser.rs`

- [ ] **Step 1: 失败测试.** 在 `mod tests` 内新增（`use` 已含 `Expr,Stmt,BinaryOp,UnaryOp`）：
```rust
    #[test]
    fn parse_function_with_params() {
        let prog = parse(&lex("int add(int a, int b){ return a+b; }").unwrap()).unwrap();
        let f = &prog.functions[0];
        assert_eq!(f.name, "add");
        assert_eq!(f.params, vec!["a".to_string(), "b".to_string()]);
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
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 实现.** 替换 `parse_func_def` 以解析参数表：
```rust
    fn parse_func_def(&mut self) -> Result<FuncDef, CompileError> {
        self.expect(&TokenKind::KwInt)?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::LParen)?;
        let mut params = Vec::new();
        if *self.peek_kind() != TokenKind::RParen {
            loop {
                self.expect(&TokenKind::KwInt)?;
                params.push(self.expect_ident()?);
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
在 `parse_primary` 增加字符串分支，并把标识符分支改为支持后缀调用 `name(...)`。把 `parse_primary` 的 `Ident`/`LParen`/`IntLit` 分支调整为：
```rust
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
```
顶部 `use` 需含 `FuncDef`（已含）。

- [ ] **Step 4: 确认（ir 仍非穷尽，故只跑 parser）.** `source "$HOME/.cargo/env" && cargo test --lib parser 2>&1 | head -30`。若因 ir 非穷尽无法编译，确认报错仅来自 ir.rs，进入 Task 4。

- [ ] **Step 5: 提交.**
```bash
git add src/parser.rs
git commit -m "feat(parser): function parameters and call expressions, string literals"
```

---

### Task 4: IR —— Call / StrLit / LoadArg、字符串表、参数与调用降级

**Files:** Modify `src/ir.rs`

- [ ] **Step 1: 失败测试.** 在 `mod tests` 新增（`lower_src` 已有；新增按需取整个 Program 的辅助）：
```rust
    fn lower_prog(src: &str) -> Program {
        lower(&parse(&lex(src).unwrap()).unwrap())
    }

    #[test]
    fn lower_call_and_string() {
        let p = lower_prog("int main(){ puts(\"hi\"); return 0; }");
        assert_eq!(p.strings, vec!["hi".to_string()]);
        let f = &p.functions[0];
        let has_strlit = f.body.iter().any(|i| matches!(i, Instr::StrLit { index: 0, .. }));
        let has_call = f.body.iter().any(|i| matches!(i, Instr::Call { name, .. } if name == "puts"));
        assert!(has_strlit && has_call);
    }

    #[test]
    fn lower_params_emit_loadarg() {
        let p = lower_prog("int add(int a, int b){ return a+b; } int main(){ return add(1,2); }");
        let add = p.functions.iter().find(|f| f.name == "add").unwrap();
        let loadargs = add.body.iter().filter(|i| matches!(i, Instr::LoadArg { .. })).count();
        assert_eq!(loadargs, 2, "two params -> two LoadArg");
    }
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head`。

- [ ] **Step 3: 实现.** `Program` 加字段：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
}
```
`Instr` 增加变体：
```rust
    Call { dst: Temp, name: String, args: Vec<Temp> },
    StrLit { dst: Temp, index: usize },
    LoadArg { dst: Temp, index: usize },
```
`Lowerer` 增加对字符串表的可变借用与（参数已用普通变量槽，无需额外字段）：
```rust
struct Lowerer<'a> {
    body: Vec<Instr>,
    next_temp: usize,
    scopes: Vec<HashMap<String, Temp>>,
    next_label: usize,
    strings: &'a mut Vec<String>,
}
```
`impl Lowerer` 改为 `impl<'a> Lowerer<'a>`。在 `lower_expr` 的 `match e` 增加：
```rust
            Expr::StrLit(s) => {
                let index = self.strings.len();
                self.strings.push(s.clone());
                let dst = self.fresh();
                self.body.push(Instr::StrLit { dst, index });
                dst
            }
            Expr::Call { name, args } => {
                let arg_temps: Vec<Temp> = args.iter().map(|a| self.lower_expr(a)).collect();
                let dst = self.fresh();
                self.body.push(Instr::Call { dst, name: name.clone(), args: arg_temps });
                dst
            }
```
`lower` 与 `lower_func` 改为维护字符串表与参数：
```rust
pub fn lower(ast: &AstProgram) -> Program {
    let mut strings = Vec::new();
    let functions = ast
        .functions
        .iter()
        .map(|f| lower_func(f, &mut strings))
        .collect();
    Program { functions, strings }
}

fn lower_func(f: &FuncDef, strings: &mut Vec<String>) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_temp: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
        strings,
    };
    // 参数占据前 N 个槽，并从入参寄存器载入
    for (index, p) in f.params.iter().enumerate() {
        let slot = lw.declare_var(p);
        lw.body.push(Instr::LoadArg { dst: slot, index });
    }
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
（`lower_binop` 不变。注意 `lower_func` 现在签名多了 `strings` 参数。）

- [ ] **Step 4: 确认（codegen 仍非穷尽，只跑 ir）.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head -30`。若 codegen 阻止编译，确认报错仅来自 codegen.rs，进入 Task 5。

- [ ] **Step 5: 提交.**
```bash
git add src/ir.rs
git commit -m "feat(ir): Call/StrLit/LoadArg instrs, program string table, param lowering"
```

---

### Task 5: codegen —— 调用约定、8 字节槽、字符串段、标签前缀

**Files:** Modify `src/codegen.rs`

- [ ] **Step 1: 失败测试.** 把 `mod tests` 的 `gen` 辅助改造为接收字符串表，并新增测试（保留已有 codegen 测试，但它们的 `gen` 调用需更新——见下）。将 `mod tests` 顶部 `gen` 辅助替换为：
```rust
    fn gen(func_body: Vec<Instr>, num_temps: usize) -> String {
        generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: func_body,
                num_temps,
            }],
            strings: vec![],
        })
    }
```
（其余已有 codegen 测试不变；它们断言的 `movz/add w9/cmp/cbz/str w9,[sp,#0]` 仍成立——注意槽位变 8 字节后，`slot(0)=0` 不变，故 `[sp, #0]` 断言仍通过；涉及 `#16` 帧大小的断言需调整：见下条。）
更新 `codegen_add_uses_add_instr`：3 个槽 ×8 = 24，对齐到 32，故把两处 `#16` 改为 `#32`：
```rust
        assert!(asm.contains("sub sp, sp, #32"));
        assert!(asm.contains("add sp, sp, #32"));
```
新增：
```rust
    #[test]
    fn codegen_prologue_saves_fp_lr() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 1 }, Instr::Return { src: 0 }],
            1,
        );
        assert!(asm.contains("stp x29, x30, [sp, #-16]!"));
        assert!(asm.contains("ldp x29, x30, [sp], #16"));
    }

    #[test]
    fn codegen_call_and_loadarg() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::LoadArg { dst: 0, index: 0 },
                    Instr::Call { dst: 1, name: "puts".to_string(), args: vec![0] },
                    Instr::Return { src: 1 },
                ],
                num_temps: 2,
            }],
            strings: vec![],
        });
        assert!(asm.contains("str w0, [sp, #0]")); // LoadArg index0 -> slot0
        assert!(asm.contains("ldr x0, [sp, #0]")); // call arg0 from slot0
        assert!(asm.contains("bl _puts"));
    }

    #[test]
    fn codegen_strlit_section() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::StrLit { dst: 0, index: 0 },
                    Instr::Return { src: 0 },
                ],
                num_temps: 1,
            }],
            strings: vec!["Hi".to_string()],
        });
        assert!(asm.contains("adrp x9, L_.str.0@PAGE"));
        assert!(asm.contains("add x9, x9, L_.str.0@PAGEOFF"));
        assert!(asm.contains("__cstring"));
        assert!(asm.contains("L_.str.0:"));
        assert!(asm.contains(".byte 72, 105, 0")); // 'H','i',0
    }

    #[test]
    fn codegen_labels_prefixed_by_func() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![Instr::Label(0), Instr::Jump(0), Instr::Const { dst: 0, value: 0 }, Instr::Return { src: 0 }],
                num_temps: 1,
            }],
            strings: vec![],
        });
        assert!(asm.contains("Lmain_0:"));
        assert!(asm.contains("b Lmain_0"));
    }
```

- [ ] **Step 2: 确认失败.** `source "$HOME/.cargo/env" && cargo test --lib codegen 2>&1 | head`。

- [ ] **Step 3: 实现.** 完整替换 `src/codegen.rs` 主体（测试模块之上）为：
```rust
use crate::ir::{BinOp, Function, Instr, Program};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".section __TEXT,__text,regular,pure_instructions\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    gen_strings(&program.strings, &mut out);
    out
}

/// 槽位 i：相对 sp 偏移 i*8 字节（8 字节以容纳指针）。
fn slot(t: usize) -> usize {
    t * 8
}

/// 栈帧大小：num_slots*8 向上对齐到 16。
fn frame_size(num_temps: usize) -> usize {
    (num_temps * 8).div_ceil(16) * 16
}

fn gen_func(func: &Function, out: &mut String) {
    let frame = frame_size(func.num_temps);
    let _ = writeln!(out, ".globl _{}", func.name);
    out.push_str(".p2align 2\n");
    let _ = writeln!(out, "_{}:", func.name);
    out.push_str("    stp x29, x30, [sp, #-16]!\n");
    out.push_str("    mov x29, sp\n");
    if frame > 0 {
        let _ = writeln!(out, "    sub sp, sp, #{}", frame);
    }
    for instr in &func.body {
        gen_instr(instr, &func.name, frame, out);
    }
}

fn gen_instr(instr: &Instr, func: &str, frame: usize, out: &mut String) {
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
            let _ = writeln!(out, "L{}_{}:", func, n);
        }
        Instr::Jump(n) => {
            let _ = writeln!(out, "    b L{}_{}", func, n);
        }
        Instr::JumpIfZero { cond, target } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*cond));
            let _ = writeln!(out, "    cbz w9, L{}_{}", func, target);
        }
        Instr::LoadArg { dst, index } => {
            let _ = writeln!(out, "    str w{}, [sp, #{}]", index, slot(*dst));
        }
        Instr::StrLit { dst, index } => {
            let _ = writeln!(out, "    adrp x9, L_.str.{}@PAGE", index);
            let _ = writeln!(out, "    add x9, x9, L_.str.{}@PAGEOFF", index);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::Call { dst, name, args } => {
            for (i, a) in args.iter().enumerate() {
                let _ = writeln!(out, "    ldr x{}, [sp, #{}]", i, slot(*a));
            }
            let _ = writeln!(out, "    bl _{}", name);
            let _ = writeln!(out, "    str w0, [sp, #{}]", slot(*dst));
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
            out.push_str("    ldp x29, x30, [sp], #16\n");
            out.push_str("    ret\n");
        }
    }
}

fn gen_strings(strings: &[String], out: &mut String) {
    if strings.is_empty() {
        return;
    }
    out.push_str(".section __TEXT,__cstring,cstring_literals\n");
    for (i, s) in strings.iter().enumerate() {
        let _ = writeln!(out, "L_.str.{}:", i);
        out.push_str("    .byte ");
        for b in s.as_bytes() {
            let _ = write!(out, "{}, ", b);
        }
        out.push_str("0\n");
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

- [ ] **Step 4: 确认通过 + 真机端到端（关键）.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"` 与 `cargo clippy --all-targets 2>&1 | grep -E "warning|error"`（应无输出）。
再真机验证调用约定/递归/字符串：
```bash
source "$HOME/.cargo/env" && cargo build -q
# 递归阶乘
echo 'int fact(int n){ if (n <= 1) return 1; return n * fact(n-1); } int main(){ return fact(5); }' > /tmp/f.c
./target/debug/bianyi /tmp/f.c -o /tmp/f && /tmp/f; echo "fact(5) exit=$? (expect 120)"
# 多函数
echo 'int add(int a, int b){ return a+b; } int main(){ return add(40, 2); }' > /tmp/a.c
./target/debug/bianyi /tmp/a.c -o /tmp/a && /tmp/a; echo "add exit=$? (expect 42)"
# Hello World
echo 'int main(){ puts("Hello, World!"); return 0; }' > /tmp/h.c
./target/debug/bianyi /tmp/h.c -o /tmp/h && /tmp/h; echo "(hello exit=$?)"
```
Expected: 120、42、打印 `Hello, World!` 且退出码 0。

- [ ] **Step 5: 提交.**
```bash
git add src/codegen.rs
git commit -m "feat(codegen): AArch64 calling convention, 8-byte slots, calls, strings, fp/lr"
```

---

### Task 6: 端到端集成测试（递归、多函数、Hello World）

**Files:** Modify `tests/integration.rs`

- [ ] **Step 1: 写测试.** 在 `tests/integration.rs` 顶部 `use` 后新增一个能捕获 stdout 的辅助，并在末尾加用例：
```rust
/// 编译 `src`，运行，返回 (退出码, stdout)。
fn compile_run_capture(src: &str, name: &str) -> (i32, String) {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");
    let bin = env!("CARGO_BIN_EXE_bianyi");
    let compile = Command::new(bin)
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run bianyi");
    assert!(compile.success(), "bianyi failed to compile {}", name);
    let out = Command::new(&exe_path).output().expect("run compiled exe");
    let code = out.status.code().expect("terminated by signal");
    (code, String::from_utf8_lossy(&out.stdout).to_string())
}
```
（`Command` 已在文件顶部 `use std::process::Command;`。）
末尾追加：
```rust
#[test]
fn m3_recursion_factorial() {
    assert_eq!(
        compile_and_run(
            "int fact(int n){ if (n <= 1) return 1; return n * fact(n-1); } int main(){ return fact(5); }",
            "m3_fact"
        ),
        120
    );
}

#[test]
fn m3_multiple_functions() {
    assert_eq!(
        compile_and_run(
            "int add(int a, int b){ return a+b; } int main(){ return add(40, 2); }",
            "m3_add"
        ),
        42
    );
}

#[test]
fn m3_recursion_fib() {
    // fib(10) = 55
    assert_eq!(
        compile_and_run(
            "int fib(int n){ if (n < 2) return n; return fib(n-1) + fib(n-2); } int main(){ return fib(10); }",
            "m3_fib"
        ),
        55
    );
}

#[test]
fn m3_hello_world() {
    let (code, stdout) = compile_run_capture(
        "int main(){ puts(\"Hello, World!\"); return 0; }",
        "m3_hello",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "Hello, World!\n"); // puts 追加换行
}
```

- [ ] **Step 2: 运行.** `source "$HOME/.cargo/env" && cargo test --test integration 2>&1 | grep "test result"`
Expected: 17 passed（13 旧 + 4 新）。

- [ ] **Step 3: 无新增产品代码.**

- [ ] **Step 4: 全量确认.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`（全绿）。

- [ ] **Step 5: 提交.**
```bash
git add tests/integration.rs
git commit -m "test: recursion, multiple functions and Hello World (puts) for M3"
```

---

## 自查（Self-Review）

**Spec 覆盖（spec §3 M3：多函数、传参、返回值、递归；AArch64 调用约定）+ 额外 Hello World：**
- 多函数/参数/返回：Task 2/3（AST+parser params、Call）、Task 4（LoadArg + Call 降级）、Task 5（序言尾声 + 参数寄存器 + bl）。
- 递归：调用约定正确（fp/lr 保存恢复、每帧独立）即天然支持；Task 6 用 fact/fib 真机验证。
- 调用约定：Task 5 `stp/ldp`、x0–x7 传参、w0 返回、16 字节对齐、≤8 参数。
- Hello World（额外）：Task 1（字符串 token）、Task 3（StrLit/Call parse）、Task 4（字符串表 + StrLit 降级）、Task 5（adrp/add PIC 取址 + __cstring 段）、Task 6 捕获 stdout 断言。

**占位符扫描：** 无 TBD；每步含完整代码与命令、预期输出。跨任务"非穷尽 match 中间态"已在 Task 2/3/4 验证步骤说明（连续做 2→3→4→5 后跑全量）。

**类型一致性：**
- AST：`FuncDef{name,params:Vec<String>,body}`、`Expr` 增 `StrLit(String)`、`Call{name,args:Vec<Expr>}`。
- IR：`Program{functions,strings:Vec<String>}`、`Instr` 增 `Call{dst,name,args:Vec<Temp>}`、`StrLit{dst,index}`、`LoadArg{dst,index}`；`Lowerer<'a>` 持 `strings:&'a mut Vec<String>`；`lower_func(f,&mut strings)`。
- codegen：`slot=t*8`、`frame_size` 用 8 字节、`gen_instr(instr,func,frame,out)` 多了 `func` 形参用于标签前缀、`gen_strings` 输出 cstring 段。字段名 `dst/name/args/index/var/src/cond/target` 全程统一。
- 旧 codegen 单测随槽宽 8 字节，帧大小断言由 `#16` 改为 `#32`（3 槽×8=24→对齐 32）；`[sp,#0]` 类断言因 slot(0)=0 不受影响。

**已知边界：** 仅 int 参数/返回；>8 参数不支持；无原型/实参校验；字符串仅作指针传参；未声明函数隐式调用（靠 libc 链接，未知符号会在链接期报错而非编译期）。
