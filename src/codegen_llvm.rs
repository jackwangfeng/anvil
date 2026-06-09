//! LLVM 后端：把目标无关的三地址 IR 翻成 **LLVM IR 文本(.ll)**,交给 `llc -O2` / `clang -O2`
//! 做 mem2reg/SROA + 全套中端优化 + 寄存器分配 + 指令选择。
//!
//! 统一模型:整个函数帧是一个大 `alloca [N x i8]`,每个槽位按 **i64** 访问
//! (GEP 到字节偏移 + 类型化 load/store)。指针经 inttoptr/ptrtoint、double 经 bitcast、
//! int 运算 trunc/ext 到 i32——这些转换 `-O2` 的 instcombine 会全部清掉。
//!
//! 不支持的特性(结构体按值传参/返回、函数指针、用户可变参数)返回 Err,提示改用原生后端。
use crate::ir::{BinOp, Function, Instr, Program};
use std::collections::HashSet;
use std::fmt::Write;

pub fn generate(program: &Program) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("target triple = \"x86_64-pc-linux-gnu\"\n");
    out.push_str("declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)\n");

    // 字符串常量
    for (i, s) in program.strings.iter().enumerate() {
        let mut bytes = String::new();
        for b in s.as_bytes() {
            let _ = write!(bytes, "\\{:02X}", b);
        }
        bytes.push_str("\\00");
        let _ = writeln!(
            out,
            "@.str.{} = private constant [{} x i8] c\"{}\"",
            i,
            s.len() + 1,
            bytes
        );
    }
    // 全局变量
    for g in &program.globals {
        match (g.init, g.size) {
            (Some(v), 8) => {
                let _ = writeln!(out, "@{} = global i64 {}", g.name, v);
            }
            (Some(v), _) => {
                let _ = writeln!(out, "@{} = global i32 {}", g.name, v);
            }
            (None, n) => {
                let _ = writeln!(out, "@{} = global [{} x i8] zeroinitializer", g.name, n.max(1));
            }
        }
    }

    let defined: HashSet<&str> = program.functions.iter().map(|f| f.name.as_str()).collect();
    let mut declared: HashSet<String> = HashSet::new();
    let mut decls = String::new();
    let mut bodies = String::new();

    for func in &program.functions {
        gen_func(func, &program.floats, &mut bodies, &defined, &mut declared, &mut decls)?;
    }
    // 顺序:target triple / 常量(out) → 外部声明(decls) → 函数定义(bodies)
    out.push_str(&decls);
    out.push_str(&bodies);
    Ok(out)
}

/// 标量返回/参数的 LLVM 类型:double / i64(整型/指针统一 8 字节)。
fn ret_type(f: &Function) -> &'static str {
    if f.name == "main" {
        return "i32"; // main 按 C 约定返回 i32
    }
    if f.ret_float {
        return "double";
    }
    // 从函数体的 Return 推断有无返回值
    let mut has_ret = false;
    for i in &f.body {
        if let Instr::Return { .. } = i {
            has_ret = true;
        }
    }
    if has_ret {
        "i64"
    } else {
        "void"
    }
}

struct Gen<'a> {
    out: String,
    n: usize,   // SSA 值计数
    syn: usize, // 合成基本块标签计数
    open: bool, // 当前基本块是否未终结
    floats: &'a [u64],
    defined: &'a HashSet<&'a str>,
    declared: &'a mut HashSet<String>,
    decls: &'a mut String,
}

