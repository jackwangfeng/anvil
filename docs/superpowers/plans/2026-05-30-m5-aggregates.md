# M5 聚合类型 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development 或 superpowers:executing-plans。Steps 用 `- [ ]`。

**Goal:** 支持 `struct`/`union`/`enum`/`typedef`，结构体变量与指针、成员访问 `.`/`->`、嵌套结构体、`sizeof(struct ...)`。

**Architecture:** `Type` 增加 `Struct(String)`/`Union(String)`（按 tag 名引用）。新增**聚合体注册表**（tag → 字段列表含偏移 + 总大小），由 parser 在解析定义时计算布局并放入 `ir::Program`/传给 lowering。enum 解析为 int 常量映射（解析期替换为 `IntLit`），`enum Tag` 视作 `int`。typedef 维护"别名→Type"映射（含 `typedef struct {...} Name;`）。成员访问降级为"基地址 + 常量字段偏移"（`FieldAddr` 指令，`add x9,x9,#offset`），叶子标量再 `LoadInd/StoreInd`。

**Tech Stack:** Rust（无第三方 crate）、Cargo、系统 `clang`、macOS/AArch64。cargo 前 `source "$HOME/.cargo/env"`。

---

## 范围与取舍

**包含**：`struct Tag { 字段 };` 定义、`struct Tag` 类型、结构体局部变量、`.`（值）与 `->`（指针）成员访问（左/右值）、嵌套结构体、结构体指针（含作为参数）、`sizeof(struct Tag)`/`sizeof(表达式)`；`union`（所有成员偏移 0，大小 = 最大成员）；`enum`（枚举量为 int 常量，支持显式赋值 `= n`，`enum Tag` 即 int）；`typedef`（`typedef T Name;`，含 `typedef struct {...} Name;` 匿名与具名）。

**不包含（明确砍掉）**：结构体数组与结构体指针算术（非 2 的幂缩放）、结构体/联合体按值传参或返回、位域、匿名成员、前向声明、函数指针。结构体只能整体取址/经指针访问，不能作为右值整体读出（只访问其标量字段）。

## 关键设计

**1. 聚合体注册表**（types.rs 定义，parser 计算，Program 携带）：
```rust
pub struct Field { pub name: String, pub ty: Type, pub offset: usize }
pub struct Aggregate { pub fields: Vec<Field>, pub size: usize, pub is_union: bool }
pub type Aggregates = std::collections::HashMap<String, Aggregate>;
```
布局规则（简化、统一 8 字节对齐，与 M4 帧一致）：struct 每个字段偏移 = 累计 `align8(前缀和)`，字段按 `align8(field_size)` 步进；总大小 = `align8(累计)`。union 所有字段偏移 0，大小 = `align8(max field size)`。

**2. 带注册表的大小计算**：`pub fn size_of(ty: &Type, aggs: &Aggregates) -> usize`（types.rs）。`Type::size()` 仅处理非聚合（Struct/Union 分支 `unreachable!`）；凡可能遇到聚合处一律用 `size_of`。

**3. 成员访问降级**：新增 `FieldAddr { dst, base, offset }` → `ldr x9,[base]; add x9,x9,#offset; str x9,[dst]`。
- `p.x`（`.`）：基地址 = `lower_lvalue(p)`（结构体变量地址）；`FieldAddr(+offset_x)`。
- `p->x`（`->`）：基地址 = `lower_expr(p)`（指针值即结构体地址）；`FieldAddr(+offset_x)`。
- 嵌套 `a.b.c`：`lower_lvalue` 递归链式 `FieldAddr`。
- 叶子标量字段右值 → `LoadInd`(width=size_of(field))，赋值 → `StoreInd`。

**4. enum**：parser 解析 `enum Tag { A, B=5, C };` 填充 `enum_consts: HashMap<String,i64>`（A=0,B=5,C=6…）。解析 primary 标识符时若是枚举量则产出 `Expr::IntLit(值)`。`enum Tag` 作为类型 = `Type::Int`。

**5. typedef**：parser 维护 `typedefs: HashMap<String, Type>`。类型说明符解析支持 `struct/union/enum Tag` 与 typedef 名（Ident）。`typedef struct {...} Name;` 匿名结构体生成合成 tag（如 `__anon_N`）注册后映射 `Name → Struct(__anon_N)`。

