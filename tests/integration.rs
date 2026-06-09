use std::process::Command;

/// 用 anvil 编译 `src`，运行产物，返回其退出码。
fn compile_and_run(src: &str, name: &str) -> i32 {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");

    let bin = env!("CARGO_BIN_EXE_anvil");
    let compile = Command::new(bin)
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run anvil");
    assert!(compile.success(), "anvil failed to compile {}", name);

    let run = Command::new(&exe_path).status().expect("run compiled exe");
    run.code().expect("program terminated by signal")
}

/// 编译 `src`，运行，返回 (退出码, stdout)。
fn compile_run_capture(src: &str, name: &str) -> (i32, String) {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");
    let bin = env!("CARGO_BIN_EXE_anvil");
    let compile = Command::new(bin)
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run anvil");
    assert!(compile.success(), "anvil failed to compile {}", name);
    let out = Command::new(&exe_path).output().expect("run compiled exe");
    let code = out.status.code().expect("terminated by signal");
    (code, String::from_utf8_lossy(&out.stdout).to_string())
}

#[test]
fn m0_return_42() {
    assert_eq!(compile_and_run("int main(){ return 42; }", "m0_return_42"), 42);
}

#[test]
fn m0_return_0() {
    assert_eq!(compile_and_run("int main(){ return 0; }", "m0_return_0"), 0);
}

#[test]
fn m1_precedence() {
    // 1 + 2*3 - (4/2) = 1 + 6 - 2 = 5
    assert_eq!(compile_and_run("int main(){ return 1+2*3-(4/2); }", "m1_prec"), 5);
}

#[test]
fn m1_left_assoc() {
    // 20 - 5 - 3 = 12
    assert_eq!(compile_and_run("int main(){ return 20-5-3; }", "m1_lassoc"), 12);
}

#[test]
fn m1_modulo() {
    // 17 % 5 = 2
    assert_eq!(compile_and_run("int main(){ return 17%5; }", "m1_mod"), 2);
}

#[test]
fn m1_unary_neg_in_expr() {
    // 10 + -3 = 7
    assert_eq!(compile_and_run("int main(){ return 10 + -3; }", "m1_neg"), 7);
}

#[test]
fn m1_parens_nested() {
    // ((2+3)*4) % 7 = 20 % 7 = 6
    assert_eq!(compile_and_run("int main(){ return ((2+3)*4)%7; }", "m1_nested"), 6);
}

#[test]
fn m2_local_var() {
    assert_eq!(
        compile_and_run("int main(){ int x = 7; int y = 6; return x*y; }", "m2_var"),
        42
    );
}

#[test]
fn m2_if_else() {
    assert_eq!(
        compile_and_run("int main(){ int x = 5; if (x > 3) return 1; else return 0; }", "m2_if"),
        1
    );
}

#[test]
fn m2_while_sum() {
    // 1+2+...+10 = 55
    assert_eq!(
        compile_and_run(
            "int main(){ int s=0; int i=1; while (i<=10) { s=s+i; i=i+1; } return s; }",
            "m2_while"
        ),
        55
    );
}

#[test]
fn m2_for_factorial() {
    // 5! = 120
    assert_eq!(
        compile_and_run(
            "int main(){ int r=1; for (int i=1; i<=5; i=i+1) r=r*i; return r; }",
            "m2_for"
        ),
        120
    );
}

#[test]
fn m2_assignment_value() {
    // 赋值表达式求值为所赋值
    assert_eq!(
        compile_and_run("int main(){ int x; int y = (x = 9); return y; }", "m2_assign_val"),
        9
    );
}

#[test]
fn m2_equality() {
    assert_eq!(compile_and_run("int main(){ int x = 4; return x == 4; }", "m2_eq"), 1);
    assert_eq!(compile_and_run("int main(){ int x = 4; return x != 4; }", "m2_ne"), 0);
}

#[test]
fn m3_recursion_factorial() {
    assert_eq!(
        compile_and_run(
            "int fact(int n){ if (n <= 1) return 1; return n * fact(n-1); } int main(){ return fact(5); }",
            "m3_fact"
        ),
        120
    );
}

#[test]
fn m3_multiple_functions() {
    assert_eq!(
        compile_and_run(
            "int add(int a, int b){ return a+b; } int main(){ return add(40, 2); }",
            "m3_add"
        ),
        42
    );
}

#[test]
fn m3_recursion_fib() {
    // fib(10) = 55
    assert_eq!(
        compile_and_run(
            "int fib(int n){ if (n < 2) return n; return fib(n-1) + fib(n-2); } int main(){ return fib(10); }",
            "m3_fib"
        ),
        55
    );
}

#[test]
fn m3_hello_world() {
    let (code, stdout) = compile_run_capture(
        "int main(){ puts(\"Hello, World!\"); return 0; }",
        "m3_hello",
    );
    assert_eq!(code, 0);
    assert_eq!(stdout, "Hello, World!\n"); // puts 追加换行
}

#[test]
fn m4_pointer_store() {
    assert_eq!(
        compile_and_run("int main(){ int x=5; int* p=&x; *p=9; return x; }", "m4_ptr"),
        9
    );
}

#[test]
fn m4_array_sum() {
    assert_eq!(
        compile_and_run(
            "int main(){ int a[3]; a[0]=10; a[1]=20; a[2]=12; int s=0; for(int i=0;i<3;i=i+1) s=s+a[i]; return s; }",
            "m4_arr"
        ),
        42
    );
}

