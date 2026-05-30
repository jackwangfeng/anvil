use std::process::Command;

/// 用 bianyi 编译 `src`，运行产物，返回其退出码。
fn compile_and_run(src: &str, name: &str) -> i32 {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");

    let bin = env!("CARGO_BIN_EXE_bianyi");
    let compile = Command::new(bin)
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run bianyi");
    assert!(compile.success(), "bianyi failed to compile {}", name);

    let run = Command::new(&exe_path).status().expect("run compiled exe");
    run.code().expect("program terminated by signal")
}

/// 编译 `src`，运行，返回 (退出码, stdout)。
fn compile_run_capture(src: &str, name: &str) -> (i32, String) {
    let dir = std::env::temp_dir();
    let c_path = dir.join(format!("{}.c", name));
    let exe_path = dir.join(name);
    std::fs::write(&c_path, src).expect("write .c");
    let bin = env!("CARGO_BIN_EXE_bianyi");
    let compile = Command::new(bin)
        .arg(&c_path)
        .arg("-o")
        .arg(&exe_path)
        .status()
        .expect("run bianyi");
    assert!(compile.success(), "bianyi failed to compile {}", name);
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
