# anvil

用 **Rust 从头手写**的一个 C 编译器，直接生成原生汇编，再调用系统汇编器链接成可执行文件。**双后端、按宿主自动选择**：

- **AArch64 / Mach-O**（Apple Silicon，macOS）—— 调 `clang`
- **x86-64 / ELF**（System V，Linux）—— 调 `gcc`

不依赖任何第三方编译相关库——词法、语法、语义、IR、寄存器/栈帧、代码生成、预处理器全部手写。目标无关的三地址 IR 之上挂两个后端，前端完全共享。

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
$ anvil demo.c -o demo && ./demo
OK total=12 half=6.000000
$ echo $?
12
```

## 编译流水线

源码经过一条单向多趟管道，每个阶段职责单一、可独立测试：

```
.c → 预处理器 → 词法 → 语法(AST) → 降级(目标无关三地址 IR) → 代码生成(.s) → 汇编链接 → 可执行文件
                                                                  ├─ AArch64 → clang
                                                                  └─ x86-64  → gcc
```

| 阶段 | 文件 | 职责 |
|------|------|------|
| 预处理器 | `src/preprocess.rs` | `#include` / `#define` 宏 / 条件编译 / `#`·`##` |
| 词法分析 | `src/lexer.rs`, `src/token.rs` | 字符流 → 带位置的 token |
| 语法分析 | `src/parser.rs`, `src/ast.rs` | 递归下降 + 优先级爬升 → AST |
| 类型系统 | `src/types.rs` | `Type`、聚合体布局、函数签名 |
| 中间代码 | `src/ir.rs` | 三地址码 IR（字节偏移栈帧模型）+ 降级 |
| 代码生成 (arm64) | `src/codegen.rs` | IR → AArch64 / Mach-O 汇编（AAPCS64） |
| 代码生成 (x86-64) | `src/codegen_x86.rs` | IR → x86-64 / ELF 汇编（System V，AT&T 语法） |
| 驱动 | `src/main.rs`, `src/lib.rs` | CLI，按宿主选后端与汇编器 |
| 报错 | `src/span.rs`, `src/error.rs` | `line:col: error: …` |

## 构建与使用

需要 Rust（`rustup`），以及对应平台的汇编/链接工具：

- **macOS / arm64**：`clang`（随 Xcode CLT 提供）
- **Linux / x86-64**：`gcc`

后端按宿主架构自动选择，无需额外参数。

```bash
cargo build --release
./target/release/anvil program.c -o program
./program
```

可选参数：

- `--target arm64|x86_64`：覆盖目标后端（交叉生成另一架构的汇编）
- `-S`：只生成汇编到 `-o` 指定的文件，不汇编链接

```bash
# 在任意平台查看某段 C 的 arm64 汇编
./target/release/anvil program.c -S --target arm64 -o program.s
```

运行测试：

```bash
cargo test     # 138 个测试：单元测试 + "编译并运行" 端到端测试
```

## 支持的 C 特性

- **类型**：`int`、`char`、`double`（`float` 视作 `double`）、指针 `T*`（多级）、数组 `T[N]`、`struct` / `union` / `enum` / `typedef`、`void`
- **表达式**：四则与取模、一元 `+ - ! ~`、比较与相等、逻辑 `&& || `（短路）、位运算 `& | ^ << >>`、三元 `?:`、`++` / `--`、复合赋值 `+= -= ...`、`sizeof`、字符串与字符字面量
- **指针/内存**：取址 `&`、解引用 `*`、下标 `a[i]`、指针算术（按元素大小缩放）、成员访问 `.` / `->`、数组到指针退化
- **语句**：声明与赋值、块作用域、`if/else`、`while`、`for`、`switch/case/default`（含 fall-through）、`break`、`continue`、`return`
- **函数**：定义与原型、参数（含 `>8` 个，超出走栈）、返回值（int/指针/`double`）、**递归**，遵循各后端调用约定
- **结构体按值 / `double` 参数**：结构体按值传参与返回、`double` 函数参数，x86-64 与 arm64 各按 System V / AAPCS64 实现（见下文「后端能力差异」）
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
| **x86** | x86-64 / ELF 第二后端，按宿主自动选择 |
| **M12** | `double` 函数参数、结构体/联合体**按值**传参与返回（System V + AAPCS64 双后端）|

设计文档与每个里程碑的实现计划见 `docs/superpowers/`。

## 后端能力差异

ABI 相关特性两后端均已实现，分别遵循各自调用约定：

| 特性 | x86-64 (System V) | arm64 (AAPCS64) |
|------|:---:|:---:|
| 整型/指针参数、递归、可变参数 `printf` | ✅ | ✅ |
| `double` 返回值、`%f` | ✅ | ✅ |
| **`double` 函数参数**（独立 FP 寄存器组） | ✅ xmm0–7 | ✅ d0–7 |
| **结构体按值传参**（小结构体走整型寄存器 / 大结构体走栈） | ✅ ≤16B→rdi.. | ✅ ≤16B→x0.. |
| **结构体按值返回**（小结构体走整型寄存器 / 大结构体走隐式指针） | ✅ rax:rdx / sret | ✅ x0:x1 / x8 |

> x86-64 后端经本机原生端到端测试（gcc）。arm64 后端产出 Apple Mach-O 汇编，
> 本机仅做汇编字符串级断言；真正的端到端验证需在 Apple Silicon 上跑集成测试。
> 可用 `--target arm64 -S` 在任意平台交叉生成 arm64 汇编以便查看。

## 已知限制

有意未实现（多为 ABI 深坑或低优先级）：

- 结构体 HFA（全 `double` 成员小结构体走 FP 寄存器）——按整型类处理，anvil↔anvil 自洽，与 clang/gcc 传 libc 时不逐位兼容
- 类型转换语法 `(type)expr`（可经赋值/初始化做隐式 int↔double 转换）
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
