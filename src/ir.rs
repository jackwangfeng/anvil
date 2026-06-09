use crate::ast::{BinaryOp, Expr, FuncDef, LogOp, Program as AstProgram, Stmt, UnaryOp};
use crate::types::{Aggregates, Signatures, Type};
use std::collections::HashMap;

/// 帧内字节偏移（既是临时量，也是变量的存放位置）。
pub type Temp = usize;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Program {
    pub functions: Vec<Function>,
    pub strings: Vec<String>,
    pub globals: Vec<GlobalVar>,
    pub floats: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalVar {
    pub name: String,
    pub size: usize,
    /// 初始化字节镜像(小端);None 表示零初始化。
    pub init: Option<Vec<u8>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub name: String,
    /// 形参（按声明顺序）。调用约定由各后端据此在序言里把寄存器/栈实参落到 `slot`。
    pub params: Vec<Param>,
    pub body: Vec<Instr>,
    pub frame_bytes: usize,
    /// 返回类型是否为 double（→ 浮点返回寄存器）。
    pub ret_float: bool,
    /// 返回的聚合体（struct/union 按值返回）字节大小；标量/指针为 `None`。
    pub ret_agg: Option<usize>,
    /// 返回大结构体（>16 字节）时，序言保存隐式返回指针的帧内槽位。
    pub sret_slot: Option<usize>,
    /// 是否为可变参数函数（其可变参数由调用方压栈传入，va_list 线性遍历栈区）。
    pub variadic: bool,
}

/// 一个形参在帧内的落点描述（目标无关）。各后端按自身 ABI 决定它来自哪个寄存器或栈位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Param {
    /// 帧内字节偏移（变量存放位置）。
    pub slot: usize,
    /// 字节大小（结构体可 >8）。
    pub size: usize,
    /// 标量 double：走 FP 寄存器组。
    pub is_float: bool,
    /// struct/union 按值传递。
    pub is_aggregate: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Instr {
    Const { dst: Temp, value: i64 },
    Bin { dst: Temp, op: BinOp, lhs: Temp, rhs: Temp },
    Neg { dst: Temp, src: Temp },
    Label(usize),
    Jump(usize),
    JumpIfZero { cond: Temp, target: usize },
    /// 取函数地址（函数名作为值/取址），得到函数指针。
    FuncAddr { dst: Temp, name: String },
    /// va_start：把 va_list（地址在 `ap`）置为首个可变参数的地址。
    VaStart { ap: Temp },
    /// va_arg：从 va_list（地址在 `ap`）取 `width` 字节到 `dst`，并把指针前进 8。
    /// `is_float` 区分整型/浮点(LLVM va_arg 需要;原生栈式遍历不区分)。
    VaArg { dst: Temp, ap: Temp, width: usize, is_float: bool },
    Call {
        dst: Temp,
        name: String,
        /// 间接调用：通过该临时量中的函数指针调用（此时忽略 name）。直接调用为 None。
        via: Option<Temp>,
        args: Vec<Temp>,
        /// 每个实参是否为浮点（double）——决定走 GP 还是 FP 寄存器。
        arg_floats: Vec<bool>,
        /// 每个实参若为按值结构体则给出其字节大小（此时实参 temp 存放的是结构体地址）。
        arg_aggs: Vec<Option<usize>>,
        ret_width: usize,
        /// 按值返回结构体时的字节大小；标量/指针为 None。
        ret_agg: Option<usize>,
        /// 结构体返回值要写入的帧内缓冲区偏移（与 ret_agg 同时存在）。
        ret_buf: Option<usize>,
        fixed: usize,
        variadic: bool,
        /// 被调方是 anvil 自定义可变参数函数：可变实参（index ≥ fixed）一律压栈传递。
        stack_varargs: bool,
        ret_float: bool,
    },
    ConstF {
        dst: Temp,
        index: usize,
    },
    BinF {
        dst: Temp,
        op: BinOp,
        lhs: Temp,
        rhs: Temp,
    },
    IntToFloat {
        dst: Temp,
        src: Temp,
    },
    FloatToInt {
        dst: Temp,
        src: Temp,
    },
    /// 64 位有符号整数二元运算（long）。与 32 位 `Bin` 并列，类比 `BinF`。
    BinL {
        dst: Temp,
        op: BinOp,
        lhs: Temp,
        rhs: Temp,
    },
    /// 64 位取负。
    NegL {
        dst: Temp,
        src: Temp,
    },
    /// 符号扩展 32→64（有符号 int → long）。
    Widen {
        dst: Temp,
        src: Temp,
    },
    /// 零扩展 32→64（无符号 int → long）。
    WidenU {
        dst: Temp,
        src: Temp,
    },
    /// 64 位整数 → double。
    LongToFloat {
        dst: Temp,
        src: Temp,
    },
    /// double → 64 位整数。
    FloatToLong {
        dst: Temp,
        src: Temp,
    },
    StrLit { dst: Temp, index: usize },
    AddrOf { dst: Temp, off: usize },
    GlobalAddr { dst: Temp, name: String },
    FieldAddr { dst: Temp, base: Temp, offset: usize },
    Copy { dst: Temp, src: Temp, width: usize },
    /// 按值拷贝 `size` 字节：`dst`、`src` 为存放目标/源地址的临时量（结构体赋值/初始化）。
    MemCpy { dst: Temp, src: Temp, size: usize },
    LoadInd { dst: Temp, addr: Temp, width: usize, signed: bool },
    StoreInd { addr: Temp, src: Temp, width: usize },
    /// base + index * size（index 为 32 位有符号，按元素字节大小缩放）。
    PtrAdd { dst: Temp, base: Temp, index: Temp, size: usize },
    /// base - index * size。
    PtrSub { dst: Temp, base: Temp, index: Temp, size: usize },
    /// `agg` 为 Some(size) 时按值返回结构体，`src` 存放结构体地址。
    Return { src: Temp, is_float: bool, width: usize, agg: Option<usize> },
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
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    // 无符号变体(仅这些运算与有符号不同)
    UDiv,
    UMod,
    UShr, // 逻辑右移
    ULt,
    UGt,
    ULe,
    UGe,
}

pub fn lower(ast: &AstProgram) -> Program {
    let mut strings = Vec::new();
    let mut floats = Vec::new();
    let global_types: HashMap<String, Type> = ast
        .globals
        .iter()
        .map(|g| (g.name.clone(), g.ty.clone()))
        .collect();
    // 有函数体的（用户自定义）函数名——用于区分自定义可变参数 vs libc（printf）。
    let defined_funcs: std::collections::HashSet<String> =
        ast.functions.iter().map(|f| f.name.clone()).collect();
    let functions = ast
        .functions
        .iter()
        .map(|f| {
            lower_func(
                f,
                &mut strings,
                &mut floats,
                &ast.aggregates,
                &ast.signatures,
                &global_types,
                &defined_funcs,
            )
        })
        .collect();
    let globals = ast
        .globals
        .iter()
        .map(|g| {
            let size = crate::types::size_of(&g.ty, &ast.aggregates);
            GlobalVar {
                name: g.name.clone(),
                size,
                init: g
                    .init
                    .as_ref()
                    .map(|e| eval_init_bytes(&g.ty, e, &ast.aggregates, size)),
            }
        })
        .collect();
    Program {
        functions,
        strings,
        globals,
        floats,
    }
}

struct Lowerer<'a> {
    body: Vec<Instr>,
    next_offset: usize,
    scopes: Vec<HashMap<String, (usize, Type)>>,
    next_label: usize,
    strings: &'a mut Vec<String>,
    floats: &'a mut Vec<u64>,
    aggregates: &'a Aggregates,
    signatures: &'a Signatures,
    globals: &'a HashMap<String, Type>,
    ret_ty: Type,
    break_targets: Vec<usize>,
    continue_targets: Vec<usize>,
    /// 命名标签（goto/label）→ IR label 号；支持前向引用（先用后定义）。
    goto_labels: HashMap<String, usize>,
    /// 有函数体的函数名集合（区分自定义可变参数与 libc 变参）。
    defined_funcs: &'a std::collections::HashSet<String>,
}

