use crate::types::{Aggregates, Signatures, Type};

#[derive(Debug, Clone, PartialEq)]
pub struct Program {
    pub functions: Vec<FuncDef>,
    pub aggregates: Aggregates,
    pub signatures: Signatures,
    pub globals: Vec<Global>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Global {
    pub name: String,
    pub ty: Type,
    pub init: Option<i64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FuncDef {
    pub name: String,
    pub params: Vec<(String, Type)>,
    pub ret: Type,
    pub body: Vec<Stmt>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    Return(Expr),
    /// <type> <name>;  或  <type> <name> = <init>;  或  <type> <name>[N];
    Declare {
        name: String,
        ty: Type,
        init: Option<Expr>,
    },
    ExprStmt(Expr),
    /// 单条多声明符（如 `int a, b;`）——在当前作用域顺序展开，不引入新作用域。
    Decls(Vec<Stmt>),
    Block(Vec<Stmt>),
    If {
        cond: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    While {
        cond: Expr,
        body: Box<Stmt>,
    },
    DoWhile {
        body: Box<Stmt>,
        cond: Expr,
    },
    For {
        init: Option<Box<Stmt>>,
        cond: Option<Expr>,
        step: Option<Expr>,
        body: Box<Stmt>,
    },
    Break,
    Continue,
    /// `goto label;`
    Goto(String),
    /// `label:`（标签,标识跳转目标）
    Label(String),
    Switch {
        cond: Expr,
        body: Vec<Stmt>,
    },
    Case(i64),
    Default,
    Empty,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    IntLit(i64),
    FloatLit(f64),
    Var(String),
    StrLit(String),
    Call {
        name: String,
        args: Vec<Expr>,
    },
    Assign {
        target: Box<Expr>,
        value: Box<Expr>,
    },
    Addr(Box<Expr>),
    Deref(Box<Expr>),
    Index {
        base: Box<Expr>,
        index: Box<Expr>,
    },
    Member {
        base: Box<Expr>,
        field: String,
        arrow: bool,
    },
    Logical {
        op: LogOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
    Ternary {
        cond: Box<Expr>,
        then_e: Box<Expr>,
        else_e: Box<Expr>,
    },
    SizeofType(Type),
    SizeofExpr(Box<Expr>),
    /// 强制类型转换 `(ty)expr`。
    Cast {
        ty: Type,
        expr: Box<Expr>,
    },
    /// 逗号运算符 `first, second`：求值 first（丢弃），结果为 second。
    Comma {
        first: Box<Expr>,
        second: Box<Expr>,
    },
    /// 聚合初始化列表 `{a, b, c}`（用于数组/结构体初始化，可嵌套）。
    InitList(Vec<Expr>),
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },
    Binary {
        op: BinaryOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,
    Plus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
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
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOp {
    And,
    Or,
}
