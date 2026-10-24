# WebAssembly modules

symjit-wasm compiles a model into a self-contained WebAssembly module (`Application::wasm`),
which a WebAssembly host — a browser, Node, wasmtime — instantiates and runs. The crate itself
compiles to WebAssembly too (`wasm32-unknown-unknown`), so the compiler can run in the browser
next to the modules it makes.

```rust
use symjit_wasm::{var, Compiler, Config, CompilerType};

let (x, y) = (var("x"), var("y"));
let app = Compiler::with_config(Config::new(CompilerType::Wasm, 0)?)
    .compile(&[x.clone(), y.clone()], &[x.sin(), y.cos()])?;
std::fs::write("model.wasm", app.wasm().unwrap())?;
```

```js
import { SymjitModel } from "./js/symjit.mjs";
const model = await SymjitModel.compile(bytes);   // bytes: the module
model.call([0.5, 1.0]);                           // Float64Array [sin(0.5), cos(1.0)]
```

## Models

All of symjit's front ends produce modules:

* `Compiler::compile`/`compile_params` (`Expr` models), and the JSON of SymPy models through
  the C interface (`compile`, as symjit's Python package encodes them);
* `Compiler::translate` and `Translator` (Symbolica evaluators: their instructions as JSON or
  in the text form of `export_instructions`);
* `Composer` programs;
* saved applications (`Application::load` of a `.sjb` file, from symjit 2.22 on or from this
  crate; the files are exchanged with symjit both ways).

Symbolica instructions with a loop (a backward `goto` whose temporaries carry values from one
iteration to the next) need the direct translator (`Config::set_direct`, option bit 0x4000,
`options({direct: true})` in `js/compiler.mjs`): the default one turns the instructions into SSA
form, which assumes forward-only code. `examples/mandelbrot.html` is such a loop.

`CompilerType::Wasm` (the default; symjit's native type names, `native`, `amd`, `arm`, ...,
select it too) makes the module; `CompilerType::ByteCode` makes none and leaves only the
bytecode interpreter (`Application::interpret`), which is kept for testing.

What is supported: loading and storing states, observables, parameters and stack slots;
constants; real and complex arithmetic (including `stable_math`); powers, `sqrt`, `Abs`,
roundings; comparisons, `And`/`Or`/`Not`, `Min`/`Max`, `Piecewise`; the loops, if/else blocks
and gotos of Symbolica evaluators and `Composer` programs; `Sum`/`Product` loops; math functions
and user-defined functions, imported from the host; recursive calls (`Defuns::add_self`).

Not supported: compression mode (`compress` has no effect here) and the SIMD, thread and
fast-complex options of symjit (they have no effect: a module evaluates one point per call).
A function body larger than 7,654,321 bytes (the JavaScript API's limit; the largest Symbolica
evaluators) is rejected.

Numerically, a module computes what the bytecode interpreter computes. `fastmath` has no
effect: WebAssembly has no fused multiply-add, and with `fastmath` off the module computes the
bits of symjit's native code for arithmetic (the tests check this against symjit 2.28's JIT).
Differences: `round` rounds halves to even (as symjit's aarch64 and riscv64 code), the sign
and payload of a NaN result are the host's, and imported functions are as accurate as the
host's (JavaScript's `Math` is not glibc's).

## User-defined functions (`Defuns`)

A module cannot call into the process that compiled it, so a user-defined function is a
declaration — its name and number of arguments — and the module imports it from the host:

```rust
let mut df = Defuns::new();
df.add_func("hyp", 2)?;            // real: (f64, f64) -> f64, imported as symjit.hyp
df.add_unary_complex("twice")?;    // complex: (re, im) -> (re, im), imported as symjit.cplx_twice
df.add_self("me");                 // the model itself (a recursive call)
config.set_defuns(df);
```

Functions take one or two arguments (complex functions one or two complex arguments) and are
called like the math functions. A model calls them as `Expr::unary("name", &x)` /
`Expr::binary`, as a SymPy function of that name, or as a Symbolica external function. The
bytecode interpreter cannot call them (it returns NaN). The JavaScript host supplies them:

```js
const model = await SymjitModel.compile(bytes, {
    hyp: (x, y) => Math.hypot(x, y),
    cplx_twice: (re, im) => [2 * re, 2 * im],
});
```

## Module interface

```wat
(import "symjit" "<name>" (func (param f64) (result f64)))                     ;; unary
(import "symjit" "<name>" (func (param f64 f64) (result f64)))                 ;; binary
(import "symjit" "cplx_<name>" (func (param f64 f64) (result f64 f64)))         ;; complex
(import "symjit" "cplx_<name>" (func (param f64 f64 f64 f64) (result f64 f64))) ;; complex
(memory (export "memory") n)
(global (export "__heap_base") i32)
(func (export "run") (param $mem i32) (param $states i32) (param $idx i32)
                     (param $params i32) (result i32))
(func (export "fast") (param $x0 f64) ... (result f64))   ;; some models
;; custom section "symjit": the call shape (JSON)
```

* **Imports.** Only the functions the model uses, from the module `"symjit"`, under symjit's
  names (`sin`, `exp`, `ln`, `log10`, `power`, `atan2`, `arcsinh`, `erf`, `gamma`, ...; complex
  ones as `cplx_<name>`, taking and returning (re, im) pairs) and the user-defined ones.
* **Memory.** Addresses below `__heap_base` hold the stack `run` uses; the host places its
  data at or above `__heap_base` (one free 64 KiB page at first; `memory.grow` for more).
* **`run`** returns 0, or 1 if its stack is exhausted (a deep recursion). Pointers are byte
  offsets into the memory; values are f64, a complex value two of them (re, im).
  * Direct: `run(mem, 0, 0, params)`. `mem` holds the states, then the observables, then the
    diffs of an ODE model; `params` the parameters. A Symbolica or `Composer` model has no
    states: its inputs are at `params` and its outputs at `mem`.
  * Indirect: `run(0, states, idx, params)`. `states` is a table of (i32 ptr, i32 len) pairs,
    one per state and then one per observable, each an f64 array; `run` reads element `idx` of
    the state arrays and writes element `idx` of the observable arrays.
* **`fast`** (models without parameters, with one output and at most 1000 states): the states
  as arguments, the output as the result.
* **The `symjit` custom section** describes the calls, so that the module's bytes are all a host
  needs: `{"version":1, "states", "obs", "params", "diffs", "mem", "complex", "fast"}` (counts
  in f64 values; `mem` is the size of the model memory). JavaScript reads it with
  `WebAssembly.Module.customSections(module, "symjit")`.

## JavaScript

`js/symjit.mjs` (no dependencies; browsers and Node) runs modules:

* `MATH`: symjit's math functions, real and complex, in JavaScript (its `power` is C's `pow`:
  `Math.pow(1, Infinity)` is NaN, C's 1);
* `SymjitModel.compile(bytes, host)` (asynchronous: browsers limit synchronous compilation on
  the main thread to small modules) and `SymjitModel.fromModule(module, host)`;
* `model.call(states, params)` for models of states, `model.evaluate(inputs)` for Symbolica and
  `Composer` models, `model.evaluateMatrix(args, n)` for n points in a row-major array (the
  layout of symjit's `evaluate_matrix`), `model.fast(...)`; `model.shape` is the custom section.

`js/compiler.mjs` drives the compiler compiled to WebAssembly
(`cargo build --release --target wasm32-unknown-unknown` → `symjit_wasm.wasm`, about 0.9 MB,
no imports) through the C interface (`rust/sympy.rs`):

```js
import { SymjitCompiler, options } from "./js/compiler.mjs";
import { SymjitModel } from "./js/symjit.mjs";

const compiler = await SymjitCompiler.load(await (await fetch("symjit_wasm.wasm")).arrayBuffer());
const bytes = compiler.translate(instructions, 2, options({ optLevel: 2 }));  // Symbolica
const model = await SymjitModel.compile(bytes);
model.evaluate([0.5, 1.5]);
```

`compiler.compile(json, opt, functions)` takes a SymPy model's JSON; `functions` declares its
user-defined functions (`{name: arity}` or `{name: [arity, "complex"]}`).

## Inspecting a module

```bash
wasm-validate model.wasm
wasm2wat model.wasm          # with local names (ret, temp, r0, ...)
wasm-objdump -x model.wasm   # sections, imports, exports, the `symjit` section
```

## Implementation

`rust/wasm/encoder.rs` encodes modules (instructions, LEB128, sections); its encodings are
pinned against wabt's `wat2wasm` by `rust/wasm/tests.rs`, generated with
`python3 rust/wasm/gen_tests.py > rust/wasm/tests.rs`. `rust/wasm/mod.rs` (`WasmGenerator`)
implements the `Generator` trait: MIR registers become locals, stack slots live below
`__heap_base`, constants are `f64.const` immediates, and the body is buffered until `seal`.
`rust/wasm/flow.rs` structures the jumps: basic blocks in nested `block`s inside a `loop`,
forward jumps as `br`, backward jumps and subroutine returns through a `br_table` dispatcher.
The generator comes from `sjb2wasm` (symjit-utils), which compiles saved MIR; here it is
symjit's only backend, run by `Application::with_mir` after the MIR is optimized (and lowered
to real operations by `Complexifier` for a complex model).
