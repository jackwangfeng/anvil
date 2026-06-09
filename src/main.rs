use anvil::preprocess::preprocess;
use anvil::{compile_to_asm_target, host_target, Target};
use std::path::Path;
use std::process::{exit, Command};

struct Options {
    input: String,
    output: String,
    target: Target,
    /// 仅生成汇编（`-S`），不汇编链接。
    asm_only: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let opts = match parse_args(&args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            exit(2);
        }
    };

    let src = match std::fs::read_to_string(&opts.input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", opts.input, e);
            exit(1);
        }
    };

    let base_dir = Path::new(&opts.input).parent().unwrap_or(Path::new("."));
    let preprocessed = match preprocess(&src, base_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}:{}", opts.input, e);
            exit(1);
        }
    };

    let asm = match compile_guarded(&preprocessed, opts.target) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}:{}", opts.input, e);
            exit(1);
        }
    };

    // -S：直接把汇编写到输出文件，不汇编链接。
    if opts.asm_only {
        if let Err(e) = std::fs::write(&opts.output, &asm) {
            eprintln!("error: cannot write '{}': {}", opts.output, e);
            exit(1);
        }
        return;
    }

    // LLVM 目标:写 .ll → llc -O2 → .s → gcc 链接。
    if opts.target == Target::Llvm {
        let ll_path = format!("{}.ll", opts.output);
        let s_path = format!("{}.s", opts.output);
        if let Err(e) = std::fs::write(&ll_path, &asm) {
            eprintln!("error: cannot write '{}': {}", ll_path, e);
            exit(1);
        }
        let llc = Command::new("llc")
            .args(["-O2", "-relocation-model=pic"])
            .arg(&ll_path)
            .arg("-o")
            .arg(&s_path)
            .status();
        match llc {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("error: llc failed: {}", s);
                exit(1);
            }
            Err(e) => {
                eprintln!("error: failed to invoke llc: {}", e);
                exit(1);
            }
        }
        let gcc = Command::new("gcc")
            .arg(&s_path)
            .arg("-no-pie")
            .arg("-o")
            .arg(&opts.output)
            .status();
        match gcc {
            Ok(s) if s.success() => {
                let _ = std::fs::remove_file(&ll_path);
                let _ = std::fs::remove_file(&s_path);
            }
            Ok(s) => {
                eprintln!("error: gcc failed: {}", s);
                exit(1);
            }
            Err(e) => {
                eprintln!("error: failed to invoke gcc: {}", e);
                exit(1);
            }
        }
        return;
    }

    let asm_path = format!("{}.s", opts.output);
    if let Err(e) = std::fs::write(&asm_path, &asm) {
        eprintln!("error: cannot write '{}': {}", asm_path, e);
        exit(1);
    }

    // 汇编 + 链接：x86-64 用本机 gcc；arm64/macOS 用 clang。
    let (assembler, extra): (&str, &[&str]) = match opts.target {
        Target::X86_64 => ("gcc", &["-no-pie"]),
        Target::Arm64 => ("clang", &[]),
        Target::Llvm => unreachable!(),
    };
    let status = Command::new(assembler)
        .arg(&asm_path)
        .args(extra)
        .arg("-o")
        .arg(&opts.output)
        .status();

    match status {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&asm_path);
        }
        Ok(s) => {
            eprintln!("error: {} failed: {}", assembler, s);
            exit(1);
        }
        Err(e) => {
            eprintln!("error: failed to invoke {}: {}", assembler, e);
            exit(1);
        }
    }
}

/// 编译并兜底捕获内部 panic（降级、代码生成阶段的未声明变量/非左值等），
/// 转成干净的 `error:` 诊断，而不是向终端用户抛 Rust backtrace。
fn compile_guarded(src: &str, target: Target) -> Result<String, String> {
    // 临时静默 panic 默认输出（线程名/backtrace 提示），仅在本次编译期间。
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        compile_to_asm_target(src, target)
    }));
    std::panic::set_hook(prev);
    match result {
        Ok(Ok(asm)) => Ok(asm),
        Ok(Err(e)) => Err(e.to_string()),
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "internal compiler error".to_string());
            Err(format!("error: {}", msg))
        }
    }
}

fn parse_args(args: &[String]) -> Result<Options, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut target = host_target();
    let mut asm_only = false;
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-o" => {
                i += 1;
                if i >= args.len() {
                    return Err("error: -o requires an argument".to_string());
                }
                output = Some(args[i].clone());
            }
            "-S" => asm_only = true,
            "--target" => {
                i += 1;
                target = match args.get(i).map(|s| s.as_str()) {
                    Some("arm64") | Some("aarch64") => Target::Arm64,
                    Some("x86_64") | Some("x86-64") | Some("amd64") => Target::X86_64,
                    Some("llvm") => Target::Llvm,
                    other => {
                        return Err(format!(
                            "error: --target expects arm64|x86_64|llvm, got {:?}",
                            other
                        ))
                    }
                };
            }
            other if other.starts_with("--target=") => {
                target = match &other["--target=".len()..] {
                    "arm64" | "aarch64" => Target::Arm64,
                    "x86_64" | "x86-64" | "amd64" => Target::X86_64,
                    "llvm" => Target::Llvm,
                    t => return Err(format!("error: unknown target '{}'", t)),
                };
            }
            other => {
                if input.is_some() {
                    return Err(format!("error: unexpected argument '{}'", other));
                }
                input = Some(other.to_string());
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| {
        "usage: anvil <input.c> [-o output] [-S] [--target arm64|x86_64]".to_string()
    })?;
    let output = output.unwrap_or_else(|| {
        Path::new(&input)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "a.out".to_string())
    });
    Ok(Options {
        input,
        output,
        target,
        asm_only,
    })
}
