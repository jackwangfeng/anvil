use crate::ir::{BinOp, Function, Instr, Program};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".section __TEXT,__text,regular,pure_instructions\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    let mut out = peephole(&out);
    gen_strings(&program.strings, &mut out);
    gen_globals(&program.globals, &mut out);
    gen_floats(&program.floats, &mut out);
    out
}

fn is_compare_binop(op: &BinOp) -> bool {
    matches!(
        op,
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
    )
}

/// 窥孔优化：相邻的 `str rN, [sp,#k]` + `ldr rM, [sp,#k]`（同槽位、同寄存器宽度类）
/// → 把内存载入改为寄存器搬移（同寄存器则删除）。保守：仅相邻两行、其间无标签/跳转。
fn peephole(asm: &str) -> String {
    let lines: Vec<&str> = asm.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        if i + 1 < lines.len() {
            if let (Some((sreg, smem)), Some((lreg, lmem))) =
                (parse_str(lines[i]), parse_ldr(lines[i + 1]))
            {
                // 同槽位且寄存器宽度类一致（w/x/d 首字母相同）
                if smem == lmem && sreg.chars().next() == lreg.chars().next() {
                    out.push(lines[i].to_string()); // 保留 store
                    if sreg != lreg {
                        let mv = if sreg.starts_with('d') { "fmov" } else { "mov" };
                        out.push(format!("    {} {}, {}", mv, lreg, sreg));
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

/// 解析 `    str <reg>, [sp, #<k>]` → (reg, "[sp, #k]")。
fn parse_str(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("str ")?;
    let (reg, mem) = rest.split_once(", ")?;
    if mem.starts_with("[sp, #") && mem.ends_with(']') {
        Some((reg, mem))
    } else {
        None
    }
}

/// 解析 `    ldr <reg>, [sp, #<k>]` → (reg, "[sp, #k]")。
fn parse_ldr(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("ldr ")?;
    let (reg, mem) = rest.split_once(", ")?;
    if mem.starts_with("[sp, #") && mem.ends_with(']') {
        Some((reg, mem))
    } else {
        None
    }
}

fn gen_floats(floats: &[u64], out: &mut String) {
    if floats.is_empty() {
        return;
    }
    out.push_str(".section __TEXT,__const\n");
    out.push_str(".p2align 3\n");
    for (i, bits) in floats.iter().enumerate() {
        let _ = writeln!(out, "Lfloat.{}:", i);
        let _ = writeln!(out, "    .quad {}", bits);
    }
}

fn gen_globals(globals: &[crate::ir::GlobalVar], out: &mut String) {
    if globals.is_empty() {
        return;
    }
    out.push_str(".section __DATA,__data\n");
    for g in globals {
        out.push_str(".globl _");
        out.push_str(&g.name);
        out.push('\n');
        out.push_str(".p2align 3\n");
        let _ = writeln!(out, "_{}:", g.name);
        match (g.init, g.size) {
            (Some(v), 8) => {
                let _ = writeln!(out, "    .quad {}", v);
            }
            (Some(v), _) => {
                let _ = writeln!(out, "    .long {}", v);
            }
            (None, n) => {
                let _ = writeln!(out, "    .zero {}", n.max(1));
            }
        }
    }
}

/// 槽位即帧内字节偏移。
fn slot(t: usize) -> usize {
    t
}

/// 栈帧大小：frame_bytes 向上对齐到 16。
fn frame_size(frame_bytes: usize) -> usize {
    frame_bytes.div_ceil(16) * 16
}

fn gen_func(func: &Function, out: &mut String) {
    let frame = frame_size(func.frame_bytes);
    let _ = writeln!(out, ".globl _{}", func.name);
    out.push_str(".p2align 2\n");
    let _ = writeln!(out, "_{}:", func.name);
    out.push_str("    stp x29, x30, [sp, #-16]!\n");
    out.push_str("    mov x29, sp\n");
    if frame > 0 {
        let _ = writeln!(out, "    sub sp, sp, #{}", frame);
    }
    // 大结构体返回：隐式返回指针在 x8，存到 sret 槽供 Return 回写。
    if let Some(s) = func.sret_slot {
        let _ = writeln!(out, "    str x8, [sp, #{}]", slot(s));
    }
    gen_params(&func.params, out);
    // 可变参数：可变实参从 [x29 + 16 + 具名栈实参字节数] 开始。
    let va_base = if func.variadic {
        let items: Vec<(bool, Option<usize>)> = func
            .params
            .iter()
            .map(|p| (p.is_float, if p.is_aggregate { Some(p.size) } else { None }))
            .collect();
        let (_, named_stack) = classify_aapcs64(&items, false, func.params.len());
        16 + named_stack
    } else {
        16
    };
    for instr in &func.body {
        gen_instr(instr, &func.name, frame, func.sret_slot, va_base, out);
    }
}

/// 一个实参/形参在 AAPCS64 下的去向。
enum Cls {
    /// 标量整型/指针 → 第 n 个整型寄存器 x{n}。
    Int(usize),
    /// 标量 double → 第 n 个 FP 寄存器 d{n}。
    Fp(usize),
    /// 按值结构体（≤16B）→ 连续整型寄存器。
    IntRegs(Vec<usize>),
    /// 压栈：位于栈实参区偏移 `off`，占 `dwords` 个 8 字节。
    Stack { off: usize, dwords: usize },
}

/// AAPCS64 实参分类：整型组 x0..x7（8），FP 组 d0..d7（8），各自独立计数；
/// 溢出按 8 字节为单位压栈。Apple 约定：可变参数（index ≥ fixed）一律压栈。
/// 大结构体返回经 x8（独立于 x0-x7，不占整型参数槽），故此处无需 sret 偏移。
/// 返回 (各项去向, 栈实参区总字节数)。
fn classify_aapcs64(
    items: &[(bool, Option<usize>)],
    variadic: bool,
    fixed: usize,
) -> (Vec<Cls>, usize) {
    let mut ngrn = 0;
    let mut nsrn = 0;
    let mut soff = 0;
    let classes = items
        .iter()
        .enumerate()
        .map(|(i, &(is_float, agg))| {
            let force_stack = variadic && i >= fixed;
            if let Some(sz) = agg {
                let ndw = sz.div_ceil(8);
                if !force_stack && sz <= 16 && ngrn + ndw <= 8 {
                    let regs = (ngrn..ngrn + ndw).collect();
                    ngrn += ndw;
                    Cls::IntRegs(regs)
                } else {
                    let off = soff;
                    soff += ndw * 8;
                    Cls::Stack { off, dwords: ndw }
                }
            } else if is_float && !force_stack {
                if nsrn < 8 {
                    nsrn += 1;
                    Cls::Fp(nsrn - 1)
                } else {
                    let off = soff;
                    soff += 8;
                    Cls::Stack { off, dwords: 1 }
                }
            } else if !force_stack && ngrn < 8 {
                ngrn += 1;
                Cls::Int(ngrn - 1)
            } else {
                let off = soff;
                soff += 8;
                Cls::Stack { off, dwords: 1 }
            }
        })
        .collect();
    (classes, soff)
}

/// 把入参从寄存器/栈落到各自帧槽位（AAPCS64）。
fn gen_params(params: &[crate::ir::Param], out: &mut String) {
    let items: Vec<(bool, Option<usize>)> = params
        .iter()
        .map(|p| (p.is_float, if p.is_aggregate { Some(p.size) } else { None }))
        .collect();
    let (classes, _) = classify_aapcs64(&items, false, params.len());
    for (p, c) in params.iter().zip(classes.iter()) {
        match c {
            Cls::Int(r) => match p.size {
                8 => {
                    let _ = writeln!(out, "    str x{}, [sp, #{}]", r, slot(p.slot));
                }
                1 => {
                    let _ = writeln!(out, "    strb w{}, [sp, #{}]", r, slot(p.slot));
                }
                _ => {
                    let _ = writeln!(out, "    str w{}, [sp, #{}]", r, slot(p.slot));
                }
            },
            Cls::Fp(r) => {
                let _ = writeln!(out, "    str d{}, [sp, #{}]", r, slot(p.slot));
            }
            Cls::IntRegs(regs) => {
                for (k, r) in regs.iter().enumerate() {
                    let _ = writeln!(out, "    str x{}, [sp, #{}]", r, slot(p.slot + k * 8));
                }
            }
            Cls::Stack { off, dwords } => {
                for k in 0..*dwords {
                    let _ = writeln!(out, "    ldr x9, [x29, #{}]", 16 + off + k * 8);
                    let _ = writeln!(out, "    str x9, [sp, #{}]", slot(p.slot + k * 8));
                }
            }
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

fn gen_instr(
    instr: &Instr,
    func: &str,
    frame: usize,
    sret_slot: Option<usize>,
    va_base: usize,
    out: &mut String,
) {
    match instr {
        Instr::Const { dst, value } => {
            if *value >= i32::MIN as i64 && *value <= i32::MAX as i64 {
                materialize_const(*value, out);
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
            } else {
                // 超 32 位字面量（long）：64 位物化，写满 8 字节
                materialize_const64(*value, out);
                let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
            }
        }
        Instr::Neg { dst, src } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            out.push_str("    neg w9, w9\n");
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
        Instr::StrLit { dst, index } => {
            let _ = writeln!(out, "    adrp x9, L_.str.{}@PAGE", index);
            let _ = writeln!(out, "    add x9, x9, L_.str.{}@PAGEOFF", index);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::AddrOf { dst, off } => {
            let _ = writeln!(out, "    add x9, sp, #{}", off);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::GlobalAddr { dst, name } => {
            let _ = writeln!(out, "    adrp x9, _{}@PAGE", name);
            let _ = writeln!(out, "    add x9, x9, _{}@PAGEOFF", name);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::FieldAddr { dst, base, offset } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    add x9, x9, #{}", offset);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::Copy { dst, src, width } => {
            if *width == 8 {
                let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*src));
                let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
            }
        }
        Instr::MemCpy { dst, src, size } => {
            // dst/src 存放目标/源地址，逐 8 字节拷贝（向上取整）。
            let _ = writeln!(out, "    ldr x11, [sp, #{}]", slot(*dst));
            let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*src));
            for k in 0..size.div_ceil(8) {
                let _ = writeln!(out, "    ldr x9, [x10, #{}]", k * 8);
                let _ = writeln!(out, "    str x9, [x11, #{}]", k * 8);
            }
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
        Instr::PtrAdd { dst, base, index, size } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*index));
            out.push_str("    sxtw x10, w10\n");
            let _ = writeln!(out, "    mov x11, #{}", size);
            out.push_str("    madd x9, x10, x11, x9\n"); // x9 = base + index*size
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::PtrSub { dst, base, index, size } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*base));
            let _ = writeln!(out, "    ldr w10, [sp, #{}]", slot(*index));
            out.push_str("    sxtw x10, w10\n");
            let _ = writeln!(out, "    mov x11, #{}", size);
            out.push_str("    msub x9, x10, x11, x9\n"); // x9 = base - index*size
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::FuncAddr { dst, name } => {
            let _ = writeln!(out, "    adrp x9, _{}@PAGE", name);
            let _ = writeln!(out, "    add x9, x9, _{}@PAGEOFF", name);
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::VaStart { ap } => {
            let _ = writeln!(out, "    add x9, x29, #{}", va_base); // 首个可变参数地址
            let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*ap)); // x10 = &va_list
            out.push_str("    str x9, [x10]\n");
        }
        Instr::VaArg { dst, ap, width } => {
            let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*ap)); // &va_list
            out.push_str("    ldr x11, [x10]\n"); // 当前遍历指针
            if *width == 8 {
                out.push_str("    ldr x9, [x11]\n");
            } else {
                out.push_str("    ldr w9, [x11]\n");
            }
            out.push_str("    add x11, x11, #8\n");
            out.push_str("    str x11, [x10]\n");
            if *width == 8 {
                let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
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
            stack_varargs: _, // arm64：可变参数本就一律压栈，无需区分
            ret_float,
        } => {
            let items: Vec<(bool, Option<usize>)> = (0..args.len())
                .map(|i| {
                    (
                        arg_floats.get(i).copied().unwrap_or(false),
                        arg_aggs.get(i).copied().flatten(),
                    )
                })
                .collect();
            let (classes, stack_bytes) = classify_aapcs64(&items, *variadic, *fixed);
            let space = stack_bytes.div_ceil(16) * 16;
            if space > 0 {
                let _ = writeln!(out, "    sub sp, sp, #{}", space);
            }
            // sub sp 之后，局部槽位访问需 +space。
            let r = |t: usize| space + slot(t);
            // 栈实参（scratch 用 x9/x10，不碰参数寄存器 x0-x7）
            for (i, (a, c)) in args.iter().zip(classes.iter()).enumerate() {
                if let Cls::Stack { off, dwords } = c {
                    if arg_aggs.get(i).copied().flatten().is_some() {
                        // 结构体：实参 temp 存地址，逐 dword 从 [addr] 拷到栈实参区
                        let _ = writeln!(out, "    ldr x9, [sp, #{}]", r(*a));
                        for k in 0..*dwords {
                            let _ = writeln!(out, "    ldr x10, [x9, #{}]", k * 8);
                            let _ = writeln!(out, "    str x10, [sp, #{}]", off + k * 8);
                        }
                    } else {
                        let _ = writeln!(out, "    ldr x9, [sp, #{}]", r(*a));
                        let _ = writeln!(out, "    str x9, [sp, #{}]", off);
                    }
                }
            }
            // 寄存器实参
            for (a, c) in args.iter().zip(classes.iter()) {
                match c {
                    Cls::Int(reg) => {
                        let _ = writeln!(out, "    ldr x{}, [sp, #{}]", reg, r(*a));
                    }
                    Cls::Fp(reg) => {
                        let _ = writeln!(out, "    ldr d{}, [sp, #{}]", reg, r(*a));
                    }
                    Cls::IntRegs(regs) => {
                        // 结构体：实参 temp 存地址，逐 dword 装入连续整型寄存器
                        let _ = writeln!(out, "    ldr x9, [sp, #{}]", r(*a));
                        for (k, reg) in regs.iter().enumerate() {
                            let _ = writeln!(out, "    ldr x{}, [x9, #{}]", reg, k * 8);
                        }
                    }
                    Cls::Stack { .. } => {}
                }
            }
            // 大结构体返回：&缓冲区放入 x8（独立于参数寄存器）。
            if matches!(ret_agg, Some(sz) if *sz > 16) {
                if let Some(buf) = ret_buf {
                    let _ = writeln!(out, "    add x8, sp, #{}", r(*buf));
                }
            }
            match via {
                // 间接调用：指针在 via 槽（sub sp 后偏移 +space），载入 x9 后 blr
                Some(t) => {
                    let _ = writeln!(out, "    ldr x9, [sp, #{}]", r(*t));
                    out.push_str("    blr x9\n");
                }
                None => {
                    let _ = writeln!(out, "    bl _{}", name);
                }
            }
            if space > 0 {
                let _ = writeln!(out, "    add sp, sp, #{}", space);
            }
            match ret_agg {
                Some(size) if *size <= 16 => {
                    // 小结构体经 x0:x1 返回，写入缓冲区
                    if let Some(buf) = ret_buf {
                        let _ = writeln!(out, "    str x0, [sp, #{}]", slot(*buf));
                        if *size > 8 {
                            let _ = writeln!(out, "    str x1, [sp, #{}]", slot(*buf + 8));
                        }
                    }
                }
                Some(_) => {} // 大结构体已由被调方经 x8 写好
                None => {
                    if *ret_float {
                        let _ = writeln!(out, "    str d0, [sp, #{}]", slot(*dst));
                    } else if *ret_width == 8 {
                        let _ = writeln!(out, "    str x0, [sp, #{}]", slot(*dst));
                    } else {
                        let _ = writeln!(out, "    str w0, [sp, #{}]", slot(*dst));
                    }
                }
            }
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
                BinOp::BitAnd => out.push_str("    and w9, w9, w10\n"),
                BinOp::BitOr => out.push_str("    orr w9, w9, w10\n"),
                BinOp::BitXor => out.push_str("    eor w9, w9, w10\n"),
                BinOp::Shl => out.push_str("    lsl w9, w9, w10\n"),
                BinOp::Shr => out.push_str("    asr w9, w9, w10\n"),
            }
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::Return { src, is_float, width, agg } => {
            match agg {
                Some(size) if *size <= 16 => {
                    // src 存结构体地址，经 x0:x1 返回
                    let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*src));
                    out.push_str("    ldr x0, [x9]\n");
                    if *size > 8 {
                        out.push_str("    ldr x1, [x9, #8]\n");
                    }
                }
                Some(size) => {
                    // 大结构体：经隐式返回指针(x8 已存入 sret 槽)回写，并在 x0 返回该指针
                    let s = sret_slot.expect("sret slot for large struct return");
                    let _ = writeln!(out, "    ldr x11, [sp, #{}]", slot(s)); // 目标指针
                    let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*src)); // 结构体地址
                    for k in 0..size.div_ceil(8) {
                        let _ = writeln!(out, "    ldr x9, [x10, #{}]", k * 8);
                        let _ = writeln!(out, "    str x9, [x11, #{}]", k * 8);
                    }
                    out.push_str("    mov x0, x11\n");
                }
                None => {
                    if *is_float {
                        let _ = writeln!(out, "    ldr d0, [sp, #{}]", slot(*src));
                    } else if *width == 8 {
                        let _ = writeln!(out, "    ldr x0, [sp, #{}]", slot(*src));
                    } else {
                        let _ = writeln!(out, "    ldr w0, [sp, #{}]", slot(*src));
                    }
                }
            }
            if frame > 0 {
                let _ = writeln!(out, "    add sp, sp, #{}", frame);
            }
            out.push_str("    ldp x29, x30, [sp], #16\n");
            out.push_str("    ret\n");
        }
        Instr::ConstF { dst, index } => {
            // 从浮点常量池按位载入（不需 FP 寄存器，纯 8 字节位拷贝）
            let _ = writeln!(out, "    adrp x9, Lfloat.{}@PAGE", index);
            let _ = writeln!(out, "    add x9, x9, Lfloat.{}@PAGEOFF", index);
            out.push_str("    ldr x9, [x9]\n");
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::BinF { dst, op, lhs, rhs } => {
            let _ = writeln!(out, "    ldr d0, [sp, #{}]", slot(*lhs));
            let _ = writeln!(out, "    ldr d1, [sp, #{}]", slot(*rhs));
            match op {
                BinOp::Add => out.push_str("    fadd d0, d0, d1\n"),
                BinOp::Sub => out.push_str("    fsub d0, d0, d1\n"),
                BinOp::Mul => out.push_str("    fmul d0, d0, d1\n"),
                BinOp::Div => out.push_str("    fdiv d0, d0, d1\n"),
                BinOp::Lt => out.push_str("    fcmp d0, d1\n    cset w9, lt\n"),
                BinOp::Gt => out.push_str("    fcmp d0, d1\n    cset w9, gt\n"),
                BinOp::Le => out.push_str("    fcmp d0, d1\n    cset w9, le\n"),
                BinOp::Ge => out.push_str("    fcmp d0, d1\n    cset w9, ge\n"),
                BinOp::Eq => out.push_str("    fcmp d0, d1\n    cset w9, eq\n"),
                BinOp::Ne => out.push_str("    fcmp d0, d1\n    cset w9, ne\n"),
                _ => {} // 浮点不支持 mod/位运算
            }
            // 比较结果在 w9（int），算术结果在 d0（double）
            if is_compare_binop(op) {
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    str d0, [sp, #{}]", slot(*dst));
            }
        }
        Instr::IntToFloat { dst, src } => {
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            out.push_str("    scvtf d0, w9\n");
            let _ = writeln!(out, "    str d0, [sp, #{}]", slot(*dst));
        }
        Instr::FloatToInt { dst, src } => {
            let _ = writeln!(out, "    ldr d0, [sp, #{}]", slot(*src));
            out.push_str("    fcvtzs w9, d0\n");
            let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
        }
        Instr::BinL { dst, op, lhs, rhs } => {
            // 64 位有符号整数运算
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*lhs));
            let _ = writeln!(out, "    ldr x10, [sp, #{}]", slot(*rhs));
            match op {
                BinOp::Add => out.push_str("    add x9, x9, x10\n"),
                BinOp::Sub => out.push_str("    sub x9, x9, x10\n"),
                BinOp::Mul => out.push_str("    mul x9, x9, x10\n"),
                BinOp::Div => out.push_str("    sdiv x9, x9, x10\n"),
                BinOp::Mod => {
                    out.push_str("    sdiv x11, x9, x10\n");
                    out.push_str("    msub x9, x11, x10, x9\n");
                }
                BinOp::Lt => out.push_str("    cmp x9, x10\n    cset w9, lt\n"),
                BinOp::Gt => out.push_str("    cmp x9, x10\n    cset w9, gt\n"),
                BinOp::Le => out.push_str("    cmp x9, x10\n    cset w9, le\n"),
                BinOp::Ge => out.push_str("    cmp x9, x10\n    cset w9, ge\n"),
                BinOp::Eq => out.push_str("    cmp x9, x10\n    cset w9, eq\n"),
                BinOp::Ne => out.push_str("    cmp x9, x10\n    cset w9, ne\n"),
                BinOp::BitAnd => out.push_str("    and x9, x9, x10\n"),
                BinOp::BitOr => out.push_str("    orr x9, x9, x10\n"),
                BinOp::BitXor => out.push_str("    eor x9, x9, x10\n"),
                BinOp::Shl => out.push_str("    lsl x9, x9, x10\n"),
                BinOp::Shr => out.push_str("    asr x9, x9, x10\n"),
            }
            // 比较结果是 32 位 0/1（在 w9），算术结果是 64 位（在 x9）
            if is_compare_binop(op) {
                let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
            }
        }
        Instr::NegL { dst, src } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*src));
            out.push_str("    neg x9, x9\n");
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::Widen { dst, src } => {
            // 符号扩展 32→64（int → long）
            let _ = writeln!(out, "    ldr w9, [sp, #{}]", slot(*src));
            out.push_str("    sxtw x9, w9\n");
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
        }
        Instr::LongToFloat { dst, src } => {
            let _ = writeln!(out, "    ldr x9, [sp, #{}]", slot(*src));
            out.push_str("    scvtf d0, x9\n");
            let _ = writeln!(out, "    str d0, [sp, #{}]", slot(*dst));
        }
        Instr::FloatToLong { dst, src } => {
            let _ = writeln!(out, "    ldr d0, [sp, #{}]", slot(*src));
            out.push_str("    fcvtzs x9, d0\n");
            let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
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

/// 把 64 位常量装入 x9：movz 最低半字，其余非零半字 movk 到对应位。
fn materialize_const64(value: i64, out: &mut String) {
    let u = value as u64;
    let h0 = u & 0xffff;
    let _ = writeln!(out, "    movz x9, #{}", h0);
    for (shift, hw) in [(16, (u >> 16) & 0xffff), (32, (u >> 32) & 0xffff), (48, (u >> 48) & 0xffff)] {
        if hw != 0 {
            let _ = writeln!(out, "    movk x9, #{}, lsl #{}", hw, shift);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, Function, Instr, Program};

    /// 全流程编译到 arm64 (Mach-O) 汇编字符串，用于 ABI 意图断言。
    /// （本机无法汇编/运行 Mach-O，故 arm64 的 AAPCS64 行为以汇编断言校验，
    /// 真正的端到端验证需在 Apple Silicon 上跑集成测试。）
    fn asm_arm64(src: &str) -> String {
        crate::compile_to_asm_target(src, crate::Target::Arm64).unwrap()
    }

    #[test]
    fn arm64_double_param_in_fp_reg() {
        // n→x0(整型组), x→d0(FP 组)，两组独立计数
        let asm = asm_arm64("double f(int n, double x){ return x; } int main(){ return 0; }");
        assert!(asm.contains("str w0, [sp, #0]")); // int 形参 n 落自 x0
        assert!(asm.contains("str d0, [sp, #8]")); // double 形参 x 落自 d0
    }

    #[test]
    fn arm64_double_args_passed_in_d_regs() {
        let asm = asm_arm64("double add(double a, double b){ return a+b; } int main(){ double r = add(1.0, 2.0); return 0; }");
        assert!(asm.contains("ldr d0, [sp")); // 实参 a → d0
        assert!(asm.contains("ldr d1, [sp")); // 实参 b → d1
    }

    #[test]
    fn arm64_struct_arg_small_in_int_regs() {
        // 16 字节结构体形参经 x0:x1 传入，被调方落到连续槽位
        let asm = asm_arm64("struct P{int x;int y;}; int s(struct P p){ return p.x+p.y; } int main(){ return 0; }");
        let body = &asm[..asm.find("_main:").unwrap_or(asm.len())]; // 取 _s 函数体
        assert!(body.contains("str x0, [sp, #0]"));
        assert!(body.contains("str x1, [sp, #8]"));
    }

    #[test]
    fn arm64_struct_arg_passed_by_loading_dwords() {
        // 调用方把结构体逐 dword 装入 x0,x1
        let asm = asm_arm64("struct P{int x;int y;}; int s(struct P p){ return p.x; } int main(){ struct P q; q.x=1; q.y=2; return s(q); }");
        assert!(asm.contains("ldr x0, [x9, #0]"));
        assert!(asm.contains("ldr x1, [x9, #8]"));
    }

    #[test]
    fn arm64_struct_arg_large_on_stack() {
        // >16 字节结构体经栈传参（sub sp + 逐 dword str）
        let asm = asm_arm64("struct B{int a;int b;int c;int d;int e;}; int s(struct B b){ return b.a; } int main(){ struct B g; g.a=1; return s(g); }");
        assert!(asm.contains("sub sp, sp, #")); // 为栈实参开辟空间
    }

    #[test]
    fn arm64_struct_return_small_x0_x1() {
        let asm = asm_arm64("struct P{int x;int y;}; struct P mk(){ struct P p; p.x=1; p.y=2; return p; } int main(){ return 0; }");
        assert!(asm.contains("ldr x0, [x9]"));
        assert!(asm.contains("ldr x1, [x9, #8]"));
    }

    #[test]
    fn arm64_struct_return_large_uses_x8() {
        // >16 字节结构体返回：序言保存 x8，Return 经隐式指针回写
        let asm = asm_arm64("struct B{int a;int b;int c;int d;int e;}; struct B mk(){ struct B z; z.a=7; return z; } int main(){ return 0; }");
        assert!(asm.contains("str x8, [sp")); // 序言保存隐式返回指针
        assert!(asm.contains("mov x0, x11")); // 返回时把指针放回 x0
    }

    #[test]
    fn arm64_variadic_doubles_still_go_on_stack() {
        // Apple 约定：可变参数（含 double）一律压栈；固定的格式串仍走 x0
        let asm = asm_arm64("int printf(char* fmt, ...); int main(){ printf(\"%f\", 3.5); return 0; }");
        assert!(asm.contains("bl _printf"));
        assert!(asm.contains("sub sp, sp, #")); // double 实参压栈
    }

    #[test]
    fn arm64_do_while_body_before_cond() {
        // do-while：循环体在条件判断之前（body 紧跟入口标签，cond 在体之后）
        let asm = asm_arm64("int main(){ int i = 0; do { i++; } while (i < 3); return i; }");
        // 含一个条件分支(cbz)用于退出循环
        assert!(asm.contains("cbz w9,"));
    }

    #[test]
    fn peephole_arm64_forwards_store_to_load() {
        let asm = "    str w9, [sp, #0]\n    ldr w0, [sp, #0]\n";
        let out = peephole(asm);
        assert!(out.contains("str w9, [sp, #0]")); // store 保留
        assert!(out.contains("mov w0, w9")); // 载入 → 寄存器搬移
        assert!(!out.contains("ldr w0, [sp, #0]"));
    }

    #[test]
    fn peephole_arm64_drops_noop_and_respects_width() {
        // 同寄存器 → 删除 load
        let a = peephole("    str x9, [sp, #8]\n    ldr x9, [sp, #8]\n");
        assert_eq!(a.matches("ldr").count(), 0);
        // 宽度类不同（w vs x）→ 不改写
        let b = peephole("    str w9, [sp, #8]\n    ldr x0, [sp, #8]\n");
        assert!(b.contains("ldr x0, [sp, #8]"));
        // 指针解引用（非 [sp,#]）→ 不碰
        let c = peephole("    str x10, [x9]\n    ldr x10, [x9]\n");
        assert!(c.contains("ldr x10, [x9]"));
    }

    #[test]
    fn arm64_long_arith_uses_64bit_ops() {
        let asm = asm_arm64("long f(long a, long b){ return a * b; } int main(){ return 0; }");
        assert!(asm.contains("mul x9, x9, x10")); // 64 位乘
        // long 返回经 x0（窥孔后可能是 ldr x0 或 mov x0,xN）
        assert!(asm.contains("ldr x0, [sp") || asm.contains("mov x0, x"));
    }

    #[test]
    fn arm64_int_to_long_sign_extends() {
        let asm = asm_arm64("long f(int x){ long y = x; return y; } int main(){ return 0; }");
        assert!(asm.contains("sxtw x9, w9")); // int→long 符号扩展
    }

    #[test]
    fn arm64_long_constant_materialized_64bit() {
        // 5e9 超 32 位 → movk 到高半字（bit 32）
        let asm = asm_arm64("long f(){ return 5000000000; } int main(){ return 0; }");
        assert!(asm.contains("movk x9, ") && asm.contains("lsl #32"));
    }

    #[test]
    fn arm64_long_compare_uses_64bit() {
        let asm = asm_arm64("int f(long a, long b){ return a < b; } int main(){ return 0; }");
        assert!(asm.contains("cmp x9, x10")); // 64 位比较
    }

    fn gen(func_body: Vec<Instr>, frame_bytes: usize) -> String {
        generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: func_body,
                frame_bytes,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        })
    }

    #[test]
    fn codegen_const_return() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 42 }, Instr::Return { src: 0, is_float: false, width: 4, agg: None }],
            8,
        );
        assert!(asm.contains(".globl _main"));
        assert!(asm.contains("_main:"));
        assert!(asm.contains("movz w9, #42"));
        assert!(asm.contains("ret"));
    }

    #[test]
    fn codegen_add_uses_add_instr() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 8, value: 2 },
                Instr::Bin { dst: 16, op: BinOp::Add, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: false, width: 4, agg: None },
            ],
            24,
        );
        assert!(asm.contains("add w9, w9, w10"));
        assert!(asm.contains("sub sp, sp, #32"));
        assert!(asm.contains("add sp, sp, #32"));
    }

    #[test]
    fn codegen_mod_uses_msub() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 17 },
                Instr::Const { dst: 8, value: 5 },
                Instr::Bin { dst: 16, op: BinOp::Mod, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: false, width: 4, agg: None },
            ],
            24,
        );
        assert!(asm.contains("sdiv w11, w9, w10"));
        assert!(asm.contains("msub w9, w11, w10, w9"));
    }

    #[test]
    fn codegen_compare_uses_cset() {
        let asm = gen(
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 8, value: 2 },
                Instr::Bin { dst: 16, op: BinOp::Lt, lhs: 0, rhs: 8 },
                Instr::Return { src: 16, is_float: false, width: 4, agg: None },
            ],
            24,
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
                Instr::Const { dst: 8, value: 7 },
                Instr::Return { src: 8, is_float: false, width: 4, agg: None },
            ],
            16,
        );
        assert!(asm.contains("Lmain_0:"));
        assert!(asm.contains("Lmain_1:"));
        assert!(asm.contains("b Lmain_0"));
        assert!(asm.contains("cbz w9, Lmain_1"));
    }

    #[test]
    fn codegen_prologue_saves_fp_lr() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 1 }, Instr::Return { src: 0, is_float: false, width: 4, agg: None }],
            8,
        );
        assert!(asm.contains("stp x29, x30, [sp, #-16]!"));
        assert!(asm.contains("ldp x29, x30, [sp], #16"));
    }

    #[test]
    fn codegen_call_and_params() {
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
        assert!(asm.contains("str w0, [sp, #0]")); // 第 0 个形参从 x0 落到槽位
        assert!(asm.contains("ldr x0, [sp, #0]")); // 调用 puts 时把实参装入 x0
        assert!(asm.contains("bl _puts"));
    }

    #[test]
    fn codegen_strlit_section() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::StrLit { dst: 0, index: 0 },
                    Instr::Return { src: 0, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 8,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec!["Hi".to_string()],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("adrp x9, L_.str.0@PAGE"));
        assert!(asm.contains("add x9, x9, L_.str.0@PAGEOFF"));
        assert!(asm.contains("__cstring"));
        assert!(asm.contains("L_.str.0:"));
        assert!(asm.contains(".byte 72, 105, 0"));
    }

    #[test]
    fn codegen_labels_prefixed_by_func() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::Label(0),
                    Instr::Jump(0),
                    Instr::Const { dst: 0, value: 0 },
                    Instr::Return { src: 0, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 8,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("Lmain_0:"));
        assert!(asm.contains("b Lmain_0"));
    }

    #[test]
    fn codegen_addr_of() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![Instr::AddrOf { dst: 8, off: 0 }, Instr::Return { src: 8, is_float: false, width: 4, agg: None }],
                frame_bytes: 16,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
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
                    Instr::Return { src: 8, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 24,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("ldr w10, [x9]"));
        assert!(asm.contains("ldrsb w10, [x9]"));
        assert!(asm.contains("str x10, [x9]"));
    }

    #[test]
    fn codegen_field_addr() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::AddrOf { dst: 0, off: 0 },
                    Instr::FieldAddr { dst: 8, base: 0, offset: 8 },
                    Instr::Return { src: 8, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 16,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("add x9, x9, #8"));
    }

    #[test]
    fn codegen_ptradd_scales() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![
                    Instr::PtrAdd { dst: 16, base: 0, index: 8, size: 4 },
                    Instr::Return { src: 16, is_float: false, width: 4, agg: None },
                ],
                frame_bytes: 24,
                params: vec![], ret_float: false, ret_agg: None, sret_slot: None, variadic: false,            }],
            strings: vec![],
            globals: vec![],
            floats: vec![],
        });
        assert!(asm.contains("madd x9, x10, x11, x9"));
        assert!(asm.contains("mov x11, #4"));
    }
}
