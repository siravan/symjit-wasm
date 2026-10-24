use anyhow::{anyhow, Result};
use std::collections::HashSet;
use std::io::{Read, Write};

use super::applet::Applet;
use super::complexify::Complexifier;
use super::config::Config;
use super::generator::Generator;
use super::mir::{CompiledMir, Mir};
use super::model::Program;
use super::symbol::Loc;
use super::utils::*;
use super::wasm::{custom_section, WasmGenerator, SHAPE_SECTION};

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum CompilerType {
    /// a WebAssembly module (default; symjit's native types, e.g. `native`, map to it)
    Wasm,
    /// no module: the bytecode interpreter only (`Application::interpret`)
    ByteCode,
}

pub struct Application {
    pub prog: Program,
    pub config: Config,
    /// the WebAssembly module (None for `ty = "bytecode"`)
    pub wasm: Option<Vec<u8>>,
    pub bytecode: CompiledMir,
    pub reals: HashSet<Loc>,
    /// the MIR before `Complexifier` (complex models), saved in `.sjb` files
    pub original: Option<Mir>,
    pub params: Vec<f64>,
    pub first_state: usize,
    pub first_param: usize,
    pub first_obs: usize,
    pub first_diff: usize,
    pub count_states: usize,
    pub count_params: usize,
    pub count_obs: usize,
    pub count_diffs: usize,
}

impl Application {
    pub fn new(prog: Program, reals: HashSet<Loc>) -> Result<Application> {
        let mut mir = Mir::new(prog.config().clone());
        let mut prog = prog;
        prog.builder.compile_mir(&mut mir)?;
        prog.builder.optimize_mir(&mut mir)?;
        Self::with_mir(prog, reals, mir)
    }

    pub fn with_mir(mut prog: Program, reals: HashSet<Loc>, mut mir: Mir) -> Result<Application> {
        let first_state = 0;
        let first_param = 0;
        let first_obs = first_state + prog.count_states;
        let first_diff = first_obs + prog.count_obs;

        let count_states = prog.count_states;
        let count_params = prog.count_params;
        let count_obs = prog.count_obs;
        let count_diffs = prog.count_diffs;

        let params = vec![0.0; count_params + 1];

        let config = prog.config().clone();
        let mut original: Option<Mir> = None;

        // the module and the interpreter run real MIR: complex operations are lowered
        if config.is_complex() {
            original = Some(mir.clone());
            let complexified = Complexifier::new(&reals, config.clone()).complexify(&mir)?;

            // stable_math appends its constants (`mir.consts` starts as a copy of these)
            if complexified.consts.len() > prog.builder.consts.len() {
                prog.builder.consts = complexified.consts.clone();
            }

            mir = complexified;
        }

        let wasm = if config.is_wasm() {
            Some(Self::compile_wasm(&mir, &mut prog)?)
        } else {
            None
        };

        // bytecode takes the ownership of mir
        let bytecode = Self::compile_bytecode(mir, &mut prog)?;

        Ok(Application {
            prog,
            config,
            wasm,
            bytecode,
            reals,
            original,
            params,
            first_state,
            first_param,
            first_obs,
            first_diff,
            count_states,
            count_params,
            count_obs,
            count_diffs,
        })
    }

    /// A light, self-contained form of the application: its module and call shape.
    pub fn seal(self) -> Result<Applet> {
        Applet::new(self)
    }

    /// The WebAssembly module (see `docs/WASM.md` for its interface); None for
    /// `ty = "bytecode"`.
    pub fn wasm(&self) -> Option<&[u8]> {
        self.wasm.as_deref()
    }

    /// Whether the module exports `fast`: one point per call, the states as arguments and
    /// the one observable as the result.
    pub fn has_fast(&self) -> bool {
        self.count_params == 0 && self.count_obs == 1 && self.count_diffs == 0
    }

    fn compile_wasm(mir: &Mir, prog: &mut Program) -> Result<Vec<u8>> {
        let mut generator = WasmGenerator::new(prog.config().clone());
        generator.set_mem_size(prog.mem_size());
        if prog.count_params == 0 && prog.count_obs == 1 && prog.count_diffs == 0 {
            generator.export_fast(prog.mem_size());
        }
        prog.builder.compile_from_mir(
            mir,
            &mut generator,
            prog.count_states,
            prog.count_obs,
            prog.count_params,
        )?;
        generator.check()?;
        let mut module = generator.bytes();
        custom_section(&mut module, SHAPE_SECTION, Self::shape(prog).as_bytes());
        Ok(module)
    }

