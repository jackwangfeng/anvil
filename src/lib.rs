// 各阶段模块将在后续 Task 中加入。
pub mod ast;
pub mod codegen;
pub mod error;
pub mod ir;
pub mod lexer;
pub mod parser;
pub mod span;
pub mod token;
pub mod types;

use error::CompileError;

pub fn compile_to_asm(src: &str) -> Result<String, CompileError> {
    let tokens = lexer::lex(src)?;
    let ast = parser::parse(&tokens)?;
    let ir = ir::lower(&ast);
    Ok(codegen::generate(&ir))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compile_to_asm_end_to_end_text() {
        let asm = compile_to_asm("int main(){ return 42; }").unwrap();
        assert!(asm.contains("_main:"));
        assert!(asm.contains("movz w9, #42"));
        assert!(asm.contains("ret"));
    }
}
