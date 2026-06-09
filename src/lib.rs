// 各阶段模块将在后续 Task 中加入。
pub mod ast;
pub mod codegen;
pub mod codegen_x86;
pub mod error;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod preprocess;
pub mod span;
pub mod token;
pub mod types;

use error::CompileError;

/// 目标后端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// AArch64 / Mach-O（Apple Silicon）。
    Arm64,
    /// x86-64 / ELF（System V，Linux）。
    X86_64,
}

/// 编译 anvil 这个二进制时所在的宿主架构，决定默认目标。
pub fn host_target() -> Target {
    if cfg!(target_arch = "x86_64") {
        Target::X86_64
    } else {
        Target::Arm64
    }
}

/// 编译为宿主默认目标的汇编。
pub fn compile_to_asm(src: &str) -> Result<String, CompileError> {
    compile_to_asm_target(src, host_target())
}

/// 编译为指定目标的汇编。
pub fn compile_to_asm_target(src: &str, target: Target) -> Result<String, CompileError> {
    let tokens = lexer::lex(src)?;
    let ast = parser::parse(&tokens)?;
    let ir = ir::lower(&ast);
    Ok(match target {
        Target::Arm64 => codegen::generate(&ir),
        Target::X86_64 => codegen_x86::generate(&ir),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_arm64_end_to_end_text() {
        let asm = compile_to_asm_target("int main(){ return 42; }", Target::Arm64).unwrap();
        assert!(asm.contains("_main:"));
        assert!(asm.contains("movz w9, #42"));
        assert!(asm.contains("ret"));
    }

    #[test]
    fn compile_x86_end_to_end_text() {
        let asm = compile_to_asm_target("int main(){ return 42; }", Target::X86_64).unwrap();
        assert!(asm.contains("main:"));
        assert!(asm.contains("movl $42,"));
        assert!(asm.contains("ret"));
    }
}
