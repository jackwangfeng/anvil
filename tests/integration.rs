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
