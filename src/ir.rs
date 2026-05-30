use crate::ast::{BinaryOp, Expr, FuncDef, Program as AstProgram, Stmt, UnaryOp};
use crate::types::{Aggregates, Signatures, Type};
use std::collections::HashMap;

/// 帧内字节偏移（既是临时量，也是变量的存放位置）。
pub type Temp = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    pub body: Vec<Instr>,
    pub frame_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    Const { dst: Temp, value: i64 },
    Bin { dst: Temp, op: BinOp, lhs: Temp, rhs: Temp },
    Neg { dst: Temp, src: Temp },
    Label(usize),
    Jump(usize),
    JumpIfZero { cond: Temp, target: usize },
    Call {
        dst: Temp,
        name: String,
        args: Vec<Temp>,
        ret_width: usize,
        fixed: usize,
        variadic: bool,
    },
    StrLit { dst: Temp, index: usize },
    LoadArg { dst: Temp, index: usize, width: usize },
    AddrOf { dst: Temp, off: usize },
    FieldAddr { dst: Temp, base: Temp, offset: usize },
    LoadInd { dst: Temp, addr: Temp, width: usize, signed: bool },
    StoreInd { addr: Temp, src: Temp, width: usize },
    PtrAdd { dst: Temp, base: Temp, index: Temp, shift: u32 },
    PtrSub { dst: Temp, base: Temp, index: Temp, shift: u32 },
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
    let mut strings = Vec::new();
    let functions = ast
        .functions
        .iter()
        .map(|f| lower_func(f, &mut strings, &ast.aggregates, &ast.signatures))
        .collect();
    Program { functions, strings }
}

struct Lowerer<'a> {
    body: Vec<Instr>,
    next_offset: usize,
    scopes: Vec<HashMap<String, (usize, Type)>>,
    next_label: usize,
    strings: &'a mut Vec<String>,
    aggregates: &'a Aggregates,
    signatures: &'a Signatures,
}