#[test]
fn m4_sizeof() {
    assert_eq!(
        compile_and_run("int main(){ return sizeof(int)+sizeof(char)+sizeof(int*); }", "m4_sizeof"),
        13
    );
}

#[test]
fn m4_strlen_via_pointer() {
    assert_eq!(
        compile_and_run(
            "int main(){ char* s=\"abcd\"; int n=0; while(*s != 0){ n=n+1; s=s+1; } return n; }",
            "m4_strlen"
        ),
        4
    );
}

#[test]
fn m4_pointer_param() {
    assert_eq!(
        compile_and_run(
            "int set9(int* p){ *p = 9; return 0; } int main(){ int x = 1; set9(&x); return x; }",
            "m4_ptr_param"
        ),
        9
    );
}

#[test]
fn m5_struct_members() {
    assert_eq!(
        compile_and_run("struct P { int x; int y; }; int main(){ struct P p; p.x=40; p.y=2; return p.x+p.y; }", "m5_struct"),
        42
    );
}

#[test]
fn m5_struct_pointer_arrow() {
    assert_eq!(
        compile_and_run("struct P { int x; int y; }; int gy(struct P* q){ return q->y; } int main(){ struct P p; p.y=7; return gy(&p); }", "m5_arrow"),
        7
    );
}

#[test]
fn m5_nested_struct() {
    assert_eq!(
        compile_and_run("struct I { int v; }; struct O { int a; struct I in; }; int main(){ struct O o; o.in.v=9; return o.in.v; }", "m5_nested"),
        9
    );
}

#[test]
fn m5_enum() {
    assert_eq!(
        compile_and_run("enum E { A, B=5, C }; int main(){ return A + B + C; }", "m5_enum"),
        11
    );
}

#[test]
fn m5_typedef_struct() {
    assert_eq!(
        compile_and_run("typedef struct { int x; int y; } Pt; int main(){ Pt p; p.x=10; p.y=32; return p.x+p.y; }", "m5_typedef"),
        42
    );
}

#[test]
fn m5_union() {
    assert_eq!(
        compile_and_run("union U { int i; char c; }; int main(){ union U u; u.i=65; return u.c; }", "m5_union"),
        65
    );
}

#[test]
fn m6_object_macro() {
    assert_eq!(compile_and_run("#define N 42\nint main(){ return N; }", "m6_obj"), 42);
}

#[test]
fn m6_function_macro() {
    assert_eq!(
        compile_and_run("#define ADD(a,b) ((a)+(b))\nint main(){ return ADD(40, 2); }", "m6_func"),
        42
    );
}

#[test]
fn m6_conditional() {
    assert_eq!(
        compile_and_run(
            "#define MAX 5\n#if MAX > 3\nint main(){ return 7; }\n#else\nint main(){ return 0; }\n#endif",
            "m6_cond"
        ),
        7
    );
}

#[test]
fn m6_ifndef_guard() {
    // 经典 include guard 风格：定义后再 ifdef 不应重复
    assert_eq!(
        compile_and_run("#ifndef X\n#define X\nint main(){ return 5; }\n#endif", "m6_guard"),
        5
    );
}

#[test]
fn m6_include() {
    // #include 解析相对 .c 文件所在目录；把头文件也写到 temp_dir
    let dir = std::env::temp_dir();
    std::fs::write(dir.join("m6_hdr.h"), "#define SECRET 42\nint helper(){ return SECRET; }\n")
        .expect("write header");
    assert_eq!(
        compile_and_run("#include \"m6_hdr.h\"\nint main(){ return helper(); }", "m6_use_include"),
        42
    );
}

#[test]
fn m7_printf_int() {
    let (code, out) = compile_run_capture(
        "int printf(const char*, ...); int main(){ printf(\"v=%d\\n\", 42); return 0; }",
        "m7_printf_int",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "v=42\n");
}

#[test]
fn m7_printf_multi() {
    let (_c, out) = compile_run_capture(
        "int printf(const char*, ...); int main(){ printf(\"%d %d %d\\n\", 11, 22, 33); return 0; }",
        "m7_printf_multi",
    );
    assert_eq!(out, "11 22 33\n");
}

#[test]
fn m7_printf_string() {
    let (_c, out) = compile_run_capture(
        "int printf(const char*, ...); int main(){ printf(\"hi %s!\\n\", \"bob\"); return 0; }",
        "m7_printf_string",
    );
    assert_eq!(out, "hi bob!\n");
}

#[test]
fn m7_malloc_pointer_return() {
    let (code, out) = compile_run_capture(
        "void* malloc(int); int printf(const char*, ...); int main(){ char* p = malloc(8); p[0]=65; p[1]=66; p[2]=0; printf(\"%s\\n\", p); return 0; }",
        "m7_malloc",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "AB\n");
}

#[test]
fn m7_fixed_arg_libc_still_works() {
    // abs 返回 int，固定参数
    assert_eq!(
        compile_and_run("int abs(int); int main(){ return abs(0-9); }", "m7_abs"),
        9
    );
}

#[test]
fn m8_logical_short_circuit() {
    assert_eq!(compile_and_run("int main(){ return (1 && 1) + (0 || 1); }", "m8_logic"), 2);
}

#[test]
fn m8_bitwise() {
    assert_eq!(
        compile_and_run("int main(){ return (6 & 3) + (5 | 2) + (5 ^ 1) + (1 << 4) + (64 >> 2); }", "m8_bit"),
        2 + 7 + 4 + 16 + 16
    );
}

