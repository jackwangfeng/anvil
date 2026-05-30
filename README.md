# bianyi

用 **Rust 从头手写**的一个 C 编译器，直接生成 **AArch64（Apple Silicon）汇编**，再调用系统 `clang` 汇编链接成原生可执行文件。

不依赖任何第三方编译相关库——词法、语法、语义、IR、寄存器/栈帧、代码生成、预处理器全部手写。目标平台：macOS / arm64。

```c
#include <stdio.h>
#include <stdlib.h>

int total = 0;
double half(int x) { return x / 2.0; }

int main() {
    for (int i = 1; i <= 5; i++) {
        if (i == 3) continue;
        total += i;                 // 1+2+4+5 = 12
    }
    if ((total > 0 && total < 100) || 0) {
        char *p = malloc(8);
        p[0] = 'O'; p[1] = 'K'; p[2] = 0;
        printf("%s total=%d half=%f\n", p, total, half(total));
    }
    return total & 0xF;
}
```

```
$ bianyi demo.c -o demo && ./demo
OK total=12 half=6.000000
$ echo $?
12
```

## 编译流水线

源码经过一条单向多趟管道，每个阶段职责单一、可独立测试：

```
.c → 预处理器 → 词法 → 语法(AST) → 降级(三地址 IR) → 代码生成(AArch64 .s) → clang 汇编链接 → 可执行文件
```

| 阶段 | 文件 | 职责 |
|------|------|------|
| 预处理器 | `src/preprocess.rs` | `#include` / `#define` 宏 / 条件编译 / `#`·`##` |
| 词法分析 | `src/lexer.rs`, `src/token.rs` | 字符流 → 带位置的 token |
| 语法分析 | `src/parser.rs`, `src/ast.rs` | 递归下降 + 优先级爬升 → AST |
| 类型系统 | `src/types.rs` | `Type`、聚合体布局、函数签名 |
| 中间代码 | `src/ir.rs` | 三地址码 IR（字节偏移栈帧模型）+ 降级 |
| 代码生成 | `src/codegen.rs` | IR → AArch64 汇编，调用约定/栈帧/寄存器 |
| 驱动 | `src/main.rs`, `src/lib.rs` | CLI，串起管道，调 `clang` |
| 报错 | `src/span.rs`, `src/error.rs` | `line:col: error: …` |

## 构建与使用

需要 Rust（`rustup`）和系统 `clang`（随 Xcode CLT 提供）。

```bash
cargo build --release
./target/release/bianyi program.c -o program
./program
```

运行测试：

```bash
cargo test     # 137 个测试：单元测试 + "编译并运行" 端到端测试
```

## 支持的 C 特性

- **类型**：`int`、`char`、`double`（`float` 视作 `double`）、指针 `T*`（多级）、数组 `T[N]`、`struct` / `union` / `enum` / `typedef`、`void`
- **表达式**：四则与取模、一元 `+ - ! ~`、比较与相等、逻辑 `&& || `（短路）、位运算 `& | ^ << >>`、三元 `?:`、`++` / `--`、复合赋值 `+= -= ...`、`sizeof`、字符串与字符字面量
- **指针/内存**：取址 `&`、解引用 `*`、下标 `a[i]`、指针算术（按元素大小缩放）、成员访问 `.` / `->`、数组到指针退化
- **语句**：声明与赋值、块作用域、`if/else`、`while`、`for`、`switch/case/default`（含 fall-through）、`break`、`continue`、`return`
- **函数**：定义与原型、参数（含 `>8` 个，超出走栈）、返回值（int/指针/`double`）、**递归**，遵循 AArch64 调用约定
- **libc 互操作**：函数原型驱动正确的返回宽度与可变参数；`printf`（可变参数走栈）、`malloc`（指针返回）等可直接用；内置最小 `<stdio.h>` / `<stdlib.h>` / `<string.h>` 原型
- **预处理器**：对象式/函数式宏、`#`（字符串化）、`##`（粘贴）、`#include "..."` 与 `<...>`、`#if/#ifdef/#ifndef/#elif/#else/#endif`、`#undef`、`defined()`
- **浮点**：`double` 字面量、算术、比较、int↔double 隐式转换、`printf("%f")`、`double` 返回值
- **注释**：`//` 与 `/* */`

## 实现里程碑

项目按里程碑递进开发，每个里程碑独立分支、TDD 提交、合并到 `main`：

| 里程碑 | 内容 |
|--------|------|
| **M0** | 端到端骨架：`return <常量>` → 可执行文件 |
| **M1** | 整数表达式：`+ - * / %`、一元、括号、优先级 |
| **M2** | 变量与控制流：局部变量、作用域、`if/else`、`while`、`for`、比较 |
| **M3** | 函数、递归、AArch64 调用约定、Hello World（`puts`） |
| **M4** | 类型系统、指针、数组、`sizeof`、`char`、字符串 |
| **M5** | 聚合类型：`struct`/`union`/`enum`/`typedef`、`.`/`->`、嵌套 |
| **M6** | 自研预处理器：宏、`#include`、条件编译 |
| **M7** | libc 互操作：函数原型、返回宽度、可变参数 `printf`、`malloc` |
| **M8** | 运算符：逻辑短路、位运算、`?:`、`++`/`--`、复合赋值、注释 |
| **M9** | `break`/`continue`、`switch`/`case`/`default` |
| **M10** | 全局变量、`>8` 参数、系统头、字符字面量 |
| **M11** | 浮点（`double`） |

设计文档与每个里程碑的实现计划见 `docs/superpowers/`。

## 已知限制

有意未实现（多为 ABI 深坑或低优先级）：

- 结构体/联合体**按值**传参或返回（仅支持经指针传递）
- `double` 函数**参数**（v0–v7 FP 寄存器；`double` 返回值与可变参数 `%f` 已支持）
- 指针相减/比较、结构体数组与结构体指针算术、多维数组、位域
- 后缀 `x++` 求值为新值而非旧值（用作语句或循环步进时无影响）
- 完整语义检查（部分错误以 panic 体现而非友好诊断）
- `char` 局部变量不截断到 8 位；整数按 32 位处理

## 项目布局

```
src/            编译器各阶段（见上表）
tests/          端到端集成测试（编译并运行 .c，比对退出码/stdout）
docs/superpowers/
  specs/        总体设计文档
  plans/        每个里程碑的实现计划
```