**已知边界**：结构体须先定义后使用（无前向声明，自引用仅可经指针：`struct Node{ int v; struct Node* next; };` —— 因 `Type::Struct(name)` 按名引用、指针大小恒 8，自引用可行）；未声明/类型错多 panic。

---

## 现状（M4 已合并 main）

- token：含 M4 全部（`& [ ] char sizeof` 等）。无 `.`、`->`、`struct/union/enum/typedef`。
- ast：`FuncDef{name,params:Vec<(String,Type)>,body}`、`Stmt{Return,Declare{name,ty,init},...}`、`Expr{IntLit,Var,StrLit,Call,Assign{target,value},Addr,Deref,Index,SizeofType,SizeofExpr,Unary,Binary}`。`Program{functions}`。
- types：`Type{Int,Char,Pointer,Array}` + `size/decay/pointee/is_pointer_like`。
- ir：`Program{functions,strings}`、`Function{name,body,frame_bytes}`、`Temp`=字节偏移、`Instr{...,AddrOf,LoadInd,StoreInd,PtrAdd,PtrSub,...}`、`Lowerer{...,scopes:Vec<HashMap<String,(usize,Type)>>}`，用 `ty.size()` 算大小/宽度。
- codegen：`slot(t)=t`、`AddrOf/LoadInd/StoreInd/PtrAdd/PtrSub`。

## 文件结构

- Modify `src/types.rs` —— `Type::Struct/Union`、`Field/Aggregate/Aggregates`、`size_of(ty,aggs)`。
- Modify `src/token.rs` —— `Dot(.) Arrow(->) KwStruct KwUnion KwEnum KwTypedef`。
- Modify `src/lexer.rs` —— `.`、`->`（`-` 后看 `>`）、关键字。
- Modify `src/ast.rs` —— 顶层项 `Item{Func(FuncDef), Struct/Union/Enum/Typedef 定义}`；`Program{items}` 或保留 functions + 附加定义；`Expr::Member{base,field,arrow}`。
- Modify `src/parser.rs` —— 顶层定义解析、类型说明符扩展、成员后缀 `.`/`->`、enum/typedef 注册表；产出 `ast::Program`（含聚合体注册表）。
- Modify `src/ir.rs` —— 携带 `aggregates`，`size_of` 经注册表，`FieldAddr`、Member 降级、结构体变量分配。
- Modify `src/codegen.rs` —— `FieldAddr`。
- Modify `tests/integration.rs` —— 结构体/嵌套/指针/enum/typedef/union 真机用例。

> 为控制改动，`ast::Program` 增加 `aggregates: types::Aggregates` 字段（parser 计算好布局），`ir::Program` 不需要再带（lowering 用 ast 传入的注册表，lower 时持有引用）。

---

### Task 1: types.rs —— 聚合类型与布局

**Files:** Modify `src/types.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增：
```rust
    use std::collections::HashMap;

    #[test]
    fn struct_layout_size() {
        let mut aggs: HashMap<String, Aggregate> = HashMap::new();
        aggs.insert(
            "P".to_string(),
            Aggregate {
                fields: vec![
                    Field { name: "x".into(), ty: Type::Int, offset: 0 },
                    Field { name: "y".into(), ty: Type::Int, offset: 8 },
                ],
                size: 16,
                is_union: false,
            },
        );
        assert_eq!(size_of(&Type::Struct("P".into()), &aggs), 16);
        assert_eq!(size_of(&Type::Pointer(Box::new(Type::Struct("P".into()))), &aggs), 8);
        assert_eq!(size_of(&Type::Int, &aggs), 4);
    }
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib struct_layout_size`。

- [ ] **Step 3: 实现.** `Type` 增加变体：
```rust
    Struct(String),
    Union(String),
```
`size()` 的 match 增加（保持可编译；聚合不应直接调 size()）：
```rust
            Type::Struct(_) | Type::Union(_) => unreachable!("use size_of with registry for aggregates"),
```
文件追加注册表类型与 `size_of`：
```rust
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub ty: Type,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Aggregate {
    pub fields: Vec<Field>,
    pub size: usize,
    pub is_union: bool,
}