impl Gen<'_> {
    fn val(&mut self) -> String {
        self.n += 1;
        format!("%v{}", self.n)
    }
    fn syn(&mut self) -> String {
        self.syn += 1;
        format!("%S{}", self.syn)
    }
    fn emit(&mut self, s: &str) {
        self.out.push_str("  ");
        self.out.push_str(s);
        self.out.push('\n');
    }
    fn label(&mut self, name: &str) {
        // 标签行(去掉 % 前缀)
        let _ = writeln!(self.out, "{}:", &name[1..]);
    }
    /// 槽位 off 的指针。
    fn slot_ptr(&mut self, off: usize) -> String {
        let p = self.val();
        self.emit(&format!("{} = getelementptr i8, ptr %frame, i64 {}", p, off));
        p
    }
    fn load_i64(&mut self, off: usize) -> String {
        let p = self.slot_ptr(off);
        let v = self.val();
        self.emit(&format!("{} = load i64, ptr {}", v, p));
        v
    }
    fn store_i64(&mut self, off: usize, val: &str) {
        let p = self.slot_ptr(off);
        self.emit(&format!("store i64 {}, ptr {}", val, p));
    }
    /// 把槽位当作 i32 取出(截断)。
    fn load_i32(&mut self, off: usize) -> String {
        let v64 = self.load_i64(off);
        let v = self.val();
        self.emit(&format!("{} = trunc i64 {} to i32", v, v64));
        v
    }
    /// 把 i32 结果符号扩展存回槽位。
    fn store_i32(&mut self, off: usize, val: &str) {
        let e = self.val();
        self.emit(&format!("{} = sext i32 {} to i64", e, val));
        self.store_i64(off, &e);
    }
    /// 把槽位当作 double 取出。
    fn load_f64(&mut self, off: usize) -> String {
        let b = self.load_i64(off);
        let v = self.val();
        self.emit(&format!("{} = bitcast i64 {} to double", v, b));
        v
    }
    fn store_f64(&mut self, off: usize, val: &str) {
        let b = self.val();
        self.emit(&format!("{} = bitcast double {} to i64", b, val));
        self.store_i64(off, &b);
    }
    /// 把槽位当作指针取出。
    fn load_ptr(&mut self, off: usize) -> String {
        let i = self.load_i64(off);
        let p = self.val();
        self.emit(&format!("{} = inttoptr i64 {} to ptr", p, i));
        p
    }
    fn store_ptr(&mut self, off: usize, ptr: &str) {
        let i = self.val();
        self.emit(&format!("{} = ptrtoint ptr {} to i64", i, ptr));
        self.store_i64(off, &i);
    }
    fn ensure_open(&mut self) {
        if !self.open {
            let l = self.syn();
            self.label(&l);
            self.open = true;
        }
    }
}

fn gen_func<'a>(
    func: &Function,
    floats: &'a [u64],
    out: &mut String,
    defined: &'a HashSet<&'a str>,
    declared: &'a mut HashSet<String>,
    decls: &'a mut String,
) -> Result<(), String> {
    // 不支持:结构体按值返回
    if func.ret_agg.is_some() {
        return Err(format!(
            "LLVM 后端暂不支持按值返回结构体(函数 {});请用原生后端",
            func.name
        ));
    }
    if func.variadic {
        return Err(format!(
            "LLVM 后端暂不支持用户自定义可变参数(函数 {});请用原生后端",
            func.name
        ));
    }

    let rty = ret_type(func);
    // 参数类型与名字
    let mut params = String::new();
    for (i, p) in func.params.iter().enumerate() {
        if p.is_aggregate {
            return Err(format!(
                "LLVM 后端暂不支持结构体按值传参(函数 {});请用原生后端",
                func.name
            ));
        }
        if i > 0 {
            params.push_str(", ");
        }
        let _ = write!(params, "{} %arg{}", if p.is_float { "double" } else { "i64" }, i);
    }

    let _ = writeln!(out, "\ndefine {} @{}({}) {{", rty, func.name, params);
    out.push_str("entry:\n");

    let mut g = Gen {
        out: String::new(),
        n: 0,
        syn: 0,
        open: true,
        floats,
        defined,
        declared,
        decls,
    };
    // 帧:对齐 16
    let frame = func.frame_bytes.max(8);
    g.emit(&format!("%frame = alloca [{} x i8], align 16", frame));
    // 形参落到各自槽位
    for (i, p) in func.params.iter().enumerate() {
        if p.is_float {
            g.store_f64(p.slot, &format!("%arg{}", i));
        } else {
            g.store_i64(p.slot, &format!("%arg{}", i));
        }
    }
    for instr in &func.body {
        gen_instr(instr, rty, &mut g)?;
    }
    // 函数末尾若仍未终结,补一个默认 return
    if g.open {
        match rty {
            "void" => g.emit("ret void"),
            "double" => g.emit("ret double 0.0"),
            "i32" => g.emit("ret i32 0"),
            _ => g.emit("ret i64 0"),
        }
    }

    out.push_str(&g.out);
    out.push_str("}\n");
    Ok(())
}

