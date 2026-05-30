use crate::ir::{Function, Instr, Program, Value};
use std::fmt::Write;

pub fn generate(program: &Program) -> String {
    let mut out = String::new();
    out.push_str(".section __TEXT,__text,regular,pure_instructions\n");
    for func in &program.functions {
        gen_func(func, &mut out);
    }
    out
}

fn gen_func(func: &Function, out: &mut String) {
    let _ = writeln!(out, ".globl _{}", func.name);
    out.push_str(".p2align 2\n");
    let _ = writeln!(out, "_{}:", func.name);
    for instr in &func.body {
        match instr {
            Instr::Return(value) => {
                match value {
                    Value::Const(v) => {
                        let _ = writeln!(out, "    mov w0, #{}", v);
                    }
                }
                out.push_str("    ret\n");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{Function, Instr, Program, Value};

    #[test]
    fn codegen_return_42() {
        let prog = Program {
            functions: vec![Function {
                name: "main".to_string(),
                body: vec![Instr::Return(Value::Const(42))],
            }],
        };
        let asm = generate(&prog);
        assert!(asm.contains(".globl _main"));
        assert!(asm.contains("_main:"));
        assert!(asm.contains("mov w0, #42"));
        assert!(asm.contains("ret"));
    }
}