impl<'a> Lowerer<'a> {
    /// 分配一个 8 字节临时量，返回其偏移。
    fn fresh(&mut self) -> Temp {
        let off = self.next_offset;
        self.next_offset += 8;
        off
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

    /// 在当前作用域声明变量，按 align8(size) 分配，返回其偏移。
    fn declare_var(&mut self, name: &str, ty: Type) -> usize {
        let aligned = self.size_of(&ty).div_ceil(8) * 8;
        let off = self.next_offset;
        self.next_offset += aligned;
        self.scopes
            .last_mut()
            .unwrap()
            .insert(name.to_string(), (off, ty));
        off
    }

    fn lookup_var(&self, name: &str) -> Option<(usize, Type)> {
        for scope in self.scopes.iter().rev() {
            if let Some(v) = scope.get(name) {
                return Some(v.clone());
            }
        }
        None
    }

    fn size_of(&self, ty: &Type) -> usize {
        crate::types::size_of(ty, self.aggregates)
    }

    fn field_info(&self, struct_ty: &Type, field: &str) -> (usize, Type) {
        self.field_info_opt(struct_ty, field)
            .expect("unknown struct or field")
    }

    fn field_info_opt(&self, struct_ty: &Type, field: &str) -> Option<(usize, Type)> {
        let name = match struct_ty {
            Type::Struct(n) | Type::Union(n) => n,
            _ => return None,
        };
        let agg = self.aggregates.get(name)?;
        let f = agg.fields.iter().find(|f| f.name == field)?;
        Some((f.offset, f.ty.clone()))
    }

    fn lower_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Return(e) => {
                let (src, _) = self.lower_expr(e);
                self.body.push(Instr::Return { src });
            }
            Stmt::Declare { name, ty, init } => {
                let off = self.declare_var(name, ty.clone());
                if let Some(e) = init {
                    let (v, _) = self.lower_expr(e);
                    let addr = self.fresh();
                    self.body.push(Instr::AddrOf { dst: addr, off });
                    self.body.push(Instr::StoreInd {
                        addr,
                        src: v,
                        width: ty.size(),
                    });
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
                let (c, _) = self.lower_expr(cond);
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
                let (c, _) = self.lower_expr(cond);
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
                self.push_scope();
                if let Some(init_s) = init {
                    self.lower_stmt(init_s);
                }
                let start = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                if let Some(c) = cond {
                    let (cv, _) = self.lower_expr(c);
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

    /// 返回 (存放右值结果的临时量, 类型)。
    fn lower_expr(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                (dst, Type::Int)
            }
            Expr::StrLit(s) => {
                let index = self.strings.len();
                self.strings.push(s.clone());
                let dst = self.fresh();
                self.body.push(Instr::StrLit { dst, index });
                (dst, Type::Pointer(Box::new(Type::Char)))
            }
            Expr::Var(name) => {
                let (off, ty) = self.lookup_var(name).expect("undeclared variable");
                match ty {
                    Type::Array(elem, _) => {
                        // 数组退化为首元素地址
                        let dst = self.fresh();
                        self.body.push(Instr::AddrOf { dst, off });
                        (dst, Type::Pointer(elem))
                    }
                    Type::Struct(_) | Type::Union(_) => {
                        // 聚合体不作整体读出，产出其地址（仅作成员访问/取址的基）
                        let dst = self.fresh();
                        self.body.push(Instr::AddrOf { dst, off });
                        (dst, ty)
                    }
                    scalar => {
                        let addr = self.fresh();
                        self.body.push(Instr::AddrOf { dst: addr, off });
                        let dst = self.fresh();
                        let width = self.size_of(&scalar);
                        self.body.push(Instr::LoadInd {
                            dst,
                            addr,
                            width,
                            signed: matches!(scalar, Type::Char),
                        });
                        (dst, scalar)
                    }
                }
            }
            Expr::Addr(inner) => {
                let (addr, ty) = self.lower_lvalue(inner);
                (addr, Type::Pointer(Box::new(ty)))
            }
            Expr::Deref(inner) => {
                let (ptr, ty) = self.lower_expr(inner);
                let pointee = ty.decay().pointee().expect("deref of non-pointer").clone();
                let dst = self.fresh();
                let width = self.size_of(&pointee);
                self.body.push(Instr::LoadInd {
                    dst,
                    addr: ptr,
                    width,
                    signed: matches!(pointee, Type::Char),
                });
                (dst, pointee)
            }
            Expr::Index { base, index } => {
                let (ptr, pointee) = self.lower_index_addr(base, index);
                let dst = self.fresh();
                let width = self.size_of(&pointee);
                self.body.push(Instr::LoadInd {
                    dst,
                    addr: ptr,
                    width,
                    signed: matches!(pointee, Type::Char),
                });
                (dst, pointee)
            }
            Expr::Member { .. } => {
                let (addr, ty) = self.lower_lvalue(e);
                match ty {
                    Type::Struct(_) | Type::Union(_) | Type::Array(..) => (addr, ty),
                    scalar => {
                        let dst = self.fresh();
                        let width = self.size_of(&scalar);
                        self.body.push(Instr::LoadInd {
                            dst,
                            addr,
                            width,
                            signed: matches!(scalar, Type::Char),
                        });
                        (dst, scalar)
                    }
                }
            }
            Expr::SizeofType(ty) => {
                let value = self.size_of(ty) as i64;
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value });
                (dst, Type::Int)
            }
            Expr::SizeofExpr(inner) => {
                let ty = self.type_of(inner);
                let value = self.size_of(&ty) as i64;
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value });
                (dst, Type::Int)
            }
            Expr::Unary { op, operand } => {
                let (src, _ty) = self.lower_expr(operand);
                match op {
                    UnaryOp::Plus => (src, Type::Int),
                    UnaryOp::Neg => {
                        let dst = self.fresh();
                        self.body.push(Instr::Neg { dst, src });
                        (dst, Type::Int)
                    }
                }
            }
            Expr::Binary { op, lhs, rhs } => self.lower_binary(*op, lhs, rhs),
            Expr::Assign { target, value } => {
                let (v, vty) = self.lower_expr(value);
                let (addr, ty) = self.lower_lvalue(target);
                let width = self.size_of(&ty);
                self.body.push(Instr::StoreInd {
                    addr,
                    src: v,
                    width,
                });
                (v, vty)
            }
            Expr::Call { name, args } => {
                let arg_temps: Vec<Temp> = args.iter().map(|a| self.lower_expr(a).0).collect();
                let dst = self.fresh();
                let sig = self.signatures.get(name);
                let ret = sig.map(|s| s.ret.clone()).unwrap_or(Type::Int);
                let ret_width = if matches!(ret, Type::Pointer(_)) { 8 } else { 4 };
                let fixed = sig.map(|s| s.fixed).unwrap_or(arg_temps.len());
                let variadic = sig.map(|s| s.variadic).unwrap_or(false);
                self.body.push(Instr::Call {
                    dst,
                    name: name.clone(),
                    args: arg_temps,
                    ret_width,
                    fixed,
                    variadic,
                });
                (dst, ret)
            }
        }
    }

    /// 返回 (左值地址临时量, 被指类型)。
    fn lower_lvalue(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::Var(name) => {
                let (off, ty) = self.lookup_var(name).expect("undeclared variable");
                let dst = self.fresh();
                self.body.push(Instr::AddrOf { dst, off });
                (dst, ty)
            }
            Expr::Deref(inner) => {
                let (ptr, ty) = self.lower_expr(inner);
                let pointee = ty.decay().pointee().expect("deref of non-pointer").clone();
                (ptr, pointee)
            }
            Expr::Index { base, index } => self.lower_index_addr(base, index),
            Expr::Member { base, field, arrow } => {
                let (base_addr, struct_ty) = if *arrow {
                    let (ptr, pty) = self.lower_expr(base);
                    (ptr, pty.decay().pointee().expect("-> on non-pointer").clone())
                } else {
                    self.lower_lvalue(base)
                };
                let (offset, fty) = self.field_info(&struct_ty, field);
                let dst = self.fresh();
                self.body.push(Instr::FieldAddr {
                    dst,
                    base: base_addr,
                    offset,
                });
                (dst, fty)
            }
            other => panic!("not an lvalue: {:?}", other),
        }
    }

    /// a[i] 的地址：base 退化为指针，addr = base + i*sizeof(elem)。返回 (地址临时量, 元素类型)。
    fn lower_index_addr(&mut self, base: &Expr, index: &Expr) -> (Temp, Type) {
        let (ptr, pty) = self.lower_expr(base);
        let elem = pty.decay().pointee().expect("index of non-pointer").clone();
        let (idx, _) = self.lower_expr(index);
        let dst = self.fresh();
        let shift = shift_of(self.size_of(&elem));
        self.body.push(Instr::PtrAdd {
            dst,
            base: ptr,
            index: idx,
            shift,
        });
        (dst, elem)
    }

    fn lower_binary(&mut self, op: BinaryOp, lhs: &Expr, rhs: &Expr) -> (Temp, Type) {
        let (l, lty) = self.lower_expr(lhs);
        let (r, rty) = self.lower_expr(rhs);
        let l_ptr = lty.is_pointer_like();
        let r_ptr = rty.is_pointer_like();
        if (op == BinaryOp::Add || op == BinaryOp::Sub) && (l_ptr ^ r_ptr) {
            let (ptr, pty, idx) = if l_ptr {
                (l, lty.clone(), r)
            } else {
                (r, rty.clone(), l)
            };
            let elem = pty.decay().pointee().unwrap().clone();
            let dst = self.fresh();
            let shift = shift_of(self.size_of(&elem));
            if op == BinaryOp::Add {
                self.body.push(Instr::PtrAdd {
                    dst,
                    base: ptr,
                    index: idx,
                    shift,
                });
            } else {
                self.body.push(Instr::PtrSub {
                    dst,
                    base: ptr,
                    index: idx,
                    shift,
                });
            }
            return (dst, pty.decay());
        }
        let dst = self.fresh();
        self.body.push(Instr::Bin {
            dst,
            op: lower_binop(op),
            lhs: l,
            rhs: r,
        });
        (dst, Type::Int)
    }

    /// 仅推断类型（用于 sizeof(expr)，不求值操作数）。
    fn type_of(&self, e: &Expr) -> Type {
        match e {
            Expr::IntLit(_)
            | Expr::Unary { .. }
            | Expr::Binary { .. }
            | Expr::Call { .. }
            | Expr::SizeofType(_)
            | Expr::SizeofExpr(_) => Type::Int,
            Expr::StrLit(_) => Type::Pointer(Box::new(Type::Char)),
            Expr::Var(name) => self
                .lookup_var(name)
                .map(|(_, t)| t.decay())
                .unwrap_or(Type::Int),
            Expr::Addr(inner) => Type::Pointer(Box::new(self.type_of_lvalue(inner))),
            Expr::Deref(inner) => self
                .type_of(inner)
                .decay()
                .pointee()
                .cloned()
                .unwrap_or(Type::Int),
            Expr::Index { base, .. } => self
                .type_of(base)
                .decay()
                .pointee()
                .cloned()
                .unwrap_or(Type::Int),
            Expr::Member { base, field, arrow } => {
                let sty = if *arrow {
                    self.type_of(base).decay().pointee().cloned().unwrap_or(Type::Int)
                } else {
                    self.type_of_lvalue(base)
                };
                self.field_info_opt(&sty, field)
                    .map(|(_, t)| t)
                    .unwrap_or(Type::Int)
            }
            Expr::Assign { value, .. } => self.type_of(value),
        }
    }

    fn type_of_lvalue(&self, e: &Expr) -> Type {
        match e {
            Expr::Var(name) => self.lookup_var(name).map(|(_, t)| t).unwrap_or(Type::Int),
            Expr::Member { base, field, arrow } => {
                let sty = if *arrow {
                    self.type_of(base).decay().pointee().cloned().unwrap_or(Type::Int)
                } else {
                    self.type_of_lvalue(base)
                };
                self.field_info_opt(&sty, field)
                    .map(|(_, t)| t)
                    .unwrap_or(Type::Int)
            }
            Expr::Deref(inner) => self
                .type_of(inner)
                .decay()
                .pointee()
                .cloned()
                .unwrap_or(Type::Int),
            Expr::Index { base, .. } => self
                .type_of(base)
                .decay()
                .pointee()
                .cloned()
                .unwrap_or(Type::Int),
            _ => Type::Int,
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

/// 元素大小（2 的幂）→ 移位量；其它退化为 0（字节寻址）。
fn shift_of(size: usize) -> u32 {
    match size {
        2 => 1,
        4 => 2,
        8 => 3,
        _ => 0,
    }
}

fn lower_func(
    f: &FuncDef,
    strings: &mut Vec<String>,
    aggregates: &Aggregates,
    signatures: &Signatures,
) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_offset: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
        strings,
        aggregates,
        signatures,
    };
    // 参数占据前若干槽，从入参寄存器直接落到各自槽位。
    for (index, (pname, pty)) in f.params.iter().enumerate() {
        let width = lw.size_of(pty);
        let off = lw.declare_var(pname, pty.clone());
        lw.body.push(Instr::LoadArg {
            dst: off,
            index,
            width,
        });
    }
    for stmt in &f.body {
        lw.lower_stmt(stmt);
    }
    Function {
        name: f.name.clone(),
        body: lw.body,
        frame_bytes: lw.next_offset,
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

    fn lower_prog(src: &str) -> Program {
        lower(&parse(&lex(src).unwrap()).unwrap())
    }

    #[test]
    fn lower_const_return() {
        let f = lower_src("int main(){ return 42; }");
        assert_eq!(f.name, "main");
        assert_eq!(f.frame_bytes, 8);
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
        let f = lower_src("int main(){ return 1+2; }");
        assert_eq!(f.frame_bytes, 24);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 1 },
                Instr::Const { dst: 8, value: 2 },
                Instr::Bin { dst: 16, op: BinOp::Add, lhs: 0, rhs: 8 },
                Instr::Return { src: 16 },
            ]
        );
    }

    #[test]
    fn lower_unary_plus_is_noop() {
        let f = lower_src("int main(){ return +7; }");
        assert_eq!(f.frame_bytes, 8);
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
        let f = lower_src("int main(){ return -7; }");
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Neg { dst: 8, src: 0 },
                Instr::Return { src: 8 },
            ]
        );
    }

    #[test]
    fn lower_declare_uses_addr_and_storeind() {
        let f = lower_src("int main(){ int x = 5; return x; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::AddrOf { .. })));
        assert!(f.body.iter().any(|i| matches!(i, Instr::StoreInd { .. })));
        assert!(f.body.iter().any(|i| matches!(i, Instr::LoadInd { .. })));
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
        assert!(labels >= 1);
        assert!(branches >= 1);
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
        assert!(jumps >= 1 && cond_jumps >= 1);
    }

    #[test]
    fn lower_call_and_string() {
        let p = lower_prog("int main(){ puts(\"hi\"); return 0; }");
        assert_eq!(p.strings, vec!["hi".to_string()]);
        let f = &p.functions[0];
        let has_strlit = f.body.iter().any(|i| matches!(i, Instr::StrLit { index: 0, .. }));
        let has_call = f
            .body
            .iter()
            .any(|i| matches!(i, Instr::Call { name, .. } if name == "puts"));
        assert!(has_strlit && has_call);
    }

    #[test]
    fn lower_params_emit_loadarg() {
        let p = lower_prog("int add(int a, int b){ return a+b; } int main(){ return add(1,2); }");
        let add = p.functions.iter().find(|f| f.name == "add").unwrap();
        let loadargs = add
            .body
            .iter()
            .filter(|i| matches!(i, Instr::LoadArg { .. }))
            .count();
        assert_eq!(loadargs, 2);
    }

    #[test]
    fn lower_addr_of_var() {
        let f = lower_src("int main(){ int x; int* p; p = &x; return 0; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::AddrOf { .. })));
    }

    #[test]
    fn lower_deref_loadind() {
        let f = lower_src("int main(){ int* p; return *p; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::LoadInd { .. })));
    }

    #[test]
    fn lower_index_uses_ptradd() {
        let f = lower_src("int main(){ int a[4]; return a[2]; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::PtrAdd { shift: 2, .. })));
    }

    #[test]
    fn lower_struct_member_uses_fieldaddr() {
        let f = lower_src(
            "struct P { int x; int y; }; int main(){ struct P p; p.y = 7; return p.y; }",
        );
        assert!(f.body.iter().any(|i| matches!(i, Instr::FieldAddr { offset: 8, .. })));
    }

    #[test]
    fn lower_arrow_member() {
        let f = lower_src("struct P { int x; }; int main(){ struct P* p; return p->x; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::FieldAddr { offset: 0, .. })));
    }
}
