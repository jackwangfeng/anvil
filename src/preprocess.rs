//! 自研 C 预处理器：对象式/函数式宏、`#`/`##`、`#include`、条件编译。
//! 在词法分析之前运行，把源码文本变换为展开后的文本，再交给 lexer。
//!
//! 已知边界（M6 取舍）：仅 `#include "相对路径"`（不搜索系统头）；`#if` 常量
//! 表达式支持 `defined`、整数、`+ - * / % ! && || == != < > <= >= ()`；
//! 宏递归用"展开中名字集合"防止无限循环（非完整 hideset 语义）。

use crate::error::CompileError;
use crate::span::Span;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub fn preprocess(src: &str, base_dir: &Path) -> Result<String, CompileError> {
    let mut pp = Preprocessor {
        macros: HashMap::new(),
        base_dir: base_dir.to_path_buf(),
        include_depth: 0,
    };
    pp.run(src)
}

#[derive(Clone)]
struct Macro {
    /// None = 对象式；Some(params) = 函数式。
    params: Option<Vec<String>>,
    body: Vec<PpTok>,
}

struct Preprocessor {
    macros: HashMap<String, Macro>,
    base_dir: PathBuf,
    include_depth: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PpTok {
    text: String,
    is_ident: bool,
}

fn err(msg: impl Into<String>) -> CompileError {
    CompileError::new(Span::new(0, 0), msg.into())
}

impl Preprocessor {
    fn run(&mut self, src: &str) -> Result<String, CompileError> {
        // 条件编译状态栈：每层 (本分支是否激活, 该 #if 链是否已有分支被采用)
        let mut cond_stack: Vec<(bool, bool)> = Vec::new();
        let mut out = String::new();

        let lines: Vec<&str> = src.split('\n').collect();
        let mut i = 0;
        while i < lines.len() {
            let raw = lines[i];
            i += 1;
            // 行续：以反斜杠结尾的行与下一行拼接
            let mut line = raw.to_string();
            while line.ends_with('\\') && i < lines.len() {
                line.pop();
                line.push_str(lines[i]);
                i += 1;
            }

            let trimmed = line.trim_start();
            let active = cond_stack.iter().all(|(a, _)| *a);

            if let Some(rest) = trimmed.strip_prefix('#') {
                let rest = rest.trim_start();
                let (directive, args) = split_directive(rest);
                match directive.as_str() {
                    "ifdef" => {
                        let name = args.trim();
                        let cond = active && self.macros.contains_key(name);
                        cond_stack.push((cond, cond));
                    }
                    "ifndef" => {
                        let name = args.trim();
                        let cond = active && !self.macros.contains_key(name);
                        cond_stack.push((cond, cond));
                    }
                    "if" => {
                        let cond = active && self.eval_cond(&args)?;
                        cond_stack.push((cond, cond));
                    }
                    "elif" => {
                        let parent_active = cond_stack
                            .iter()
                            .take(cond_stack.len().saturating_sub(1))
                            .all(|(a, _)| *a);
                        let top = cond_stack
                            .last_mut()
                            .ok_or_else(|| err("#elif without #if"))?;
                        if top.1 {
                            top.0 = false; // 已有分支被采用
                        } else {
                            let c = parent_active && self.eval_cond_str(&args);
                            top.0 = c;
                            if c {
                                top.1 = true;
                            }
                        }
                    }
                    "else" => {
                        let top = cond_stack
                            .last_mut()
                            .ok_or_else(|| err("#else without #if"))?;
                        top.0 = !top.1;
                        top.1 = true;
                    }
                    "endif" => {
                        cond_stack.pop().ok_or_else(|| err("#endif without #if"))?;
                    }
                    "define" if active => self.do_define(&args)?,
                    "undef" if active => {
                        self.macros.remove(args.trim());
                    }
                    "include" if active => {
                        let included = self.do_include(&args)?;
                        out.push_str(&included);
                        if !out.ends_with('\n') {
                            out.push('\n');
                        }
                    }
                    _ => {} // 非激活分支内的指令、或未知指令：忽略
                }
                continue;
            }

            if active {
                let expanded = self.expand_line(&line)?;
                out.push_str(&expanded);
                out.push('\n');
            }
        }

        if !cond_stack.is_empty() {
            return Err(err("unterminated #if/#ifdef"));
        }
        Ok(out)
    }

