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
    out.push_str("%struct.__va_list_tag = type { i32, i32, ptr, ptr }\n");
    out.push_str("declare void @llvm.memcpy.p0.p0.i64(ptr, ptr, i64, i1)\n");
    out.push_str("declare void @llvm.va_start(ptr)\n");
    out.push_str("declare void @llvm.va_end(ptr)\n");

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
    // 全局变量:统一用 [N x i8] 字节镜像
    for g in &program.globals {
        if g.is_extern {
            // extern:声明外部符号,由链接器解析(如 stdin/stdout/stderr)
            let _ = writeln!(out, "@{} = external global [{} x i8]", g.name, g.size.max(1));
            continue;
        }
        match &g.init {
            Some(bytes) => {
                let mut s = String::new();
                for b in bytes {
                    let _ = write!(s, "\\{:02X}", b);
                }
                let _ = writeln!(
                    out,
                    "@{} = global [{} x i8] c\"{}\"",
                    g.name,
                    bytes.len(),
                    s
                );
            }
            None => {
                let _ = writeln!(out, "@{} = global [{} x i8] zeroinitializer", g.name, g.size.max(1));
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

/// 返回类型的 LLVM 文本:`[N x i8]`(结构体) / double / i64 / i32(main) / void。
fn ret_type(f: &Function) -> String {
    if let Some(sz) = f.ret_agg {
        return format!("[{} x i8]", sz);
    }
    if f.name == "main" {
        return "i32".into(); // main 按 C 约定返回 i32
    }
    if f.ret_float {
        return "double".into();
    }
    let has_ret = f.body.iter().any(|i| matches!(i, Instr::Return { .. }));
    if has_ret {
        "i64".into()
    } else {
        "void".into()
    }
}

/// 标量/聚合形参或实参的 LLVM 类型:double / `[N x i8]`(结构体) / i64。
fn scalar_or_agg_ty(is_float: bool, agg: Option<usize>) -> String {
    if let Some(sz) = agg {
        format!("[{} x i8]", sz)
    } else if is_float {
        "double".into()
    } else {
        "i64".into()
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
    let rty = ret_type(func);
    // 参数类型与名字
    let mut params = String::new();
    for (i, p) in func.params.iter().enumerate() {
        if i > 0 {
            params.push_str(", ");
        }
        let pty = scalar_or_agg_ty(p.is_float, if p.is_aggregate { Some(p.size) } else { None });
        let _ = write!(params, "{} %arg{}", pty, i);
    }
    if func.variadic {
        if !params.is_empty() {
            params.push_str(", ");
        }
        params.push_str("...");
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
        if p.is_aggregate {
            let pp = g.slot_ptr(p.slot);
            g.emit(&format!("store [{} x i8] %arg{}, ptr {}", p.size, i, pp));
        } else if p.is_float {
            g.store_f64(p.slot, &format!("%arg{}", i));
        } else {
            g.store_i64(p.slot, &format!("%arg{}", i));
        }
    }
    for instr in &func.body {
        gen_instr(instr, &rty, &mut g)?;
    }
    // 函数末尾若仍未终结,补一个默认 return
    if g.open {
        match rty.as_str() {
            "void" => g.emit("ret void"),
            "double" => g.emit("ret double 0.0"),
            "i32" => g.emit("ret i32 0"),
            "i64" => g.emit("ret i64 0"),
            other => g.emit(&format!("ret {} zeroinitializer", other)), // 结构体
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
        Instr::WidenU { dst, src } => {
            g.ensure_open();
            let v = g.load_i32(*src);
            let w = g.val();
            g.emit(&format!("{} = zext i32 {} to i64", w, v));
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
            if let Some(sz) = agg {
                // 结构体按值返回:src 存结构体地址,载入聚合值返回
                let p = g.load_ptr(*src);
                let v = g.val();
                g.emit(&format!("{} = load [{} x i8], ptr {}", v, sz, p));
                g.emit(&format!("ret [{} x i8] {}", sz, v));
                g.open = false;
                return Ok(());
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
            ret_buf,
            fixed,
            variadic,
            stack_varargs: _,
            ret_float,
        } => {
            g.ensure_open();
            // 实参:结构体 [N x i8](从地址载入聚合值)/ double / i64
            let mut argvals: Vec<(String, String)> = Vec::with_capacity(args.len());
            for (i, a) in args.iter().enumerate() {
                if let Some(sz) = arg_aggs.get(i).copied().flatten() {
                    let p = g.load_ptr(*a); // 实参 temp 存结构体地址
                    let v = g.val();
                    g.emit(&format!("{} = load [{} x i8], ptr {}", v, sz, p));
                    argvals.push((format!("[{} x i8]", sz), v));
                } else if arg_floats.get(i).copied().unwrap_or(false) {
                    let v = g.load_f64(*a);
                    argvals.push(("double".to_string(), v));
                } else {
                    let v = g.load_i64(*a);
                    argvals.push(("i64".to_string(), v));
                }
            }
            // 调用结果类型
            let cret: String = if let Some(sz) = ret_agg {
                format!("[{} x i8]", sz)
            } else if *ret_float {
                "double".into()
            } else if *ret_width == 8 {
                "i64".into()
            } else {
                "i32".into()
            };
            // 间接调用:取函数指针;直接调用:@name(必要时声明外部函数)
            let callee = if let Some(t) = via {
                g.load_ptr(*t)
            } else {
                if !g.defined.contains(name.as_str()) && !g.declared.contains(name) {
                    g.declared.insert(name.clone());
                    let mut ptys = String::new();
                    for k in 0..*fixed.min(&args.len()) {
                        if k > 0 {
                            ptys.push_str(", ");
                        }
                        ptys.push_str(argvals.get(k).map(|(t, _)| t.as_str()).unwrap_or("i64"));
                    }
                    if *variadic {
                        if !ptys.is_empty() {
                            ptys.push_str(", ");
                        }
                        ptys.push_str("...");
                    }
                    let _ = writeln!(g.decls, "declare {} @{}({})", cret, name, ptys);
                }
                format!("@{}", name)
            };
            let arglist = argvals
                .iter()
                .map(|(t, v)| format!("{} {}", t, v))
                .collect::<Vec<_>>()
                .join(", ");
            // 变参调用需显式函数类型 `<ret> (<fixed...>, ...)`
            let head = if *variadic {
                let mut fixed_tys = String::new();
                for k in 0..*fixed.min(&args.len()) {
                    if k > 0 {
                        fixed_tys.push_str(", ");
                    }
                    fixed_tys.push_str(argvals.get(k).map(|(t, _)| t.as_str()).unwrap_or("i64"));
                }
                if !fixed_tys.is_empty() {
                    fixed_tys.push_str(", ");
                }
                format!("{} ({}...)", cret, fixed_tys)
            } else {
                cret.clone()
            };
            if cret == "void" {
                g.emit(&format!("call {} {}({})", head, callee, arglist));
            } else {
                let r = g.val();
                g.emit(&format!("{} = call {} {}({})", r, head, callee, arglist));
                if let Some(sz) = ret_agg {
                    // 结构体返回值写入缓冲区(ret_buf)
                    if let Some(buf) = ret_buf {
                        let p = g.slot_ptr(*buf);
                        g.emit(&format!("store [{} x i8] {}, ptr {}", sz, r, p));
                    }
                } else {
                    match cret.as_str() {
                        "double" => g.store_f64(*dst, &r),
                        "i64" => g.store_i64(*dst, &r),
                        _ => g.store_i32(*dst, &r),
                    }
                }
            }
        }
        Instr::FuncAddr { dst, name } => {
            g.ensure_open();
            // 取函数地址 → 函数指针(以 i64 存槽位)
            g.store_ptr(*dst, &format!("@{}", name));
        }
        Instr::VaStart { ap } => {
            g.ensure_open();
            // 分配真正的 LLVM va_list,va_start,然后把其地址存入 anvil 的 va_list 变量槽
            let vl = g.val();
            g.emit(&format!("{} = alloca %struct.__va_list_tag", vl));
            g.emit(&format!("call void @llvm.va_start(ptr {})", vl));
            let apaddr = g.load_ptr(*ap); // ap 存的是 va_list 变量地址
            let vlint = g.val();
            g.emit(&format!("{} = ptrtoint ptr {} to i64", vlint, vl));
            g.emit(&format!("store i64 {}, ptr {}", vlint, apaddr));
        }
        Instr::VaArg { dst, ap, width, is_float } => {
            g.ensure_open();
            let apaddr = g.load_ptr(*ap);
            let vlint = g.val();
            g.emit(&format!("{} = load i64, ptr {}", vlint, apaddr));
            let vl = g.val();
            g.emit(&format!("{} = inttoptr i64 {} to ptr", vl, vlint));
            if *is_float {
                let v = g.val();
                g.emit(&format!("{} = va_arg ptr {}, double", v, vl));
                g.store_f64(*dst, &v);
            } else if *width == 8 {
                let v = g.val();
                g.emit(&format!("{} = va_arg ptr {}, i64", v, vl));
                g.store_i64(*dst, &v);
            } else {
                let v = g.val();
                g.emit(&format!("{} = va_arg ptr {}, i32", v, vl));
                g.store_i32(*dst, &v);
            }
        }
    }
    Ok(())
}

fn is_cmp(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::Lt
            | BinOp::Gt
            | BinOp::Le
            | BinOp::Ge
            | BinOp::Eq
            | BinOp::Ne
            | BinOp::ULt
            | BinOp::UGt
            | BinOp::ULe
            | BinOp::UGe
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
        BinOp::UDiv => "udiv",
        BinOp::UMod => "urem",
        BinOp::UShr => "lshr",
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
        BinOp::ULt => "ult",
        BinOp::UGt => "ugt",
        BinOp::ULe => "ule",
        BinOp::UGe => "uge",
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
    fn struct_byval_uses_byte_array_type() {
        let ir = llvm("struct P{int x;int y;}; int f(struct P p){ return p.x; } int main(){ struct P q; q.x=1; q.y=2; return f(q); }").unwrap();
        assert!(ir.contains("[16 x i8]")); // 16 字节结构体按值用 [16 x i8]
    }

    #[test]
    fn function_pointer_indirect_call() {
        let ir = llvm("int g(int x){return x;} int main(){ int(*f)(int)=g; return f(1); }").unwrap();
        assert!(ir.contains("ptrtoint ptr @g")); // 取函数地址
        assert!(ir.contains("call i32 %")); // 经函数指针的间接调用(g 返回 int)
    }

    #[test]
    fn user_varargs_uses_llvm_va() {
        let ir = llvm("int s(int n, ...){ va_list ap; va_start(ap,n); int v=va_arg(ap,int); va_end(ap); return v; } int main(){ return s(1, 42); }").unwrap();
        assert!(ir.contains("define i64 @s(i64 %arg0, ...)")); // 变参函数定义
        assert!(ir.contains("@llvm.va_start"));
        assert!(ir.contains("va_arg ptr"));
    }
}
