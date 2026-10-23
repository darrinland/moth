//! Generates the variation code from `variations.def`.
//!
//! The catalog is the single source for every variation. This script parses it once and
//! writes two files into `OUT_DIR`: `variations.rs`, the `Variation` enum and its CPU `apply`
//! (included by `flame/variations.rs`), and `variations.wgsl`, the same maths as a WGSL
//! function for the GPU renderer. Both come from one parse, so they cannot disagree.

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::PathBuf;

/// Names that every body may read and that no `let` may redefine.
const RESERVED: &[&str] = &["x", "y", "a", "b", "c", "d", "e", "f", "PI", "EPS"];
const FUNCTIONS_1: &[&str] = &[
    "sin", "cos", "tan", "sinh", "cosh", "exp", "sqrt", "floor", "abs", "lt0",
];
const FUNCTIONS_2: &[&str] = &["atan2"];
const FUNCTIONS_0: &[&str] = &["coin"];

#[derive(Clone, Debug)]
enum Expr {
    Num(f64),
    Var(String),
    Neg(Box<Expr>),
    Bin(char, Box<Expr>, Box<Expr>),
    Call(String, Vec<Expr>),
}

struct Variation {
    name: String,
    flam3: String,
    lets: Vec<(String, Expr)>,
    out: (Expr, Expr),
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Ident(String),
    Num(f64),
    Str(String),
    Sym(char),
}

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let source = manifest.join("variations.def");
    println!("cargo:rerun-if-changed={}", source.display());
    println!("cargo:rerun-if-changed=build.rs");

    let text = std::fs::read_to_string(&source).expect("read variations.def");
    let variations = parse(&text);
    std::fs::write(out_dir.join("variations.rs"), rust_source(&variations)).unwrap();
    std::fs::write(out_dir.join("variations.wgsl"), wgsl_source(&variations)).unwrap();
}

fn tokenize(src: &str) -> Vec<Tok> {
    let chars: Vec<char> = src.chars().collect();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
        } else if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            toks.push(Tok::Ident(chars[start..i].iter().collect()));
        } else if c.is_ascii_digit() {
            let start = i;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                i += 1;
            }
            if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                i += 1;
                if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            let text: String = chars[start..i].iter().collect();
            let value = text
                .parse()
                .unwrap_or_else(|_| panic!("variations.def: bad number {text}"));
            toks.push(Tok::Num(value));
        } else if c == '"' {
            let start = i + 1;
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                i += 1;
            }
            toks.push(Tok::Str(chars[start..i].iter().collect()));
            i += 1;
        } else {
            toks.push(Tok::Sym(c));
            i += 1;
        }
    }
    toks
}

struct Parser {
    toks: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Tok {
        let tok = self
            .toks
            .get(self.pos)
            .cloned()
            .expect("variations.def: unexpected end of file");
        self.pos += 1;
        tok
    }

    fn is_sym(&self, sym: char) -> bool {
        matches!(self.peek(), Some(Tok::Sym(c)) if *c == sym)
    }

    fn is_ident(&self, word: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(w)) if w == word)
    }

    fn expect_sym(&mut self, sym: char) {
        match self.next() {
            Tok::Sym(c) if c == sym => {}
            other => panic!("variations.def: expected '{sym}', found {other:?}"),
        }
    }

    fn expect_word(&mut self, word: &str) {
        match self.next() {
            Tok::Ident(w) if w == word => {}
            other => panic!("variations.def: expected '{word}', found {other:?}"),
        }
    }

    fn ident(&mut self) -> String {
        match self.next() {
            Tok::Ident(name) => name,
            other => panic!("variations.def: expected a name, found {other:?}"),
        }
    }

    fn variation(&mut self) -> Variation {
        self.expect_word("variation");
        let name = self.ident();
        let flam3 = match self.next() {
            Tok::Str(s) => s,
            other => panic!("variations.def: {name} needs its flam3 name, found {other:?}"),
        };
        self.expect_sym('{');
        let mut lets = Vec::new();
        while self.is_ident("let") {
            self.next();
            let local = self.ident();
            self.expect_sym('=');
            let value = self.expr();
            self.expect_sym(';');
            lets.push((local, value));
        }
        self.expect_word("out");
        self.expect_sym('(');
        let x = self.expr();
        self.expect_sym(',');
        let y = self.expr();
        self.expect_sym(')');
        if self.is_sym(';') {
            self.next();
        }
        self.expect_sym('}');
        Variation {
            name,
            flam3,
            lets,
            out: (x, y),
        }
    }

    fn expr(&mut self) -> Expr {
        let mut left = self.term();
        while let Some(Tok::Sym(op @ ('+' | '-'))) = self.peek().cloned() {
            self.next();
            let right = self.term();
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        left
    }

    fn term(&mut self) -> Expr {
        let mut left = self.unary();
        while let Some(Tok::Sym(op @ ('*' | '/'))) = self.peek().cloned() {
            self.next();
            let right = self.unary();
            left = Expr::Bin(op, Box::new(left), Box::new(right));
        }
        left
    }

    fn unary(&mut self) -> Expr {
        if self.is_sym('-') {
            self.next();
            Expr::Neg(Box::new(self.unary()))
        } else {
            self.primary()
        }
    }

    fn primary(&mut self) -> Expr {
        match self.next() {
            Tok::Num(value) => Expr::Num(value),
            Tok::Ident(name) if self.is_sym('(') => {
                self.next();
                let mut args = Vec::new();
                if !self.is_sym(')') {
                    loop {
                        args.push(self.expr());
                        if !self.is_sym(',') {
                            break;
                        }
                        self.next();
                    }
                }
                self.expect_sym(')');
                Expr::Call(name, args)
            }
            Tok::Ident(name) => Expr::Var(name),
            Tok::Sym('(') => {
                let inner = self.expr();
                self.expect_sym(')');
                inner
            }
            other => panic!("variations.def: unexpected {other:?} in an expression"),
        }
    }
}