pub type Aggregates = HashMap<String, Aggregate>;

/// 带聚合体注册表的大小计算。
pub fn size_of(ty: &Type, aggs: &Aggregates) -> usize {
    match ty {
        Type::Struct(name) | Type::Union(name) => {
            aggs.get(name).map(|a| a.size).unwrap_or(0)
        }
        Type::Array(elem, n) => size_of(elem, aggs) * n,
        other => other.size(),
    }
}
```
（`decay/pointee/is_pointer_like` 不变；注意 `pointee` 对 Struct 返回 None，符合预期。）

- [ ] **Step 4: 通过.** `source "$HOME/.cargo/env" && cargo test --lib types` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/types.rs
git commit -m "feat(types): Struct/Union types, aggregate layout registry, size_of"
```

---

### Task 2: lexer —— `.`/`->` 与聚合关键字

**Files:** Modify `src/token.rs`, `src/lexer.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增：
```rust
    #[test]
    fn lex_m5_tokens() {
        assert_eq!(
            kinds(". -> struct union enum typedef"),
            vec![
                TokenKind::Dot,
                TokenKind::Arrow,
                TokenKind::KwStruct,
                TokenKind::KwUnion,
                TokenKind::KwEnum,
                TokenKind::KwTypedef,
                TokenKind::Eof,
            ]
        );
    }
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib lex_m5_tokens`。

- [ ] **Step 3: 实现.** `src/token.rs`：加 `Dot,`（标点区）、`Arrow,`，关键字区加 `KwStruct, KwUnion, KwEnum, KwTypedef,`。
`src/lexer.rs`：
- `.` 并入单字符标点集合与内层映射（`'.' => TokenKind::Dot`）。
- `->`：现有 `-` 目前直接产 `Minus`（在单字符标点分支）。需把 `-` 从该分支拆出单独处理：从字符集合里去掉 `'-'`，新增分支：
```rust
            '-' => {
                if i + 1 < chars.len() && chars[i + 1] == '>' {
                    tokens.push(Token { kind: TokenKind::Arrow, span: Span::new(line, col) });
                    i += 2;
                    col += 2;
                } else {
                    tokens.push(Token { kind: TokenKind::Minus, span: Span::new(line, col) });
                    i += 1;
                    col += 1;
                }
            }
```
- 关键字匹配增加 `"struct"=>KwStruct, "union"=>KwUnion, "enum"=>KwEnum, "typedef"=>KwTypedef`。

- [ ] **Step 4: 通过.** `source "$HOME/.cargo/env" && cargo test --lib lexer` + `cargo build`。

- [ ] **Step 5: 提交.**
```bash
git add src/token.rs src/lexer.rs
git commit -m "feat(lexer): . and -> operators, struct/union/enum/typedef keywords"
```

---

### Task 3: AST —— 顶层项与成员表达式

**Files:** Modify `src/ast.rs`

- [ ] **Step 1: 实现（随 parser 验证）.** 顶部已 `use crate::types::Type;`，增 `use crate::types::Aggregates;`。
`Program` 改为携带聚合注册表（保留 functions 便于现有代码）：
```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<FuncDef>,
    pub aggregates: Aggregates,
}
```
`Expr` 增加成员访问：
```rust
    Member {
        base: Box<Expr>,
        field: String,
        arrow: bool,
    },
