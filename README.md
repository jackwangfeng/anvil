# anvil

用 **Rust 从头手写**的一个 C 编译器，直接生成原生汇编，再调用系统汇编器链接成可执行文件。**多后端**：

- **AArch64 / Mach-O**（Apple Silicon，macOS）—— 调 `clang`（手写代码生成）
- **x86-64 / ELF**（System V，Linux）—— 调 `gcc`（手写代码生成）
- **LLVM IR**（`--target llvm`）—— 输出 `.ll` 交 `llc -O2` + `gcc`，白嫖 LLVM 全套优化与寄存器分配

不依赖任何第三方编译相关库——词法、语法、语义、IR、寄存器/栈帧、代码生成、预处理器全部手写。目标无关的三地址 IR 之上挂三个后端，前端完全共享；x86-64/arm64 默认按宿主自动选择，`--target llvm` 走优化路线。

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
| 代码生成 (LLVM) | `src/codegen_llvm.rs` | IR → LLVM IR（.ll），交 `llc -O2` 优化 |
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

- `--target arm64|x86_64|llvm`：覆盖目标后端
- `-S`：只生成汇编/IR 到 `-o` 指定的文件，不汇编链接

```bash
# 在任意平台查看某段 C 的 arm64 汇编
./target/release/anvil program.c -S --target arm64 -o program.s

# 走 LLVM 优化路线(需系统有 llc/llvm 与 gcc)
./target/release/anvil program.c --target llvm -o program   # llc -O2 优化
./target/release/anvil program.c -S --target llvm -o program.ll   # 看生成的 LLVM IR
```

> `--target llvm` 适合「要性能」：把目标无关 IR 翻成 LLVM IR(统一 i64 槽位模型),
> 交 `llc -O2` 做 mem2reg/SROA + 全套中端优化 + 寄存器分配 + 指令选择。
> **覆盖全部特性**:标量/指针/数组/多维/结构体(字段访问 + 按值传参/返回,用 `[N x i8]` 交 LLVM 做 ABI)、
> 函数指针(取址 + 间接调用)、用户可变参数(LLVM 原生 `va_list`/`va_arg`)、控制流、调用(含 `printf`)。
> 集成测试里有 9 组「原生后端 vs LLVM 后端结果对拍」确保两条路逐位一致。

运行测试：

```bash
cargo test     # 138 个测试：单元测试 + "编译并运行" 端到端测试
```

## 支持的 C 特性

- **类型**：`int`、`char`、`long`（64 位，含 `long long` / `long int`）、`short`、**`unsigned`**（真无符号:无符号除/模/右移/比较、零扩展）/ `signed`、`double`（`float` 视作 `double`）、指针 `T*`（多级）、数组 `T[N]`（含多维 `T[M][N]`）、**函数指针** `RET (*f)(...)`、`struct` / `union` / `enum`（可作类型）/ `typedef`、`void`
- **类型转换**：`(type)expr` 强制转换；int/char ↔ long ↔ double 的隐式与显式数值转换、指针重解释
- **表达式**：四则与取模、一元 `+ - ! ~`、比较与相等、逻辑 `&& || `（短路）、位运算 `& | ^ << >>`、三元 `?:`、`++` / `--`、复合赋值 `+= -= ...`、逗号运算符 `a, b`、`sizeof`（`sizeof expr` 与 `sizeof(type)` 两种形式）、字符串与字符字面量
- **整数字面量**：十进制、十六进制 `0x`、二进制 `0b`、八进制 `0777`，`L`/`U` 后缀
- **指针/内存**：取址 `&`、解引用 `*`、下标 `a[i]`、指针算术（按元素大小缩放）、指针比较 `== != < > <= >=`、指针相减 `p - q`（得元素个数）、成员访问 `.` / `->`、数组到指针退化
- **初始化**：聚合初始化列表 `int a[3]={1,2,3}`、`int a[]={...}`（推断长度）、`struct P p={1,2}`，不足部分零填充（`{0}` 惯用法清零）；**局部与全局皆可**(全局静态表写成字节镜像)
- **语句**：声明与赋值（单条可声明多个变量 `int a, b = 2, *p;`）、块作用域、`if/else`、`while`、`do/while`、`for`、`switch/case/default`（含 fall-through）、`break`、`continue`、`goto` + 标签（前向/后向）、`return`
- **存储类**：`static` / `extern` / `register` / `auto`（语法接受，语义当前忽略）
- **函数**：定义与原型、参数（含 `>8` 个，超出走栈）、返回值（int/指针/`double`）、**递归**、**函数指针调用**（`f(x)` / `(*f)(x)` / 回调参数）、**用户自定义可变参数**（`va_list` / `va_start` / `va_arg` / `va_end`），遵循各后端调用约定
- **结构体按值 / `double` 参数**：结构体按值传参与返回、`double` 函数参数，x86-64 与 arm64 各按 System V / AAPCS64 实现（见下文「后端能力差异」）
- **libc 互操作**：函数原型驱动正确的返回宽度与可变参数；`printf`（可变参数走栈）、`malloc`（指针返回）等可直接用；内置最小 `<stdio.h>` / `<stdlib.h>` / `<string.h>` 原型
- **预处理器**：对象式/函数式宏、`#`（字符串化）、`##`（粘贴）、`#include "..."` 与 `<...>`、`#if/#ifdef/#ifndef/#elif/#else/#endif`、`#undef`、`defined()`
- **浮点**：`double` 字面量（含科学计数法 `1e10` / `2.5e-3`）、算术、比较、int↔double 隐式转换、`printf("%f")`、`double` 返回值
- **字符串**：相邻字符串字面量自动拼接 `"foo" "bar"` → `"foobar"`
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
| **M13** | `long`（64 位整数，按宽度生成 32/64 位指令）、`(type)expr` 强制类型转换 |
| **M14** | `do/while` 循环、逗号运算符 |
| **M15** | 单条声明多个变量（局部 + 全局，`int a, b, *p;`，指针/数组按声明符各自绑定）|
| **M16** | `unsigned` / `short` / `signed`、八进制字面量 `0777`、`sizeof expr`（无括号形式）|
| **M17** | 存储类关键字（`static`/`extern`/…，接受并忽略）、科学计数法浮点、相邻字符串拼接、`goto` + 标签 |
| **M18** | 聚合初始化列表（数组/结构体，含长度推断与零填充）、指针比较与相减 |
| **M19** | 多维数组、`enum` 作类型、函数指针（声明/取址/间接调用/回调）、用户自定义可变参数 |
| **M20** | 优化：常量折叠（中端，目标无关）+ 窥孔（后端，去冗余存取）|
| **M21** | LLVM 后端（`--target llvm`）：输出 LLVM IR，`llc -O2` 优化；与原生后端对拍一致 |
| **M22** | LLVM 后端补齐函数指针、结构体按值、用户可变参数(全特性覆盖) + 修复无 return 函数坠落 |
| **M23** | 全局聚合初始化(数组/结构体表)、真 `unsigned` 语义(无符号除/模/右移/比较/零扩展)、`char` 8 位截断核验 |