    fn do_define(&mut self, args: &str) -> Result<(), CompileError> {
        let args = args.trim_start();
        // 提取宏名
        let name_end = args
            .find(|c: char| !is_ident_char(c))
            .unwrap_or(args.len());
        if name_end == 0 {
            return Err(err("#define needs a name"));
        }
        let name = args[..name_end].to_string();
        let rest = &args[name_end..];
        if let Some(after) = rest.strip_prefix('(') {
            // 函数式宏：解析参数表
            let close = after.find(')').ok_or_else(|| err("unterminated macro params"))?;
            let params: Vec<String> = after[..close]
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let body = tokenize(after[close + 1..].trim());
            self.macros.insert(
                name,
                Macro {
                    params: Some(params),
                    body,
                },
            );
        } else {
            let body = tokenize(rest.trim());
            self.macros.insert(name, Macro { params: None, body });
        }
        Ok(())
    }

    fn do_include(&mut self, args: &str) -> Result<String, CompileError> {
        let a = args.trim();
        let path = if let Some(inner) = a.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
            self.base_dir.join(inner)
        } else if a.starts_with('<') {
            // 系统头：M6 不提供，返回空（best-effort）
            return Ok(String::new());
        } else {
            return Err(err(format!("bad #include: {}", a)));
        };
        if self.include_depth > 50 {
            return Err(err("#include nested too deep"));
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| err(format!("cannot include '{}': {}", path.display(), e)))?;
        self.include_depth += 1;
        // 递归预处理被包含文件（共享宏表）；其相对包含基于该文件目录
        let saved_dir = self.base_dir.clone();
        if let Some(parent) = path.parent() {
            self.base_dir = parent.to_path_buf();
        }
        let result = self.run(&content);
        self.base_dir = saved_dir;
        self.include_depth -= 1;
        result
    }

    /// 对一行做宏展开，输出文本。
    fn expand_line(&self, line: &str) -> Result<String, CompileError> {
        let toks = tokenize(line);
        let expanded = self.expand(toks, &HashSet::new())?;
        Ok(toks_to_string(&expanded))
    }

    /// 展开一串 pp-token。hideset 为正在展开的宏名集合（防止无限递归）。
    fn expand(&self, toks: Vec<PpTok>, hideset: &HashSet<String>) -> Result<Vec<PpTok>, CompileError> {
        let mut out = Vec::new();
        let mut i = 0;
        while i < toks.len() {
            let t = &toks[i];
            if t.is_ident && !hideset.contains(&t.text) {
                if let Some(m) = self.macros.get(&t.text) {
                    match &m.params {
                        None => {
                            // 对象式
                            let mut hs = hideset.clone();
                            hs.insert(t.text.clone());
                            let body = self.expand(m.body.clone(), &hs)?;
                            out.extend(body);
                            i += 1;
                            continue;
                        }
                        Some(params) => {
                            // 函数式：向后看是否有 '('
                            let mut j = i + 1;
                            while j < toks.len() && !toks[j].is_ident && toks[j].text.trim().is_empty() {
                                j += 1;
                            }
                            if j < toks.len() && toks[j].text == "(" {
                                let (args, next) = gather_args(&toks, j)?;
                                let substituted = self.substitute(m, params, &args)?;
                                let mut hs = hideset.clone();
                                hs.insert(t.text.clone());
                                let expanded = self.expand(substituted, &hs)?;
                                out.extend(expanded);
                                i = next;
                                continue;
                            }
                        }
                    }
                }
            }
            out.push(t.clone());
            i += 1;
        }
        Ok(out)
    }

    /// 函数式宏体替换：处理参数代入、`#` 字符串化、`##` 粘贴。
    fn substitute(
        &self,
        m: &Macro,
        params: &[String],
        args: &[Vec<PpTok>],
    ) -> Result<Vec<PpTok>, CompileError> {
        let arg_of = |name: &str| -> Option<usize> { params.iter().position(|p| p == name) };
        let body = &m.body;
        let mut out: Vec<PpTok> = Vec::new();
        let mut i = 0;
        while i < body.len() {
            let t = &body[i];
            // `#` 字符串化：# 后跟参数名
            if t.text == "#" && !t.is_ident {
                let mut j = i + 1;
                while j < body.len() && body[j].text.trim().is_empty() && !body[j].is_ident {
                    j += 1;
                }
                if j < body.len() && body[j].is_ident {
                    if let Some(idx) = arg_of(&body[j].text) {
                        let s = toks_to_string(args.get(idx).map(|v| v.as_slice()).unwrap_or(&[]));
                        out.push(PpTok { text: format!("\"{}\"", s.trim()), is_ident: false });
                        i = j + 1;
                        continue;
                    }
                }
            }
            // `##` 粘贴：左右相邻 token 文本拼接
            if i + 1 < body.len() {
                // 看下一个非空白是否是 ##
                let mut k = i + 1;
                while k < body.len() && body[k].text.trim().is_empty() && !body[k].is_ident {
                    k += 1;
                }
                if k < body.len() && body[k].text == "##" {
                    // 找 ## 之后的 token
                    let mut r = k + 1;
                    while r < body.len() && body[r].text.trim().is_empty() && !body[r].is_ident {
                        r += 1;
                    }
                    let left = self.maybe_arg_text(&body[i], &arg_of, args);
                    let right = if r < body.len() {
                        self.maybe_arg_text(&body[r], &arg_of, args)
                    } else {
                        String::new()
                    };
                    let pasted = format!("{}{}", left.trim(), right.trim());
                    let is_ident = pasted.chars().all(is_ident_char) && !pasted.is_empty();
                    out.push(PpTok { text: pasted, is_ident });
                    i = r + 1;
                    continue;
                }
            }
            // 普通参数代入（展开后代入）
            if t.is_ident {
                if let Some(idx) = arg_of(&t.text) {
                    let expanded = self.expand(
                        args.get(idx).cloned().unwrap_or_default(),
                        &HashSet::new(),
                    )?;
                    out.extend(expanded);
                    i += 1;
                    continue;
                }
            }
            out.push(t.clone());
            i += 1;
        }
        Ok(out)
    }

