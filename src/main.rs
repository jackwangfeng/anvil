use bianyi::compile_to_asm;
use bianyi::preprocess::preprocess;
use std::path::Path;
use std::process::{exit, Command};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input, output) = match parse_args(&args) {
        Ok(v) => v,
        Err(msg) => {
            eprintln!("{}", msg);
            exit(2);
        }
    };

    let src = match std::fs::read_to_string(&input) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("error: cannot read '{}': {}", input, e);
            exit(1);
        }
    };

    let base_dir = Path::new(&input).parent().unwrap_or(Path::new("."));
    let preprocessed = match preprocess(&src, base_dir) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{}:{}", input, e);
            exit(1);
        }
    };

    let asm = match compile_to_asm(&preprocessed) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{}:{}", input, e);
            exit(1);
        }
    };

    let asm_path = format!("{}.s", output);
    if let Err(e) = std::fs::write(&asm_path, &asm) {
        eprintln!("error: cannot write '{}': {}", asm_path, e);
        exit(1);
    }

    let status = Command::new("clang")
        .arg(&asm_path)
        .arg("-o")
        .arg(&output)
        .status();

    match status {
        Ok(s) if s.success() => {
            let _ = std::fs::remove_file(&asm_path);
        }
        Ok(s) => {
            eprintln!("error: clang failed: {}", s);
            exit(1);
        }
        Err(e) => {
            eprintln!("error: failed to invoke clang: {}", e);
            exit(1);
        }
    }
}

fn parse_args(args: &[String]) -> Result<(String, String), String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
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
            other => {
                if input.is_some() {
                    return Err(format!("error: unexpected argument '{}'", other));
                }
                input = Some(other.to_string());
            }
        }
        i += 1;
    }
    let input = input.ok_or_else(|| "usage: bianyi <input.c> [-o output]".to_string())?;
    let output = output.unwrap_or_else(|| {
        Path::new(&input)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "a.out".to_string())
    });
    Ok((input, output))
}
