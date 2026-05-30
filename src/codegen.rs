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
    bytes.div_ceil(16) * 16
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
                    // w9 % w10 = w9 - (w9 / w10) * w10
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
        let asm = gen(
            vec![
                Instr::Const { dst: 1, value: 9 },
                Instr::Store { var: 0, src: 1 },
                Instr::Load { dst: 2, var: 0 },
                Instr::Return { src: 2 },
            ],
            3,
        );
        assert!(asm.contains("str w9, [sp, #0]"));
        assert!(asm.contains("ldr w9, [sp, #0]"));
    }
}