    /// The shape of the module's calls, as JSON (its custom section `symjit`): the counts of
    /// states, observables, parameters and diffs, in f64 values (a complex value is two), the
    /// size of the model memory in f64 values (states, then observables, then diffs),
    /// whether the model is complex and whether the module exports `fast`. A Symbolica or
    /// `Composer` model has no states: its inputs are its parameters.
    fn shape(prog: &Program) -> String {
        format!(
            "{{\"version\":1,\"states\":{},\"obs\":{},\"params\":{},\"diffs\":{},\"mem\":{},\
             \"complex\":{},\"fast\":{}}}",
            prog.count_states,
            prog.count_obs,
            prog.count_params,
            prog.count_diffs,
            prog.mem_size(),
            prog.config().is_complex(),
            prog.count_params == 0 && prog.count_obs == 1 && prog.count_diffs == 0
        )
    }

    fn compile_bytecode(mir: Mir, prog: &mut Program) -> Result<CompiledMir> {
        let mem: Vec<f64> = vec![0.0; prog.mem_size()];
        let stack: Vec<f64> = vec![0.0; prog.builder.stack_size()];

        Ok(CompiledMir::new(mir, mem, stack))
    }

    /// Writes `what` to the file `name`: "wasm" (the module), "bytecode" (the MIR as text)
    /// or "stats" (MIR statistics). Returns false if there is nothing to write.
    pub fn dump(&mut self, name: &str, what: &str) -> bool {
        match what {
            "wasm" | "scalar" => match &self.wasm {
                Some(wasm) => std::fs::write(name, wasm).is_ok(),
                None => false,
            },
            "bytecode" => {
                self.bytecode.dump(name);
                true
            }
            "stats" => {
                let size = self.wasm.as_ref().map_or(0, |w| w.len());
                self.bytecode.mir.print_stats(name, size);
                true
            }
            _ => false,
        }
    }

    /// The module's bytes (empty for `ty = "bytecode"`).
    pub fn dumps(&self) -> Vec<u8> {
        self.wasm.clone().unwrap_or_default()
    }

    pub fn measure(&self, what: &str) -> usize {
        match what {
            "wasm-size" | "ker-scalar-size" => self.wasm.as_ref().map_or(0, |w| w.len()),
            "mem-size" => self.prog.mem_size(),
            "mir-size" => self.bytecode.mir.code.ip,
            "mir-bytes" => self.bytecode.mir.code.buf.len(),
            "stack-size" => self.prog.builder.stack_size(),
            "version" => {
                let major: usize = env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap_or(99);
                let minor: usize = env!("CARGO_PKG_VERSION_MINOR").parse().unwrap_or(99);
                let patch: usize = env!("CARGO_PKG_VERSION_PATCH").parse().unwrap_or(99);
                major * 10000 + minor * 100 + patch
            }
            _ => 0,
        }
    }

    /************************** save/load ******************************/

    const MAGIC: u64 = 0x40568795410d08e9;

    /// Loads a saved application (`.sjb`, from symjit 2.22 on or this crate) and compiles it
    /// into a module; `config` supplies the user-defined functions.
    pub fn load(stream: &mut impl Read, config: &Config) -> Result<Self> {
        Self::load_with_version(stream, config, 0)
    }

    /// Reads a saved application (`.sjb`) without compiling it: the program, the saved MIR
    /// (before `Complexifier` for complex models) and the real locations of a complex model.
    /// For code generators outside the crate (see `backend`).
    pub fn read_sjb(
        stream: &mut impl Read,
        config: &Config,
    ) -> Result<(Program, HashSet<Loc>, Mir)> {
        let (prog, reals, mir, _mask) = Self::read_parts(stream, config)?;
        Ok((prog, reals, mir))
    }