fn gen_instr(instr: &Instr, rty: &str, g: &mut Gen) -> Result<(), String> {
    match instr {
        Instr::Const { dst, value } => {
            g.ensure_open();
            g.store_i64(*dst, &value.to_string());
        }
        Instr::ConstF { dst, index } => {
            // double 的 64 位模式直接以 i64 存入槽位(BinF/Return 取用时 bitcast)
            g.ensure_open();
            let bits = g.floats[*index] as i64; // 以有符号 i64 打印同一 64 位模式
            g.store_i64(*dst, &bits.to_string());
        }
        Instr::Label(n) => {
            if g.open {
                g.emit(&format!("br label %L{}", n));
            }
            let l = format!("%L{}", n);
            g.label(&l);
            g.open = true;
        }
        Instr::Jump(n) => {
            g.ensure_open();
            g.emit(&format!("br label %L{}", n));
            g.open = false;
        }
        Instr::JumpIfZero { cond, target } => {
            g.ensure_open();
            let c = g.load_i32(*cond);
            let z = g.val();
            g.emit(&format!("{} = icmp eq i32 {}, 0", z, c));
            let cont = g.syn();
            g.emit(&format!("br i1 {}, label %L{}, label {}", z, target, cont));
            g.label(&cont);
            g.open = true;
        }
        Instr::Bin { dst, op, lhs, rhs } => {
            g.ensure_open();
            let a = g.load_i32(*lhs);
            let b = g.load_i32(*rhs);
            let r = int_binop(g, *op, &a, &b, "i32");
            g.store_i32(*dst, &r);
        }
        Instr::BinL { dst, op, lhs, rhs } => {
            g.ensure_open();
            let a = g.load_i64(*lhs);
            let b = g.load_i64(*rhs);
            if is_cmp(*op) {
                let r = int_cmp(g, *op, &a, &b, "i64"); // → i32 (0/1)
                g.store_i32(*dst, &r);
            } else {
                let r = int_binop(g, *op, &a, &b, "i64");
                g.store_i64(*dst, &r);
            }
        }
        Instr::Neg { dst, src } => {
            g.ensure_open();
            let v = g.load_i32(*src);
            let r = g.val();
            g.emit(&format!("{} = sub i32 0, {}", r, v));
            g.store_i32(*dst, &r);
        }
        Instr::NegL { dst, src } => {
            g.ensure_open();
            let v = g.load_i64(*src);
            let r = g.val();
            g.emit(&format!("{} = sub i64 0, {}", r, v));
            g.store_i64(*dst, &r);
        }
        Instr::Widen { dst, src } => {
            g.ensure_open();
            let v = g.load_i32(*src);
            let w = g.val();
            g.emit(&format!("{} = sext i32 {} to i64", w, v));
            g.store_i64(*dst, &w);
        }
        Instr::IntToFloat { dst, src } => {
            g.ensure_open();
            let v = g.load_i32(*src);
            let f = g.val();
            g.emit(&format!("{} = sitofp i32 {} to double", f, v));
            g.store_f64(*dst, &f);
        }
        Instr::LongToFloat { dst, src } => {
            g.ensure_open();
            let v = g.load_i64(*src);
            let f = g.val();
            g.emit(&format!("{} = sitofp i64 {} to double", f, v));
            g.store_f64(*dst, &f);
        }
        Instr::FloatToInt { dst, src } => {
            g.ensure_open();
            let v = g.load_f64(*src);
            let r = g.val();
            g.emit(&format!("{} = fptosi double {} to i32", r, v));
            g.store_i32(*dst, &r);
        }
        Instr::FloatToLong { dst, src } => {
            g.ensure_open();
            let v = g.load_f64(*src);
            let r = g.val();
            g.emit(&format!("{} = fptosi double {} to i64", r, v));
            g.store_i64(*dst, &r);
        }
        Instr::BinF { dst, op, lhs, rhs } => {
            g.ensure_open();
            let a = g.load_f64(*lhs);
            let b = g.load_f64(*rhs);
            if is_cmp(*op) {
                let pred = fcmp_pred(*op);
                let c = g.val();
                g.emit(&format!("{} = fcmp {} double {}, {}", c, pred, a, b));
                let r = g.val();
                g.emit(&format!("{} = zext i1 {} to i32", r, c));
                g.store_i32(*dst, &r);
            } else {
                let opn = match op {
                    BinOp::Add => "fadd",
                    BinOp::Sub => "fsub",
                    BinOp::Mul => "fmul",
                    BinOp::Div => "fdiv",
                    _ => return Err("LLVM: 非法浮点运算".into()),
                };
                let r = g.val();
                g.emit(&format!("{} = {} double {}, {}", r, opn, a, b));
                g.store_f64(*dst, &r);
            }
        }
        Instr::StrLit { dst, index } => {
            g.ensure_open();
            g.store_ptr(*dst, &format!("@.str.{}", index));
        }
        Instr::GlobalAddr { dst, name } => {
            g.ensure_open();
            g.store_ptr(*dst, &format!("@{}", name));
        }
        Instr::AddrOf { dst, off } => {
            g.ensure_open();
            let p = g.slot_ptr(*off);
            g.store_ptr(*dst, &p);
        }
        Instr::FieldAddr { dst, base, offset } => {
            g.ensure_open();
            let bp = g.load_ptr(*base);
            let p = g.val();
            g.emit(&format!("{} = getelementptr i8, ptr {}, i64 {}", p, bp, offset));
            g.store_ptr(*dst, &p);
        }
        Instr::PtrAdd { dst, base, index, size } => {
            g.ensure_open();
            ptr_arith(g, *dst, *base, *index, *size, false);
        }
        Instr::PtrSub { dst, base, index, size } => {
            g.ensure_open();
            ptr_arith(g, *dst, *base, *index, *size, true);
        }
        Instr::Copy { dst, src, width } => {
            g.ensure_open();
            if *width == 8 {
                let v = g.load_i64(*src);
                g.store_i64(*dst, &v);
            } else {
                let v = g.load_i32(*src);
                g.store_i32(*dst, &v);
            }
        }
        Instr::LoadInd { dst, addr, width, signed } => {
            g.ensure_open();
            let p = g.load_ptr(*addr);
            match *width {
                8 => {
                    let v = g.val();
                    g.emit(&format!("{} = load i64, ptr {}, align 1", v, p));
                    g.store_i64(*dst, &v);
                }
                1 => {
                    let v = g.val();
                    g.emit(&format!("{} = load i8, ptr {}, align 1", v, p));
                    let e = g.val();
                    let op = if *signed { "sext" } else { "zext" };
                    g.emit(&format!("{} = {} i8 {} to i32", e, op, v));
                    g.store_i32(*dst, &e);
                }
                _ => {
                    let v = g.val();
                    g.emit(&format!("{} = load i32, ptr {}, align 1", v, p));
                    g.store_i32(*dst, &v);
                }
            }
        }
        Instr::StoreInd { addr, src, width } => {
            g.ensure_open();
            let p = g.load_ptr(*addr);
            match *width {
                8 => {
                    let v = g.load_i64(*src);
                    g.emit(&format!("store i64 {}, ptr {}, align 1", v, p));
                }
                1 => {
                    let v = g.load_i32(*src);
                    let t = g.val();
                    g.emit(&format!("{} = trunc i32 {} to i8", t, v));
                    g.emit(&format!("store i8 {}, ptr {}, align 1", t, p));
                }
                _ => {
                    let v = g.load_i32(*src);
                    g.emit(&format!("store i32 {}, ptr {}, align 1", v, p));
                }
            }
        }
        Instr::MemCpy { dst, src, size } => {
            g.ensure_open();
            let d = g.load_ptr(*dst);
            let s = g.load_ptr(*src);
            g.emit(&format!(
                "call void @llvm.memcpy.p0.p0.i64(ptr {}, ptr {}, i64 {}, i1 false)",
                d, s, size
            ));
        }
        Instr::Return { src, is_float, width, agg } => {
            g.ensure_open();
            if agg.is_some() {
                return Err("LLVM: 结构体按值返回不支持".into());
            }
            match rty {
                "void" => g.emit("ret void"),
                "double" => {
                    let v = g.load_f64(*src);
                    g.emit(&format!("ret double {}", v));
                }
                "i32" => {
                    // main 或 int 返回:取 i32
                    let v = if *is_float {
                        let f = g.load_f64(*src);
                        let t = g.val();
                        g.emit(&format!("{} = fptosi double {} to i32", t, f));
                        t
                    } else {
                        g.load_i32(*src)
                    };
                    g.emit(&format!("ret i32 {}", v));
                }
                _ => {
                    // i64 返回
                    let v = if *width == 8 {
                        g.load_i64(*src)
                    } else {
                        let v32 = g.load_i32(*src);
                        let e = g.val();
                        g.emit(&format!("{} = sext i32 {} to i64", e, v32));
                        e
                    };
                    g.emit(&format!("ret i64 {}", v));
                }
            }
            g.open = false;
        }
        Instr::Call {
            dst,
            name,
            via,
            args,
            arg_floats,
            arg_aggs,
            ret_width,
            ret_agg,
            ret_buf: _,
            fixed,
            variadic,
            stack_varargs: _,
            ret_float,
        } => {
            g.ensure_open();
            if via.is_some() {
                return Err("LLVM: 函数指针间接调用不支持;请用原生后端".into());
            }
            if ret_agg.is_some() || arg_aggs.iter().any(|a| a.is_some()) {
                return Err("LLVM: 结构体按值传参/返回不支持;请用原生后端".into());
            }
            // 取实参(double 或 i64)
            let mut argvals: Vec<(String, String)> = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                if arg_floats.get(i).copied().unwrap_or(false) {
                    let v = g.load_f64(*a);
                    argvals.push(("double".to_string(), v));
                } else {
                    let v = g.load_i64(*a);
                    argvals.push(("i64".to_string(), v));
                }
            }
            // 调用结果类型
            let cret = if *ret_float {
                "double"
            } else if *ret_width == 8 {
                "i64"
            } else {
                "i32"
            };
            // 声明外部函数(非本程序定义的)
            if !g.defined.contains(name.as_str()) && !g.declared.contains(name) {
                g.declared.insert(name.clone());
                let mut ptys = String::new();
                for k in 0..*fixed.min(&args.len()) {
                    if k > 0 {
                        ptys.push_str(", ");
                    }
                    ptys.push_str(if argvals.get(k).map(|(t, _)| t.as_str()) == Some("double") {
                        "double"
                    } else {
                        "i64"
                    });
                }
                if *variadic {
                    if !ptys.is_empty() {
                        ptys.push_str(", ");
                    }
                    ptys.push_str("...");
                }
                let _ = writeln!(g.decls, "declare {} @{}({})", cret, name, ptys);
            }
            // 组装调用
            let arglist = argvals
                .iter()
                .map(|(t, v)| format!("{} {}", t, v))
                .collect::<Vec<_>>()
                .join(", ");
            let callee_ty = if *variadic {
                // 变参需显式函数类型
                let mut fixed_tys = String::new();
                for k in 0..*fixed.min(&args.len()) {
                    if k > 0 {
                        fixed_tys.push_str(", ");
                    }
                    fixed_tys.push_str(
                        if argvals.get(k).map(|(t, _)| t.as_str()) == Some("double") {
                            "double"
                        } else {
                            "i64"
                        },
                    );
                }
                if !fixed_tys.is_empty() {
                    fixed_tys.push_str(", ");
                }
                format!("{} ({}...)", cret, fixed_tys)
            } else {
                String::new()
            };
            if cret == "void" {
                g.emit(&format!("call void @{}({})", name, arglist));
            } else {
                let r = g.val();
                if *variadic {
                    g.emit(&format!("{} = call {} @{}({})", r, callee_ty, name, arglist));
                } else {
                    g.emit(&format!("{} = call {} @{}({})", r, cret, name, arglist));
                }
                // 存结果
                match cret {
                    "double" => g.store_f64(*dst, &r),
                    "i64" => g.store_i64(*dst, &r),
                    _ => g.store_i32(*dst, &r),
                }
            }
        }
        Instr::FuncAddr { .. } => {
            return Err("LLVM: 函数指针(取函数地址)不支持;请用原生后端".into())
        }
        Instr::VaStart { .. } | Instr::VaArg { .. } => {
            return Err("LLVM: 用户可变参数不支持;请用原生后端".into())
        }
    }
    Ok(())
}