    fn maybe_arg_text(
        &self,
        t: &PpTok,
        arg_of: &impl Fn(&str) -> Option<usize>,
        args: &[Vec<PpTok>],
    ) -> String {
        if t.is_ident {
            if let Some(idx) = arg_of(&t.text) {
                return toks_to_string(args.get(idx).map(|v| v.as_slice()).unwrap_or(&[]));
            }
        }
        t.text.clone()
    }

    fn eval_cond(&self, expr: &str) -> Result<bool, CompileError> {
        Ok(self.eval_cond_str(expr))
    }

    /// 计算 #if/#elif 条件：处理 defined(X)/defined X，宏展开，再求整数常量表达式 != 0。
    fn eval_cond_str(&self, expr: &str) -> bool {
        // 先处理 defined
        let pre = self.replace_defined(expr);
        // 宏展开
        let toks = tokenize(&pre);
        let expanded = self.expand(toks, &HashSet::new()).unwrap_or_default();
        let s = toks_to_string(&expanded);
        // 未定义标识符在 #if 中视为 0
        let mut ev = CondEval::new(&s);
        ev.parse().map(|v| v != 0).unwrap_or(false)
    }

    fn replace_defined(&self, expr: &str) -> String {
        let toks = tokenize(expr);
        let mut out = String::new();
        let mut i = 0;
        while i < toks.len() {
            if toks[i].is_ident && toks[i].text == "defined" {
                // defined(NAME) 或 defined NAME
                let mut j = i + 1;
                while j < toks.len() && toks[j].text.trim().is_empty() && !toks[j].is_ident {
                    j += 1;
                }
                let (name, next) = if j < toks.len() && toks[j].text == "(" {
                    let mut k = j + 1;
                    while k < toks.len() && !toks[k].is_ident {
                        k += 1;
                    }
                    let name = toks.get(k).map(|t| t.text.clone()).unwrap_or_default();
                    let mut e = k + 1;
                    while e < toks.len() && toks[e].text != ")" {
                        e += 1;
                    }
                    (name, e + 1)
                } else {
                    (toks.get(j).map(|t| t.text.clone()).unwrap_or_default(), j + 1)
                };
                out.push_str(if self.macros.contains_key(&name) { "1" } else { "0" });
                i = next;
            } else {
                out.push_str(&toks[i].text);
                i += 1;
            }
        }
        out
    }
}

/// 收集函数式宏调用的实参。toks[open] 必须是 '('。返回 (各参数 token 列表, 右括号后的下标)。
fn gather_args(toks: &[PpTok], open: usize) -> Result<(Vec<Vec<PpTok>>, usize), CompileError> {
    let mut args: Vec<Vec<PpTok>> = Vec::new();
    let mut cur: Vec<PpTok> = Vec::new();
    let mut depth = 0i32;
    let mut i = open;
    loop {
        if i >= toks.len() {
            return Err(err("unterminated macro argument list"));
        }
        let t = &toks[i];
        if t.text == "(" {
            depth += 1;
            if depth > 1 {
                cur.push(t.clone());
            }
        } else if t.text == ")" {
            depth -= 1;
            if depth == 0 {
                args.push(trim_ws(cur));
                return Ok((args, i + 1));
            }
            cur.push(t.clone());
        } else if t.text == "," && depth == 1 {
            args.push(trim_ws(std::mem::take(&mut cur)));
        } else {
            cur.push(t.clone());
        }
        i += 1;
    }
}

