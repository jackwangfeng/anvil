//! x86-64 (System V AMD64) 后端：把目标无关的三地址 IR 翻成 ELF/Linux 汇编。
//!
//! 与 AArch64 后端共享同一套 `ir::Program`。栈帧采用 **rbp 相对**寻址：
//! 序言 `push rbp; mov rsp→rbp; sub rsp, frame` 之后，槽位 `t` 位于 `[rbp - frame + t]`，
//! 因此调用时压栈调整 rsp 不影响局部寻址。语法为 GNU as 默认的 AT&T。
use crate::ir::{BinOp, Function, Instr, Program};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".text\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    let mut out = peephole(&out);
    gen_strings(&program.strings, &mut out);
    gen_globals(&program.globals, &mut out);
    gen_floats(&program.floats, &mut out);
    // 标注栈不可执行（消除 GNU ld 的 executable-stack 警告）。
    out.push_str(".section .note.GNU-stack,\"\",@progbits\n");
    out
}

/// 窥孔优化：相邻的"存某帧槽位 + 立即从同槽位载入"——把内存载入改成寄存器/立即数搬移，
/// 二者相等则整条删去。保守：仅匹配 `disp(%rbp)` 槽位、同助记符、相邻两行（其间无标签/跳转）。
fn peephole(asm: &str) -> String {
    // 仅对函数代码段（.text 段，gen_strings 等尚未追加）逐行处理。
    let lines: Vec<&str> = asm.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if i + 1 < lines.len() {
            if let (Some((mn, src, mem)), Some((mn2, mem2, dst))) =
                (parse_store(lines[i]), parse_load(lines[i + 1]))
            {
                if mn == mn2 && mem == mem2 {
                    out.push(lines[i].to_string()); // 保留 store（槽位可能后续仍被读）
                    if src != dst {
                        out.push(format!("    {} {}, {}", mn, src, dst));
                    }
                    i += 2;
                    continue;
                }
            }
        }
        out.push(lines[i].to_string());
        i += 1;
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

const MOV_MNEMONICS: [&str; 4] = ["movl", "movq", "movb", "movsd"];

/// 解析 `    <mn> <src>, disp(%rbp)`：src 为寄存器/立即数，目标是帧槽位。
fn parse_store(line: &str) -> Option<(&str, &str, &str)> {
    let t = line.trim_start();
    let (mn, rest) = t.split_once(' ')?;
    if !MOV_MNEMONICS.contains(&mn) {
        return None;
    }
    let (src, mem) = rest.rsplit_once(", ")?;
    if !is_rbp_slot(mem) || !(src.starts_with('%') || src.starts_with('$')) {
        return None;
    }
    Some((mn, src, mem))
}

/// 解析 `    <mn> disp(%rbp), <dst>`：从帧槽位载入到寄存器。
fn parse_load(line: &str) -> Option<(&str, &str, &str)> {
    let t = line.trim_start();
    let (mn, rest) = t.split_once(' ')?;
    if !MOV_MNEMONICS.contains(&mn) {
        return None;
    }
    let (mem, dst) = rest.split_once(", ")?;
    if !is_rbp_slot(mem) || !dst.starts_with('%') {
        return None;
    }
    Some((mn, mem, dst))
}

fn is_rbp_slot(s: &str) -> bool {
    s.ends_with("(%rbp)") && s[..s.len() - 6].chars().all(|c| c.is_ascii_digit() || c == '-')
}

/// frame_bytes 向上对齐到 16（保证调用点 rsp 16 字节对齐）。
fn frame_size(frame_bytes: usize) -> usize {
    frame_bytes.div_ceil(16) * 16
}

/// 槽位 `t` 的 rbp 相对内存操作数。
fn loc(t: usize, frame: usize) -> String {
    format!("{}(%rbp)", t as i64 - frame as i64)
}

/// 整型实参寄存器（System V）：第 0..6 个。
const IARG64: [&str; 6] = ["%rdi", "%rsi", "%rdx", "%rcx", "%r8", "%r9"];
const IARG32: [&str; 6] = ["%edi", "%esi", "%edx", "%ecx", "%r8d", "%r9d"];
const IARG8: [&str; 6] = ["%dil", "%sil", "%dl", "%cl", "%r8b", "%r9b"];

fn gen_func(func: &Function, out: &mut String) {
    let frame = frame_size(func.frame_bytes);
    let _ = writeln!(out, ".globl {}", func.name);
    out.push_str(".p2align 4\n");
    let _ = writeln!(out, "{}:", func.name);
    out.push_str("    pushq %rbp\n");
    out.push_str("    movq %rsp, %rbp\n");
    if frame > 0 {
        let _ = writeln!(out, "    subq ${}, %rsp", frame);
    }
    // 大结构体返回：隐式返回指针在 rdi，存到 sret 槽供 Return 回写。
    let sret = func.sret_slot.is_some();
    if let Some(s) = func.sret_slot {
        let _ = writeln!(out, "    movq %rdi, {}", loc(s, frame));
    }
    gen_params(&func.params, sret, frame, out);
    // 可变参数：可变实参从 [rbp + 16 + 具名栈实参字节数] 开始（具名实参多走寄存器，通常为 16）。
    let va_base = if func.variadic {
        let items: Vec<(bool, Option<usize>)> = func
            .params
            .iter()
            .map(|p| (p.is_float, if p.is_aggregate { Some(p.size) } else { None }))
            .collect();
        let (_, _, named_stack) = classify_sysv(&items, sret, None);
        16 + named_stack
    } else {
        16
    };
    for instr in &func.body {
        gen_instr(instr, &func.name, frame, func.sret_slot, va_base, out);
    }
    // 兜底尾声:函数体未以 return 结束时避免坠落到下个函数(如无 return 的 void 函数)。
    // 若上方已有 ret,这段是不可达死代码。
    out.push_str("    movl $0, %eax\n    leave\n    ret\n");
}

/// 一个实参/形参在 System V 下的去向。
enum Cls {
    /// 标量整型/指针 → 第 n 个整型寄存器。
    Int(usize),
    /// 标量 double → 第 n 个 xmm 寄存器。
    Xmm(usize),
    /// 按值结构体（≤16B）→ 连续整型寄存器。
    IntRegs(Vec<usize>),
    /// 压栈：位于栈实参区偏移 `off`，占 `dwords` 个 8 字节。
    Stack { off: usize, dwords: usize },
}

/// System V 实参分类：整型组 rdi..r9（6），浮点组 xmm0..7（8），各自独立计数；
/// 溢出按 8 字节为单位压栈。`sret` 为真时 rdi 已被隐式返回指针占用。
/// 返回 (各项去向, 用到的 xmm 数, 栈实参区总字节数)。
fn classify_sysv(
    items: &[(bool, Option<usize>)],
    sret: bool,
    stack_after: Option<usize>,
) -> (Vec<Cls>, usize, usize) {
    let mut int_reg = if sret { 1 } else { 0 };
    let mut xmm = 0;
    let mut soff = 0;
    let classes = items
        .iter()
        .enumerate()
        .map(|(i, &(is_float, agg))| {
            // 自定义可变参数：index ≥ fixed 的实参一律压栈
            if stack_after.is_some_and(|f| i >= f) {
                let off = soff;
                let sz = agg.map(|s| s.div_ceil(8) * 8).unwrap_or(8);
                soff += sz;
                return Cls::Stack { off, dwords: sz / 8 };
            }
            if let Some(sz) = agg {
                let ndw = sz.div_ceil(8);
                if sz <= 16 && int_reg + ndw <= 6 {
                    let regs = (int_reg..int_reg + ndw).collect();
                    int_reg += ndw;
                    Cls::IntRegs(regs)
                } else {
                    let off = soff;
                    soff += ndw * 8;
                    Cls::Stack { off, dwords: ndw }
                }
            } else if is_float {
                if xmm < 8 {
                    xmm += 1;
                    Cls::Xmm(xmm - 1)
                } else {
                    let off = soff;
                    soff += 8;
                    Cls::Stack { off, dwords: 1 }
                }
            } else if int_reg < 6 {
                int_reg += 1;
                Cls::Int(int_reg - 1)
            } else {
                let off = soff;
                soff += 8;
                Cls::Stack { off, dwords: 1 }
            }
        })
        .collect();
    (classes, xmm, soff)
}

/// 把入参从寄存器/栈落到各自帧槽位（System V）。
fn gen_params(params: &[crate::ir::Param], sret: bool, frame: usize, out: &mut String) {
    let items: Vec<(bool, Option<usize>)> = params
        .iter()
        .map(|p| (p.is_float, if p.is_aggregate { Some(p.size) } else { None }))
        .collect();
    let (classes, _, _) = classify_sysv(&items, sret, None);
    for (p, c) in params.iter().zip(classes.iter()) {
        match c {
            Cls::Int(r) => match p.size {
                8 => {
                    let _ = writeln!(out, "    movq {}, {}", IARG64[*r], loc(p.slot, frame));
                }
                1 => {
                    let _ = writeln!(out, "    movb {}, {}", IARG8[*r], loc(p.slot, frame));
                }
                _ => {
                    let _ = writeln!(out, "    movl {}, {}", IARG32[*r], loc(p.slot, frame));
                }
            },
            Cls::Xmm(r) => {
                let _ = writeln!(out, "    movsd %xmm{}, {}", r, loc(p.slot, frame));
            }
            Cls::IntRegs(regs) => {
                for (k, r) in regs.iter().enumerate() {
                    let _ = writeln!(out, "    movq {}, {}", IARG64[*r], loc(p.slot + k * 8, frame));
                }
            }
            Cls::Stack { off, dwords } => {
                for k in 0..*dwords {
                    let _ = writeln!(out, "    movq {}(%rbp), %rax", 16 + off + k * 8);
                    let _ = writeln!(out, "    movq %rax, {}", loc(p.slot + k * 8, frame));
                }
            }
        }
    }
}

fn gen_strings(strings: &[String], out: &mut String) {
    if strings.is_empty() {
        return;
    }
    out.push_str(".section .rodata\n");
    for (i, s) in strings.iter().enumerate() {
        let _ = writeln!(out, ".Lstr.{}:", i);
        out.push_str("    .byte ");
        for b in s.as_bytes() {
            let _ = write!(out, "{}, ", b);
        }
        out.push_str("0\n");
    }
}

fn gen_floats(floats: &[u64], out: &mut String) {
    if floats.is_empty() {
        return;
    }
    out.push_str(".section .rodata\n");
    out.push_str(".p2align 3\n");
    for (i, bits) in floats.iter().enumerate() {
        let _ = writeln!(out, ".Lfloat.{}:", i);
        let _ = writeln!(out, "    .quad {}", bits);
    }
}

fn gen_globals(globals: &[crate::ir::GlobalVar], out: &mut String) {
    if globals.is_empty() {
        return;
    }
    out.push_str(".data\n");
    for g in globals {
        let _ = writeln!(out, ".globl {}", g.name);
        out.push_str(".p2align 3\n");
        let _ = writeln!(out, "{}:", g.name);
        match &g.init {
            Some(bytes) => {
                out.push_str("    .byte ");
                for (k, b) in bytes.iter().enumerate() {
                    if k > 0 {
                        out.push_str(", ");
                    }
                    let _ = write!(out, "{}", b);
                }
                out.push('\n');
            }
            None => {
                let _ = writeln!(out, "    .zero {}", g.size.max(1));
            }
        }
    }
}

fn gen_instr(
    instr: &Instr,
    func: &str,
    frame: usize,
    sret_slot: Option<usize>,
    va_base: usize,
    out: &mut String,
) {
    let m = |t: usize| loc(t, frame);
    match instr {
        Instr::Const { dst, value } => {
            if *value >= i32::MIN as i64 && *value <= i32::MAX as i64 {
                let _ = writeln!(out, "    movl ${}, {}", *value as i32, m(*dst));
            } else {
                // 超 32 位字面量（long）：64 位物化，写满 8 字节
                let _ = writeln!(out, "    movabsq ${}, %rax", value);
                let _ = writeln!(out, "    movq %rax, {}", m(*dst));
            }
        }
        Instr::Neg { dst, src } => {
            let _ = writeln!(out, "    movl {}, %eax", m(*src));
            out.push_str("    negl %eax\n");
            let _ = writeln!(out, "    movl %eax, {}", m(*dst));
        }
        Instr::Label(n) => {
            let _ = writeln!(out, ".L{}_{}:", func, n);
        }
        Instr::Jump(n) => {
            let _ = writeln!(out, "    jmp .L{}_{}", func, n);
        }
        Instr::JumpIfZero { cond, target } => {
            let _ = writeln!(out, "    movl {}, %eax", m(*cond));
            out.push_str("    testl %eax, %eax\n");
            let _ = writeln!(out, "    je .L{}_{}", func, target);
        }
        Instr::StrLit { dst, index } => {
            let _ = writeln!(out, "    leaq .Lstr.{}(%rip), %rax", index);
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::AddrOf { dst, off } => {
            let _ = writeln!(out, "    leaq {}, %rax", loc(*off, frame));
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::GlobalAddr { dst, name } => {
            let _ = writeln!(out, "    leaq {}(%rip), %rax", name);
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::FieldAddr { dst, base, offset } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*base));
            let _ = writeln!(out, "    addq ${}, %rax", offset);
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::Copy { dst, src, width } => {
            if *width == 8 {
                let _ = writeln!(out, "    movq {}, %rax", m(*src));
                let _ = writeln!(out, "    movq %rax, {}", m(*dst));
            } else {
                let _ = writeln!(out, "    movl {}, %eax", m(*src));
                let _ = writeln!(out, "    movl %eax, {}", m(*dst));
            }
        }
        Instr::MemCpy { dst, src, size } => {
            // dst/src 存放目标/源地址，逐 dword 拷贝（向上取整到 8 字节）。
            let _ = writeln!(out, "    movq {}, %r11", m(*dst));
            let _ = writeln!(out, "    movq {}, %r10", m(*src));
            for k in 0..size.div_ceil(8) {
                let _ = writeln!(out, "    movq {}(%r10), %rax", k * 8);
                let _ = writeln!(out, "    movq %rax, {}(%r11)", k * 8);
            }
        }
        Instr::LoadInd { dst, addr, width, signed } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*addr));
            match (*width, *signed) {
                (1, true) => out.push_str("    movsbl (%rax), %ecx\n"),
                (1, false) => out.push_str("    movzbl (%rax), %ecx\n"),
                (8, _) => out.push_str("    movq (%rax), %rcx\n"),
                _ => out.push_str("    movl (%rax), %ecx\n"),
            }
            if *width == 8 {
                let _ = writeln!(out, "    movq %rcx, {}", m(*dst));
            } else {
                let _ = writeln!(out, "    movl %ecx, {}", m(*dst));
            }
        }
        Instr::StoreInd { addr, src, width } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*addr));
            if *width == 8 {
                let _ = writeln!(out, "    movq {}, %rcx", m(*src));
                out.push_str("    movq %rcx, (%rax)\n");
            } else {
                let _ = writeln!(out, "    movl {}, %ecx", m(*src));
                match *width {
                    1 => out.push_str("    movb %cl, (%rax)\n"),
                    _ => out.push_str("    movl %ecx, (%rax)\n"),
                }
            }
        }
        Instr::PtrAdd { dst, base, index, size } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*base));
            let _ = writeln!(out, "    movslq {}, %rcx", m(*index));
            scale_index(*size, out); // rcx *= size
            out.push_str("    addq %rcx, %rax\n");
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::PtrSub { dst, base, index, size } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*base));
            let _ = writeln!(out, "    movslq {}, %rcx", m(*index));
            scale_index(*size, out); // rcx *= size
            out.push_str("    subq %rcx, %rax\n");
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::FuncAddr { dst, name } => {
            let _ = writeln!(out, "    leaq {}(%rip), %rax", name);
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::VaStart { ap } => {
            // *ap = 首个可变参数地址（rbp + va_base）
            let _ = writeln!(out, "    leaq {}(%rbp), %rax", va_base);
            let _ = writeln!(out, "    movq {}, %rcx", m(*ap)); // rcx = &va_list
            out.push_str("    movq %rax, (%rcx)\n");
        }
        Instr::VaArg { dst, ap, width, is_float: _ } => {
            let _ = writeln!(out, "    movq {}, %rcx", m(*ap)); // rcx = &va_list
            out.push_str("    movq (%rcx), %rdx\n"); // rdx = 当前遍历指针
            if *width == 8 {
                out.push_str("    movq (%rdx), %rax\n");
            } else {
                out.push_str("    movl (%rdx), %eax\n");
            }
            out.push_str("    addq $8, %rdx\n"); // 前进一个槽
            out.push_str("    movq %rdx, (%rcx)\n");
            if *width == 8 {
                let _ = writeln!(out, "    movq %rax, {}", m(*dst));
            } else {
                let _ = writeln!(out, "    movl %eax, {}", m(*dst));
            }
        }
        Instr::Call {
            dst,
            name,
            via,
            args,
            arg_floats,
            arg_aggs,
            ret_width,
            ret_agg,
            ret_buf,
            fixed,
            variadic,
            stack_varargs,
            ret_float,
        } => {
            // 大结构体返回（>16B）走隐式指针：&缓冲区放入 rdi，实参整型组从 rsi 起。
            let sret = matches!(ret_agg, Some(sz) if *sz > 16);
            let items: Vec<(bool, Option<usize>)> = (0..args.len())
                .map(|i| {
                    (
                        arg_floats.get(i).copied().unwrap_or(false),
                        arg_aggs.get(i).copied().flatten(),
                    )
                })
                .collect();
            // 自定义可变参数被调方：index ≥ fixed 的实参压栈
            let stack_after = if *stack_varargs { Some(*fixed) } else { None };
            let (classes, xmm_used, stack_bytes) = classify_sysv(&items, sret, stack_after);
            let space = stack_bytes.div_ceil(16) * 16;
            if space > 0 {
                let _ = writeln!(out, "    subq ${}, %rsp", space);
            }
            // 栈实参先做（scratch 用 rax/r10，不碰实参寄存器）
            for (i, (a, c)) in args.iter().zip(classes.iter()).enumerate() {
                if let Cls::Stack { off, dwords } = c {
                    if arg_aggs.get(i).copied().flatten().is_some() {
                        // 结构体：实参 temp 存地址，逐 dword 从 [addr] 拷到栈实参区
                        let _ = writeln!(out, "    movq {}, %r10", loc(*a, frame));
                        for k in 0..*dwords {
                            let _ = writeln!(out, "    movq {}(%r10), %rax", k * 8);
                            let _ = writeln!(out, "    movq %rax, {}(%rsp)", off + k * 8);
                        }
                    } else {
                        let _ = writeln!(out, "    movq {}, %rax", loc(*a, frame));
                        let _ = writeln!(out, "    movq %rax, {}(%rsp)", off);
                    }
                }
            }
            // 寄存器实参（局部寻址是 rbp 相对，不受 rsp 调整影响）
            for (a, c) in args.iter().zip(classes.iter()) {
                match c {
                    Cls::Int(r) => {
                        let _ = writeln!(out, "    movq {}, {}", loc(*a, frame), IARG64[*r]);
                    }
                    Cls::Xmm(r) => {
                        let _ = writeln!(out, "    movsd {}, %xmm{}", loc(*a, frame), r);
                    }
                    Cls::IntRegs(regs) => {
                        // 结构体：实参 temp 存地址，逐 dword 装入连续整型寄存器
                        let _ = writeln!(out, "    movq {}, %r10", loc(*a, frame));
                        for (k, r) in regs.iter().enumerate() {
                            let _ = writeln!(out, "    movq {}(%r10), {}", k * 8, IARG64[*r]);
                        }
                    }
                    Cls::Stack { .. } => {}
                }
            }
            if sret {
                if let Some(buf) = ret_buf {
                    let _ = writeln!(out, "    leaq {}, %rdi", loc(*buf, frame));
                }
            }
            if *variadic {
                // 可变参数函数（如 printf）要求 al = 传入的向量(xmm)寄存器数。
                let _ = writeln!(out, "    movb ${}, %al", xmm_used);
            }
            match via {
                // 间接调用：指针在 via 槽，载入非实参寄存器 r11 后 call *r11
                Some(t) => {
                    let _ = writeln!(out, "    movq {}, %r11", loc(*t, frame));
                    out.push_str("    call *%r11\n");
                }
                None => {
                    let _ = writeln!(out, "    call {}@PLT", name);
                }
            }
            if space > 0 {
                let _ = writeln!(out, "    addq ${}, %rsp", space);
            }
            match ret_agg {
                Some(size) if *size <= 16 => {
                    // 小结构体经 rax:rdx 返回，写入缓冲区
                    if let Some(buf) = ret_buf {
                        let _ = writeln!(out, "    movq %rax, {}", loc(*buf, frame));
                        if *size > 8 {
                            let _ = writeln!(out, "    movq %rdx, {}", loc(*buf + 8, frame));
                        }
                    }
                }
                Some(_) => {} // 大结构体已由被调方经隐式指针写好
                None => {
                    if *ret_float {
                        let _ = writeln!(out, "    movsd %xmm0, {}", m(*dst));
                    } else if *ret_width == 8 {
                        let _ = writeln!(out, "    movq %rax, {}", m(*dst));
                    } else {
                        let _ = writeln!(out, "    movl %eax, {}", m(*dst));
                    }
                }
            }
        }
        Instr::Bin { dst, op, lhs, rhs } => {
            let _ = writeln!(out, "    movl {}, %eax", m(*lhs));
            let _ = writeln!(out, "    movl {}, %ecx", m(*rhs));
            match op {
                BinOp::Add => out.push_str("    addl %ecx, %eax\n"),
                BinOp::Sub => out.push_str("    subl %ecx, %eax\n"),
                BinOp::Mul => out.push_str("    imull %ecx, %eax\n"),
                BinOp::Div => out.push_str("    cltd\n    idivl %ecx\n"),
                BinOp::Mod => out.push_str("    cltd\n    idivl %ecx\n    movl %edx, %eax\n"),
                BinOp::Lt => out.push_str("    cmpl %ecx, %eax\n    setl %al\n    movzbl %al, %eax\n"),
                BinOp::Gt => out.push_str("    cmpl %ecx, %eax\n    setg %al\n    movzbl %al, %eax\n"),
                BinOp::Le => out.push_str("    cmpl %ecx, %eax\n    setle %al\n    movzbl %al, %eax\n"),
                BinOp::Ge => out.push_str("    cmpl %ecx, %eax\n    setge %al\n    movzbl %al, %eax\n"),
                BinOp::Eq => out.push_str("    cmpl %ecx, %eax\n    sete %al\n    movzbl %al, %eax\n"),
                BinOp::Ne => out.push_str("    cmpl %ecx, %eax\n    setne %al\n    movzbl %al, %eax\n"),
                BinOp::BitAnd => out.push_str("    andl %ecx, %eax\n"),
                BinOp::BitOr => out.push_str("    orl %ecx, %eax\n"),
                BinOp::BitXor => out.push_str("    xorl %ecx, %eax\n"),
                BinOp::Shl => out.push_str("    shll %cl, %eax\n"),
                BinOp::Shr => out.push_str("    sarl %cl, %eax\n"),
                BinOp::UDiv => out.push_str("    xorl %edx, %edx\n    divl %ecx\n"),
                BinOp::UMod => {
                    out.push_str("    xorl %edx, %edx\n    divl %ecx\n    movl %edx, %eax\n")
                }
                BinOp::UShr => out.push_str("    shrl %cl, %eax\n"),
                BinOp::ULt => out.push_str("    cmpl %ecx, %eax\n    setb %al\n    movzbl %al, %eax\n"),
                BinOp::UGt => out.push_str("    cmpl %ecx, %eax\n    seta %al\n    movzbl %al, %eax\n"),
                BinOp::ULe => out.push_str("    cmpl %ecx, %eax\n    setbe %al\n    movzbl %al, %eax\n"),
                BinOp::UGe => out.push_str("    cmpl %ecx, %eax\n    setae %al\n    movzbl %al, %eax\n"),
            }
            let _ = writeln!(out, "    movl %eax, {}", m(*dst));
        }
        Instr::Return { src, is_float, width, agg } => {
            match agg {
                Some(size) if *size <= 16 => {
                    // src 存结构体地址，经 rax:rdx 返回
                    let _ = writeln!(out, "    movq {}, %r10", m(*src));
                    out.push_str("    movq (%r10), %rax\n");
                    if *size > 8 {
                        out.push_str("    movq 8(%r10), %rdx\n");
                    }
                }
                Some(size) => {
                    // 大结构体：经隐式返回指针回写，rax 返回该指针
                    let s = sret_slot.expect("sret slot for large struct return");
                    let _ = writeln!(out, "    movq {}, %r11", loc(s, frame)); // 目标指针
                    let _ = writeln!(out, "    movq {}, %r10", m(*src)); // 结构体地址
                    for k in 0..size.div_ceil(8) {
                        let _ = writeln!(out, "    movq {}(%r10), %rax", k * 8);
                        let _ = writeln!(out, "    movq %rax, {}(%r11)", k * 8);
                    }
                    out.push_str("    movq %r11, %rax\n");
                }
                None => {
                    if *is_float {
                        let _ = writeln!(out, "    movsd {}, %xmm0", m(*src));
                    } else if *width == 8 {
                        let _ = writeln!(out, "    movq {}, %rax", m(*src));
                    } else {
                        let _ = writeln!(out, "    movl {}, %eax", m(*src));
                    }
                }
            }
            out.push_str("    leave\n");
            out.push_str("    ret\n");
        }
        Instr::ConstF { dst, index } => {
            // 纯 8 字节位拷贝（不经 xmm）。
            let _ = writeln!(out, "    movq .Lfloat.{}(%rip), %rax", index);
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::BinF { dst, op, lhs, rhs } => {
            let _ = writeln!(out, "    movsd {}, %xmm0", m(*lhs));
            let _ = writeln!(out, "    movsd {}, %xmm1", m(*rhs));
            match op {
                BinOp::Add => out.push_str("    addsd %xmm1, %xmm0\n"),
                BinOp::Sub => out.push_str("    subsd %xmm1, %xmm0\n"),
                BinOp::Mul => out.push_str("    mulsd %xmm1, %xmm0\n"),
                BinOp::Div => out.push_str("    divsd %xmm1, %xmm0\n"),
                // ucomisd 设置的标志按无符号语义解释（a 在 xmm0、b 在 xmm1）
                BinOp::Lt => out.push_str("    ucomisd %xmm1, %xmm0\n    setb %al\n    movzbl %al, %eax\n"),
                BinOp::Gt => out.push_str("    ucomisd %xmm1, %xmm0\n    seta %al\n    movzbl %al, %eax\n"),
                BinOp::Le => out.push_str("    ucomisd %xmm1, %xmm0\n    setbe %al\n    movzbl %al, %eax\n"),
                BinOp::Ge => out.push_str("    ucomisd %xmm1, %xmm0\n    setae %al\n    movzbl %al, %eax\n"),
                BinOp::Eq => out.push_str("    ucomisd %xmm1, %xmm0\n    sete %al\n    movzbl %al, %eax\n"),
                BinOp::Ne => out.push_str("    ucomisd %xmm1, %xmm0\n    setne %al\n    movzbl %al, %eax\n"),
                _ => {} // 浮点不支持 mod/位运算
            }
            if is_compare_binop(op) {
                let _ = writeln!(out, "    movl %eax, {}", m(*dst));
            } else {
                let _ = writeln!(out, "    movsd %xmm0, {}", m(*dst));
            }
        }
        Instr::IntToFloat { dst, src } => {
            let _ = writeln!(out, "    movl {}, %eax", m(*src));
            out.push_str("    cvtsi2sd %eax, %xmm0\n");
            let _ = writeln!(out, "    movsd %xmm0, {}", m(*dst));
        }
        Instr::FloatToInt { dst, src } => {
            let _ = writeln!(out, "    movsd {}, %xmm0", m(*src));
            out.push_str("    cvttsd2si %xmm0, %eax\n");
            let _ = writeln!(out, "    movl %eax, {}", m(*dst));
        }
        Instr::BinL { dst, op, lhs, rhs } => {
            // 64 位有符号整数运算
            let _ = writeln!(out, "    movq {}, %rax", m(*lhs));
            let _ = writeln!(out, "    movq {}, %rcx", m(*rhs));
            match op {
                BinOp::Add => out.push_str("    addq %rcx, %rax\n"),
                BinOp::Sub => out.push_str("    subq %rcx, %rax\n"),
                BinOp::Mul => out.push_str("    imulq %rcx, %rax\n"),
                BinOp::Div => out.push_str("    cqto\n    idivq %rcx\n"),
                BinOp::Mod => out.push_str("    cqto\n    idivq %rcx\n    movq %rdx, %rax\n"),
                BinOp::Lt => out.push_str("    cmpq %rcx, %rax\n    setl %al\n    movzbl %al, %eax\n"),
                BinOp::Gt => out.push_str("    cmpq %rcx, %rax\n    setg %al\n    movzbl %al, %eax\n"),
                BinOp::Le => out.push_str("    cmpq %rcx, %rax\n    setle %al\n    movzbl %al, %eax\n"),
                BinOp::Ge => out.push_str("    cmpq %rcx, %rax\n    setge %al\n    movzbl %al, %eax\n"),
                BinOp::Eq => out.push_str("    cmpq %rcx, %rax\n    sete %al\n    movzbl %al, %eax\n"),
                BinOp::Ne => out.push_str("    cmpq %rcx, %rax\n    setne %al\n    movzbl %al, %eax\n"),
                BinOp::BitAnd => out.push_str("    andq %rcx, %rax\n"),
                BinOp::BitOr => out.push_str("    orq %rcx, %rax\n"),
                BinOp::BitXor => out.push_str("    xorq %rcx, %rax\n"),
                BinOp::Shl => out.push_str("    salq %cl, %rax\n"),
                BinOp::Shr => out.push_str("    sarq %cl, %rax\n"),
                BinOp::UDiv => out.push_str("    xorl %edx, %edx\n    divq %rcx\n"),
                BinOp::UMod => {
                    out.push_str("    xorl %edx, %edx\n    divq %rcx\n    movq %rdx, %rax\n")
                }
                BinOp::UShr => out.push_str("    shrq %cl, %rax\n"),
                BinOp::ULt => out.push_str("    cmpq %rcx, %rax\n    setb %al\n    movzbl %al, %eax\n"),
                BinOp::UGt => out.push_str("    cmpq %rcx, %rax\n    seta %al\n    movzbl %al, %eax\n"),
                BinOp::ULe => out.push_str("    cmpq %rcx, %rax\n    setbe %al\n    movzbl %al, %eax\n"),
                BinOp::UGe => out.push_str("    cmpq %rcx, %rax\n    setae %al\n    movzbl %al, %eax\n"),
            }
            // 比较结果是 32 位 0/1（已在 eax），算术结果是 64 位（在 rax）
            if is_compare_binop(op) {
                let _ = writeln!(out, "    movl %eax, {}", m(*dst));
            } else {
                let _ = writeln!(out, "    movq %rax, {}", m(*dst));
            }
        }
        Instr::NegL { dst, src } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*src));
            out.push_str("    negq %rax\n");
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::Widen { dst, src } => {
            // 符号扩展 32→64（int → long）
            let _ = writeln!(out, "    movslq {}, %rax", m(*src));
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::WidenU { dst, src } => {
            // 零扩展 32→64（unsigned int → ulong）:movl 自动清零高 32 位
            let _ = writeln!(out, "    movl {}, %eax", m(*src));
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
        Instr::LongToFloat { dst, src } => {
            let _ = writeln!(out, "    movq {}, %rax", m(*src));
            out.push_str("    cvtsi2sdq %rax, %xmm0\n");
            let _ = writeln!(out, "    movsd %xmm0, {}", m(*dst));
        }
        Instr::FloatToLong { dst, src } => {
            let _ = writeln!(out, "    movsd {}, %xmm0", m(*src));
            out.push_str("    cvttsd2siq %xmm0, %rax\n");
            let _ = writeln!(out, "    movq %rax, {}", m(*dst));
        }
    }
}