设计文档与每个里程碑的实现计划见 `docs/superpowers/`。

## 优化

在朴素 codegen 之上做了两个**语义保持**的轻量优化(始终开启):

- **常量折叠**(中端,`src/ir.rs`):降级时把纯整数常量表达式(算术/位/移位/比较/一元)在编译期求值成单个常数,按 **32 位回绕**语义计算以与运行时逐位一致;除零不折叠。如 `1 + 2*3 - (4/2) + 60*60` → 单条 `mov $3605`。
- **窥孔**(后端,两个 codegen 各一个 pass):消除相邻的"存帧槽位 + 立即从同槽位载入"——把内存载入改写成寄存器/立即数搬移(同寄存器则整条删除)。保守:仅匹配 `(%rbp)` / `[sp,#]` 槽位、同宽度、相邻两行。

更进一步(SSA + 数据流 + 寄存器分配,或接 LLVM 后端)见「已知限制」与设计文档。

## 后端能力差异

ABI 相关特性两后端均已实现，分别遵循各自调用约定：

| 特性 | x86-64 (System V) | arm64 (AAPCS64) |
|------|:---:|:---:|
| 整型/指针参数、递归、可变参数 `printf` | ✅ | ✅ |
| `double` 返回值、`%f` | ✅ | ✅ |
| **`double` 函数参数**（独立 FP 寄存器组） | ✅ xmm0–7 | ✅ d0–7 |
| **结构体按值传参**（小结构体走整型寄存器 / 大结构体走栈） | ✅ ≤16B→rdi.. | ✅ ≤16B→x0.. |
| **结构体按值返回**（小结构体走整型寄存器 / 大结构体走隐式指针） | ✅ rax:rdx / sret | ✅ x0:x1 / x8 |
| **函数指针间接调用** | ✅ `call *r11` | ✅ `blr` |
| **用户自定义可变参数** | ✅ 自定义压栈 | ✅ 自定义压栈 |

LLVM 后端(`--target llvm`)覆盖以上**全部**特性:结构体按值/函数指针用 LLVM 一等公民表达,
可变参数用 LLVM 原生 `va_list`/`va_arg`;均有原生↔LLVM 对拍测试。

> x86-64 后端经本机原生端到端测试（gcc）。arm64 后端产出 Apple Mach-O 汇编，
> 本机仅做汇编字符串级断言；真正的端到端验证需在 Apple Silicon 上跑集成测试。
> 可用 `--target arm64 -S` 在任意平台交叉生成 arm64 汇编以便查看。

## 已知限制

有意未实现（多为 ABI 深坑或低优先级）：

- 结构体 HFA（全 `double` 成员小结构体走 FP 寄存器）——按整型类处理，anvil↔anvil 自洽，与 clang/gcc 传 libc 时不逐位兼容
- `short` 按 32 位处理(无独立 16 位)、`long double`
- 全局聚合初始化仅支持**整数常量**元素(不支持初始化为字符串字面量/其他全局的地址,需重定位)
- 存储类**语义**(`static` 局部仍是栈变量、`extern` 不解析为外部链接;均只在语法上被接受)
- 用户自定义可变参数采用**自定义约定**（可变实参一律压栈，`va_list` 线性遍历）——anvil↔anvil 自洽且本机验证通过，但与 gcc 的 System V 寄存器变参 ABI 不兼容（外部代码无法调用 anvil 定义的变参函数；调用 libc `printf` 不受影响）
- 位域
- 后缀 `x++` 求值为新值而非旧值（用作语句或循环步进时无影响）
- 完整语义检查（部分错误以 panic 体现而非友好诊断）
- `int` 按 32 位、`long`/指针按 64 位处理；`char` 正确截断到 8 位(signed 符号扩展 / unsigned 零扩展)

## 项目布局

```
src/            编译器各阶段（见上表）
tests/          端到端集成测试（编译并运行 .c，比对退出码/stdout）
docs/superpowers/
  specs/        总体设计文档
  plans/        每个里程碑的实现计划
```
