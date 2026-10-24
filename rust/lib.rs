//! symjit-wasm: symjit's compiler with WebAssembly as its only backend.
//!
//! A model (SymPy JSON, `Expr`, a Symbolica evaluator or a `Composer` program) is compiled into
//! a self-contained WebAssembly module (`Application::wasm`), which the host instantiates and
//! runs (see `docs/WASM.md`). The crate itself has no native code generator, executable memory
//! or threads, so it compiles to WebAssembly too and runs in a browser. The bytecode
//! interpreter (`Application::interpret`) is kept for testing.

#![allow(uncommon_codepoints)]

mod allocator;
mod applet;
mod block;
mod builder;
mod code;
mod compactor;
mod compiler;
mod complexify;
mod composer;
mod config;
mod defuns;
mod expr;
mod generator;
mod instruction;
mod mir;
mod model;
mod node;
mod operation;
mod parser;
mod runnable;
mod serializer;
mod statement;
mod strings;
mod symbol;
mod topology;
mod types;
mod utils;

// the only backend: WebAssembly modules
mod wasm;

// the C interface used by the Python package (compiling SymPy models)
#[cfg(feature = "sympy")]
pub mod sympy;

pub use applet::Applet;
pub use compiler::{Compiler, Translator};
pub use composer::Composer;
pub use config::Config;
pub use defuns::Defuns;
pub use expr::Expr;
pub use instruction::{BuiltinSymbol, Instruction, Slot, SymbolicaModel};
pub use model::{CellModel, Program};
pub use num_complex::{Complex, ComplexFloat};
pub use runnable::{Application, CompilerType};
pub use serializer::MirWriter;
pub use types::{ElemType, Element};
pub use utils::Storage;
pub use wasm::IMPORT_MODULE;

pub fn var(name: &str) -> Expr {
    Expr::var(name)
}

pub fn double(val: f64) -> Expr {
    Expr::from(val)
}

pub fn int(val: i32) -> Expr {
    Expr::from(val)
}

fn bool_to_f64(b: bool) -> f64 {
    const T: f64 = f64::from_bits(!0);
    const F: f64 = f64::from_bits(0);
    if b {
        T
    } else {
        F
    }
}

pub fn boolean(val: bool) -> Expr {
    Expr::from(bool_to_f64(val))
}

// the C functions and their types stay at the crate root, as before the `sympy` feature
#[cfg(feature = "sympy")]
pub use sympy::*;