/// 把 %rcx 乘以元素字节大小 size（2 的幂用移位，否则用 imulq）。
fn scale_index(size: usize, out: &mut String) {
    match size {
        0 | 1 => {}
        2 => out.push_str("    shlq $1, %rcx\n"),
        4 => out.push_str("    shlq $2, %rcx\n"),
        8 => out.push_str("    shlq $3, %rcx\n"),
        16 => out.push_str("    shlq $4, %rcx\n"),
        n => {
            let _ = writeln!(out, "    imulq ${}, %rcx, %rcx", n);
        }
    }
}

fn is_compare_binop(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::Lt
            | BinOp::Gt
            | BinOp::Le
            | BinOp::Ge
            | BinOp::Eq
            | BinOp::Ne
            | BinOp::ULt
            | BinOp::UGt
            | BinOp::ULe
            | BinOp::UGe
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, Function, Instr, Program};

    fn gen(body: Vec<Instr>, frame_bytes: usize) -> String {
        generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                params: vec![],
                body,
                frame_bytes,
                ret_float: false,
                ret_agg: None,
                sret_slot: None, variadic: false,
            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        })
    }

    #[test]
    fn peephole_forwards_store_to_load() {
        let asm = "    movl $42, -8(%rbp)\n    movl -8(%rbp), %eax\n";
        let out = peephole(asm);
        assert!(out.contains("movl $42, -8(%rbp)")); // store 保留
        assert!(out.contains("movl $42, %eax")); // 内存载入 → 立即数搬移
        assert!(!out.contains("movl -8(%rbp), %eax")); // 原内存载入消失
    }

    #[test]
    fn peephole_drops_noop_reload() {
        let asm = "    movl %eax, -8(%rbp)\n    movl -8(%rbp), %eax\n";
        let out = peephole(asm);
        assert_eq!(out.matches("movl").count(), 1); // 同源 load 删除，仅留 store
    }

    #[test]
    fn peephole_keeps_different_slot() {
        // 不同槽位不应被改写
        let asm = "    movl $1, -8(%rbp)\n    movl -16(%rbp), %eax\n";
        let out = peephole(asm);
        assert!(out.contains("movl -16(%rbp), %eax"));
    }

    #[test]
    fn const_return() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 42 }, Instr::Return { src: 0, is_float: false, width: 4, agg: None }],
            8,
        );
        assert!(asm.contains(".globl main"));
        assert!(asm.contains("main:"));
        assert!(asm.contains("movl $42,"));
        assert!(asm.contains("leave"));
        assert!(asm.contains("ret"));
    }

    #[test]
    fn prologue_sets_up_rbp_frame() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 1 }, Instr::Return { src: 0, is_float: false, width: 4, agg: None }],
            8,
        );
        assert!(asm.contains("pushq %rbp"));
        assert!(asm.contains("movq %rsp, %rbp"));
        assert!(asm.contains("subq $16, %rsp")); // 8 → 对齐到 16
    }

    #[test]
    fn add_uses_addl() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 8, value: 2 },
                Instr::Bin { dst: 16, op: BinOp::Add, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: false, width: 4, agg: None },
            ],
            24,
        );
        assert!(asm.contains("addl %ecx, %eax"));
    }

    #[test]
    fn div_uses_cltd_idivl() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 17 },
                Instr::Const { dst: 8, value: 5 },
                Instr::Bin { dst: 16, op: BinOp::Mod, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: false, width: 4, agg: None },
            ],
            24,
        );
        assert!(asm.contains("cltd"));
        assert!(asm.contains("idivl %ecx"));
        assert!(asm.contains("movl %edx, %eax")); // mod 取余数
    }

    #[test]
    fn call_and_params_use_sysv_regs() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                params: vec![crate::ir::Param { slot: 0, size: 4, is_float: false, is_aggregate: false }],
                body: vec![
                    Instr::Call {
                        dst: 8,
                        name: "puts".to_string(),
                        via: None,
                        args: vec![0],
                        arg_floats: vec![false],
                        arg_aggs: vec![None],
                        ret_width: 4,
                        ret_agg: None,
                        ret_buf: None,
                        fixed: 1,
                        variadic: false,
                        stack_varargs: false,                        ret_float: false,
                    },
                    Instr::Return { src: 8, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 16,
                ret_float: false,
                ret_agg: None,
                sret_slot: None, variadic: false,
            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("movl %edi,")); // 第 0 个入参在 edi
        assert!(asm.contains(", %rdi")); // 调用时第 0 个实参装入 rdi
        assert!(asm.contains("call puts@PLT"));
    }

    #[test]
    fn variadic_call_sets_al() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                params: vec![],
                body: vec![Instr::Call {
                    dst: 0,
                    name: "printf".to_string(),
                    via: None,
                    args: vec![],
                    arg_floats: vec![],
                    arg_aggs: vec![],
                    ret_width: 4,
                    ret_agg: None,
                    ret_buf: None,
                    fixed: 1,
                    variadic: true,
                    stack_varargs: false,                    ret_float: false,
                }],
                frame_bytes: 16,
                ret_float: false,
                ret_agg: None,
                sret_slot: None, variadic: false,
            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("movb $0, %al"));
    }

    #[test]
    fn float_arith_uses_xmm() {
        let asm = gen(
            vec![
                Instr::ConstF { dst: 0, index: 0 },
                Instr::ConstF { dst: 8, index: 1 },
                Instr::BinF { dst: 16, op: BinOp::Add, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: true, width: 8, agg: None },
            ],
            24,
        );
        assert!(asm.contains("addsd %xmm1, %xmm0"));
        assert!(asm.contains("movsd %xmm0, -16(%rbp)")); // 结果落回槽位 16（frame 32）
    }

    #[test]
    fn ptradd_scales_by_size() {
        let asm = gen(
            vec![
                Instr::PtrAdd { dst: 16, base: 0, index: 8, size: 4 },
                Instr::Return { src: 16, is_float: false, width: 8, agg: None },
            ],
            24,
        );
        assert!(asm.contains("shlq $2, %rcx")); // ×4 用移位
        assert!(asm.contains("addq %rcx, %rax"));
    }

    #[test]
    fn ptradd_nonpow2_size_uses_imul() {
        let asm = gen(
            vec![
                Instr::PtrAdd { dst: 16, base: 0, index: 8, size: 12 },
                Instr::Return { src: 16, is_float: false, width: 8, agg: None },
            ],
            24,
        );
        assert!(asm.contains("imulq $12, %rcx, %rcx")); // 非 2 的幂用乘法
    }

    #[test]
    fn strlit_rip_relative() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::StrLit { dst: 0, index: 0 },
                    Instr::Return { src: 0, is_float: false, width: 8, agg: None },
                ],
                frame_bytes: 8,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec!["Hi".to_string()],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("leaq .Lstr.0(%rip), %rax"));
        assert!(asm.contains(".Lstr.0:"));
        assert!(asm.contains(".byte 72, 105, 0"));
        assert!(asm.contains(".section .rodata"));
    }
}
