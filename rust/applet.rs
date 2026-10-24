use anyhow::{anyhow, Result};

use super::config::Config;
use super::runnable::Application;

/// A compiled model reduced to what a host needs to run it: the WebAssembly module and the
/// shape of its calls (see `docs/WASM.md`). `Application::seal` makes one.
#[derive(Clone)]
pub struct Applet {
    pub wasm: Vec<u8>,
    pub config: Config,
    pub count_states: usize,
    pub count_obs: usize,
    pub count_params: usize,
    pub count_diffs: usize,
    /// f64 values of the model memory of a direct call (states, observables, diffs)
    pub mem_size: usize,
    /// the module exports `fast` (one point per call, the states as arguments)
    pub has_fast: bool,
}

impl Applet {
    pub fn new(app: Application) -> Result<Applet> {
        let has_fast = app.has_fast();
        let mem_size = app.prog.mem_size();
        let wasm = app
            .wasm
            .ok_or_else(|| anyhow!("a bytecode application has no WebAssembly module"))?;
        Ok(Applet {
            wasm,
            config: app.config,
            count_states: app.count_states,
            count_obs: app.count_obs,
            count_params: app.count_params,
            count_diffs: app.count_diffs,
            mem_size,
            has_fast,
        })
    }

    /// The WebAssembly module.
    pub fn wasm(&self) -> &[u8] {
        &self.wasm
    }
}