    // the parts of a saved application and the mask of its prepared kernels
    fn read_parts(
        stream: &mut impl Read,
        config: &Config,
    ) -> Result<(Program, HashSet<Loc>, Mir, usize)> {
        let mut bytes: [u8; 8] = [0; 8];

        stream.read_exact(&mut bytes)?;

        if u64::from_le_bytes(bytes) != Self::MAGIC {
            return Err(anyhow!("invalid magic number (Application)"));
        }

        stream.read_exact(&mut bytes)?;

        let version = u64::from_le_bytes(bytes) as usize;

        if version < 3 {
            return Err(anyhow!("invalid sjb version"));
        }

        let prog = Program::load_with_version(stream, config, version)?;

        stream.read_exact(&mut bytes)?;
        let mask = u64::from_le_bytes(bytes) as usize;

        let mir = Mir::load_with_version(stream, prog.config(), version)?;

        let reals = load_reals(stream)?;

        Ok((prog, reals, mir, mask))
    }
}

fn save_reals(stream: &mut impl Write, reals: &HashSet<Loc>) -> Result<()> {
    let num_elems = reals.len() as u64;
    stream.write_all(&num_elems.to_le_bytes())?;

    for r in reals.iter() {
        let b: u64 = match r {
            Loc::Mem(idx) => 0x100000000 | (*idx as u64),
            Loc::Stack(idx) => 0x200000000 | (*idx as u64),
            Loc::Param(idx) => 0x300000000 | (*idx as u64),
        };
        stream.write_all(&b.to_le_bytes())?;
    }

    Ok(())
}

fn load_reals(stream: &mut impl Read) -> Result<HashSet<Loc>> {
    let mut bytes: [u8; 8] = [0; 8];

    stream.read_exact(&mut bytes)?;
    let num_elems = u64::from_le_bytes(bytes);

    let mut reals: HashSet<Loc> = HashSet::new();

    for _ in 0..num_elems {
        stream.read_exact(&mut bytes)?;
        let b = u64::from_le_bytes(bytes);

        let r = match b >> 32 {
            1 => Loc::Mem((b & 0xffffffff) as u32),
            2 => Loc::Stack((b & 0xffffffff) as u32),
            3 => Loc::Param((b & 0xffffffff) as u32),
            _ => return Err(anyhow!("invalid loc")),
        };
        reals.insert(r);
    }

    Ok(reals)
}

impl Storage for Application {
    fn save(&self, stream: &mut impl Write) -> Result<()> {
        stream.write_all(&Self::MAGIC.to_le_bytes())?;

        let version: u64 = 4;
        stream.write_all(&version.to_le_bytes())?;

        self.prog.save(stream)?;

        // the mask of the prepared machine-code kernels: none here
        let mask: u64 = 0;
        stream.write_all(&mask.to_le_bytes())?;

        match &self.original {
            Some(mir) => mir.save(stream)?,
            None => self.bytecode.mir.save(stream)?,
        }

        save_reals(stream, &self.reals)?;

        Ok(())
    }

    fn load_with_version(stream: &mut impl Read, config: &Config, _version: usize) -> Result<Self> {
        let (prog, reals, mir, _mask) = Self::read_parts(stream, config)?;
        Application::with_mir(prog, reals, mir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{var, Compiler, CompilerType, Config};

    #[test]
    fn sjb_fields_are_64_bit() {
        // a 32-bit build (WebAssembly) writes the files of a 64-bit one: every count is a u64
        let (x, y) = (var("x"), var("y"));
        let config = Config::new(CompilerType::Wasm, 0).unwrap();
        let app = Compiler::with_config(config)
            .compile(&[x.clone(), y.clone()], &[&(&x * &y) + &x.sin()])
            .unwrap();
        let mut bytes = Vec::new();
        app.save(&mut bytes).unwrap();
        assert_eq!(bytes[..8], Application::MAGIC.to_le_bytes());
        assert_eq!(bytes[8..16], 4u64.to_le_bytes());
        // the Program header, then the Config: its magic, the options with the type (native)
        assert_eq!(bytes[24..32], 0x802c3c77c7422e70u64.to_le_bytes());
        assert_eq!(bytes[36..40], [0, 0, 0, 0]);

        let restored = Application::load(&mut bytes.as_slice(), &app.config).unwrap();
        assert_eq!(restored.wasm(), app.wasm());
        assert_eq!(
            (restored.count_states, restored.count_obs),
            (app.count_states, app.count_obs)
        );
    }
}
