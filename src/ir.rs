use crate::ast::{Expr, FuncDef, Program as AstProgram, Stmt};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Instr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    Return(Value),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Const(i64),
}

pub fn lower(ast: &AstProgram) -> Program {
    Program {
        functions: ast.functions.iter().map(lower_func).collect(),
    }
}

fn lower_func(f: &FuncDef) -> Function {
    let mut body = Vec::new();
    for stmt in &f.body {
        match stmt {
            Stmt::Return(expr) => body.push(Instr::Return(lower_expr(expr))),
        }
    }
    Function {
        name: f.name.clone(),
        body,
    }
}

fn lower_expr(expr: &Expr) -> Value {
    match expr {
        Expr::IntLit(v) => Value::Const(*v),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    #[test]
    fn lower_return_42() {
        let ast = parse(&lex("int main(){ return 42; }").unwrap()).unwrap();
        let ir = lower(&ast);
        assert_eq!(ir.functions.len(), 1);
        assert_eq!(ir.functions[0].name, "main");
        assert_eq!(ir.functions[0].body, vec![Instr::Return(Value::Const(42))]);
    }
}