```
（struct/union/enum/typedef 的"定义"不进入运行期 AST——它们在 parser 阶段被消化为注册表/常量/别名；故 AST 只需新增 `Member` 与 `Program.aggregates`。）

- [ ] **Step 2: 红灯指向 parser/ir（预期）.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 提交.**
```bash
git add src/ast.rs
git commit -m "feat(ast): Program carries aggregates, Member expression"
```

---

### Task 4: parser —— 顶层定义、类型说明符、成员访问

**Files:** Modify `src/parser.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增（`use crate::types::Type;` 已有）：
```rust
    #[test]
    fn parse_struct_def_and_member() {
        let prog = parse(&lex("struct P { int x; int y; }; int main(){ struct P p; p.x = 3; return p.x; }").unwrap()).unwrap();
        let agg = prog.aggregates.get("P").unwrap();
        assert_eq!(agg.fields.len(), 2);
        assert_eq!(agg.fields[0].name, "x");
        assert_eq!(agg.fields[1].offset, 8); // 8 字节对齐布局
    }

    #[test]
    fn parse_arrow_member() {
        let e = parse_return_expr("int main(){ return p->x; }");
        assert_eq!(
            e,
            Expr::Member { base: Box::new(Expr::Var("p".into())), field: "x".into(), arrow: true }
        );
    }

    #[test]
    fn parse_enum_constants() {
        // 枚举量解析期即替换为 IntLit
        let e = parse_return_expr("enum E { A, B, C }; int main(){ return B; }");
        assert_eq!(e, Expr::IntLit(1));
    }

    #[test]
    fn parse_typedef_alias() {
        let prog = parse(&lex("typedef int MyInt; int main(){ MyInt x; x = 7; return x; }").unwrap()).unwrap();
        // 声明用别名解析为 int
        match &prog.functions[0].body[0] {
            Stmt::Declare { ty, .. } => assert_eq!(*ty, Type::Int),
            other => panic!("{:?}", other),
        }
    }
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo build 2>&1 | head`。

