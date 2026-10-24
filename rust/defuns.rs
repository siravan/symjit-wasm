use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::fmt;

use super::code::{Func, VirtualTable};

/// User-defined functions of a model. A WebAssembly module cannot hold function pointers or
/// closures, so a user-defined function is only declared here (its name and number of
/// arguments): the module imports it from the `symjit` import module, beside the math
/// functions, and the host supplies it when it instantiates the module (see `docs/WASM.md`).
/// The bytecode interpreter cannot call them (it returns NaN).
///
/// `add_self` declares the name by which a model calls itself (recursion).
#[derive(Clone, Default)]
pub struct Defuns {
    pub funcs: HashMap<String, Func>,
}

impl fmt::Debug for Defuns {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names: Vec<&String> = self.funcs.keys().collect();
        names.sort();
        write!(f, "Defuns{:?}", names)
    }
}

impl Defuns {
    pub fn new() -> Defuns {
        Defuns {
            funcs: HashMap::new(),
        }
    }

    fn declare(&mut self, name: &str, num_args: usize, complex: bool) -> Result<()> {
        if !(1..=2).contains(&num_args) {
            return Err(anyhow!(
                "`{}`: only unary and binary functions are supported",
                name
            ));
        }
        let key = if complex {
            format!("cplx_{}", name)
        } else {
            name.to_string()
        };
        if VirtualTable::from_str(&key).is_ok() {
            return Err(anyhow!("cannot redefine function {}.", name));
        }
        self.funcs.insert(key, Func::Import { num_args, complex });
        Ok(())
    }

    /// Declares a real function `name` of `num_args` (1 or 2) arguments. The module imports
    /// it as `symjit.<name>`: `(param f64 [f64]) (result f64)`.
    pub fn add_func(&mut self, name: &str, num_args: usize) -> Result<()> {
        self.declare(name, num_args, false)
    }

    pub fn add_unary(&mut self, name: &str) -> Result<()> {
        self.declare(name, 1, false)
    }

    pub fn add_binary(&mut self, name: &str) -> Result<()> {
        self.declare(name, 2, false)
    }

    /// Declares a complex function `name` of `num_args` (1 or 2) complex arguments, for
    /// complex models. The module imports it as `symjit.cplx_<name>`:
    /// `(param f64 f64 [f64 f64]) (result f64 f64)`, (re, im) pairs.
    pub fn add_func_complex(&mut self, name: &str, num_args: usize) -> Result<()> {
        self.declare(name, num_args, true)
    }

    pub fn add_unary_complex(&mut self, name: &str) -> Result<()> {
        self.declare(name, 1, true)
    }

    pub fn add_binary_complex(&mut self, name: &str) -> Result<()> {
        self.declare(name, 2, true)
    }

    /// Declares `name` as the model itself: calling it is a recursive call.
    pub fn add_self(&mut self, name: &str) {
        self.funcs.insert(name.to_string(), Func::Recursive);
    }

    pub fn len(&self) -> usize {
        self.funcs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