fn trim_ws(mut v: Vec<PpTok>) -> Vec<PpTok> {
    while v.first().map(|t| t.text.trim().is_empty() && !t.is_ident) == Some(true) {
        v.remove(0);
    }
    while v.last().map(|t| t.text.trim().is_empty() && !t.is_ident) == Some(true) {
        v.pop();
    }
    v
}

fn split_directive(rest: &str) -> (String, String) {
    let end = rest.find(|c: char| c.is_whitespace()).unwrap_or(rest.len());
    (rest[..end].to_string(), rest[end..].trim_start().to_string())
}

fn is_ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

/// 把一行/一段文本切成 pp-token：标识符、空白、字符串、`##`、其它单字符。
fn tokenize(s: &str) -> Vec<PpTok> {
    let chars: Vec<char> = s.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            let mut t = String::new();
            while i < chars.len() && chars[i].is_whitespace() {
                t.push(chars[i]);
                i += 1;
            }
            toks.push(PpTok { text: t, is_ident: false });
        } else if is_ident_start(c) {
            let mut t = String::new();
            while i < chars.len() && is_ident_char(chars[i]) {
                t.push(chars[i]);
                i += 1;
            }
            toks.push(PpTok { text: t, is_ident: true });
        } else if c.is_ascii_digit() {
            let mut t = String::new();
            while i < chars.len() && (is_ident_char(chars[i]) || chars[i] == '.') {
                t.push(chars[i]);
                i += 1;
            }
            toks.push(PpTok { text: t, is_ident: false });
        } else if c == '"' || c == '\'' {
            let q = c;
            let mut t = String::new();
            t.push(c);
            i += 1;
            while i < chars.len() {
                t.push(chars[i]);
                if chars[i] == '\\' && i + 1 < chars.len() {
                    t.push(chars[i + 1]);
                    i += 2;
                    continue;
                }
                if chars[i] == q {
                    i += 1;
                    break;
                }
                i += 1;
            }
            toks.push(PpTok { text: t, is_ident: false });
        } else if c == '#' && i + 1 < chars.len() && chars[i + 1] == '#' {
            toks.push(PpTok { text: "##".to_string(), is_ident: false });
            i += 2;
        } else {
            toks.push(PpTok { text: c.to_string(), is_ident: false });
            i += 1;
        }
    }
    toks
}

fn toks_to_string(toks: &[PpTok]) -> String {
    toks.iter().map(|t| t.text.as_str()).collect()
}

/// 极简整数常量表达式求值器（用于 #if）。支持 + - * / % ! && || == != < > <= >= ()。
struct CondEval {
    chars: Vec<char>,
    pos: usize,
}