- [ ] **Step 3: 实现.** 顶部 `use crate::types::{Aggregate, Aggregates, Field, Type};`，`use std::collections::HashMap;`。
`parse` 改为构造带注册表的 `Parser`，并解析顶层项：
```rust
pub fn parse(tokens: &[Token]) -> Result<Program, CompileError> {
    let mut p = Parser {
        tokens,
        pos: 0,
        aggregates: HashMap::new(),
        typedefs: HashMap::new(),
        enum_consts: HashMap::new(),
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
    anon_counter: usize,
}
```
`parse_program` 改为分发顶层项（函数 / struct|union 定义 / enum 定义 / typedef）：
```rust
    fn parse_program(&mut self) -> Result<Program, CompileError> {
        let mut functions = Vec::new();
        while *self.peek_kind() != TokenKind::Eof {
            match self.peek_kind() {
                TokenKind::KwStruct | TokenKind::KwUnion => {
                    // 可能是顶层定义 `struct T {...};` 也可能是返回类型（M5 返回类型固定 int，故顶层聚合一定是定义）
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
                _ => functions.push(self.parse_func_def()?),
            }
        }
        Ok(Program {
            functions,
            aggregates: self.aggregates.clone(),
        })
    }
```
聚合定义（struct/union），计算布局并注册；返回 tag 名：
```rust
    fn parse_aggregate_def(&mut self) -> Result<String, CompileError> {
        let is_union = *self.peek_kind() == TokenKind::KwUnion;
        self.pos += 1; // struct/union
        // 可选 tag
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
            let fty = self
                .parse_type_specifier()
                .ok_or_else(|| CompileError::new(self.tokens[self.pos].span, "expected field type".into()))?;
            let fname = self.expect_ident()?;
            self.expect(&TokenKind::Semicolon)?;
            let fsize = self.size_of(&fty);
            let aligned = fsize.div_ceil(8) * 8;
            let foff = if is_union { 0 } else { offset };
            fields.push(Field { name: fname, ty: fty, offset: foff });
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
            Aggregate { fields, size, is_union },
        );
        Ok(tag)
    }
```
枚举定义（填充常量）：
```rust
    fn parse_enum_def(&mut self) -> Result<(), CompileError> {
        self.expect(&TokenKind::KwEnum)?;
        // 可选 tag，忽略其名（enum 即 int）
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
                    return Err(CompileError::new(self.tokens[self.pos].span, "expected enum value".into()));
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
```
typedef：
```rust
    fn parse_typedef(&mut self) -> Result<(), CompileError> {
        self.expect(&TokenKind::KwTypedef)?;
        let ty = self
            .parse_type_specifier()
            .ok_or_else(|| CompileError::new(self.tokens[self.pos].span, "expected type in typedef".into()))?;
        let name = self.expect_ident()?;
        self.expect(&TokenKind::Semicolon)?;
        self.typedefs.insert(name, ty);
        Ok(())
    }
```
**统一类型说明符**（替换 `try_parse_base_type`，支持基础类型/struct/union/enum/typedef 名 + `*`）：
```rust
    fn parse_type_specifier(&mut self) -> Option<Type> {
        let mut ty = match self.peek_kind() {
            TokenKind::KwInt => {
                self.pos += 1;
                Type::Int
            }
            TokenKind::KwChar => {
                self.pos += 1;
                Type::Char
            }
            TokenKind::KwStruct | TokenKind::KwUnion => {
                // 内联定义 `struct T {...}` 或引用 `struct T`
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
                    // 内联定义：回退由 parse_aggregate_def 重新解析
                    self.pos = save;
                    let t = self.parse_aggregate_def().ok()?;
                    if is_union { Type::Union(t) } else { Type::Struct(t) }
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
```
把原 `try_parse_base_type` 的所有调用改为 `parse_type_specifier`（`parse_declaration`、`parse_func_def` 参数、`parse_unary` 的 sizeof、`parse_for` 内对声明的判断也要识别这些类型起始）。`parse_stmt` 的声明分发条件由 `KwInt|KwChar` 扩展为"类型起始"：增加 `KwStruct|KwUnion|KwEnum`，以及 typedef 名 Ident（需向前看）。为简化，新增判断函数：
```rust
    fn at_type_start(&self) -> bool {
        match self.peek_kind() {
            TokenKind::KwInt | TokenKind::KwChar | TokenKind::KwStruct | TokenKind::KwUnion | TokenKind::KwEnum => true,
            TokenKind::Ident(name) => self.typedefs.contains_key(name),
            _ => false,
        }
    }
```
`parse_stmt` 用 `if self.at_type_start() { return self.parse_declaration(); }` 放在 match 之前（或并入）。`parse_for` 的 init 判断改为 `if self.at_type_start()`。
`size_of` 辅助（parser 内，供布局计算）：
```rust
    fn size_of(&self, ty: &Type) -> usize {
        crate::types::size_of(ty, &self.aggregates)
    }
```
`parse_declaration` 改用 `parse_type_specifier`：
```rust
    fn parse_declaration(&mut self) -> Result<Stmt, CompileError> {
        let mut ty = self
            .parse_type_specifier()
            .expect("parse_declaration without a type");
        let name = self.expect_ident()?;
        if *self.peek_kind() == TokenKind::LBracket {
            self.pos += 1;
            let n = match self.peek_kind() {
                TokenKind::IntLit(v) => *v as usize,
                _ => return Err(CompileError::new(self.tokens[self.pos].span, "expected array size".into())),
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
`parse_func_def` 参数用 `parse_type_specifier`（替换 `try_parse_base_type`）。
`parse_postfix` 增加 `.`/`->`（与 `[]` 并列）：
```rust
    fn parse_postfix(&mut self) -> Result<Expr, CompileError> {
        let mut e = self.parse_primary()?;
        loop {
            match self.peek_kind() {
                TokenKind::LBracket => {
                    self.pos += 1;
                    let index = self.parse_expr()?;
                    self.expect(&TokenKind::RBracket)?;
                    e = Expr::Index { base: Box::new(e), index: Box::new(index) };
                }
                TokenKind::Dot => {
                    self.pos += 1;
                    let field = self.expect_ident()?;
                    e = Expr::Member { base: Box::new(e), field, arrow: false };
                }
                TokenKind::Arrow => {
                    self.pos += 1;
                    let field = self.expect_ident()?;
                    e = Expr::Member { base: Box::new(e), field, arrow: true };
                }
                _ => break,
            }
        }
        Ok(e)
    }
```
赋值左值合法集合增加 `Member`：`parse_assign` 的 match 加 `Expr::Member { .. }`。
`parse_primary` 的标识符分支：若是枚举量则产 `IntLit`（在 `Ident` 处理处，函数调用判断之前）：
```rust
            TokenKind::Ident(name) => {
                let name = name.clone();
                self.pos += 1;
                if *self.peek_kind() == TokenKind::LParen {
                    // ……调用（原逻辑不变）
                    ...
                } else if let Some(v) = self.enum_consts.get(&name) {
                    Ok(Expr::IntLit(*v))
                } else {
                    Ok(Expr::Var(name))
                }
            }
