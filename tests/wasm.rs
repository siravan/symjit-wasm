//! symjit-wasm end to end: models are compiled into modules, validated with wabt's
//! `wasm-validate` and run in Node through js/symjit.mjs (tests/runner.mjs). Their outputs are
//! compared with symjit's JIT (`stable`, a dev-dependency) and with the crate's bytecode
//! interpreter. The steps that need `node` or `wasm-validate` are skipped when the tool is not
//! on PATH.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};
use symjit_wasm as sw;
use symjit_wasm::Storage;

const POINTS: [[f64; 3]; 4] = [
    [0.5, 2.0, 0.25],
    [-1.25, 0.75, 1.5],
    [1.5, -0.5, 0.125],
    [0.0, 3.0, -0.75],
];

fn dir(name: &str) -> PathBuf {
    let d = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("wasm")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn have(tool: &str) -> bool {
    let ok = Command::new(tool)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !ok {
        eprintln!("skipped: `{}` is not available", tool);
    }
    ok
}

/****************************** models ******************************/

// the same model in both crates (symjit-wasm and symjit)
macro_rules! trig {
    ($c:ident) => {{
        // sin(x)*y + exp(-x*x), cos(y)/(1 + x*x), x*y - z
        let (x, y, z) = ($c::var("x"), $c::var("y"), $c::var("z"));
        let one = $c::Expr::from(1.0);
        let a = &(&x.sin() * &y) + &(-&(&x * &x)).exp();
        let b = &y.cos() / &(&one + &(&x * &x));
        let c = &(&x * &y) - &z;
        (vec![x, y, z], vec![a, b, c])
    }};
}

macro_rules! rational {
    ($c:ident) => {{
        // rational functions and square roots only: exact in WebAssembly
        let (x, y, z) = ($c::var("x"), $c::var("y"), $c::var("z"));
        let one = $c::Expr::from(1.0);
        let a = &(&(&x * &x) - &(&y * &z)) / &(&one + &(&z * &z));
        let b = &(&(&x * &y).sqrt() + &z) * &(&x - &$c::Expr::from(0.75));
        let c = &(&one / &(&(&(&x * &x) + &(&y * &y)) + &one)) - &z;
        (vec![x, y, z], vec![a, b, c])
    }};
}

macro_rules! big {
    ($c:ident) => {{
        // sum of (k x + y)^2 / (1 + z^2 k), k = 1..39: register pressure
        let (x, y, z) = ($c::var("x"), $c::var("y"), $c::var("z"));
        let one = $c::Expr::from(1.0);
        let mut s = $c::Expr::from(0.0);
        let mut t = $c::Expr::from(1.0);
        for k in 1..40 {
            let k = $c::Expr::from(k as f64);
            let u = &(&k * &x) + &y;
            s = &s + &(&(&u * &u) / &(&one + &(&(&z * &z) * &k)));
            t = &t * &(&one + &(&u / &(&k * &k)));
        }
        (vec![x, y, z], vec![s, t])
    }};
}

fn config(opt_level: u8) -> sw::Config {
    let mut config = sw::Config::new(sw::CompilerType::Wasm, 0).unwrap();
    config.set_opt_level(opt_level);
    config.set_cse(true);
    config
}

fn jit_config(opt_level: u8) -> symjit::Config {
    let mut config = symjit::Config::new(symjit::CompilerType::Native, 0).unwrap();
    config.set_opt_level(opt_level);
    config.set_cse(true);
    config.set_fastmath(false);
    config
}

// the option word of a Symbolica model (symjit's `compile_evaluator`)
fn symbolica_opt(opt_level: u32, complex: bool) -> u32 {
    let cse = 0x4;
    let symbolica = 0x40;
    let compact = 0x1000;
    cse | symbolica | compact | if complex { 0x20 } else { 0 } | (opt_level << 8)
}

/****************************** running ******************************/

fn write_module(d: &Path, name: &str, wasm: &[u8]) -> PathBuf {
    let path = d.join(format!("{}.wasm", name));
    std::fs::write(&path, wasm).unwrap();
    if have("wasm-validate") {
        let out = Command::new("wasm-validate").arg(&path).output().unwrap();
        assert!(
            out.status.success(),
            "{}: {}",
            name,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    path
}

fn encode(x: f64) -> Value {
    if x.is_finite() && !(x == 0.0 && x.is_sign_negative()) {
        json!(x)
    } else if x == 0.0 {
        json!("-0")
    } else if x.is_nan() {
        json!("NaN")
    } else if x > 0.0 {
        json!("Infinity")
    } else {
        json!("-Infinity")
    }
}

fn decode(v: &Value) -> f64 {
    match v {
        Value::String(s) => match s.as_str() {
            "-0" => -0.0,
            "NaN" => f64::NAN,
            "Infinity" => f64::INFINITY,
            "-Infinity" => f64::NEG_INFINITY,
            s => panic!("unexpected value {}", s),
        },
        v => v.as_f64().unwrap(),
    }
}

fn points(points: &[Vec<f64>]) -> Value {
    Value::Array(
        points
            .iter()
            .map(|p| Value::Array(p.iter().map(|x| encode(*x)).collect()))
            .collect(),
    )
}

/// Runs the requests (see tests/runner.mjs) in Node; None without node.
fn node(requests: Vec<Value>) -> Option<Vec<Value>> {
    if !have("node") {
        return None;
    }
    let runner = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/runner.mjs");
    let mut child = Command::new("node")
        .arg(runner)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(Value::Array(requests).to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
    Some(answer.as_array().unwrap().clone())
}

/// The outputs of one request; panics on an error.
fn obs(answer: &Value) -> Vec<Vec<f64>> {
    if let Some(e) = answer.get("error") {
        panic!("{}", e);
    }
    answer["obs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row.as_array().unwrap().iter().map(decode).collect())
        .collect()
}

fn run(module: &Path, kind: &str, pts: &[Vec<f64>], host: Value) -> Option<Vec<Vec<f64>>> {
    let req = json!({"module": module, "kind": kind, "points": points(pts), "host": host});
    node(vec![req]).map(|a| obs(&a[0]))
}

fn close(a: &[Vec<f64>], b: &[Vec<f64>], rtol: f64) {
    assert_eq!(a.len(), b.len());
    for (ra, rb) in a.iter().zip(b) {
        assert_eq!(ra.len(), rb.len());
        for (x, y) in ra.iter().zip(rb) {
            assert!(
                (x - y).abs() <= rtol * y.abs().max(1e-300),
                "{:?} != {:?}",
                a,
                b
            );
        }
    }
}

// the bits of the outputs, with every NaN as the same one: WebAssembly does not specify the
// sign and payload of a NaN result
fn bits(a: &[Vec<f64>]) -> Vec<Vec<u64>> {
    a.iter()
        .map(|r| {
            r.iter()
                .map(|x| if x.is_nan() { f64::NAN } else { *x }.to_bits())
                .collect()
        })
        .collect()
}

fn pts() -> Vec<Vec<f64>> {
    POINTS.iter().map(|p| p.to_vec()).collect()
}

/// A model of states on the crate's bytecode interpreter.
fn interpret(app: &mut sw::Application, pts: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let (s, o) = (app.count_states, app.count_obs);
    pts.iter()
        .map(|p| {
            let mem = app.bytecode.mem_mut();
            mem.fill(0.0);
            mem[..s].copy_from_slice(p);
            app.bytecode.exec(&[0.0]);
            app.bytecode.mem()[s..s + o].to_vec()
        })
        .collect()
}

/****************************** tests ******************************/

#[test]
fn expr_models_match_the_jit() {
    let d = dir("expr");
    for opt_level in [0, 1, 2, 3] {
        for (name, exact) in [("trig", false), ("rational", true), ("big", true)] {
            let (states, obs) = match name {
                "trig" => trig!(sw),
                "rational" => rational!(sw),
                _ => big!(sw),
            };
            let mut app = sw::Compiler::with_config(config(opt_level))
                .compile(&states, &obs)
                .unwrap();
            let module = write_module(&d, &format!("{}{}", name, opt_level), app.wasm().unwrap());

            let (states, obs) = match name {
                "trig" => trig!(symjit),
                "rational" => rational!(symjit),
                _ => big!(symjit),
            };
            let mut jit = symjit::Compiler::with_config(jit_config(opt_level))
                .compile(&states, &obs)
                .unwrap();
            let reference: Vec<Vec<f64>> = POINTS.iter().map(|p| jit.call(p)).collect();
            let interpreted = interpret(&mut app, &pts());

            if let Some(obs) = run(&module, "call", &pts(), json!({})) {
                let what = format!("{} at opt level {}", name, opt_level);
                if exact {
                    assert_eq!(bits(&obs), bits(&reference), "{}", what);
                    assert_eq!(bits(&obs), bits(&interpreted), "{}", what);
                } else {
                    // JavaScript's Math functions are not Rust's (or glibc's)
                    close(&obs, &reference, 1e-14);
                    close(&obs, &interpreted, 1e-14);
                }
            }
        }
    }
}

#[test]
fn fast_export() {
    // one observable and no parameters: the module also exports `fast(x, y, z)`
    let d = dir("fast");
    let (states, obs) = rational!(sw);
    let app = sw::Compiler::with_config(config(2))
        .compile(&states, &obs[..1])
        .unwrap();
    assert!(app.has_fast());
    let module = write_module(&d, "one", app.wasm().unwrap());
    let (states, obs) = rational!(symjit);
    let mut jit = symjit::Compiler::with_config(jit_config(2))
        .compile(&states, &obs[..1])
        .unwrap();
    let reference: Vec<Vec<f64>> = POINTS.iter().map(|p| jit.call(p)).collect();
    if let Some(fast) = run(&module, "fast", &pts(), json!({})) {
        assert_eq!(bits(&fast), bits(&reference));
        let call = run(&module, "call", &pts(), json!({})).unwrap();
        assert_eq!(bits(&call), bits(&reference));
    }
}

// a Symbolica evaluator's instructions, in the text form of `Compiler::translate`:
// arithmetic, powers and a division of the inputs (exact) ...
const ARITHMETIC: &str = "([\
    ('add', ('temp', 0), [('param', 0), ('param', 1), ('const', 0)], 0),\
    ('mul', ('temp', 1), [('temp', 0), ('param', 2), ('param', 0)], 0),\
    ('div', ('temp', 2), ('temp', 1), ('param', 1), False, False),\
    ('pow', ('temp', 3), ('param', 2), 3, False),\
    ('pow', ('temp', 4), ('temp', 0), -2, False),\
    ('add', ('out', 0), [('temp', 2), ('temp', 3)], 0),\
    ('mul', ('out', 1), [('temp', 4), ('const', 1)], 0),\
    ('assign', ('out', 2), ('temp', 1))\
    ], 5, [3/4, 5/2])";

// ... math functions ...
const FUNCTIONS: &str = "([\
    ('fun', ('temp', 0), sin, [], [('param', 0)], False),\
    ('fun', ('temp', 1), exp, [], [('param', 1)], False),\
    ('fun', ('temp', 2), log, [], [('param', 2)], False),\
    ('fun', ('temp', 3), sqrt, [], [('param', 0)], False),\
    ('mul', ('out', 0), [('temp', 0), ('temp', 1)], 0),\
    ('add', ('out', 1), [('temp', 2), ('temp', 3)], 0)\
    ], 4, [])";

// ... and a branch: out0 = p0 < p1 ? p0 * p2 : p1 + p2 (the condition from symjit's `lt`)
const BRANCH: &str = "([\
    ('fun', ('temp', 0), lt, [], [('param', 0), ('param', 1)], True),\
    ('if_else', ('temp', 0), 0),\
    ('mul', ('temp', 1), [('param', 0), ('param', 2)], 3),\
    ('goto', 1),\
    ('label', 0),\
    ('add', ('temp', 2), [('param', 1), ('param', 2)], 3),\
    ('label', 1),\
    ('join', ('out', 0), ('temp', 0), ('temp', 1), ('temp', 2))\
    ], 3, [])";

fn translate(text: &str, opt: u32, df: Option<sw::Defuns>) -> sw::Application {
    let mut config = sw::Config::from_name("wasm", opt).unwrap();
    if let Some(df) = df {
        config.set_defuns(df);
    }
    sw::Compiler::with_config(config)
        .translate(text.to_string(), 3)
        .unwrap()
}

fn jit_translate(text: &str, opt: u32) -> symjit::Application {
    symjit::Compiler::with_config(symjit::Config::from_name("native", opt).unwrap())
        .translate(text.to_string(), 3)
        .unwrap()
}

#[test]
fn symbolica_models_match_the_jit() {
    let d = dir("symbolica");
    for (name, text, exact, complex) in [
        ("arithmetic", ARITHMETIC, true, false),
        ("functions", FUNCTIONS, false, false),
        ("arithmetic_c", ARITHMETIC, true, true),
        ("functions_c", FUNCTIONS, false, true),
    ] {
        for opt_level in [0, 1, 2, 3] {
            let opt = symbolica_opt(opt_level, complex);
            let mut app = translate(text, opt, None);
            let module = write_module(&d, &format!("{}{}", name, opt_level), app.wasm().unwrap());
            let jit = jit_translate(text, opt);
            let what = format!("{} at opt level {}", name, opt_level);

            // the inputs of a point: three reals, or three complex numbers (re, im)
            let inputs: Vec<Vec<f64>> = POINTS
                .iter()
                .map(|p| {
                    if complex {
                        p.iter().flat_map(|x| [*x + 0.5, *x * 0.25 - 1.0]).collect()
                    } else {
                        p.iter().map(|x| x.abs() + 0.5).collect()
                    }
                })
                .collect();
            let n_out = if text == ARITHMETIC { 3 } else { 2 };
            let reference: Vec<Vec<f64>> = inputs
                .iter()
                .map(|p| {
                    if complex {
                        let args: Vec<num_complex::Complex<f64>> = p
                            .chunks(2)
                            .map(|c| num_complex::Complex::new(c[0], c[1]))
                            .collect();
                        let mut outs = vec![num_complex::Complex::new(0.0, 0.0); n_out];
                        jit.evaluate(&args, &mut outs);
                        outs.iter().flat_map(|z| [z.re, z.im]).collect()
                    } else {
                        let mut outs = vec![0.0; n_out];
                        jit.evaluate(p, &mut outs);
                        outs
                    }
                })
                .collect();
            let interpreted: Vec<Vec<f64>> = inputs
                .iter()
                .map(|p| {
                    let mut outs = vec![0.0; n_out * if complex { 2 } else { 1 }];
                    app.interpret(p, &mut outs);
                    outs
                })
                .collect();

            for kind in ["evaluate", "matrix"] {
                if let Some(obs) = run(&module, kind, &inputs, json!({})) {
                    if exact {
                        assert_eq!(bits(&obs), bits(&reference), "{} ({})", what, kind);
                        assert_eq!(bits(&obs), bits(&interpreted), "{} ({})", what, kind);
                    } else {
                        close(&obs, &reference, 1e-14);
                        close(&obs, &interpreted, 1e-14);
                    }
                }
            }
        }
    }
}

#[test]
fn branches_match_the_jit() {
    let d = dir("branch");
    for opt_level in [0, 1, 2, 3] {
        let opt = symbolica_opt(opt_level, false);
        let app = translate(BRANCH, opt, None);
        let module = write_module(&d, &format!("branch{}", opt_level), app.wasm().unwrap());
        let jit = jit_translate(BRANCH, opt);
        let inputs: Vec<Vec<f64>> = pts();
        let reference: Vec<Vec<f64>> = inputs
            .iter()
            .map(|p| {
                let mut outs = vec![0.0];
                jit.evaluate(p, &mut outs);
                outs
            })
            .collect();
        // both sides of the branch are taken
        let taken: Vec<bool> = inputs.iter().map(|p| p[0] < p[1]).collect();
        assert!(taken.contains(&true) && taken.contains(&false));
        if let Some(obs) = run(&module, "evaluate", &inputs, json!({})) {
            assert_eq!(bits(&obs), bits(&reference), "opt level {}", opt_level);
        }
    }
}

#[test]
fn user_functions_are_imported() {
    // functions declared in `Defuns` are imported from the module "symjit" and supplied by
    // the host
    let d = dir("defuns");
    let real = "([\
        ('external_fun', ('temp', 0), hyp, [('param', 0), ('param', 1)]),\
        ('external_fun', ('temp', 1), sq, [('param', 2)]),\
        ('add', ('out', 0), [('temp', 0), ('temp', 1)], 3)\
        ], 2, [])";
    let mut df = sw::Defuns::new();
    df.add_func("hyp", 2).unwrap();
    df.add_unary("sq").unwrap();
    let app = translate(real, symbolica_opt(2, false), Some(df));
    let module = write_module(&d, "real", app.wasm().unwrap());
    let host = json!({"hyp": "(x, y) => Math.hypot(x, y)", "sq": "(x) => x * x"});
    let reference: Vec<Vec<f64>> = POINTS
        .iter()
        .map(|p| vec![p[0].hypot(p[1]) + p[2] * p[2]])
        .collect();
    if let Some(obs) = run(&module, "evaluate", &pts(), host) {
        close(&obs, &reference, 1e-15);
    }

    let complex = "([\
        ('external_fun', ('temp', 0), twice, [('param', 0)]),\
        ('mul', ('out', 0), [('temp', 0), ('param', 1)], 0)\
        ], 1, [])";
    let mut df = sw::Defuns::new();
    df.add_unary_complex("twice").unwrap();
    let app = translate(complex, symbolica_opt(2, true), Some(df));
    let module = write_module(&d, "complex", app.wasm().unwrap());
    let host = json!({"cplx_twice": "(re, im) => [2 * re, 2 * im]"});
    let inputs: Vec<Vec<f64>> = POINTS
        .iter()
        // three complex inputs (`translate`), the third unused
        .map(|p| vec![p[0], p[1], p[2], 0.5, 0.0, 0.0])
        .collect();
    let reference: Vec<Vec<f64>> = inputs
        .iter()
        .map(|p| {
            let z = num_complex::Complex::new(2.0 * p[0], 2.0 * p[1])
                * num_complex::Complex::new(p[2], p[3]);
            vec![z.re, z.im]
        })
        .collect();
    if let Some(obs) = run(&module, "evaluate", &inputs, host) {
        close(&obs, &reference, 1e-15);
    }

    // an `Expr` model (as SymPy models) calling them
    let (x, y, z) = (sw::var("x"), sw::var("y"), sw::var("z"));
    let obs = &sw::Expr::binary("hyp", &x, &y) - &sw::Expr::unary("sq", &z);
    let mut df = sw::Defuns::new();
    df.add_func("hyp", 2).unwrap();
    df.add_unary("sq").unwrap();
    let mut config = config(2);
    config.set_defuns(df);
    let app = sw::Compiler::with_config(config)
        .compile(&[x, y, z], &[obs])
        .unwrap();
    let expr = write_module(&d, "expr", app.wasm().unwrap());
    let host = json!({"hyp": "(x, y) => Math.hypot(x, y)", "sq": "(x) => x * x"});
    let reference: Vec<Vec<f64>> = POINTS
        .iter()
        .map(|p| vec![p[0].hypot(p[1]) - p[2] * p[2]])
        .collect();
    if let Some(obs) = run(&expr, "call", &pts(), host) {
        close(&obs, &reference, 1e-15);
    }

    // a host that lacks the function cannot instantiate the module
    if let Some(answer) = node(vec![
        json!({"module": module, "kind": "evaluate", "points": points(&inputs)}),
    ]) {
        let e = answer[0]["error"].as_str().unwrap();
        assert!(
            e.contains("no host implementation for: cplx_twice"),
            "{}",
            e
        );
    }
}

#[test]
fn invalid_functions_are_rejected() {
    let mut df = sw::Defuns::new();
    assert!(df.add_func("f3", 3).is_err());
    assert!(df.add_func("sin", 1).is_err());
    // an undeclared function
    let text = "([('external_fun', ('out', 0), nowhere, [('param', 0)])], 0, [])";
    let mut config = sw::Config::from_name("wasm", symbolica_opt(2, false)).unwrap();
    config.set_defuns(sw::Defuns::new());
    assert!(sw::Compiler::with_config(config)
        .translate(text.to_string(), 1)
        .is_err());
}

#[test]
fn modules_describe_their_calls() {
    let (states, obs) = trig!(sw);
    let app = sw::Compiler::with_config(config(2))
        .compile(&states, &obs)
        .unwrap();
    let wasm = app.wasm().unwrap();
    let d = dir("shape");
    let module = write_module(&d, "trig", wasm);
    if let Some(answer) = node(vec![
        json!({"module": module, "kind": "call", "points": points(&pts())}),
    ]) {
        let shape = &answer[0]["shape"];
        assert_eq!(shape["states"], 3);
        assert_eq!(shape["obs"], 3);
        assert_eq!(shape["params"], 0);
        assert_eq!(shape["complex"], false);
        assert_eq!(shape["fast"], false);
        let imports: Vec<&str> = answer[0]["imports"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for f in ["sin", "cos", "exp"] {
            assert!(imports.contains(&f), "{:?}", imports);
        }
    }
    // the applet holds the module and the shape
    let applet = app.seal().unwrap();
    assert_eq!(
        (applet.count_states, applet.count_obs, applet.has_fast),
        (3, 3, false)
    );
    assert!(applet.wasm().starts_with(b"\0asm\x01\0\0\0"));

    // a bytecode application has no module
    let (states, obs) = trig!(sw);
    let config = sw::Config::new(sw::CompilerType::ByteCode, 0).unwrap();
    let app = sw::Compiler::with_config(config)
        .compile(&states, &obs)
        .unwrap();
    assert!(app.wasm().is_none());
    assert!(app.seal().is_err());
}

#[test]
fn sjb_files_are_exchanged_with_symjit() {
    let d = dir("sjb");
    let reference = {
        let (states, obs) = big!(symjit);
        let mut jit = symjit::Compiler::with_config(jit_config(3))
            .compile(&states, &obs)
            .unwrap();
        // saved by symjit (machine code for this machine), loaded here
        let mut bytes = Vec::new();
        symjit::Storage::save(&jit, &mut bytes).unwrap();
        std::fs::write(d.join("from_symjit.sjb"), &bytes).unwrap();
        POINTS.iter().map(|p| jit.call(p)).collect::<Vec<_>>()
    };
    let bytes = std::fs::read(d.join("from_symjit.sjb")).unwrap();
    let config = sw::Config::new(sw::CompilerType::Wasm, 0).unwrap();
    let app = sw::Application::load(&mut bytes.as_slice(), &config).unwrap();
    let module = write_module(&d, "from_symjit", app.wasm().unwrap());
    if let Some(obs) = run(&module, "call", &pts(), json!({})) {
        assert_eq!(bits(&obs), bits(&reference));
    }

    // saved here, loaded back (the same module) and by symjit (the same results)
    let mut saved = Vec::new();
    app.save(&mut saved).unwrap();
    let again = sw::Application::load(&mut saved.as_slice(), &config).unwrap();
    assert_eq!(again.wasm(), app.wasm());
    let jit_config = symjit::Config::new(symjit::CompilerType::Native, 0).unwrap();
    let mut jit = symjit::Application::load(&mut saved.as_slice(), &jit_config).unwrap();
    let obs: Vec<Vec<f64>> = POINTS.iter().map(|p| jit.call(p)).collect();
    assert_eq!(bits(&obs), bits(&reference));

    // a complex Symbolica model, both ways
    let opt = symbolica_opt(2, true);
    let app = translate(FUNCTIONS, opt, None);
    let mut saved = Vec::new();
    app.save(&mut saved).unwrap();
    let again = sw::Application::load(&mut saved.as_slice(), &config).unwrap();
    assert_eq!(again.wasm(), app.wasm());
    let jit = symjit::Application::load(&mut saved.as_slice(), &jit_config).unwrap();
    let args = [
        num_complex::Complex::new(0.5, -0.25),
        num_complex::Complex::new(1.5, 0.75),
        num_complex::Complex::new(2.0, 1.0),
    ];
    let mut outs = vec![num_complex::Complex::new(0.0, 0.0); 2];
    jit.evaluate(&args, &mut outs);
    let mut wasm_app = again;
    let mut interpreted = vec![num_complex::Complex::new(0.0, 0.0); 2];
    wasm_app.interpret(&args, &mut interpreted);
    for (a, b) in outs.iter().zip(&interpreted) {
        assert!((a - b).norm() <= 1e-14 * a.norm());
    }
}

/// The crate compiled to WebAssembly (as for a browser) compiles models in Node; the modules
/// it writes compute the bits of the modules compiled natively.
#[test]
fn compiler_runs_in_webassembly() {
    if !have("node") {
        return;
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let target = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wasm32-build");
    let out = Command::new(env!("CARGO"))
        .args([
            "build",
            "--release",
            "--offline",
            "--target",
            "wasm32-unknown-unknown",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if err.contains("wasm32-unknown-unknown") && err.contains("not installed") {
            eprintln!("skipped: the wasm32-unknown-unknown target is not installed");
            return;
        }
        panic!("{}", err);
    }
    let compiler = target.join("wasm32-unknown-unknown/release/symjit_wasm.wasm");

    let d = dir("in-wasm");
    let mut requests = Vec::new();
    let mut expected = Vec::new();
    for (name, text, complex) in [
        ("arithmetic", ARITHMETIC, false),
        ("functions", FUNCTIONS, false),
        ("branch", BRANCH, false),
        ("arithmetic_c", ARITHMETIC, true),
    ] {
        let opt = symbolica_opt(2, complex);
        let inputs: Vec<Vec<f64>> = POINTS
            .iter()
            .map(|p| {
                if complex {
                    p.iter().flat_map(|x| [*x + 0.5, *x * 0.25 - 1.0]).collect()
                } else {
                    p.iter().map(|x| x.abs() + 0.5).collect()
                }
            })
            .collect();
        let native = write_module(&d, name, translate(text, opt, None).wasm().unwrap());
        expected.push(run(&native, "evaluate", &inputs, json!({})).unwrap());
        requests.push(json!({"name": name, "text": text, "opt": opt, "points": points(&inputs)}));
    }
    // a SymPy model (the JSON of symjit's Python package): x y + sq(x), sq from the host
    let var = |name: &str| json!({"type": "Var", "name": name});
    let model = json!({
        "iv": {"name": "t"}, "params": [], "states": [{"name": "x"}, {"name": "y"}],
        "algs": [], "odes": [],
        "obs": [{"lhs": var("$0"), "rhs": {"type": "Tree", "op": "plus", "args": [
            {"type": "Tree", "op": "times", "args": [var("x"), var("y")]},
            {"type": "Tree", "op": "sq", "args": [var("x")]}
        ]}}]
    });
    let xy: Vec<Vec<f64>> = POINTS.iter().map(|p| p[..2].to_vec()).collect();
    expected.push(xy.iter().map(|p| vec![p[0] * p[1] + p[0] * p[0]]).collect());
    requests.push(json!({
        "name": "sympy", "json": model.to_string(), "opt": symbolica_opt(2, false),
        "points": points(&xy), "functions": {"sq": 1}, "host": {"sq": "(x) => x * x"}
    }));

    let script = root.join("tests/compiler_test.mjs");
    let mut child = Command::new("node")
        .arg(script)
        .arg(&compiler)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(Value::Array(requests).to_string().as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let answer: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(answer["version"].as_str().unwrap().starts_with("0."));
    for (a, e) in answer["results"].as_array().unwrap().iter().zip(&expected) {
        assert_eq!(bits(&obs(a)), bits(e), "{}", a["name"]);
    }
}

// f(n) = n < 1 ? 0 : n + f(n - 1), a model that calls itself (`Defuns::add_self`)
const RECURSIVE: &str = "([\
    ('fun', ('temp', 0), lt, [], [('param', 0), ('const', 0)], True),\
    ('if_else', ('temp', 0), 0),\
    ('assign', ('temp', 1), ('const', 1)),\
    ('goto', 1),\
    ('label', 0),\
    ('add', ('temp', 2), [('param', 0), ('const', 2)], 1),\
    ('external_fun', ('temp', 3), me, [('temp', 2)]),\
    ('add', ('temp', 4), [('param', 0), ('temp', 3)], 1),\
    ('label', 1),\
    ('join', ('out', 0), ('temp', 0), ('temp', 1), ('temp', 4))\
    ], 5, [1, 0, -1])";

#[test]
fn recursion_matches_the_jit() {
    let d = dir("recursion");
    for opt_level in [0, 2, 3] {
        let opt = symbolica_opt(opt_level, false);
        let mut df = sw::Defuns::new();
        df.add_self("me");
        let mut config = sw::Config::from_name("wasm", opt).unwrap();
        config.set_defuns(df);
        let app = sw::Compiler::with_config(config)
            .translate(RECURSIVE.to_string(), 1)
            .unwrap();
        let module = write_module(&d, &format!("sum{}", opt_level), app.wasm().unwrap());

        let mut df = symjit::Defuns::new();
        df.add_self("me");
        let mut config = symjit::Config::from_name("native", opt).unwrap();
        config.set_defuns(df);
        let jit = symjit::Compiler::with_config(config)
            .translate(RECURSIVE.to_string(), 1)
            .unwrap();
        let inputs: Vec<Vec<f64>> = [0.0, 1.0, 5.0, 100.0, 2.5]
            .iter()
            .map(|n| vec![*n])
            .collect();
        let reference: Vec<Vec<f64>> = inputs
            .iter()
            .map(|p| {
                let mut outs = vec![0.0];
                jit.evaluate(p, &mut outs);
                outs
            })
            .collect();
        assert_eq!(reference[3], vec![5050.0]);
        if let Some(obs) = run(&module, "evaluate", &inputs, json!({})) {
            assert_eq!(bits(&obs), bits(&reference), "opt level {}", opt_level);
        }
    }
}

// the escape count of the Mandelbrot iteration z = z^2 + c (at most 100), a loop whose
// temporaries carry values from one iteration to the next: it needs the direct translator
// (examples/mandelbrot.html)
const ESCAPE: &str = "([\
    ('assign', ('temp', 0), ('const', 0)),\
    ('assign', ('temp', 1), ('const', 0)),\
    ('label', 0),\
    ('mul', ('temp', 2), [('temp', 0), ('temp', 0)], 0),\
    ('add', ('temp', 0), [('temp', 2), ('param', 0)], 0),\
    ('add', ('temp', 1), [('temp', 1), ('const', 1)], 0),\
    ('fun', ('temp', 3), abs, [], [('temp', 0)], False),\
    ('fun', ('temp', 4), lt, [], [('temp', 3), ('const', 2)], True),\
    ('fun', ('temp', 5), lt, [], [('temp', 1), ('const', 3)], True),\
    ('fun', ('temp', 6), and, [], [('temp', 4), ('temp', 5)], True),\
    ('if_else', ('temp', 6), 1),\
    ('goto', 0),\
    ('label', 1),\
    ('assign', ('out', 0), ('temp', 1))\
    ], 7, [0, 1, 2, 100])";

#[test]
fn loops_in_direct_mode() {
    let d = dir("loops");
    let direct = 0x4000;
    let app = sw::Compiler::with_config(
        sw::Config::from_name("wasm", symbolica_opt(2, true) | direct).unwrap(),
    )
    .translate(ESCAPE.to_string(), 1)
    .unwrap();
    let module = write_module(&d, "escape", app.wasm().unwrap());
    let inputs: Vec<Vec<f64>> = [
        [0.0, 0.0],
        [-1.0, 0.0],
        [0.3, 0.5],
        [1.0, 1.0],
        [-0.75, 0.1],
    ]
    .iter()
    .map(|c| c.to_vec())
    .collect();
    let reference: Vec<Vec<f64>> = inputs
        .iter()
        .map(|c| {
            let c = num_complex::Complex::new(c[0], c[1]);
            let (mut z, mut n) = (num_complex::Complex::new(0.0, 0.0), 0);
            while n < 100 {
                z = z * z + c;
                n += 1;
                if z.norm() >= 2.0 {
                    break;
                }
            }
            vec![n as f64, 0.0]
        })
        .collect();
    if let Some(obs) = run(&module, "matrix", &inputs, json!({})) {
        assert_eq!(obs, reference);
    }
}