fn parse(text: &str) -> Vec<Variation> {
    let mut parser = Parser {
        toks: tokenize(text),
        pos: 0,
    };
    let mut variations = Vec::new();
    while parser.peek().is_some() {
        variations.push(parser.variation());
    }
    validate(&variations);
    variations
}

/// Checks names and arities so a typo fails the build with a message, not later in a shader.
fn validate(variations: &[Variation]) {
    for v in variations {
        let mut known: HashSet<&str> = RESERVED.iter().copied().collect();
        for (local, value) in &v.lets {
            check_expr(value, &known, &v.name);
            assert!(
                !known.contains(local.as_str()),
                "variations.def: {}: '{local}' is already defined or reserved",
                v.name
            );
            known.insert(local);
        }
        check_expr(&v.out.0, &known, &v.name);
        check_expr(&v.out.1, &known, &v.name);
    }
}

fn check_expr(expr: &Expr, known: &HashSet<&str>, who: &str) {
    match expr {
        Expr::Num(_) => {}
        Expr::Var(name) => assert!(
            known.contains(name.as_str()),
            "variations.def: {who}: unknown name '{name}'"
        ),
        Expr::Neg(inner) => check_expr(inner, known, who),
        Expr::Bin(_, left, right) => {
            check_expr(left, known, who);
            check_expr(right, known, who);
        }
        Expr::Call(name, args) => {
            let arity = if FUNCTIONS_1.contains(&name.as_str()) {
                1
            } else if FUNCTIONS_2.contains(&name.as_str()) {
                2
            } else if FUNCTIONS_0.contains(&name.as_str()) {
                0
            } else {
                panic!("variations.def: {who}: unknown function '{name}'")
            };
            assert_eq!(
                args.len(),
                arity,
                "variations.def: {who}: {name} takes {arity} argument(s)"
            );
            for arg in args {
                check_expr(arg, known, who);
            }
        }
    }
}

/// Rust source: an expression with any compound parts parenthesized.
fn rust_expr(expr: &Expr) -> String {
    match expr {
        Expr::Num(value) => format!("{value:?}"),
        Expr::Var(name) => match name.as_str() {
            "a" | "b" | "c" | "d" | "e" | "f" => format!("aff.{name}"),
            _ => name.clone(),
        },
        Expr::Neg(inner) => format!("-{}", rust_operand(inner)),
        Expr::Bin(op, left, right) => {
            format!("{} {op} {}", rust_operand(left), rust_operand(right))
        }
        Expr::Call(name, args) => match name.as_str() {
            "atan2" => format!(
                "{}.atan2({})",
                rust_operand(&args[0]),
                rust_operand(&args[1])
            ),
            "lt0" => format!(
                "(if {} < 0.0 {{ 1.0 }} else {{ 0.0 }})",
                rust_operand(&args[0])
            ),
            "coin" => "(if rng.random_bool(0.5) { 1.0 } else { 0.0 })".to_string(),
            _ => format!("{}.{name}()", rust_operand(&args[0])),
        },
    }
}