impl<'a> Lowerer<'a> {
    /// 分配一个 8 字节临时量，返回其偏移。
    fn fresh(&mut self) -> Temp {
        let off = self.next_offset;
        self.next_offset += 8;
        off
    }

    /// 分配 `size` 字节（向上对齐到 8）的匿名缓冲区，返回其偏移。
    fn alloc_bytes(&mut self, size: usize) -> usize {
        let aligned = size.div_ceil(8).max(1) * 8;
        let off = self.next_offset;
        self.next_offset += aligned;
        off
    }

    fn new_label(&mut self) -> usize {
        let l = self.next_label;
        self.next_label += 1;
        l
    }

    /// 命名标签 → IR label 号（不存在则分配，支持 goto 前向引用）。
    fn goto_label(&mut self, name: &str) -> usize {
        if let Some(&l) = self.goto_labels.get(name) {
            l
        } else {
            let l = self.new_label();
            self.goto_labels.insert(name.to_string(), l);
            l
        }
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

    /// 把表达式 `e` 求值并存入帧偏移 `off`（按 ty 转换；结构体走整体拷贝）。
    fn store_at(&mut self, off: usize, e: &Expr, ty: &Type) {
        let (v0, vty) = self.lower_expr(e);
        let v = self.coerce(v0, &vty, ty);
        let addr = self.fresh();
        self.body.push(Instr::AddrOf { dst: addr, off });
        self.store_into(addr, v, ty);
    }

    /// 把帧偏移 `off` 处类型为 ty 的对象零初始化（标量写 0，聚合体逐元素/字段递归）。
    fn zero_at(&mut self, off: usize, ty: &Type) {
        match ty {
            Type::Array(elem, n) => {
                let esize = self.size_of(elem);
                for i in 0..*n {
                    self.zero_at(off + i * esize, elem);
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                let fields = self
                    .aggregates
                    .get(name)
                    .map(|a| a.fields.clone())
                    .unwrap_or_default();
                for f in &fields {
                    self.zero_at(off + f.offset, &f.ty);
                }
            }
            _ => {
                let z = self.fresh();
                self.body.push(Instr::Const { dst: z, value: 0 });
                let addr = self.fresh();
                self.body.push(Instr::AddrOf { dst: addr, off });
                let width = self.size_of(ty);
                self.body.push(Instr::StoreInd { addr, src: z, width });
            }
        }
    }

    /// 用初始化列表 items 初始化帧偏移 off 处类型为 ty 的聚合体；缺省元素零填充。
    fn lower_aggregate_init(&mut self, off: usize, ty: &Type, items: &[Expr]) {
        match ty {
            Type::Array(elem, n) => {
                let esize = self.size_of(elem);
                for i in 0..*n {
                    let eoff = off + i * esize;
                    match items.get(i) {
                        Some(Expr::InitList(inner)) => self.lower_aggregate_init(eoff, elem, inner),
                        Some(e) => self.store_at(eoff, e, elem),
                        None => self.zero_at(eoff, elem),
                    }
                }
            }
            Type::Struct(name) | Type::Union(name) => {
                let fields = self
                    .aggregates
                    .get(name)
                    .map(|a| a.fields.clone())
                    .unwrap_or_default();
                for (i, f) in fields.iter().enumerate() {
                    let foff = off + f.offset;
                    match items.get(i) {
                        Some(Expr::InitList(inner)) => self.lower_aggregate_init(foff, &f.ty, inner),
                        Some(e) => self.store_at(foff, e, &f.ty),
                        None => self.zero_at(foff, &f.ty),
                    }
                }
            }
            _ => {
                if let Some(e) = items.first() {
                    self.store_at(off, e, ty);
                }
            }
        }
    }

    /// 把 `src` 写入地址 `addr`：结构体按值整体拷贝，标量按宽度存储。
    /// （结构体值在本 IR 中统一以地址表示，故 `src` 此时存的是源结构体地址。）
    fn store_into(&mut self, addr: Temp, src: Temp, ty: &Type) {
        match ty {
            Type::Struct(_) | Type::Union(_) => {
                let size = self.size_of(ty);
                self.body.push(Instr::MemCpy { dst: addr, src, size });
            }
            _ => {
                let width = self.size_of(ty);
                self.body.push(Instr::StoreInd { addr, src, width });
            }
        }
    }

    /// 按需在数值类型间插入转换指令（int/char ↔ long/指针 ↔ double），返回结果临时量。
    fn coerce(&mut self, t: Temp, from: &Type, to: &Type) -> Temp {
        use NumKind::*;
        let (fk, tk) = (num_kind(from), num_kind(to));
        if fk == tk {
            return t; // 同类（如 int↔char、long↔指针）无需转换
        }
        let dst = self.fresh();
        let instr = match (fk, tk) {
            (Float, Narrow) => Instr::FloatToInt { dst, src: t },
            (Float, Wide) => Instr::FloatToLong { dst, src: t },
            (Narrow, Float) => Instr::IntToFloat { dst, src: t },
            (Wide, Float) => Instr::LongToFloat { dst, src: t },
            // 32→64 扩展:无符号源零扩展,有符号源符号扩展
            (Narrow, Wide) => {
                if from.is_unsigned() {
                    Instr::WidenU { dst, src: t }
                } else {
                    Instr::Widen { dst, src: t }
                }
            }
            (Wide, Narrow) => return t, // 截断：取低 32 位，无需指令
            _ => return t,              // void/struct 等不转换
        };
        self.body.push(instr);
        dst
    }

    /// 取变量地址：局部用 AddrOf，全局用 GlobalAddr。返回 (地址临时量, 变量类型)。
    fn var_addr(&mut self, name: &str) -> (Temp, Type) {
        if let Some((off, ty)) = self.lookup_var(name) {
            let dst = self.fresh();
            self.body.push(Instr::AddrOf { dst, off });
            (dst, ty)
        } else if let Some(ty) = self.globals.get(name).cloned() {
            let dst = self.fresh();
            self.body.push(Instr::GlobalAddr {
                dst,
                name: name.to_string(),
            });
            (dst, ty)
        } else {
            panic!("undeclared variable: {}", name);
        }
    }

    /// name 是否是一个函数（已声明签名且不是变量）。
    fn is_function_name(&self, name: &str) -> bool {
        self.lookup_var(name).is_none()
            && !self.globals.contains_key(name)
            && self.signatures.contains_key(name)
    }

    fn var_type(&self, name: &str) -> Option<Type> {
        self.lookup_var(name)
            .map(|(_, t)| t)
            .or_else(|| self.globals.get(name).cloned())
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
                let (v, ety) = self.lower_expr(e);
                let ret_ty = self.ret_ty.clone();
                match ret_ty {
                    Type::Struct(_) | Type::Union(_) => {
                        // v 存放结构体地址；后端按 size 经返回寄存器或隐式指针回写。
                        let size = self.size_of(&ret_ty);
                        self.body.push(Instr::Return {
                            src: v,
                            is_float: false,
                            width: 8,
                            agg: Some(size),
                        });
                    }
                    _ => {
                        let src = self.coerce(v, &ety, &ret_ty);
                        self.body.push(Instr::Return {
                            src,
                            is_float: matches!(ret_ty, Type::Double),
                            width: if matches!(ret_ty, Type::Pointer(_) | Type::Long | Type::ULong) {
                                8
                            } else {
                                4
                            },
                            agg: None,
                        });
                    }
                }
            }
            Stmt::Declare { name, ty, init } => {
                let off = self.declare_var(name, ty.clone());
                match init {
                    Some(Expr::InitList(items)) => {
                        let ty = ty.clone();
                        self.lower_aggregate_init(off, &ty, items);
                    }
                    Some(e) => {
                        let (v0, vty) = self.lower_expr(e);
                        let v = self.coerce(v0, &vty, ty);
                        let addr = self.fresh();
                        self.body.push(Instr::AddrOf { dst: addr, off });
                        self.store_into(addr, v, ty);
                    }
                    None => {}
                }
            }
            Stmt::ExprStmt(e) => {
                let _ = self.lower_expr(e);
            }
            Stmt::Decls(stmts) => {
                // 多声明符：在当前作用域顺序展开（不新建作用域）
                for st in stmts {
                    self.lower_stmt(st);
                }
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
                self.break_targets.push(end);
                self.continue_targets.push(start);
                self.lower_stmt(body);
                self.break_targets.pop();
                self.continue_targets.pop();
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
            }
            Stmt::DoWhile { body, cond } => {
                // 先执行 body，再判断条件；continue 跳到条件判断，break 跳到结尾
                let start = self.new_label();
                let cont = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                self.break_targets.push(end);
                self.continue_targets.push(cont);
                self.lower_stmt(body);
                self.break_targets.pop();
                self.continue_targets.pop();
                self.body.push(Instr::Label(cont));
                let (c, _) = self.lower_expr(cond);
                // 条件为假则退出，否则回到 start
                self.body.push(Instr::JumpIfZero { cond: c, target: end });
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
                let cont = self.new_label();
                let end = self.new_label();
                self.body.push(Instr::Label(start));
                if let Some(c) = cond {
                    let (cv, _) = self.lower_expr(c);
                    self.body.push(Instr::JumpIfZero {
                        cond: cv,
                        target: end,
                    });
                }
                self.break_targets.push(end);
                self.continue_targets.push(cont); // continue 跳到 step 之前
                self.lower_stmt(body);
                self.break_targets.pop();
                self.continue_targets.pop();
                self.body.push(Instr::Label(cont));
                if let Some(st) = step {
                    let _ = self.lower_expr(st);
                }
                self.body.push(Instr::Jump(start));
                self.body.push(Instr::Label(end));
                self.pop_scope();
            }
            Stmt::Goto(name) => {
                let l = self.goto_label(name);
                self.body.push(Instr::Jump(l));
            }
            Stmt::Label(name) => {
                let l = self.goto_label(name);
                self.body.push(Instr::Label(l));
            }
            Stmt::Break => {
                if let Some(&t) = self.break_targets.last() {
                    self.body.push(Instr::Jump(t));
                }
            }
            Stmt::Continue => {
                if let Some(&t) = self.continue_targets.last() {
                    self.body.push(Instr::Jump(t));
                }
            }
            Stmt::Switch { cond, body } => {
                let (c, _) = self.lower_expr(cond);
                let end = self.new_label();
                // 给每个 case/default 分配标签
                let item_labels: Vec<Option<usize>> = body
                    .iter()
                    .map(|s| match s {
                        Stmt::Case(_) | Stmt::Default => Some(self.new_label()),
                        _ => None,
                    })
                    .collect();
                // 分发：c != v 为零（即相等）时跳到对应 case
                let mut default_label = None;
                for (item, lbl) in body.iter().zip(item_labels.iter()) {
                    match item {
                        Stmt::Case(v) => {
                            let tv = self.fresh();
                            self.body.push(Instr::Const { dst: tv, value: *v });
                            let ne = self.fresh();
                            self.body.push(Instr::Bin {
                                dst: ne,
                                op: BinOp::Ne,
                                lhs: c,
                                rhs: tv,
                            });
                            self.body.push(Instr::JumpIfZero {
                                cond: ne,
                                target: lbl.unwrap(),
                            });
                        }
                        Stmt::Default => default_label = *lbl,
                        _ => {}
                    }
                }
                match default_label {
                    Some(d) => self.body.push(Instr::Jump(d)),
                    None => self.body.push(Instr::Jump(end)),
                }
                // 函数体（case/default 处放标签，break 跳到 end）
                self.break_targets.push(end);
                for (item, lbl) in body.iter().zip(item_labels.iter()) {
                    match item {
                        Stmt::Case(_) | Stmt::Default => {
                            self.body.push(Instr::Label(lbl.unwrap()));
                        }
                        other => self.lower_stmt(other),
                    }
                }
                self.break_targets.pop();
                self.body.push(Instr::Label(end));
            }
            // case/default 在 switch 外无意义（由 Switch 处理），单独出现时忽略
            Stmt::Case(_) | Stmt::Default => {}
        }
    }

