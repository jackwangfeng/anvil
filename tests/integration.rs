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