fn rust_operand(expr: &Expr) -> String {
    match expr {
        Expr::Bin(..) | Expr::Neg(_) => format!("({})", rust_expr(expr)),
        _ => rust_expr(expr),
    }
}

/// WGSL source: the same expression, with PI and EPS written out as literals.
fn wgsl_expr(expr: &Expr) -> String {
    match expr {
        Expr::Num(value) => format!("{value:?}"),
        Expr::Var(name) => match name.as_str() {
            "PI" => "3.141592653589793".to_string(),
            "EPS" => "1e-10".to_string(),
            _ => name.clone(),
        },
        Expr::Neg(inner) => format!("-{}", wgsl_operand(inner)),
        Expr::Bin(op, left, right) => {
            format!("{} {op} {}", wgsl_operand(left), wgsl_operand(right))
        }
        Expr::Call(name, args) => match name.as_str() {
            "lt0" => format!("select(0.0, 1.0, {} < 0.0)", wgsl_operand(&args[0])),
            "coin" => "coin".to_string(),
            _ => {
                let args: Vec<String> = args.iter().map(wgsl_expr).collect();
                format!("{name}({})", args.join(", "))
            }
        },
    }
}

fn wgsl_operand(expr: &Expr) -> String {
    match expr {
        Expr::Bin(..) | Expr::Neg(_) => format!("({})", wgsl_expr(expr)),
        _ => wgsl_expr(expr),
    }
}

fn rust_source(variations: &[Variation]) -> String {
    let mut s = String::new();
    s.push_str("// Generated by build.rs from variations.def. Edit that file, not this one.\n\n");
    s.push_str("#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum Variation {\n");
    for v in variations {
        writeln!(s, "    {},", v.name).unwrap();
    }
    s.push_str("}\n\nimpl Variation {\n");
    s.push_str("    pub const ALL: &'static [Variation] = &[\n");
    for v in variations {
        writeln!(s, "        Variation::{},", v.name).unwrap();
    }
    s.push_str("    ];\n\n    pub fn name(self) -> &'static str {\n        match self {\n");
    for v in variations {
        writeln!(s, "            Variation::{} => \"{}\",", v.name, v.flam3).unwrap();
    }
    s.push_str("        }\n    }\n\n");
    s.push_str(
        "    /// Applies the unweighted variation to a point that has already been through\n\
         \x20   /// the transform's affine (`aff` supplies the coefficients). `rng` is used only by\n\
         \x20   /// the variations that are random, such as `julia`.\n\
         \x20   #[inline]\n\
         \x20   pub fn apply<R: Rng + ?Sized>(self, x: f64, y: f64, aff: &Affine, rng: &mut R) -> (f64, f64) {\n\
         \x20       match self {\n",
    );
    for v in variations {
        if v.lets.is_empty() {
            writeln!(
                s,
                "            Variation::{} => ({}, {}),",
                v.name,
                rust_expr(&v.out.0),
                rust_expr(&v.out.1)
            )
            .unwrap();
        } else {
            writeln!(s, "            Variation::{} => {{", v.name).unwrap();
            for (local, value) in &v.lets {
                writeln!(s, "                let {local} = {};", rust_expr(value)).unwrap();
            }
            writeln!(
                s,
                "                ({}, {})\n            }}",
                rust_expr(&v.out.0),
                rust_expr(&v.out.1)
            )
            .unwrap();
        }
    }
    s.push_str("        }\n    }\n}\n");
    s
}

fn wgsl_source(variations: &[Variation]) -> String {
    let mut s = String::new();
    s.push_str("// Generated by build.rs from variations.def. Edit that file, not this one.\n");
    s.push_str(
        "// `id` is the variation's index in `Variation::ALL`. `coin` is 0.0 or 1.0 at random.\n\n",
    );
    s.push_str(
        "fn variation_apply(id: u32, x: f32, y: f32, a: f32, b: f32, c: f32, d: f32, e: f32, \
         f: f32, coin: f32) -> vec2<f32> {\n    switch id {\n",
    );
    for (index, v) in variations.iter().enumerate() {
        writeln!(s, "        case {index}u: {{").unwrap();
        for (local, value) in &v.lets {
            writeln!(s, "            let {local} = {};", wgsl_expr(value)).unwrap();
        }
        writeln!(
            s,
            "            return vec2<f32>({}, {});\n        }}",
            wgsl_expr(&v.out.0),
            wgsl_expr(&v.out.1)
        )
        .unwrap();
    }
    s.push_str("        default: {\n            return vec2<f32>(x, y);\n        }\n    }\n}\n");
    s
}