    /// 返回 (存放右值结果的临时量, 类型)。
    fn lower_expr(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::IntLit(v) => {
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: *v });
                // 超出 32 位范围的字面量按 long（保证 64 位物化与运算）
                let ty = if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                    Type::Int
                } else {
                    Type::Long
                };
                (dst, ty)
            }
            Expr::FloatLit(v) => {
                let index = self.floats.len();
                self.floats.push(v.to_bits());
                let dst = self.fresh();
                self.body.push(Instr::ConstF { dst, index });
                (dst, Type::Double)
            }
            Expr::StrLit(s) => {
                let index = self.strings.len();
                self.strings.push(s.clone());
                let dst = self.fresh();
                self.body.push(Instr::StrLit { dst, index });
                (dst, Type::Pointer(Box::new(Type::Char)))
            }
            Expr::Var(name) if self.is_function_name(name) => {
                // 函数名作为值 → 函数指针（取函数地址）
                let dst = self.fresh();
                self.body.push(Instr::FuncAddr { dst, name: name.clone() });
                let ret = self
                    .signatures
                    .get(name)
                    .map(|s| s.ret.clone())
                    .unwrap_or(Type::Int);
                (dst, Type::FnPtr(Box::new(ret)))
            }
            Expr::Var(name) => {
                let (addr, ty) = self.var_addr(name);
                match ty {
                    Type::Array(elem, _) => (addr, Type::Pointer(elem)), // 退化为首元素地址
                    Type::Struct(_) | Type::Union(_) => (addr, ty),      // 聚合体产出地址
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
            Expr::Addr(inner) => {
                // &函数名 == 函数地址（函数指针）
                if let Expr::Var(name) = inner.as_ref() {
                    if self.is_function_name(name) {
                        return self.lower_expr(inner);
                    }
                }
                let (addr, ty) = self.lower_lvalue(inner);
                (addr, Type::Pointer(Box::new(ty)))
            }
            Expr::Deref(inner) => {
                let (ptr, ty) = self.lower_expr(inner);
                // 解引用函数指针仍是函数（调用时再用），值不变
                if matches!(ty, Type::FnPtr(_)) {
                    return (ptr, ty);
                }
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
                match pointee {
                    // 聚合体元素（多维数组的子数组、结构体数组的元素）产出地址，不按标量载入
                    Type::Array(..) | Type::Struct(_) | Type::Union(_) => (ptr, pointee),
                    scalar => {
                        let dst = self.fresh();
                        let width = self.size_of(&scalar);
                        self.body.push(Instr::LoadInd {
                            dst,
                            addr: ptr,
                            width,
                            signed: matches!(scalar, Type::Char),
                        });
                        (dst, scalar)
                    }
                }
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
                // sizeof 不让数组退化为指针：sizeof arr 应得整个数组大小
                let ty = match inner.as_ref() {
                    Expr::Var(name) => self.var_type(name).unwrap_or(Type::Int),
                    _ => self.type_of(inner),
                };
                let value = self.size_of(&ty) as i64;
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value });
                (dst, Type::Int)
            }
            Expr::Cast { ty, expr } => {
                let (v, vty) = self.lower_expr(expr);
                let r = self.coerce(v, &vty, ty);
                (r, ty.clone())
            }
            Expr::Comma { first, second } => {
                // 求值 first（仅副作用，丢弃结果），整体取 second 的值与类型
                let _ = self.lower_expr(first);
                self.lower_expr(second)
            }
            Expr::InitList(items) => {
                // 初始化列表只应出现在聚合体声明的初始化中（由 Declare 特判处理）；
                // 若作为普通值出现，退化为其首个元素。
                match items.first() {
                    Some(e) => self.lower_expr(e),
                    None => {
                        let dst = self.fresh();
                        self.body.push(Instr::Const { dst, value: 0 });
                        (dst, Type::Int)
                    }
                }
            }
            Expr::Unary { op, operand } => {
                // 常量折叠（如 -5、~0xF）
                if let Some(v) = const_eval(e) {
                    let dst = self.fresh();
                    self.body.push(Instr::Const { dst, value: v as i64 });
                    return (dst, Type::Int);
                }
                let (src, ty) = self.lower_expr(operand);
                match op {
                    UnaryOp::Plus => (src, ty),
                    UnaryOp::Neg => match num_kind(&ty) {
                        NumKind::Wide => {
                            let dst = self.fresh();
                            self.body.push(Instr::NegL { dst, src });
                            (dst, Type::Long)
                        }
                        NumKind::Float => {
                            // 浮点取负：0.0 - x
                            let zero = self.fresh();
                            let index = self.floats.len();
                            self.floats.push(0f64.to_bits());
                            self.body.push(Instr::ConstF { dst: zero, index });
                            let dst = self.fresh();
                            self.body.push(Instr::BinF { dst, op: BinOp::Sub, lhs: zero, rhs: src });
                            (dst, Type::Double)
                        }
                        _ => {
                            let dst = self.fresh();
                            self.body.push(Instr::Neg { dst, src });
                            (dst, Type::Int)
                        }
                    },
                }
            }
            Expr::Binary { op, lhs, rhs } => {
                // 常量折叠：整个表达式是常量 → 直接发一个常数
                if let Some(v) = const_eval(e) {
                    let dst = self.fresh();
                    self.body.push(Instr::Const { dst, value: v as i64 });
                    return (dst, Type::Int);
                }
                self.lower_binary(*op, lhs, rhs)
            }
            Expr::Logical { op, lhs, rhs } => {
                let result = self.fresh();
                match op {
                    LogOp::And => {
                        let lfalse = self.new_label();
                        let lend = self.new_label();
                        let (la, _) = self.lower_expr(lhs);
                        self.body.push(Instr::JumpIfZero { cond: la, target: lfalse });
                        let (lb, _) = self.lower_expr(rhs);
                        self.body.push(Instr::JumpIfZero { cond: lb, target: lfalse });
                        self.body.push(Instr::Const { dst: result, value: 1 });
                        self.body.push(Instr::Jump(lend));
                        self.body.push(Instr::Label(lfalse));
                        self.body.push(Instr::Const { dst: result, value: 0 });
                        self.body.push(Instr::Label(lend));
                    }
                    LogOp::Or => {
                        let lcheck = self.new_label();
                        let lfalse = self.new_label();
                        let lend = self.new_label();
                        let (la, _) = self.lower_expr(lhs);
                        self.body.push(Instr::JumpIfZero { cond: la, target: lcheck });
                        self.body.push(Instr::Const { dst: result, value: 1 });
                        self.body.push(Instr::Jump(lend));
                        self.body.push(Instr::Label(lcheck));
                        let (lb, _) = self.lower_expr(rhs);
                        self.body.push(Instr::JumpIfZero { cond: lb, target: lfalse });
                        self.body.push(Instr::Const { dst: result, value: 1 });
                        self.body.push(Instr::Jump(lend));
                        self.body.push(Instr::Label(lfalse));
                        self.body.push(Instr::Const { dst: result, value: 0 });
                        self.body.push(Instr::Label(lend));
                    }
                }
                (result, Type::Int)
            }
            Expr::Ternary { cond, then_e, else_e } => {
                // 两分支取公共类型（如 int : long → long），各自转换到该类型再合并
                let common = common_type(&self.type_of(then_e), &self.type_of(else_e));
                let width = self.size_of(&common);
                let lelse = self.new_label();
                let lend = self.new_label();
                let (c, _) = self.lower_expr(cond);
                self.body.push(Instr::JumpIfZero { cond: c, target: lelse });
                let (tv0, tty) = self.lower_expr(then_e);
                let tv = self.coerce(tv0, &tty, &common);
                let result = self.fresh();
                self.body.push(Instr::Copy { dst: result, src: tv, width });
                self.body.push(Instr::Jump(lend));
                self.body.push(Instr::Label(lelse));
                let (ev0, ety) = self.lower_expr(else_e);
                let ev = self.coerce(ev0, &ety, &common);
                self.body.push(Instr::Copy { dst: result, src: ev, width });
                self.body.push(Instr::Label(lend));
                (result, common)
            }
            Expr::Assign { target, value } => {
                let (v0, vty) = self.lower_expr(value);
                let (addr, ty) = self.lower_lvalue(target);
                let v = self.coerce(v0, &vty, &ty);
                self.store_into(addr, v, &ty);
                (v, ty)
            }
            Expr::Call { name, args } => {
                // 若 name 是函数指针变量 → 间接调用；否则按函数名直接调用。
                if let Some(Type::FnPtr(ret)) = self.var_type(name) {
                    let (fp, _) = self.lower_expr(&Expr::Var(name.clone()));
                    self.emit_call(String::new(), Some(fp), *ret, args, &[], args.len(), false, false)
                } else {
                    let sig = self.signatures.get(name).cloned();
                    let param_types: Vec<Type> =
                        sig.as_ref().map(|s| s.params.clone()).unwrap_or_default();
                    let ret = sig.as_ref().map(|s| s.ret.clone()).unwrap_or(Type::Int);
                    let fixed = sig.as_ref().map(|s| s.fixed).unwrap_or(args.len());
                    let variadic = sig.as_ref().map(|s| s.variadic).unwrap_or(false);
                    // 自定义可变参数函数：可变实参压栈传递
                    let stack_varargs = variadic && self.defined_funcs.contains(name);
                    self.emit_call(
                        name.clone(),
                        None,
                        ret,
                        args,
                        &param_types,
                        fixed,
                        variadic,
                        stack_varargs,
                    )
                }
            }
            Expr::CallPtr { func, args } => {
                let (fp, fty) = self.lower_expr(func);
                let ret = match fty {
                    Type::FnPtr(r) => *r,
                    _ => Type::Int,
                };
                self.emit_call(String::new(), Some(fp), ret, args, &[], args.len(), false, false)
            }
            Expr::VaStart { ap } => {
                let (ap_addr, _) = self.lower_lvalue(ap);
                self.body.push(Instr::VaStart { ap: ap_addr });
                let dst = self.fresh();
                self.body.push(Instr::Const { dst, value: 0 });
                (dst, Type::Int)
            }
            Expr::VaArg { ap, ty } => {
                let (ap_addr, _) = self.lower_lvalue(ap);
                let dst = self.fresh();
                let width = self.size_of(ty);
                let is_float = matches!(ty, Type::Double);
                self.body.push(Instr::VaArg { dst, ap: ap_addr, width, is_float });
                (dst, ty.clone())
            }
        }
    }

    /// 发射一次调用（直接或经函数指针 `via`），处理实参分类、结构体返回缓冲区。
    #[allow(clippy::too_many_arguments)]
    fn emit_call(
        &mut self,
        name: String,
        via: Option<Temp>,
        ret: Type,
        args: &[Expr],
        param_types: &[Type],
        fixed: usize,
        variadic: bool,
        stack_varargs: bool,
    ) -> (Temp, Type) {
        let mut arg_temps: Vec<Temp> = Vec::with_capacity(args.len());
        let mut arg_floats: Vec<bool> = Vec::with_capacity(args.len());
        let mut arg_aggs: Vec<Option<usize>> = Vec::with_capacity(args.len());
        for (i, a) in args.iter().enumerate() {
            let (t0, ty0) = self.lower_expr(a);
            let (t, ty) = match param_types.get(i) {
                Some(pt) => (self.coerce(t0, &ty0, pt), pt.clone()),
                None => (t0, ty0),
            };
            arg_temps.push(t);
            arg_floats.push(matches!(ty, Type::Double));
            arg_aggs.push(match ty {
                Type::Struct(_) | Type::Union(_) => Some(self.size_of(&ty)),
                _ => None,
            });
        }
        let ret_float = matches!(ret, Type::Double);
        let ret_agg = match ret {
            Type::Struct(_) | Type::Union(_) => Some(self.size_of(&ret)),
            _ => None,
        };
        let ret_width =
            if matches!(
                ret,
                Type::Pointer(_) | Type::Double | Type::Long | Type::ULong | Type::FnPtr(_)
            ) {
                8
            } else {
                4
            };
        if let Some(size) = ret_agg {
            let buf = self.alloc_bytes(size);
            self.body.push(Instr::Call {
                dst: 0,
                name,
                via,
                args: arg_temps,
                arg_floats,
                arg_aggs,
                ret_width,
                ret_agg,
                ret_buf: Some(buf),
                fixed,
                variadic,
                stack_varargs,
                ret_float,
            });
            let addr = self.fresh();
            self.body.push(Instr::AddrOf { dst: addr, off: buf });
            (addr, ret)
        } else {
            let dst = self.fresh();
            self.body.push(Instr::Call {
                dst,
                name,
                via,
                args: arg_temps,
                arg_floats,
                arg_aggs,
                ret_width,
                ret_agg: None,
                ret_buf: None,
                fixed,
                variadic,
                stack_varargs,
                ret_float,
            });
            (dst, ret)
        }
    }

    /// 返回 (左值地址临时量, 被指类型)。
    fn lower_lvalue(&mut self, e: &Expr) -> (Temp, Type) {
        match e {
            Expr::Var(name) => self.var_addr(name),
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
        let size = self.size_of(&elem);
        self.body.push(Instr::PtrAdd {
            dst,
            base: ptr,
            index: idx,
            size,
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
            let size = self.size_of(&elem);
            if op == BinaryOp::Add {
                self.body.push(Instr::PtrAdd {
                    dst,
                    base: ptr,
                    index: idx,
                    size,
                });
            } else {
                self.body.push(Instr::PtrSub {
                    dst,
                    base: ptr,
                    index: idx,
                    size,
                });
            }
            return (dst, pty.decay());
        }
        // 两个指针：比较走 64 位；相减得元素个数（字节差 / sizeof(elem)）
        if l_ptr && r_ptr {
            if is_compare(op) {
                let dst = self.fresh();
                self.body.push(Instr::BinL { dst, op: lower_binop(op), lhs: l, rhs: r });
                return (dst, Type::Int);
            }
            if op == BinaryOp::Sub {
                let diff = self.fresh();
                self.body.push(Instr::BinL { dst: diff, op: BinOp::Sub, lhs: l, rhs: r });
                let elem = lty.decay().pointee().cloned().unwrap_or(Type::Char);
                let esize = self.size_of(&elem).max(1);
                if esize == 1 {
                    return (diff, Type::Long);
                }
                let szt0 = self.fresh();
                self.body.push(Instr::Const { dst: szt0, value: esize as i64 });
                // 32 位常量需符号扩展为干净的 64 位再喂给 BinL（否则高 32 位为栈残留）
                let szt = self.coerce(szt0, &Type::Int, &Type::Long);
                let q = self.fresh();
                self.body.push(Instr::BinL { dst: q, op: BinOp::Div, lhs: diff, rhs: szt });
                return (q, Type::Long);
            }
        }
        // 浮点运算：任一操作数为 double 即走 FP 路径（int 操作数提升为 double）
        if matches!(lty, Type::Double) || matches!(rty, Type::Double) {
            let lf = self.coerce(l, &lty, &Type::Double);
            let rf = self.coerce(r, &rty, &Type::Double);
            let dst = self.fresh();
            self.body.push(Instr::BinF {
                dst,
                op: lower_binop(op),
                lhs: lf,
                rhs: rf,
            });
            let result_ty = if is_compare(op) {
                Type::Int
            } else {
                Type::Double
            };
            return (dst, result_ty);
        }
        // 任一操作数无符号 → 整个运算走无符号语义(C 的通常算术转换简化:无符号优先)
        let uns = lty.is_unsigned() || rty.is_unsigned();
        // 64 位整数运算：任一操作数为 long/ulong 即走 64 位路径
        if matches!(lty, Type::Long | Type::ULong) || matches!(rty, Type::Long | Type::ULong) {
            let common = if uns { Type::ULong } else { Type::Long };
            let ll = self.coerce(l, &lty, &common);
            let rr = self.coerce(r, &rty, &common);
            let dst = self.fresh();
            self.body.push(Instr::BinL {
                dst,
                op: lower_binop_u(op, uns),
                lhs: ll,
                rhs: rr,
            });
            let result_ty = if is_compare(op) { Type::Int } else { common };
            return (dst, result_ty);
        }
        let dst = self.fresh();
        self.body.push(Instr::Bin {
            dst,
            op: lower_binop_u(op, uns),
            lhs: l,
            rhs: r,
        });
        let result_ty = if is_compare(op) || !uns { Type::Int } else { Type::UInt };
        (dst, result_ty)
    }

    /// 仅推断类型（用于 sizeof / 三元公共类型，不求值操作数）。
    fn type_of(&self, e: &Expr) -> Type {
        match e {
            Expr::IntLit(v) => {
                if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 {
                    Type::Int
                } else {
                    Type::Long
                }
            }
            Expr::Logical { .. } | Expr::SizeofType(_) | Expr::SizeofExpr(_) => Type::Int,
            Expr::Cast { ty, .. } => ty.clone(),
            Expr::Call { name, .. } => self
                .signatures
                .get(name)
                .map(|s| s.ret.clone())
                .unwrap_or(Type::Int),
            Expr::Unary { op, operand } => match op {
                UnaryOp::Neg | UnaryOp::Plus => self.type_of(operand),
            },
            Expr::Binary { op, lhs, rhs } => {
                if is_compare(*op) {
                    return Type::Int;
                }
                let lt = self.type_of(lhs).decay();
                let rt = self.type_of(rhs).decay();
                // 指针算术：指针 ± 整数 → 指针
                if matches!(op, BinaryOp::Add | BinaryOp::Sub)
                    && (lt.is_pointer_like() ^ rt.is_pointer_like())
                {
                    return if lt.is_pointer_like() { lt } else { rt };
                }
                common_type(&lt, &rt)
            }
            Expr::Ternary { then_e, .. } => self.type_of(then_e),
            Expr::FloatLit(_) => Type::Double,
            Expr::StrLit(_) => Type::Pointer(Box::new(Type::Char)),
            Expr::Var(name) => self.var_type(name).map(|t| t.decay()).unwrap_or(Type::Int),
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
            Expr::Comma { second, .. } => self.type_of(second),
            Expr::InitList(items) => items.first().map(|e| self.type_of(e)).unwrap_or(Type::Int),
            Expr::CallPtr { func, .. } => match self.type_of(func) {
                Type::FnPtr(r) => *r,
                _ => Type::Int,
            },
            Expr::VaStart { .. } => Type::Int,
            Expr::VaArg { ty, .. } => ty.clone(),
        }
    }

    fn type_of_lvalue(&self, e: &Expr) -> Type {
        match e {
            Expr::Var(name) => self.var_type(name).unwrap_or(Type::Int),
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

/// 数值类型的运算宽度类别。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NumKind {
    /// 32 位整数：int / char。
    Narrow,
    /// 64 位整数语义：long / 指针 / 数组（退化后的地址）。
    Wide,
    /// 浮点：double。
    Float,
    /// 不参与数值转换：void / struct / union。
    Other,
}

fn num_kind(ty: &Type) -> NumKind {
    match ty {
        Type::Int | Type::Char | Type::UInt | Type::UChar => NumKind::Narrow,
        Type::Long | Type::ULong | Type::Pointer(_) | Type::Array(..) | Type::FnPtr(_) => {
            NumKind::Wide
        }
        Type::Double => NumKind::Float,
        _ => NumKind::Other,
    }
}

/// 二元/三元运算的公共结果类型（C 的“通常算术转换”简化版）。
fn common_type(a: &Type, b: &Type) -> Type {
    match (num_kind(a), num_kind(b)) {
        (NumKind::Float, _) | (_, NumKind::Float) => Type::Double,
        (NumKind::Wide, _) | (_, NumKind::Wide) => {
            // 两侧同为指针时保留指针类型，否则按 64 位整数
            if a.is_pointer_like() {
                a.decay()
            } else if b.is_pointer_like() {
                b.decay()
            } else {
                Type::Long
            }
        }
        _ => Type::Int,
    }
}

/// 64 位常量整数求值(用于全局初始化器)。支持字面量/一元/二元算术与位运算。
fn const_i64(e: &Expr) -> Option<i64> {
    match e {
        Expr::IntLit(v) => Some(*v),
        Expr::Cast { expr, .. } => const_i64(expr),
        Expr::Unary { op, operand } => {
            let v = const_i64(operand)?;
            Some(match op {
                UnaryOp::Neg => v.wrapping_neg(),
                UnaryOp::Plus => v,
            })
        }
        Expr::Binary { op, lhs, rhs } => {
            let a = const_i64(lhs)?;
            let b = const_i64(rhs)?;
            Some(match op {
                BinaryOp::Add => a.wrapping_add(b),
                BinaryOp::Sub => a.wrapping_sub(b),
                BinaryOp::Mul => a.wrapping_mul(b),
                BinaryOp::Div => {
                    if b == 0 {
                        return None;
                    }
                    a.wrapping_div(b)
                }
                BinaryOp::Mod => {
                    if b == 0 {
                        return None;
                    }
                    a.wrapping_rem(b)
                }
                BinaryOp::BitAnd => a & b,
                BinaryOp::BitOr => a | b,
                BinaryOp::BitXor => a ^ b,
                BinaryOp::Shl => a.wrapping_shl(b as u32),
                BinaryOp::Shr => a.wrapping_shr(b as u32),
                BinaryOp::Lt => (a < b) as i64,
                BinaryOp::Gt => (a > b) as i64,
                BinaryOp::Le => (a <= b) as i64,
                BinaryOp::Ge => (a >= b) as i64,
                BinaryOp::Eq => (a == b) as i64,
                BinaryOp::Ne => (a != b) as i64,
            })
        }
        _ => None,
    }
}

/// 计算全局初始化器的字节镜像(小端,长度 = size_of(ty))。
fn eval_init_bytes(ty: &Type, e: &Expr, aggs: &Aggregates, size: usize) -> Vec<u8> {
    let mut buf = vec![0u8; size];
    write_init(&mut buf, 0, ty, e, aggs);
    buf
}

fn write_init(buf: &mut [u8], off: usize, ty: &Type, e: &Expr, aggs: &Aggregates) {
    match ty {
        Type::Array(elem, n) => {
            if let Expr::InitList(items) = e {
                let esz = crate::types::size_of(elem, aggs);
                for (i, it) in items.iter().enumerate().take(*n) {
                    write_init(buf, off + i * esz, elem, it, aggs);
                }
            }
        }
        Type::Struct(name) | Type::Union(name) => {
            if let Expr::InitList(items) = e {
                if let Some(agg) = aggs.get(name) {
                    let fields = agg.fields.clone();
                    for (i, f) in fields.iter().enumerate() {
                        if let Some(it) = items.get(i) {
                            write_init(buf, off + f.offset, &f.ty, it, aggs);
                        }
                    }
                }
            }
        }
        _ => {
            // 标量:取常量整数(若是 {expr} 取首项),写小端 width 字节
            let scalar = match e {
                Expr::InitList(items) => items.first(),
                other => Some(other),
            };
            let v = scalar.and_then(const_i64).unwrap_or(0);
            let w = crate::types::size_of(ty, aggs);
            for k in 0..w.min(buf.len() - off) {
                buf[off + k] = ((v >> (k * 8)) & 0xff) as u8;
            }
        }
    }
}

/// 常量折叠：纯整数常量表达式在编译期求值（按 32 位回绕，匹配 anvil 的 int 运行时语义）。
/// 任一叶子非 i32 范围常量、或除零，则返回 None（不折叠）。
fn const_eval(e: &Expr) -> Option<i32> {
    match e {
        Expr::IntLit(v) if *v >= i32::MIN as i64 && *v <= i32::MAX as i64 => Some(*v as i32),
        Expr::Unary { op, operand } => {
            let v = const_eval(operand)?;
            Some(match op {
                UnaryOp::Neg => v.wrapping_neg(),
                UnaryOp::Plus => v,
            })
        }
        Expr::Binary { op, lhs, rhs } => {
            let a = const_eval(lhs)?;
            let b = const_eval(rhs)?;
            let r = match op {
                BinaryOp::Add => a.wrapping_add(b),
                BinaryOp::Sub => a.wrapping_sub(b),
                BinaryOp::Mul => a.wrapping_mul(b),
                BinaryOp::Div => {
                    if b == 0 {
                        return None;
                    }
                    a.wrapping_div(b)
                }
                BinaryOp::Mod => {
                    if b == 0 {
                        return None;
                    }
                    a.wrapping_rem(b)
                }
                BinaryOp::BitAnd => a & b,
                BinaryOp::BitOr => a | b,
                BinaryOp::BitXor => a ^ b,
                BinaryOp::Shl => a.wrapping_shl(b as u32),
                BinaryOp::Shr => a.wrapping_shr(b as u32),
                BinaryOp::Lt => (a < b) as i32,
                BinaryOp::Gt => (a > b) as i32,
                BinaryOp::Le => (a <= b) as i32,
                BinaryOp::Ge => (a >= b) as i32,
                BinaryOp::Eq => (a == b) as i32,
                BinaryOp::Ne => (a != b) as i32,
            };
            Some(r)
        }
        _ => None,
    }
}

fn is_compare(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::Lt | BinaryOp::Gt | BinaryOp::Le | BinaryOp::Ge | BinaryOp::Eq | BinaryOp::Ne
    )
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
        BinaryOp::BitAnd => BinOp::BitAnd,
        BinaryOp::BitOr => BinOp::BitOr,
        BinaryOp::BitXor => BinOp::BitXor,
        BinaryOp::Shl => BinOp::Shl,
        BinaryOp::Shr => BinOp::Shr,
    }
}

/// 同 lower_binop,但 `uns` 为真时把除/模/右移/有序比较换成无符号变体。
fn lower_binop_u(op: BinaryOp, uns: bool) -> BinOp {
    if !uns {
        return lower_binop(op);
    }
    match op {
        BinaryOp::Div => BinOp::UDiv,
        BinaryOp::Mod => BinOp::UMod,
        BinaryOp::Shr => BinOp::UShr,
        BinaryOp::Lt => BinOp::ULt,
        BinaryOp::Gt => BinOp::UGt,
        BinaryOp::Le => BinOp::ULe,
        BinaryOp::Ge => BinOp::UGe,
        other => lower_binop(other), // 其余运算有/无符号相同
    }
}


#[allow(clippy::too_many_arguments)]
fn lower_func(
    f: &FuncDef,
    strings: &mut Vec<String>,
    floats: &mut Vec<u64>,
    aggregates: &Aggregates,
    signatures: &Signatures,
    globals: &HashMap<String, Type>,
    defined_funcs: &std::collections::HashSet<String>,
) -> Function {
    let mut lw = Lowerer {
        body: Vec::new(),
        next_offset: 0,
        scopes: vec![HashMap::new()],
        next_label: 0,
        strings,
        floats,
        aggregates,
        signatures,
        globals,
        ret_ty: f.ret.clone(),
        break_targets: Vec::new(),
        continue_targets: Vec::new(),
        goto_labels: HashMap::new(),
        defined_funcs,
    };
    let ret_agg = match f.ret {
        Type::Struct(_) | Type::Union(_) => Some(lw.size_of(&f.ret)),
        _ => None,
    };
    // 大结构体返回（>16 字节）走隐式指针：序言把它存到此槽，Return 时据此回写。
    let sret_slot = match ret_agg {
        Some(sz) if sz > 16 => Some(lw.alloc_bytes(8)),
        _ => None,
    };
    // 形参占据前若干槽；具体来自哪个寄存器/栈位由后端按 ABI 决定（见 Function.params）。
    let mut params = Vec::with_capacity(f.params.len());
    for (pname, pty) in f.params.iter() {
        let size = lw.size_of(pty);
        let off = lw.declare_var(pname, pty.clone());
        params.push(crate::ir::Param {
            slot: off,
            size,
            is_float: matches!(pty, Type::Double),
            is_aggregate: matches!(pty, Type::Struct(_) | Type::Union(_)),
        });
    }
    for stmt in &f.body {
        lw.lower_stmt(stmt);
    }
    Function {
        name: f.name.clone(),
        params,
        body: lw.body,
        frame_bytes: lw.next_offset,
        ret_float: matches!(f.ret, Type::Double),
        ret_agg,
        sret_slot,
        variadic: f.variadic,
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
                Instr::Return { src: 0, is_float: false, width: 4, agg: None },
            ]
        );
    }

    #[test]
    fn lower_add() {
        // 非常量操作数：仍走 Bin（常量会被折叠，故用变量）
        let f = lower_src("int main(){ int x = 1; return x + 2; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::Bin { op: BinOp::Add, .. })));
    }

    #[test]
    fn lower_const_folds_arithmetic() {
        // 1 + 2*3 - (4/2) = 5 全是常量 → 折成单个 Const，无 Bin
        let f = lower_src("int main(){ return 1 + 2*3 - (4/2); }");
        assert!(!f.body.iter().any(|i| matches!(i, Instr::Bin { .. })));
        assert!(f.body.iter().any(|i| matches!(i, Instr::Const { value: 5, .. })));
    }

    #[test]
    fn lower_const_folds_bitwise_and_compare() {
        let f = lower_src("int main(){ return (0xF0 | 0x0F) == 255; }");
        assert!(!f.body.iter().any(|i| matches!(i, Instr::Bin { .. })));
        assert!(f.body.iter().any(|i| matches!(i, Instr::Const { value: 1, .. })));
    }

    #[test]
    fn lower_no_fold_div_by_zero() {
        // 除零不在编译期折叠（交由运行时）
        let f = lower_src("int main(){ return 1 / 0; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::Bin { op: BinOp::Div, .. })));
    }

    #[test]
    fn lower_unary_plus_is_noop() {
        let f = lower_src("int main(){ return +7; }");
        assert_eq!(f.frame_bytes, 8);
        assert_eq!(
            f.body,
            vec![
                Instr::Const { dst: 0, value: 7 },
                Instr::Return { src: 0, is_float: false, width: 4, agg: None },
            ]
        );
    }

    #[test]
    fn lower_unary_neg() {
        // 非常量操作数仍走 Neg；常量 -x 会被折叠
        let f = lower_src("int main(){ int x = 7; return -x; }");
        assert!(f.body.iter().any(|i| matches!(i, Instr::Neg { .. })));
        let g = lower_src("int main(){ return -7; }");
        assert!(g.body.iter().any(|i| matches!(i, Instr::Const { value: -7, .. })));
        assert!(!g.body.iter().any(|i| matches!(i, Instr::Neg { .. })));
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
    fn lower_records_params() {
        let p = lower_prog("int add(int a, int b){ return a+b; } int main(){ return add(1,2); }");
        let add = p.functions.iter().find(|f| f.name == "add").unwrap();
        assert_eq!(add.params.len(), 2);
        assert!(add.params.iter().all(|p| !p.is_float && !p.is_aggregate && p.size == 4));
    }

    #[test]
    fn lower_double_param_marked_float() {
        let p = lower_prog("double f(int a, double b){ return b; } int main(){ return 0; }");
        let f = p.functions.iter().find(|f| f.name == "f").unwrap();
        assert_eq!(f.params.len(), 2);
        assert!(!f.params[0].is_float);
        assert!(f.params[1].is_float);
        assert!(f.ret_float);
    }

    #[test]
    fn lower_struct_param_marked_aggregate() {
        let p = lower_prog(
            "struct P { int x; int y; }; int f(struct P p){ return p.x; } int main(){ return 0; }",
        );
        let f = p.functions.iter().find(|f| f.name == "f").unwrap();
        assert_eq!(f.params.len(), 1);
        assert!(f.params[0].is_aggregate);
        assert_eq!(f.params[0].size, 16); // anvil 把每个字段按 8 字节槽位排布
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
        assert!(f.body.iter().any(|i| matches!(i, Instr::PtrAdd { size: 4, .. })));
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
