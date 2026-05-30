use crate::ast::{BinaryOp, Expr, FuncDef, Program as AstProgram, Stmt, UnaryOp};

/// 临时量编号（从 0 递增）。codegen 据此分配栈槽。
pub type Temp = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Instr>,
    pub num_temps: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    Const { dst: Temp, value: i64 },
    Bin { dst: Temp, op: BinOp, lhs: Temp, rhs: Temp },
    Neg { dst: Temp, src: Temp },
    Return { src: Temp },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
}

pub fn lower(ast: &AstProgram) -> Program {
    Program {
        functions: ast.functions.iter().map(lower_func).collect(),
    }
}

struct Lowerer {
    body: Vec<Instr>,
    next_temp: usize,
}

impl Lowerer {
    fn fresh(&mut self) -> Temp {
        let t = self.next_temp;
        self.next_temp += 1;
        t
    }

    /// 把表达式降到一串指令，返回存放其结果的临时量。
    fn lower_expr(&mut self, e: &Expr) -> Temp {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                dst
            }
            Expr::Unary { op, operand } => {
                let src = self.lower_expr(operand);
                match op {
                    UnaryOp::Plus => src, // 恒等：复用操作数临时量
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.body.push(Instr::Neg { dst, src });
                        dst
                    }
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                let a = self.lower_expr(lhs);
                let b = self.lower_expr(rhs);
                let dst = self.fresh();
                self.body.push(Instr::Bin {
                    dst,
                    op: lower_binop(*op),
                    lhs: a,
                    rhs: b,
                });
                dst
            }
        }
    }
}

fn lower_binop(op: BinaryOp) -> BinOp {
    match op {
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Mod,
    }
}

fn lower_func(f: &FuncDef) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_temp: 0,
    };
    for stmt in &f.body {
        match stmt {
            Stmt::Return(expr) => {
                let src = lw.lower_expr(expr);
                lw.body.push(Instr::Return { src });
            }
        }
    }
    Function {
        name: f.name.clone(),
        body: lw.body,
        num_temps: lw.next_temp,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lexer::lex;
    use crate::parser::parse;

    fn lower_src(src: &str) -> Function {
        let ast = parse(&lex(src).unwrap()).unwrap();
        let ir = lower(&ast);
        ir.functions.into_iter().next().unwrap()
    }

    #[test]
    fn lower_const_return() {
        // return 42  =>  t0 = const 42 ; return t0
        let f = lower_src("int main(){ return 42; }");
        assert_eq!(f.name, "main");
        assert_eq!(f.num_temps, 1);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 42 },
                Instr::Return { src: 0 },
            ]
        );
    }

    #[test]
    fn lower_add() {
        // return 1+2  =>  t0=1; t1=2; t2=t0+t1; return t2
        let f = lower_src("int main(){ return 1+2; }");
        assert_eq!(f.num_temps, 3);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 1, value: 2 },
                Instr::Bin { dst: 2, op: BinOp::Add, lhs: 0, rhs: 1 },
                Instr::Return { src: 2 },
            ]
        );
    }

    #[test]
    fn lower_unary_plus_is_noop() {
        // return +7  =>  t0=7; return t0  (一元 + 不产生指令)
        let f = lower_src("int main(){ return +7; }");
        assert_eq!(f.num_temps, 1);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Return { src: 0 },
            ]
        );
    }

    #[test]
    fn lower_unary_neg() {
        // return -7  =>  t0=7; t1=neg t0; return t1
        let f = lower_src("int main(){ return -7; }");
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Neg { dst: 1, src: 0 },
                Instr::Return { src: 1 },
            ]
        );
    }
}