fn is_cmp(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge | BinOp::Eq | BinOp::Ne
    )
}

fn int_binop(g: &mut Gen, op: BinOp, a: &str, b: &str, ty: &str) -> String {
    if is_cmp(op) {
        return int_cmp(g, op, a, b, ty);
    }
    let opn = match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::Div => "sdiv",
        BinOp::Mod => "srem",
        BinOp::BitAnd => "and",
        BinOp::BitOr => "or",
        BinOp::BitXor => "xor",
        BinOp::Shl => "shl",
        BinOp::Shr => "ashr",
        _ => unreachable!(),
    };
    let r = g.val();
    g.emit(&format!("{} = {} {} {}, {}", r, opn, ty, a, b));
    r
}

fn int_cmp(g: &mut Gen, op: BinOp, a: &str, b: &str, ty: &str) -> String {
    let pred = match op {
        BinOp::Lt => "slt",
        BinOp::Gt => "sgt",
        BinOp::Le => "sle",
        BinOp::Ge => "sge",
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        _ => unreachable!(),
    };
    let c = g.val();
    g.emit(&format!("{} = icmp {} {} {}, {}", c, pred, ty, a, b));
    let r = g.val();
    g.emit(&format!("{} = zext i1 {} to i32", r, c));
    r
}

fn fcmp_pred(op: BinOp) -> &'static str {
    // 有序比较(NaN 视为不满足),与原生后端 ucomisd/fcmp 行为足够接近
    match op {
        BinOp::Lt => "olt",
        BinOp::Gt => "ogt",
        BinOp::Le => "ole",
        BinOp::Ge => "oge",
        BinOp::Eq => "oeq",
        BinOp::Ne => "une",
        _ => unreachable!(),
    }
}

