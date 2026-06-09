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
