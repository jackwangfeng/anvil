use crate::ir::{BinOp, Function, Instr, Program};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".section __TEXT,__text,regular,pure_instructions\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    gen_strings(&program.strings, &mut out);
    gen_globals(&program.globals, &mut out);
    out
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
    for instr in &func.body {
        gen_instr(instr, &func.name, frame, out);
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
        Instr::LoadArg { dst, index, width } => {
            if *index < 8 {
                match *width {
                    8 => {
                        let _ = writeln!(out, "    str x{}, [sp, #{}]", index, slot(*dst));
                    }
                    1 => {
                        let _ = writeln!(out, "    strb w{}, [sp, #{}]", index, slot(*dst));
                    }
                    _ => {
                        let _ = writeln!(out, "    str w{}, [sp, #{}]", index, slot(*dst));
                    }
                }
            } else {
                // 第 9+ 个参数：调用方放在栈上，位于 [x29 + 16 + (index-8)*8]
                let off = 16 + (index - 8) * 8;
                let _ = writeln!(out, "    ldr x9, [x29, #{}]", off);
                if *width == 8 {
                    let _ = writeln!(out, "    str x9, [sp, #{}]", slot(*dst));
                } else {
                    let _ = writeln!(out, "    str w9, [sp, #{}]", slot(*dst));
                }
            }
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
        Instr::Call { dst, name, args, ret_width, fixed, variadic } => {
            // 寄存器可容纳的参数数：可变参数函数下，仅固定参数进寄存器（≤8）；否则前 8 个进寄存器。
            let nreg = if *variadic { (*fixed).min(8) } else { args.len().min(8) };
            let stack_args = &args[nreg..];
            let space = (stack_args.len() * 8).div_ceil(16) * 16;
            if space > 0 {
                let _ = writeln!(out, "    sub sp, sp, #{}", space);
            }
            // 栈传参（含可变参数与第 9+ 个参数），每个 8 字节，从 sp 起步
            for (k, a) in stack_args.iter().enumerate() {
                let _ = writeln!(out, "    ldr x9, [sp, #{}]", space + slot(*a));
                let _ = writeln!(out, "    str x9, [sp, #{}]", k * 8);
            }
            // 寄存器参数 x0..x{nreg-1}
            for (i, a) in args.iter().take(nreg).enumerate() {
                let _ = writeln!(out, "    ldr x{}, [sp, #{}]", i, space + slot(*a));
            }
            let _ = writeln!(out, "    bl _{}", name);
            if space > 0 {
                let _ = writeln!(out, "    add sp, sp, #{}", space);
            }
            if *ret_width == 8 {
                let _ = writeln!(out, "    str x0, [sp, #{}]", slot(*dst));
            } else {
                let _ = writeln!(out, "    str w0, [sp, #{}]", slot(*dst));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{BinOp, Function, Instr, Program};

    fn gen(func_body: Vec<Instr>, frame_bytes: usize) -> String {
        generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: func_body,
                frame_bytes,
            }],
            strings: vec![],
            globals: vec![],
        })
    }

    #[test]
    fn codegen_const_return() {
        let asm = gen(
            vec![Instr::Const { dst: 0, value: 42 }, Instr::Return { src: 0 }],
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
                Instr::Return { src: 16 },
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
                Instr::Return { src: 16 },
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
                Instr::Return { src: 16 },
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
                Instr::Return { src: 8 },
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
            vec![Instr::Const { dst: 0, value: 1 }, Instr::Return { src: 0 }],
            8,
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
                    Instr::LoadArg { dst: 0, index: 0, width: 4 },
                    Instr::Call {
                        dst: 8,
                        name: "puts".to_string(),
                        args: vec![0],
                        ret_width: 4,
                        fixed: 1,
                        variadic: false,
                    },
                    Instr::Return { src: 8 },
                ],
                frame_bytes: 16,
            }],
            strings: vec![],
            globals: vec![],
        });
        assert!(asm.contains("str w0, [sp, #0]"));
        assert!(asm.contains("ldr x0, [sp, #0]"));
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
                frame_bytes: 8,
            }],
            strings: vec!["Hi".to_string()],
            globals: vec![],
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
                    Instr::Return { src: 0 },
                ],
                frame_bytes: 8,
            }],
            strings: vec![],
            globals: vec![],
        });
        assert!(asm.contains("Lmain_0:"));
        assert!(asm.contains("b Lmain_0"));
    }

    #[test]
    fn codegen_addr_of() {
        let asm = generate(&Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![Instr::AddrOf { dst: 8, off: 0 }, Instr::Return { src: 8 }],
                frame_bytes: 16,
            }],
            strings: vec![],
            globals: vec![],
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
            globals: vec![],
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
                    Instr::Return { src: 8 },
                ],
                frame_bytes: 16,
            }],
            strings: vec![],
            globals: vec![],
        });
        assert!(asm.contains("add x9, x9, #8"));
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
            globals: vec![],
        });
        assert!(asm.contains("add x9, x9, w10, sxtw #2"));
    }
}
