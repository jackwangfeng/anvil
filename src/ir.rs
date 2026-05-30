use crate::ast::{BinaryOp, Expr, FuncDef, Program as AstProgram, Stmt, UnaryOp};
use std::collections::HashMap;

/// 槽位编号（从 0 递增）。codegen 据此分配栈槽；既可能是具名变量槽，也可能是匿名临时量。
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
    Load { dst: Temp, var: Temp },
    Store { var: Temp, src: Temp },
    Copy { dst: Temp, src: Temp },
    Label(usize),
    Jump(usize),
    JumpIfZero { cond: Temp, target: usize },
    Return { src: Temp },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Lt,
    Gt,
    Le,
    Ge,
    Eq,
    Ne,
}

pub fn lower(ast: &AstProgram) -> Program {
    Program {
        functions: ast.functions.iter().map(lower_func).collect(),
    }
}

struct Lowerer {
    body: Vec<Instr>,
    next_temp: usize,
    scopes: Vec<HashMap<String, Temp>>,
    next_label: usize,
}

impl Lowerer {
    fn fresh(&mut self) -> Temp {
        let t = self.next_temp;
        self.next_temp += 1;
        t
    }

    fn new_label(&mut self) -> usize {
        let l = self.next_label;
        self.next_label += 1;
        l
    }

    fn push_scope(&mut self) {
        self.scopes.push(HashMap::new());
    }

    fn pop_scope(&mut self) {
        self.scopes.pop();
    }

    /// 在当前作用域声明变量，分配一个槽位。
    fn declare_var(&mut self, name: &str) -> Temp {
        let slot = self.fresh();
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), slot);
        slot
    }

    /// 由内向外查找变量槽位。
    fn lookup_var(&self, name: &str) -> Option<Temp> {
        for scope in self.scopes.iter().rev() {
            if let Some(&slot) = scope.get(name) {
                return Some(slot);
            }
        }
        None
    }

    fn lower_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Return(e) => {
                let src = self.lower_expr(e);
                self.body.push(Instr::Return { src });
            }
            Stmt::Declare { name, init } => {
                let slot = self.declare_var(name);
                if let Some(e) = init {
                    let v = self.lower_expr(e);
                    self.body.push(Instr::Store { var: slot, src: v });
                }
            }
            Stmt::ExprStmt(e) => {
                let _ = self.lower_expr(e);
            }
            Stmt::Empty => {}
            Stmt::Block(stmts) => {
                self.push_scope();
                for st in stmts {
                    self.lower_stmt(st);
                }
                self.pop_scope();
            }
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                let c = self.lower_expr(cond);
                let else_label = self.new_label();
                self.body.push(Instr::JumpIfZero {
                    cond: c,
                    target: else_label,
                });
                self.lower_stmt(then_branch);
                if let Some(else_s) = else_branch {
                    let end_label = self.new_label();
                    self.body.push(Instr::Jump(end_label));
                    self.body.push(Instr::Label(else_label));
                    self.lower_stmt(else_s);
                    self.body.push(Instr::Label(end_label));
                } else {
                    self.body.push(Instr::Label(else_label));
                }
            }
            Stmt::While { cond, body } => {
                let start = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                let c = self.lower_expr(cond);
                self.body.push(Instr::JumpIfZero {
                    cond: c,
                    target: end,
                });
                self.lower_stmt(body);
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
            }
            Stmt::For {
                init,
                cond,
                step,
                body,
            } => {
                self.push_scope(); // for 的 init 声明作用域限于循环
                if let Some(init_s) = init {
                    self.lower_stmt(init_s);
                }
                let start = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                if let Some(c) = cond {
                    let cv = self.lower_expr(c);
                    self.body.push(Instr::JumpIfZero {
                        cond: cv,
                        target: end,
                    });
                }
                self.lower_stmt(body);
                if let Some(st) = step {
                    let _ = self.lower_expr(st);
                }
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
                self.pop_scope();
            }
        }
    }

    /// 把表达式降到一串指令，返回存放其结果的临时量。
    fn lower_expr(&mut self, e: &Expr) -> Temp {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                dst
            }
            Expr::Var(name) => {
                let slot = self.lookup_var(name).expect("undeclared variable");
                let dst = self.fresh();
                self.body.push(Instr::Load { dst, var: slot });
                dst
            }
            Expr::Assign { name, value } => {
                let v = self.lower_expr(value);
                let slot = self.lookup_var(name).expect("undeclared variable");
                self.body.push(Instr::Store { var: slot, src: v });
                v // 赋值表达式求值为所赋的值
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
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::Ne => BinOp::Ne,
    }
}

fn lower_func(f: &FuncDef) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_temp: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
    };
    for stmt in &f.body {
        lw.lower_stmt(stmt);
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

    #[test]
    fn lower_declare_and_return_var() {
        // int x = 5; return x;  —— 变量 x 占槽 0
        let f = lower_src("int main(){ int x = 5; return x; }");
        let has_store_to_var0 = f.body.iter().any(|i| matches!(i, Instr::Store { var: 0, .. }));
        let has_load_var0 = f.body.iter().any(|i| matches!(i, Instr::Load { var: 0, .. }));
        assert!(has_store_to_var0, "expected a Store to var slot 0");
        assert!(has_load_var0, "expected a Load from var slot 0");
    }

    #[test]
    fn lower_if_emits_labels_and_branch() {
        let f = lower_src("int main(){ if (1) return 2; return 3; }");
        let labels = f.body.iter().filter(|i| matches!(i, Instr::Label(_))).count();
        let branches = f
            .body
            .iter()
            .filter(|i| matches!(i, Instr::JumpIfZero { .. }))
            .count();
        assert!(labels >= 1, "if should emit at least one label");
        assert!(branches >= 1, "if should emit a conditional branch");
    }

    #[test]
    fn lower_while_emits_loop() {
        let f = lower_src("int main(){ int x = 0; while (x < 3) x = x + 1; return x; }");
        let jumps = f.body.iter().filter(|i| matches!(i, Instr::Jump(_))).count();
        let cond_jumps = f
            .body
            .iter()
            .filter(|i| matches!(i, Instr::JumpIfZero { .. }))
            .count();
        assert!(
            jumps >= 1 && cond_jumps >= 1,
            "while should emit back-edge jump and exit branch"
        );
    }
}