#[test]
fn m8_not_and_bitnot() {
    // !0=1, !5=0, (~0 & 1)=1  => 2
    assert_eq!(compile_and_run("int main(){ return !0 + !5 + (~0 & 1); }", "m8_not"), 2);
}

#[test]
fn m8_ternary() {
    assert_eq!(compile_and_run("int main(){ int x=7; return x > 5 ? 100 : 200; }", "m8_tern"), 100);
}

#[test]
fn m8_incdec_and_compound() {
    assert_eq!(
        compile_and_run("int main(){ int s=0; for(int i=0;i<5;i++){ s += i; } return s; }", "m8_incr"),
        10
    );
}

#[test]
fn m9_break_in_for() {
    assert_eq!(
        compile_and_run("int main(){ int s=0; for(int i=0;i<10;i++){ if(i==5) break; s+=i; } return s; }", "m9_break"),
        10
    );
}

#[test]
fn m9_continue_in_for() {
    assert_eq!(
        compile_and_run("int main(){ int s=0; for(int i=0;i<5;i++){ if(i==2) continue; s+=i; } return s; }", "m9_cont"),
        8
    );
}

#[test]
fn m9_switch_break() {
    assert_eq!(
        compile_and_run("int main(){ int x=2; int r=0; switch(x){ case 1: r=10; break; case 2: r=20; break; default: r=99; } return r; }", "m9_switch"),
        20
    );
}

#[test]
fn m9_switch_default() {
    assert_eq!(
        compile_and_run("int main(){ int x=7; int r=0; switch(x){ case 1: r=10; break; default: r=99; } return r; }", "m9_default"),
        99
    );
}

#[test]
fn m9_switch_fallthrough() {
    assert_eq!(
        compile_and_run("int main(){ int r=0; switch(1){ case 1: r+=1; case 2: r+=10; break; case 3: r+=100; } return r; }", "m9_fall"),
        11
    );
}

#[test]
fn m9_while_break() {
    assert_eq!(
        compile_and_run("int main(){ int i=0; while(1){ i++; if(i>=42) break; } return i; }", "m9_while_break"),
        42
    );
}

#[test]
fn m10_global_var() {
    assert_eq!(
        compile_and_run("int counter = 10; int bump(){ counter = counter + 1; return counter; } int main(){ bump(); bump(); return counter; }", "m10_glob"),
        12
    );
}

#[test]
fn m10_global_array() {
    assert_eq!(
        compile_and_run("int arr[3]; int main(){ arr[0]=10; arr[1]=20; arr[2]=12; return arr[0]+arr[1]+arr[2]; }", "m10_garr"),
        42
    );
}

#[test]
fn m10_many_args() {
    assert_eq!(
        compile_and_run("int sum10(int a,int b,int c,int d,int e,int f,int g,int h,int i,int j){ return a+b+c+d+e+f+g+h+i+j; } int main(){ return sum10(1,2,3,4,5,6,7,8,9,10); }", "m10_args"),
        55
    );
}

#[test]
fn m10_char_literal() {
    assert_eq!(compile_and_run("int main(){ return 'A'; }", "m10_charlit"), 65);
}

#[test]
fn m10_system_header_printf() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ printf(\"x=%d\\n\", 99); return 0; }",
        "m10_sysh",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "x=99\n");
}

#[test]
fn m10_realistic_program() {
    // 综合：系统头 + 全局 + 循环 + printf + malloc + 字符字面量
    let src = "#include <stdio.h>\n#include <stdlib.h>\nint total = 0;\nint add(int x){ total = total + x; return total; }\nint main(){ for (int i = 1; i <= 5; i++) add(i); char* p = malloc(4); p[0]='O'; p[1]='K'; p[2]=0; printf(\"%s %d\\n\", p, total); return total; }";
    let (code, out) = compile_run_capture(src, "m10_real");
    assert_eq!(code, 15);
    assert_eq!(out, "OK 15\n");
}

#[test]
fn m11_double_arithmetic() {
    // 3.5 + 2.0 = 5.5, 截断为 int = 5
    assert_eq!(compile_and_run("int main(){ double x=3.5; double y=2.0; return x+y; }", "m11_dadd"), 5);
    assert_eq!(compile_and_run("int main(){ return 3.0 * 4.0; }", "m11_dmul"), 12);
}

#[test]
fn m11_double_compare() {
    assert_eq!(compile_and_run("int main(){ return 7.0 / 2.0 > 3.0; }", "m11_dcmp"), 1);
}

#[test]
fn m11_int_double_promote() {
    assert_eq!(compile_and_run("int main(){ int n=5; double h = n / 2.0; return h > 2.0; }", "m11_promote"), 1);
}

#[test]
fn m11_printf_float() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ printf(\"pi=%f\\n\", 3.14159); return 0; }",
        "m11_pf",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "pi=3.141590\n");
}

#[test]
fn m11_double_return() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\ndouble half(int x){ return x / 2.0; }\nint main(){ double h = half(9); printf(\"%f\\n\", h); return h > 4.0; }",
        "m11_dret",
    );
    assert_eq!(code, 1);
    assert_eq!(out, "4.500000\n");
}

// ---- M12: double 参数 + 结构体按值传参/返回（System V ABI）----

#[test]
fn m12_double_params_add() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\ndouble add(double a, double b){ return a + b; }\nint main(){ printf(\"%f\\n\", add(1.5, 2.25)); return 0; }",
        "m12_dadd",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "3.750000\n");
}

