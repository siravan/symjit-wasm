# symjit-wasm

A stripped-down symjit whose only backend is WebAssembly. It compiles the same models as
symjit (SymPy, `Expr`, Symbolica evaluators, `Composer` programs, saved `.sjb` files) into
self-contained WebAssembly modules, and, having no machine-code generator, executable memory
or threads, it compiles to WebAssembly itself: the compiler runs in a browser next to the
modules it makes. It is meant for symjit in Symbolica's WebAssembly build.

Based on symjit 2.28 (`../stable`), with the WebAssembly generator of `sjb2wasm`
(symjit-utils) as the sole backend:

* removed: the amd64, arm64 and riscv64 backends, `memory.rs` and `machine.rs` (executable
  memory), the assembler, lockstep evaluation, `Matrix`, threads (rayon) and the native
  dependencies (region, libc, hugepage, memmap2, corosensei, rand, wide);
* `Application` holds the module (`wasm()`) and the bytecode interpreter (`interpret`, for
  testing); it has no `exec_*`, `evaluate_*`, `call` or `fast_func`;
* `Applet` is a light struct: the module and the shape of its calls (`Application::seal`);
* `Defuns` declares user-defined functions, which the module imports from its host;
* `.sjb` files are read and written as before, with 64-bit fields on 32-bit targets too.

```bash
cargo build --release                                   # the Rust library (rlib + cdylib)
cargo build --release --target wasm32-unknown-unknown   # the compiler for the browser
cargo test                                              # needs node and wabt for the end-to-end tests
cargo test --lib --target wasm32-wasip1 --no-run        # the unit tests as WebAssembly ...
node tests/wasi_run.mjs target/wasm32-wasip1/debug/deps/symjit_wasm-*.wasm   # ... run in Node
```

```rust
use symjit_wasm::{var, Compiler, CompilerType, Config};

let (x, y) = (var("x"), var("y"));
let app = Compiler::with_config(Config::new(CompilerType::Wasm, 0)?)
    .compile(&[x.clone(), y.clone()], &[&x.sin() * &y])?;
let module: &[u8] = app.wasm().unwrap();
```

```js
import { SymjitModel } from "./js/symjit.mjs";
const model = await SymjitModel.compile(module);
model.call([0.5, 2.0]);
```

`examples/mandelbrot.html` compiles a Mandelbrot model in the browser and paints it on a canvas
(`cargo build --release --target wasm32-unknown-unknown`, then `python3 -m http.server` in this
directory and open `http://localhost:8000/examples/mandelbrot.html`).

`docs/WASM.md` describes the models, the module interface, user-defined functions and the
JavaScript side (`js/symjit.mjs` runs modules, `js/compiler.mjs` drives the compiler compiled
to WebAssembly).

The tests (`tests/wasm.rs`) compile models, validate the modules with `wasm-validate`, run
them in Node and compare them with symjit's JIT (`../stable`, a native-only dev-dependency)
and with the interpreter; one test builds the crate for `wasm32-unknown-unknown` and compiles
models with it in Node.