```
（注意：枚举量判断放在"非调用"分支内。）

- [ ] **Step 4: 确认（ir 非穷尽，先跑 parser）.** `source "$HOME/.cargo/env" && cargo test --lib parser 2>&1 | head -40`，确认其余报错仅来自 ir.rs。

- [ ] **Step 5: 提交.**
```bash
git add src/parser.rs
git commit -m "feat(parser): struct/union/enum/typedef definitions, type specifiers, member access"
```

---

### Task 5: IR —— 注册表、成员降级、结构体变量

**Files:** Modify `src/ir.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增：
```rust
    #[test]
    fn lower_struct_member_uses_fieldaddr() {
        let f = lower_src("struct P { int x; int y; }; int main(){ struct P p; p.y = 7; return p.y; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::FieldAddr { offset: 8, .. })));
    }

    #[test]
    fn lower_arrow_member() {
        let f = lower_src("struct P { int x; }; int main(){ struct P* p; return p->x; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::FieldAddr { offset: 0, .. })));
    }
```
（`lower_src` 取第一个函数；注意现在程序可能含多个顶层项但仅 1 个函数。）

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head`。

- [ ] **Step 3: 实现.** `Instr` 增 `FieldAddr { dst: Temp, base: Temp, offset: usize }`。
`Lowerer` 增加聚合注册表引用：字段 `aggregates: &'a Aggregates`（与 `strings` 同生命周期）。`lower` 传入：
```rust
pub fn lower(ast: &AstProgram) -> Program {
    let mut strings = Vec::new();
    let functions = ast
        .functions
        .iter()
        .map(|f| lower_func(f, &mut strings, &ast.aggregates))
        .collect();
    Program { functions, strings }
}
```
`Lowerer` 结构与 `lower_func` 增参；`use crate::types::{Aggregates, Type};`。
新增 `size_of` 方法并把 lowering 内所有 `X.size()` 改为 `self.size_of(&X)`（declare_var 对齐、LoadInd/StoreInd 宽度、sizeof、PtrAdd scale 用的 elem 大小、Var/Deref/Index 宽度）：
```rust
    fn size_of(&self, ty: &Type) -> usize {
        crate::types::size_of(ty, self.aggregates)
    }
```
`declare_var` 用 `self.size_of(&ty)` 的对齐：
```rust
    fn declare_var(&mut self, name: &str, ty: Type) -> usize {
        let aligned = self.size_of(&ty).div_ceil(8) * 8;
        let off = self.next_offset;
        self.next_offset += aligned;
        self.scopes.last_mut().unwrap().insert(name.to_string(), (off, ty));
        off
    }
```
`lower_lvalue` 增加 `Member` 分支：
```rust
            Expr::Member { base, field, arrow } => {
                let (base_addr, struct_ty) = if *arrow {
                    let (ptr, pty) = self.lower_expr(base);
                    (ptr, pty.decay().pointee().expect("-> on non-pointer").clone())
                } else {
                    self.lower_lvalue(base)
                };
                let (offset, fty) = self.field_info(&struct_ty, field);
                let dst = self.fresh();
                self.body.push(Instr::FieldAddr { dst, base: base_addr, offset });
                (dst, fty)
            }
```
`lower_expr` 增加 `Member` 分支（叶子标量 LoadInd，聚合则返回地址）：
```rust
            Expr::Member { .. } => {
                let (addr, ty) = self.lower_lvalue(e);
                match ty {
                    Type::Struct(_) | Type::Union(_) | Type::Array(..) => (addr, ty),
                    scalar => {
                        let dst = self.fresh();
                        self.body.push(Instr::LoadInd {
                            dst,
                            addr,
                            width: self.size_of(&scalar),
                            signed: matches!(scalar, Type::Char),
                        });
                        (dst, scalar)
                    }
                }
            }
```
`field_info` 辅助（查字段偏移与类型）：
```rust
    fn field_info(&self, struct_ty: &Type, field: &str) -> (usize, Type) {
        let name = match struct_ty {
            Type::Struct(n) | Type::Union(n) => n,
            _ => panic!("member access on non-struct"),
        };
        let agg = self.aggregates.get(name).expect("unknown struct");
        let f = agg.fields.iter().find(|f| f.name == field).expect("unknown field");
        (f.offset, f.ty.clone())
    }
```
注意 `Assign`/`Var`/`Deref`/`Index` 等中 `ty.size()`→`self.size_of(&ty)`。`type_of` 里若需可暂不支持 Member（sizeof(member) 较少见）；为安全在 `type_of`/`type_of_lvalue` 增加 `Expr::Member` 分支返回字段类型（用 `self.field_info`，但 type_of 是 &self 可调用）。具体在 `type_of` match 增：
```rust
            Expr::Member { base, field, arrow } => {
                let sty = if *arrow {
                    self.type_of(base).decay().pointee().cloned().unwrap_or(Type::Int)
                } else {
                    self.type_of_lvalue(base)
                };
                self.field_info_opt(&sty, field).map(|(_, t)| t).unwrap_or(Type::Int)
            }
```
并加 `field_info_opt`（返回 Option，供 type_of 容错）与在 `type_of_lvalue` 同样处理 Member。（`field_info_opt` 实现同 `field_info` 但用 Option。）