#[test]
fn m12_double_params_mixed_int_double() {
    // 整型与浮点形参各走独立寄存器组；int 实参隐式转 double
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\ndouble f(int n, double x){ return n * x; }\nint main(){ printf(\"%f\\n\", f(3, 1.5)); return 0; }",
        "m12_dmix",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "4.500000\n");
}

#[test]
fn m12_int_arg_coerced_to_double_param() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\ndouble half(double x){ return x / 2; }\nint main(){ printf(\"%f\\n\", half(9)); return 0; }",
        "m12_coerce",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "4.500000\n");
}

#[test]
fn m12_many_double_params() {
    // 9 个 double：前 8 个进 xmm0-7，第 9 个溢出到栈
    let src = "double s(double a,double b,double c,double d,double e,double f,double g,double h,double i){ return a+b+c+d+e+f+g+h+i; }\nint main(){ int r = s(1,2,3,4,5,6,7,8,9); return r; }";
    assert_eq!(compile_and_run(src, "m12_many_d"), 45);
}

#[test]
fn m12_struct_byval_arg_small() {
    // ≤16 字节结构体经整型寄存器传参
    let src = "struct P { int x; int y; }; int sum(struct P p){ return p.x + p.y; }\nint main(){ struct P q; q.x = 10; q.y = 20; return sum(q); }";
    assert_eq!(compile_and_run(src, "m12_s_small"), 30);
}

#[test]
fn m12_struct_return_small() {
    // ≤16 字节结构体经 rax:rdx 返回
    let src = "struct P { int x; int y; }; struct P make(int a, int b){ struct P p; p.x = a; p.y = b; return p; }\nint main(){ struct P q = make(7, 8); return q.x + q.y; }";
    assert_eq!(compile_and_run(src, "m12_s_ret"), 15);
}

#[test]
fn m12_struct_byval_arg_large() {
    // >16 字节结构体经栈传参
    let src = "struct Big { int a; int b; int c; int d; int e; }; int sum(struct Big b){ return b.a+b.b+b.c+b.d+b.e; }\nint main(){ struct Big g; g.a=1; g.b=2; g.c=3; g.d=4; g.e=5; return sum(g); }";
    assert_eq!(compile_and_run(src, "m12_s_big"), 15);
}

#[test]
fn m12_struct_return_large_sret() {
    // >16 字节结构体经隐式指针(sret)返回
    let src = "struct Big { int a; int b; int c; int d; int e; }; struct Big mk(int base){ struct Big b; b.a=base; b.b=base+1; b.c=base+2; b.d=base+3; b.e=base+4; return b; }\nint main(){ struct Big g = mk(100); return g.a + g.e; }";
    assert_eq!(compile_and_run(src, "m12_s_bigret"), 204);
}

#[test]
fn m12_struct_byval_semantics() {
    // 按值传参：被调方修改不影响调用方副本（返回调用方的 r.x，应仍为 5）
    let src = "struct P { int x; int y; }; int mutate(struct P p){ p.x = 999; return p.x; }\nint main(){ struct P r; r.x = 5; r.y = 6; mutate(r); return r.x; }";
    assert_eq!(compile_and_run(src, "m12_s_sem"), 5);
}

#[test]
fn m12_struct_roundtrip() {
    // 返回的结构体直接作为另一个函数的按值实参
    let src = "struct P { int x; int y; }; struct P make(int a,int b){ struct P p; p.x=a; p.y=b; return p; } int sum(struct P p){ return p.x+p.y; }\nint main(){ return sum(make(40, 2)); }";
    assert_eq!(compile_and_run(src, "m12_s_rt"), 42);
}

// ---- M13: long（64 位整数）+ (type)expr 强制类型转换 ----

#[test]
fn m13_long_arithmetic_64bit() {
    // 10^6 * 10^6 = 10^12，远超 32 位
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ long x = 1000000; long y = x * x; printf(\"%ld\\n\", y); return 0; }",
        "m13_long_arith",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "1000000000000\n");
}

#[test]
fn m13_long_param_and_return() {
    // 形参与返回值都是 64 位
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nlong mul(long a, long b){ return a * b; }\nint main(){ printf(\"%ld\\n\", mul(3000000000, 3)); return 0; }",
        "m13_long_pr",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "9000000000\n");
}

#[test]
fn m13_long_global_accumulate() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nlong total = 0;\nint main(){ int i; for (i = 0; i < 5; i++) total = total + 1000000000; printf(\"%ld\\n\", total); return 0; }",
        "m13_long_glob",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "5000000000\n");
}

#[test]
fn m13_long_recursion_factorial() {
    // 20! 需要 64 位；三元分支 int : long 取公共类型 long
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nlong fact(int n){ return n <= 1 ? 1 : n * (long)fact(n-1); }\nint main(){ printf(\"%ld\\n\", fact(20)); return 0; }",
        "m13_long_fact",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "2432902008176640000\n");
}

#[test]
fn m13_long_comparison() {
    // 5e9 > 4e9 的 64 位比较
    let src = "int main(){ long a = 5000000000; long b = 4000000000; return a > b; }";
    assert_eq!(compile_and_run(src, "m13_long_cmp"), 1);
}

#[test]
fn m13_cast_double_to_int_truncates() {
    let src = "int main(){ double d = 7.9; int n = (int)d; return n; }";
    assert_eq!(compile_and_run(src, "m13_cast_d2i"), 7);
}

#[test]
fn m13_cast_int_to_double_in_expr() {
    // (double)7 / 2 = 3.5（若按 int 除则为 3）
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ int x = 7; printf(\"%f\\n\", (double)x / 2); return 0; }",
        "m13_cast_i2d",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "3.500000\n");
}

#[test]
fn m13_cast_int_to_long_widens() {
    // (long)大int 相乘不溢出
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ int x = 100000; long y = (long)x * x; printf(\"%ld\\n\", y); return 0; }",
        "m13_cast_i2l",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "10000000000\n");
}

// ---- M14: do-while 循环 + 逗号运算符 ----

#[test]
fn m14_do_while_basic() {
    // do 体至少执行一次，再判条件：0+1+2+3+4 = 10
    let src = "int main(){ int i = 0; int sum = 0; do { sum = sum + i; i++; } while (i < 5); return sum; }";
    assert_eq!(compile_and_run(src, "m14_dw"), 10);
}

#[test]
fn m14_do_while_runs_once_when_false() {
    // 条件起始即假，do 体仍执行一次
    let src = "int main(){ int n = 0; do { n++; } while (0); return n; }";
    assert_eq!(compile_and_run(src, "m14_dw_once"), 1);
}

#[test]
fn m14_do_while_break_continue() {
    let src = "int main(){ int k = 0; do { k++; if (k == 3) break; } while (k < 100); return k; }";
    assert_eq!(compile_and_run(src, "m14_dw_bc"), 3);
}

#[test]
fn m14_comma_operator_value() {
    // (b = 3, b * 2) 求值为 6
    let src = "int main(){ int b; int a = (b = 3, b * 2); return a; }";
    assert_eq!(compile_and_run(src, "m14_comma"), 6);
}

#[test]
fn m14_comma_in_for_clauses() {
    // for 的 init/step 用逗号运算符同时推进两个变量，相遇于 5
    let src = "int main(){ int i; int j; int n = 0; for (i = 0, j = 10; i < j; i++, j--) n++; return n; }";
    assert_eq!(compile_and_run(src, "m14_comma_for"), 5);
}

// ---- M15: 单条声明多个变量 ----

#[test]
fn m15_multi_declarator_with_init() {
    let src = "int main(){ int a = 1, b = 2, c = 3; return a + b + c; }";
    assert_eq!(compile_and_run(src, "m15_md_init"), 6);
}

#[test]
fn m15_multi_declarator_no_init() {
    let src = "int main(){ int x, y; x = 4; y = 5; return x + y; }";
    assert_eq!(compile_and_run(src, "m15_md_noinit"), 9);
}

#[test]
fn m15_pointer_binds_per_declarator() {
    // `int *p, q;` → p 是 int*，q 是 int；通过 p 改 q
    let src = "int main(){ int q; int *p, r; p = &q; *p = 42; r = q; return r; }";
    assert_eq!(compile_and_run(src, "m15_md_ptr"), 42);
}

#[test]
fn m15_multi_declarator_array_and_scalar() {
    let src = "int main(){ int arr[3], n; arr[0]=10; arr[1]=20; arr[2]=30; n = arr[0]+arr[1]+arr[2]; return n; }";
    assert_eq!(compile_and_run(src, "m15_md_arr"), 60);
}

#[test]
fn m15_long_multi_declarator() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ long a = 3000000000, b = 3000000000; printf(\"%ld\\n\", a + b); return 0; }",
        "m15_md_long",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "6000000000\n");
}

#[test]
fn m15_global_multi_declarator() {
    // 全局单条多声明符（含 long 与指针绑定）
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint a, b = 5, c = 10;\nlong big = 3000000000;\nint main(){ a = 1; printf(\"%d %d %d %ld\\n\", a, b, c, big); return b + c; }",
        "m15_global_md",
    );
    assert_eq!(code, 15);
    assert_eq!(out, "1 5 10 3000000000\n");
}

// ---- M16: unsigned/short、八进制字面量、sizeof 不带括号 ----

#[test]
fn m16_unsigned_and_short() {
    // unsigned/short 当作 int 处理；unsigned long 仍是 64 位
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ unsigned int u = 100; short s = 7; unsigned long ul = 5000000000; printf(\"%d %d %ld\\n\", u, s, ul); return 0; }",
        "m16_uns",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "100 7 5000000000\n");
}

#[test]
fn m16_unsigned_char() {
    // unsigned char → char
    let src = "int main(){ unsigned char c = 65; return c; }";
    assert_eq!(compile_and_run(src, "m16_uchar"), 65);
}

#[test]
fn m16_octal_literal() {
    // 0777 = 511，010 = 8（用 printf 避免退出码 8 位截断）
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ printf(\"%d %d\\n\", 0777, 010); return 0; }",
        "m16_octal",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "511 8\n");
}

#[test]
fn m16_octal_zero_still_zero() {
    let src = "int main(){ int z = 0; return z + 5; }";
    assert_eq!(compile_and_run(src, "m16_oct0"), 5);
}

#[test]
fn m16_sizeof_without_parens_var() {
    // sizeof 变量、表达式不需括号
    let src = "int main(){ int x; double d; return sizeof x + sizeof d; }"; // 4 + 8 = 12
    assert_eq!(compile_and_run(src, "m16_szv"), 12);
}

#[test]
fn m16_sizeof_array_not_decayed() {
    // sizeof arr 得整个数组大小（不退化为指针）
    let src = "int main(){ int arr[5]; return sizeof arr; }"; // 5*4 = 20
    assert_eq!(compile_and_run(src, "m16_szarr"), 20);
}

#[test]
fn m16_sizeof_type_still_works() {
    // 带括号的 sizeof(type) 仍可用
    let src = "int main(){ return sizeof(long) + sizeof(char); }"; // 8 + 1 = 9
    assert_eq!(compile_and_run(src, "m16_szty"), 9);
}

// ---- M17: 存储类、科学计数法、字符串拼接、goto ----

#[test]
fn m17_storage_class_keywords() {
    // static/extern/register 语法被接受（语义忽略）：静态全局 + register 局部
    let src = "static int g = 41; int helper(){ return g; } int main(){ register int x = helper(); return x + 1; }";
    assert_eq!(compile_and_run(src, "m17_storage"), 42);
}

#[test]
fn m17_scientific_notation() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ printf(\"%f %f %f\\n\", 1e3, 2.5e-2, 1E6); return 0; }",
        "m17_sci",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "1000.000000 0.025000 1000000.000000\n");
}

#[test]
fn m17_string_concatenation() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ printf(\"%s\\n\", \"Hello, \" \"world\" \"!\"); return 0; }",
        "m17_strcat",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "Hello, world!\n");
}

#[test]
fn m17_goto_forward() {
    // 向前跳过中间代码
    let src = "int main(){ int x = 1; goto skip; x = 99; skip: return x; }";
    assert_eq!(compile_and_run(src, "m17_goto_fwd"), 1);
}

#[test]
fn m17_goto_backward_loop() {
    // 向后跳实现循环：0+1+2+3+4 = 10
    let src = "int main(){ int i = 0; int sum = 0; loop: sum = sum + i; i++; if (i < 5) goto loop; return sum; }";
    assert_eq!(compile_and_run(src, "m17_goto_bwd"), 10);
}

#[test]
fn m17_goto_error_cleanup_pattern() {
    // 经典 goto 错误清理：跳到统一出口
    let src = "int f(int ok){ int rc = 0; if (!ok) { rc = -1; goto done; } rc = 5; done: return rc; } int main(){ return f(0) + 1; }"; // -1 + 1 = 0
    assert_eq!(compile_and_run(src, "m17_goto_cleanup"), 0);
}

// ---- M18: 聚合初始化列表 + 指针比较/相减 ----

#[test]
fn m18_array_initializer() {
    let src = "int main(){ int a[3] = {10, 20, 30}; return a[0] + a[1] + a[2]; }";
    assert_eq!(compile_and_run(src, "m18_arr_init"), 60);
}

#[test]
fn m18_array_initializer_inferred_size() {
    // int a[] = {...} 长度推断；sizeof 验证
    let src = "int main(){ int a[] = {1, 2, 3, 4, 5}; return sizeof a / sizeof a[0]; }"; // 5
    assert_eq!(compile_and_run(src, "m18_arr_infer"), 5);
}

#[test]
fn m18_array_zero_fill() {
    // 不足部分零填充：{7} → [7,0,0,0,0]
    let src = "int main(){ int z[5] = {7}; return z[0] + z[1] + z[2] + z[3] + z[4]; }";
    assert_eq!(compile_and_run(src, "m18_arr_zero"), 7);
}

#[test]
fn m18_array_full_zero_idiom() {
    // {0} 惯用法全清零
    let src = "int main(){ int b[10] = {0}; int s = 0; for (int i=0;i<10;i++) s += b[i]; return s + 5; }";
    assert_eq!(compile_and_run(src, "m18_zero_idiom"), 5);
}

#[test]
fn m18_struct_initializer() {
    let src = "struct P { int x; int y; }; int main(){ struct P p = {40, 2}; return p.x + p.y; }";
    assert_eq!(compile_and_run(src, "m18_struct_init"), 42);
}

#[test]
fn m18_pointer_subtraction() {
    // q - p 得元素个数
    let src = "int main(){ int a[10]; int *p = &a[2]; int *q = &a[9]; int d = q - p; return d; }"; // 7
    assert_eq!(compile_and_run(src, "m18_ptr_sub"), 7);
}

#[test]
fn m18_pointer_comparison() {
    let src = "int main(){ int a[5]; int *p = &a[1]; int *q = &a[3]; return (p < q) + (p != q) + (q > p); }"; // 1+1+1=3
    assert_eq!(compile_and_run(src, "m18_ptr_cmp"), 3);
}

#[test]
fn m18_pointer_walk_with_compare() {
    // 用指针比较驱动遍历
    let src = "int main(){ int a[4]; a[0]=1; a[1]=2; a[2]=3; a[3]=4; int *p = a; int *end = a + 4; int s = 0; while (p < end) { s += *p; p = p + 1; } return s; }"; // 10
    assert_eq!(compile_and_run(src, "m18_ptr_walk"), 10);
}

// ---- M19: enum 当类型 / 多维数组 / 函数指针 / 用户可变参数 ----

#[test]
fn m19_enum_as_type() {
    let src = "enum Color { RED, GREEN, BLUE }; enum Color pick(enum Color c){ return c; } int main(){ enum Color c = GREEN; return pick(c) + BLUE; }"; // 1 + 2
    assert_eq!(compile_and_run(src, "m19_enum_ty"), 3);
}

#[test]
fn m19_multidim_array() {
    let src = "int main(){ int a[3][4]; for (int i=0;i<3;i++) for (int j=0;j<4;j++) a[i][j]=i*10+j; return a[2][3]; }"; // 23
    assert_eq!(compile_and_run(src, "m19_md_arr"), 23);
}

#[test]
fn m19_multidim_sizeof() {
    let src = "int main(){ int a[3][4]; return sizeof a; }"; // 3*4*4 = 48
    assert_eq!(compile_and_run(src, "m19_md_sz"), 48);
}

#[test]
fn m19_function_pointer_variable() {
    let src = "int add(int a,int b){return a+b;} int mul(int a,int b){return a*b;} int main(){ int (*f)(int,int) = add; int r = f(3,4); f = &mul; return r + f(5,6); }"; // 7 + 30
    assert_eq!(compile_and_run(src, "m19_fp_var"), 37);
}

#[test]
fn m19_function_pointer_callback() {
    let src = "int apply(int (*op)(int,int), int x, int y){ return op(x,y); } int sub(int a,int b){return a-b;} int main(){ return apply(sub, 50, 8); }"; // 42
    assert_eq!(compile_and_run(src, "m19_fp_cb"), 42);
}

#[test]
fn m19_function_pointer_deref_call() {
    let src = "int neg(int x){return -x;} int main(){ int (*f)(int) = neg; return (*f)(-42); }"; // 42
    assert_eq!(compile_and_run(src, "m19_fp_deref"), 42);
}

#[test]
fn m19_variadic_int() {
    let src = "int sum(int n, ...){ va_list ap; va_start(ap, n); int t=0; for(int i=0;i<n;i++) t+=va_arg(ap,int); va_end(ap); return t; } int main(){ return sum(5, 10,20,30,40,50); }"; // 150
    assert_eq!(compile_and_run(src, "m19_va_int"), 150);
}

#[test]
fn m19_variadic_double() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\ndouble favg(int n, ...){ va_list ap; va_start(ap, n); double s=0; for(int i=0;i<n;i++) s+=va_arg(ap,double); va_end(ap); return s/n; } int main(){ printf(\"%f\\n\", favg(4, 1.0, 2.0, 3.0, 4.0)); return 0; }",
        "m19_va_dbl",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "2.500000\n");
}

// ---- M21: LLVM 后端(--target llvm)与原生后端结果对拍 ----

/// 用指定 target 编译并运行，返回 (退出码, stdout)。
fn compile_run_target(src: &str, name: &str, target: &str) -> (i32, String) {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");
    let bin = env!("CARGO_BIN_EXE_anvil");
    let compile = Command::new(bin)
        .arg(&c_path)
        .args(["--target", target])
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run anvil");
    assert!(compile.success(), "anvil --target {} failed for {}", target, name);
    let out = Command::new(&exe_path).output().expect("run exe");
    (out.status.code().expect("signal"), String::from_utf8_lossy(&out.stdout).to_string())
}

/// 同一程序经原生 x86-64 后端与 LLVM 后端应产生完全一致的结果。
fn cross_check(src: &str, name: &str) {
    let native = compile_run_target(src, &format!("{}_n", name), "x86_64");
    let llvm = compile_run_target(src, &format!("{}_l", name), "llvm");
    assert_eq!(native, llvm, "native vs llvm 不一致 ({})", name);
}

#[test]
fn m21_llvm_arithmetic_and_recursion() {
    cross_check(
        "int fib(int n){ return n<2?n:fib(n-1)+fib(n-2); } int main(){ int s=0; for(int i=0;i<12;i++) s+=fib(i); return s; }",
        "m21_fib",
    );
}

#[test]
fn m21_llvm_pointers_and_arrays() {
    cross_check(
        "int main(){ int a[6]; for(int i=0;i<6;i++) a[i]=i*i; int*p=a; int*e=a+6; int s=0; while(p<e){ s+=*p; p++; } return s; }",
        "m21_ptr",
    );
}

#[test]
fn m21_llvm_struct_and_long() {
    cross_check(
        "struct P{int x;int y;}; int main(){ struct P p; p.x=40; p.y=2; long b=100000; long sq=b*b; return p.x+p.y + (int)(sq/100000); }",
        "m21_struct",
    );
}

#[test]
fn m21_llvm_double_and_printf() {
    let src = "#include <stdio.h>\ndouble half(int x){ return x/2.0; } int main(){ printf(\"%d %f\\n\", 7, half(9)); return 0; }";
    cross_check(src, "m21_dbl");
}

#[test]
fn m21_llvm_control_flow() {
    cross_check(
        "int main(){ int s=0; for(int i=1;i<=100;i++){ if(i%3==0) continue; if(i>50) break; s+=i; } return s % 200; }",
        "m21_cf",
    );
}

#[test]
fn m_void_fn_without_return_no_fallthrough() {
    // 回归:无 return 的 void 函数不能坠落到下一个函数(曾导致 main 被重入)
    let src = "void noop(){ int x = 1; } int main(){ noop(); return 42; }";
    assert_eq!(compile_and_run(src, "void_noret"), 42);
}

#[test]
fn m_void_fn_before_main_returns_cleanly() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nvoid greet(){ printf(\"hi\\n\"); } int main(){ greet(); greet(); return 0; }",
        "void_before_main",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "hi\nhi\n"); // 恰好两次,不重入
}

#[test]
fn m21_llvm_function_pointers() {
    cross_check(
        "int sq(int x){return x*x;} int cu(int x){return x*x*x;} int apply(int(*f)(int),int x){return f(x);} int main(){ int(*g)(int)=sq; int a=g(5); g=cu; return a + apply(g,3) + apply(sq,4); }",
        "m21_fnptr",
    );
}

#[test]
fn m21_llvm_struct_by_value() {
    cross_check(
        "struct P{int x;int y;}; struct P mk(int a,int b){struct P p; p.x=a; p.y=b; return p;} int sum(struct P p){return p.x+p.y;} int main(){ struct P q=mk(40,2); return sum(q) + sum(mk(10,20)); }",
        "m21_sbv",
    );
}

#[test]
fn m21_llvm_user_varargs() {
    cross_check(
        "int vsum(int n, ...){ va_list ap; va_start(ap,n); int s=0; for(int i=0;i<n;i++) s+=va_arg(ap,int); va_end(ap); return s; } int main(){ return vsum(6, 1,2,3,4,5,6); }",
        "m21_va",
    );
}

#[test]
fn m21_llvm_varargs_double() {
    let src = "#include <stdio.h>\ndouble favg(int n, ...){ va_list ap; va_start(ap,n); double s=0; for(int i=0;i<n;i++) s+=va_arg(ap,double); va_end(ap); return s/n; } int main(){ printf(\"%f\\n\", favg(4, 2.0,4.0,6.0,8.0)); return 0; }";
    cross_check(src, "m21_va_d");
}

// ---- M23: 全局聚合初始化 ----

#[test]
fn m23_global_array_init() {
    let src = "int a[] = {2,3,5,7,11}; int main(){ int s=0; for(int i=0;i<5;i++) s+=a[i]; return s; }"; // 28
    assert_eq!(compile_and_run(src, "m23_garr"), 28);
}

#[test]
fn m23_global_array_partial_zerofill() {
    let src = "int a[5] = {10, 20}; int main(){ return a[0]+a[1]+a[2]+a[3]+a[4]; }"; // 30
    assert_eq!(compile_and_run(src, "m23_gpart"), 30);
}

#[test]
fn m23_global_struct_array() {
    let src = "struct P{int x;int y;}; struct P t[3] = {{1,2},{3,4},{5,6}}; int main(){ return t[0].x + t[1].y + t[2].x; }"; // 1+4+5=10
    assert_eq!(compile_and_run(src, "m23_gsa"), 10);
}

#[test]
fn m23_global_long_array() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nlong b[2] = {10000000000, 5}; int main(){ printf(\"%ld\\n\", b[0] + b[1]); return 0; }",
        "m23_glong",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "10000000005\n");
}

#[test]
fn m23_global_init_cross_check() {
    // 原生 vs LLVM 一致
    cross_check(
        "int tab[6] = {1,2,4,8,16,32}; struct P{int a;int b;}; struct P ps[2]={{7,8},{9,10}}; int main(){ int s=0; for(int i=0;i<6;i++) s+=tab[i]; return s + ps[0].a + ps[1].b; }",
        "m23_gcc",
    );
}

// ---- M24: unsigned 语义 ----

#[test]
fn m24_unsigned_division() {
    // 4000000000 / 3:无符号除 = 1333333333(有符号会因 4e9 溢出 i32 而不同)
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ unsigned int a = 4000000000; printf(\"%u\\n\", a / 3); return 0; }",
        "m24_udiv",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "1333333333\n");
}

#[test]
fn m24_unsigned_comparison() {
    // big = 0xFFFFFFFF;无符号 big>1 为真;有符号(=-1)则为假
    let src = "int main(){ unsigned int big = 0xFFFFFFFF; return big > 1; }";
    assert_eq!(compile_and_run(src, "m24_ucmp"), 1);
}

#[test]
fn m24_unsigned_right_shift_logical() {
    // 0x80000000u >> 4 = 0x08000000(逻辑);算术右移会是 0xF8000000
    let src = "int main(){ unsigned int x = 0x80000000; return (x >> 4) == 0x08000000; }";
    assert_eq!(compile_and_run(src, "m24_ushr"), 1);
}

#[test]
fn m24_unsigned_char_zero_extends() {
    // unsigned char 200 → 200(非 -56)
    let src = "int main(){ unsigned char c = 200; int v = c; return v; }";
    assert_eq!(compile_and_run(src, "m24_uchar"), 200);
}

#[test]
fn m24_unsigned_cross_check() {
    cross_check(
        "#include <stdio.h>\nint main(){ unsigned int a=4000000000, b=7; unsigned long c=18000000000; printf(\"%u %u %d %lu\\n\", a/b, a%b, (a>100), c+1); return (a>100)+((a>>1)==2000000000); }",
        "m24_ucc",
    );
}

// ---- M24: char 8 位截断(回归;现有 width=1 存取已正确) ----

#[test]
fn m24_char_truncation_and_sign() {
    let (code, out) = compile_run_capture(
        "#include <stdio.h>\nint main(){ char c=200; signed char s=130; char of=100+100; unsigned char u=200; printf(\"%d %d %d %d\\n\", c, s, of, u); return 0; }",
        "m24_char",
    );
    assert_eq!(code, 0);
    assert_eq!(out, "-56 -126 -56 200\n"); // signed char 截断/符号扩展;unsigned char 零扩展
}

#[test]
fn m24_char_cross_check() {
    cross_check(
        "#include <stdio.h>\nint main(){ char c=200; unsigned char u=200; char w='A'+256; printf(\"%d %d %d\\n\", c, u, w); return 0; }",
        "m24_char_cc",
    );
}