impl CondEval {
    fn new(s: &str) -> Self {
        CondEval { chars: s.chars().collect(), pos: 0 }
    }
    fn parse(&mut self) -> Option<i64> {
        let v = self.or()?;
        Some(v)
    }
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }
    fn peek2(&self) -> String {
        self.chars.iter().skip(self.pos).take(2).collect()
    }
    fn or(&mut self) -> Option<i64> {
        let mut l = self.and()?;
        loop {
            self.skip_ws();
            if self.peek2() == "||" {
                self.pos += 2;
                let r = self.and()?;
                l = ((l != 0) || (r != 0)) as i64;
            } else {
                break;
            }
        }
        Some(l)
    }
    fn and(&mut self) -> Option<i64> {
        let mut l = self.cmp()?;
        loop {
            self.skip_ws();
            if self.peek2() == "&&" {
                self.pos += 2;
                let r = self.cmp()?;
                l = ((l != 0) && (r != 0)) as i64;
            } else {
                break;
            }
        }
        Some(l)
    }
    fn cmp(&mut self) -> Option<i64> {
        let l = self.add()?;
        self.skip_ws();
        let two = self.peek2();
        if two == "==" || two == "!=" || two == "<=" || two == ">=" {
            self.pos += 2;
            let r = self.add()?;
            return Some(match two.as_str() {
                "==" => (l == r) as i64,
                "!=" => (l != r) as i64,
                "<=" => (l <= r) as i64,
                ">=" => (l >= r) as i64,
                _ => 0,
            });
        }
        if self.pos < self.chars.len() && (self.chars[self.pos] == '<' || self.chars[self.pos] == '>') {
            let op = self.chars[self.pos];
            self.pos += 1;
            let r = self.add()?;
            return Some(if op == '<' { (l < r) as i64 } else { (l > r) as i64 });
        }
        Some(l)
    }
    fn add(&mut self) -> Option<i64> {
        let mut l = self.mul()?;
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                break;
            }
            match self.chars[self.pos] {
                '+' => { self.pos += 1; l += self.mul()?; }
                '-' => { self.pos += 1; l -= self.mul()?; }
                _ => break,
            }
        }
        Some(l)
    }
    fn mul(&mut self) -> Option<i64> {
        let mut l = self.unary()?;
        loop {
            self.skip_ws();
            if self.pos >= self.chars.len() {
                break;
            }
            match self.chars[self.pos] {
                '*' => { self.pos += 1; l *= self.unary()?; }
                '/' => { self.pos += 1; let r = self.unary()?; if r == 0 { return Some(0); } l /= r; }
                '%' => { self.pos += 1; let r = self.unary()?; if r == 0 { return Some(0); } l %= r; }
                _ => break,
            }
        }
        Some(l)
    }
    fn unary(&mut self) -> Option<i64> {
        self.skip_ws();
        if self.pos < self.chars.len() && self.chars[self.pos] == '!' && self.peek2() != "!=" {
            self.pos += 1;
            let v = self.unary()?;
            return Some((v == 0) as i64);
        }
        if self.pos < self.chars.len() && self.chars[self.pos] == '-' {
            self.pos += 1;
            let v = self.unary()?;
            return Some(-v);
        }
        self.primary()
    }
    fn primary(&mut self) -> Option<i64> {
        self.skip_ws();
        if self.pos >= self.chars.len() {
            return None;
        }
        if self.chars[self.pos] == '(' {
            self.pos += 1;
            let v = self.or()?;
            self.skip_ws();
            if self.pos < self.chars.len() && self.chars[self.pos] == ')' {
                self.pos += 1;
            }
            return Some(v);
        }
        if self.chars[self.pos].is_ascii_digit() {
            let mut n = String::new();
            while self.pos < self.chars.len() && self.chars[self.pos].is_ascii_digit() {
                n.push(self.chars[self.pos]);
                self.pos += 1;
            }
            return n.parse().ok();
        }
        // 未定义标识符 → 0（并跳过它）
        if is_ident_start(self.chars[self.pos]) {
            while self.pos < self.chars.len() && is_ident_char(self.chars[self.pos]) {
                self.pos += 1;
            }
            return Some(0);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn pp(src: &str) -> String {
        preprocess(src, Path::new(".")).unwrap()
    }

    fn norm(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn object_macro() {
        assert_eq!(norm(&pp("#define N 42\nint x = N;")), "int x = 42;");
    }

    #[test]
    fn recursive_object_macro() {
        assert_eq!(norm(&pp("#define A B\n#define B 5\nint x = A;")), "int x = 5;");
    }

    #[test]
    fn function_macro() {
        assert_eq!(
            norm(&pp("#define ADD(a,b) ((a)+(b))\nint x = ADD(1, 2);")),
            "int x = ((1)+(2));"
        );
    }

    #[test]
    fn stringize() {
        assert_eq!(norm(&pp("#define STR(x) #x\nchar* s = STR(hi);")), "char* s = \"hi\";");
    }

    #[test]
    fn token_paste() {
        assert_eq!(norm(&pp("#define CAT(a,b) a##b\nint CAT(foo, bar);")), "int foobar;");
    }

    #[test]
    fn ifdef_conditional() {
        let s = pp("#define FEAT\n#ifdef FEAT\nint a;\n#else\nint b;\n#endif");
        assert!(s.contains("int a;"));
        assert!(!s.contains("int b;"));
    }

    #[test]
    fn ifndef_conditional() {
        let s = pp("#ifndef FOO\nint a;\n#endif");
        assert!(s.contains("int a;"));
    }

    #[test]
    fn if_expression() {
        let s = pp("#define V 3\n#if V > 2\nint big;\n#else\nint small;\n#endif");
        assert!(s.contains("int big;"));
        assert!(!s.contains("int small;"));
    }

    #[test]
    fn if_defined() {
        let s = pp("#define X\n#if defined(X)\nint a;\n#endif");
        assert!(s.contains("int a;"));
    }

    #[test]
    fn elif_chain() {
        let s = pp("#define V 2\n#if V == 1\nint a;\n#elif V == 2\nint b;\n#else\nint c;\n#endif");
        assert!(s.contains("int b;") && !s.contains("int a;") && !s.contains("int c;"));
    }

    #[test]
    fn undef_macro() {
        let s = pp("#define N 1\n#undef N\n#ifdef N\nint a;\n#endif");
        assert!(!s.contains("int a;"));
    }
}