- [ ] **Step 4: 确认（codegen 非穷尽，先跑 ir）.** `source "$HOME/.cargo/env" && cargo test --lib ir 2>&1 | head -40`，确认其余报错仅来自 codegen.rs。

- [ ] **Step 5: 提交.**
```bash
git add src/ir.rs
git commit -m "feat(ir): aggregate registry, member access via FieldAddr, struct vars"
```

---

### Task 6: codegen —— FieldAddr

**Files:** Modify `src/codegen.rs`

- [ ] **Step 1: 失败测试.** `mod tests` 增：
```rust
    #[test]
    fn codegen_field_addr() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::AddrOf { dst: 0, off: 0 },
                    Instr::FieldAddr { dst: 8, base: 0, offset: 8 },
                    Instr::Return { src: 8 },
                ],
                frame_bytes: 16,
            }],
            strings: vec![],
        });
        assert!(asm.contains("add x9, x9, #8"));
    }
```

- [ ] **Step 2: 失败.** `source "$HOME/.cargo/env" && cargo test --lib codegen_field_addr 2>&1 | head`。

- [ ] **Step 3: 实现.** `gen_instr` 增分支：
```rust
        Instr::FieldAddr { dst, base, offset } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    add x9, x9, #{}", offset);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
```

- [ ] **Step 4: 通过 + 真机端到端（关键）.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`，`cargo clippy --all-targets 2>&1 | grep -E "warning|error"`（应空）。真机：
```bash
source "$HOME/.cargo/env" && cargo build -q
echo 'struct P { int x; int y; }; int main(){ struct P p; p.x=40; p.y=2; return p.x+p.y; }' > /tmp/s.c
./target/debug/bianyi /tmp/s.c -o /tmp/s && /tmp/s; echo "struct: $? (expect 42)"
echo 'struct P { int x; int y; }; int gety(struct P* q){ return q->y; } int main(){ struct P p; p.y=7; return gety(&p); }' > /tmp/a.c
./target/debug/bianyi /tmp/a.c -o /tmp/a && /tmp/a; echo "arrow/param: $? (expect 7)"
echo 'struct Inner { int v; }; struct Outer { int a; struct Inner in; }; int main(){ struct Outer o; o.in.v = 9; return o.in.v; }' > /tmp/n.c
./target/debug/bianyi /tmp/n.c -o /tmp/n && /tmp/n; echo "nested: $? (expect 9)"
echo 'enum E { A, B, C }; int main(){ return A + B + C; }' > /tmp/e.c
./target/debug/bianyi /tmp/e.c -o /tmp/e && /tmp/e; echo "enum: $? (expect 3)"
echo 'typedef struct { int x; } Pt; int main(){ Pt p; p.x = 5; return p.x; }' > /tmp/td.c
./target/debug/bianyi /tmp/td.c -o /tmp/td && /tmp/td; echo "typedef: $? (expect 5)"
echo 'union U { int i; char c; }; int main(){ union U u; u.i = 65; return u.c; }' > /tmp/u.c
./target/debug/bianyi /tmp/u.c -o /tmp/u && /tmp/u; echo "union: $? (expect 65)"
```
Expected: 42、7、9、3、5、65。

- [ ] **Step 5: 提交.**
```bash
git add src/codegen.rs
git commit -m "feat(codegen): FieldAddr for struct member access"
```

---

### Task 7: 端到端集成测试

**Files:** Modify `tests/integration.rs`

- [ ] **Step 1: 写测试.** 末尾追加：
```rust
#[test]
fn m5_struct_members() {
    assert_eq!(
        compile_and_run("struct P { int x; int y; }; int main(){ struct P p; p.x=40; p.y=2; return p.x+p.y; }", "m5_struct"),
        42
    );
}