fn ptr_arith(g: &mut Gen, dst: usize, base: usize, index: usize, size: usize, sub: bool) {
    let bp = g.load_ptr(base);
    let idx32 = g.load_i32(index);
    let idx = g.val();
    g.emit(&format!("{} = sext i32 {} to i64", idx, idx32));
    let off = g.val();
    g.emit(&format!("{} = mul i64 {}, {}", off, idx, size));
    let signed_off = if sub {
        let s = g.val();
        g.emit(&format!("{} = sub i64 0, {}", s, off));
        s
    } else {
        off
    };
    let p = g.val();
    g.emit(&format!("{} = getelementptr i8, ptr {}, i64 {}", p, bp, signed_off));
    g.store_ptr(dst, &p);
}

#[cfg(test)]
mod tests {
    use crate::{compile_to_asm_target, Target};

    fn llvm(src: &str) -> Result<String, String> {
        let tokens = crate::lexer::lex(src).unwrap();
        let ast = crate::parser::parse(&tokens).unwrap();
        let ir = crate::ir::lower(&ast);
        super::generate(&ir)
    }

    #[test]
    fn emits_valid_module_header_and_define() {
        let ir = compile_to_asm_target("int main(){ return 42; }", Target::Llvm).unwrap();
        assert!(ir.starts_with("target triple"));
        assert!(ir.contains("define i32 @main()"));
        assert!(ir.contains("alloca"));
        assert!(ir.contains("ret i32"));
    }

    #[test]
    fn declares_external_printf_as_variadic() {
        let ir = llvm("int printf(char*, ...); int main(){ printf(\"hi\"); return 0; }").unwrap();
        assert!(ir.contains("declare i32 @printf(i64, ...)"));
    }

    #[test]
    fn struct_byval_param_is_unsupported() {
        let r = llvm("struct P{int x;}; int f(struct P p){ return p.x; } int main(){ return 0; }");
        assert!(r.is_err());
    }

    #[test]
    fn function_pointer_is_unsupported() {
        let r = llvm("int g(int x){return x;} int main(){ int(*f)(int)=g; return f(1); }");
        assert!(r.is_err());
    }

    #[test]
    fn user_varargs_is_unsupported() {
        let r = llvm("int s(int n, ...){ va_list ap; va_start(ap,n); va_end(ap); return 0; } int main(){ return 0; }");
        assert!(r.is_err());
    }
}