#[test]
fn m5_struct_pointer_arrow() {
    assert_eq!(
        compile_and_run("struct P { int x; int y; }; int gy(struct P* q){ return q->y; } int main(){ struct P p; p.y=7; return gy(&p); }", "m5_arrow"),
        7
    );
}

#[test]
fn m5_nested_struct() {
    assert_eq!(
        compile_and_run("struct I { int v; }; struct O { int a; struct I in; }; int main(){ struct O o; o.in.v=9; return o.in.v; }", "m5_nested"),
        9
    );
}

#[test]
fn m5_enum() {
    assert_eq!(
        compile_and_run("enum E { A, B=5, C }; int main(){ return A + B + C; }", "m5_enum"),
        11
    );
}

#[test]
fn m5_typedef_struct() {
    assert_eq!(
        compile_and_run("typedef struct { int x; int y; } Pt; int main(){ Pt p; p.x=10; p.y=32; return p.x+p.y; }", "m5_typedef"),
        42
    );
}

#[test]
fn m5_union() {
    assert_eq!(
        compile_and_run("union U { int i; char c; }; int main(){ union U u; u.i=65; return u.c; }", "m5_union"),
        65
    );
}
```

- [ ] **Step 2: 运行.** `source "$HOME/.cargo/env" && cargo test --test integration 2>&1 | grep "test result"`（28 passed = 22 旧 + 6 新）。

- [ ] **Step 3: 无新增产品代码.**

- [ ] **Step 4: 全量.** `source "$HOME/.cargo/env" && cargo test 2>&1 | grep "test result"`（全绿）。

- [ ] **Step 5: 提交.**
```bash
git add tests/integration.rs
git commit -m "test: struct/nested/arrow/enum/typedef/union end-to-end for M5"
```

---

## 自查（Self-Review）

**Spec 覆盖（spec §3 M5：struct、union、enum、typedef、成员访问 ./->、嵌套；复合类型布局、成员偏移、字段对齐）：**
- struct/union 定义与布局：Task 1（注册表/size_of）、Task 4（parse_aggregate_def 计算偏移，8 字节对齐；union 偏移 0）。
- 成员访问 `.`/`->`：Task 2（token）、Task 4（postfix）、Task 5（lower_lvalue/lower_expr Member + FieldAddr）、Task 6（汇编）。
- 嵌套：Task 5 lower_lvalue 递归链式 FieldAddr。
- enum：Task 4（parse_enum_def + primary 替换为 IntLit）。
- typedef：Task 4（parse_typedef + parse_type_specifier 识别别名，含匿名 struct）。
- 字段对齐/偏移：Task 1/4（统一 8 字节对齐）。

**占位符扫描：** 无 TBD；跨任务"非穷尽 match 中间态"在 Task 3/4/5 验证步骤说明。

**类型一致性：**
- types：`Type::Struct(String)/Union(String)`、`Field{name,ty,offset}`、`Aggregate{fields,size,is_union}`、`Aggregates`、`size_of(ty,aggs)`。
- ast：`Program{functions,aggregates}`、`Expr::Member{base,field,arrow}`。
- ir：`Instr::FieldAddr{dst,base,offset}`、`Lowerer.aggregates: &Aggregates`、`lower_func(f,&mut strings,&aggregates)`、`size_of` 全程用注册表。
- codegen：`FieldAddr` → `ldr x9,[base]; add x9,x9,#offset; str x9,[dst]`。

**已知边界：** 无结构体数组/结构体指针算术（非 2 幂缩放）、无按值传/返结构体、无位域/匿名成员/前向声明；结构体须先定义后用（自引用经指针可行）；未声明/类型错多 panic。char 联合体读取经 1 字节 `ldrsb`（`u.c` 读 65→'A'=65）。
