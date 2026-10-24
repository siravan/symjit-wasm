use super::runnable::CompilerType;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::sync::Arc;

use super::code::{Func, VirtualTable};
use super::defuns::Defuns;
use super::operation::Operation;
use super::symbol::Loc;
use super::utils::Storage;

pub const USE_SIMD: u32 = 0x0000_0001;
pub const USE_THREADS: u32 = 0x0000_0002;
pub const CSE: u32 = 0x0000_0004;
pub const FASTMATH: u32 = 0x0000_0008;

pub const ENABLE_SIMD512: u32 = 0x0000_0010;
pub const COMPLEX: u32 = 0x0000_0020;
pub const SYMBOLICA: u32 = 0x0000_0040;
pub const SIMD_BRANCH: u32 = 0x0000_0080;

pub const OPT_LEVEL_MASK: u32 = 0x0000_0f00;
pub const OPT_LEVEL_SHIFT: usize = 8;

pub const COMPACT: u32 = 0x0000_1000;
pub const COMPRESS: u32 = 0x0000_2000;
pub const DIRECT: u32 = 0x0000_4000;
pub const FAST_COMPLEX: u32 = 0x0000_8000;

pub const DEBUG_BYTECODE: u32 = 0x0001_0000;
pub const DEBUG_SCALAR: u32 = 0x0002_0000;
pub const DEBUG_SIMD: u32 = 0x0004_0000;
pub const DEBUG_STATS: u32 = 0x0008_0000;

pub const HUGE: u32 = 0x0010_0000;
pub const PARALLEL_MUL: u32 = 0x0020_0000;
pub const DIRECT_ARENA: u32 = 0x0040_0000;
pub const DIRECT_ARENA_IDENTITY_OUTPUT: u32 = 0x0080_0000;

pub const DEBUG_LOCK: u32 = 0x0100_0000;
pub const DIRECT_ARENA_OPERATION_MASK: u32 = 0x0600_0000;
pub const DIRECT_ARENA_OPERATION_SHIFT: usize = 25;

pub const DEBUG_TOPOLOGY: u32 = 0x1000_0000;
pub const DEBUG_INSTRUCTIONS: u32 = 0x2000_0000;
pub const COROUTINES: u32 = 0x4000_0000;
/// Range-safe complex division, abs, sqrt and negative integer powers (see `Config::stable_math`).
pub const STABLE_MATH: u32 = 0x0800_0000;

// remaining bits:
// 0x8000_0000

pub const SPILL_AREA: usize = 16;
pub const ABI_AREA: usize = SPILL_AREA;
pub const SLICE_CAP: usize = 2048;

// LoadArgs/SaveArgs encode the count in six bits; the high bits are flags.
// This limit is separate from the external-function argument capacity.
pub const COMPRESSED_ARGS_CAP: usize = 64;

pub const DEFAULT_STACK_LIMIT: usize = 1 << 20;

#[derive(Clone)]
pub struct Config {
    pub opt: u32,
    pub ty: CompilerType,
    pub df: Option<Arc<Defuns>>,
    pub stack: usize,
    pub args: u32,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct ConfigToml {
    ty: String,
    options: Options,
    debug: DebugOptions,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Options {
    use_simd: bool,
    enable_simd512: bool,
    use_threads: bool,
    cse: bool,
    fastmath: bool,
    complex: bool,
    symbolica: bool,
    simd_branch: bool,
    compact: bool,
    compress: bool,
    direct: bool,
    fast_complex: bool,
    huge: bool,
    parallel_mul: bool,
    direct_arena: bool,
    direct_arena_identity_output: bool,
    opt_level: u8,
    stack_limit: usize,
    num_args: u32,
    coroutines: bool,
    stable_math: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct DebugOptions {
    bytecode: bool,
    scalar: bool,
    simd: bool,
    stats: bool,
    topology: bool,
    lock: bool,
    instructions: bool,
}

#[derive(Debug)]
pub enum KernelType {
    ColumnFirst(bool),
    RowFirst,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Config {{")?;

        if self.use_simd() {
            write!(f, "simd, ")?;
        }
        if self.use_simd512() {
            write!(f, "simd512, ")?;
        }
        if self.use_threads() {
            write!(f, "threads, ")?;
        }
        if self.cse() {
            write!(f, "cse, ")?;
        }
        if self.fastmath() {
            write!(f, "fastmath, ")?;
        }
        if self.is_complex() {
            write!(f, "complex, ")?;
        }
        if self.symbolica() {
            write!(f, "symbolica, ")?;
        }
        if self.simd_branch() {
            write!(f, "simd_branch, ")?;
        }
        if self.compact() {
            write!(f, "compact, ")?;
        }
        if self.compress() {
            write!(f, "compress, ")?;
        }
        if self.direct() {
            write!(f, "direct, ")?;
        }
        if self.fast_complex() {
            write!(f, "fast_complex, ")?;
        }
        if self.huge() {
            write!(f, "huge, ")?;
        }
        if self.parallel_mul() {
            write!(f, "parallel_mul, ")?;
        }
        if self.coroutines() {
            write!(f, "coroutines, ")?;
        }
        if self.stable_math() {
            write!(f, "stable_math, ")?;
        }
        if self.debug_bytecode() {
            write!(f, "debug_bytecode, ")?;
        }
        if self.debug_scalar() {
            write!(f, "debug_scalar, ")?;
        }
        if self.debug_simd() {
            write!(f, "debug_simd, ")?;
        }
        if self.debug_stats() {
            write!(f, "debug_stats, ")?;
        }
        if self.debug_topology() {
            write!(f, "debug_topology, ")?;
        }
        if self.debug_instructions() {
            write!(f, "debug_instructions, ")?;
        }
        if self.debug_lock() {
            write!(f, "debug_lock, ")?;
        }

        write!(
            f,
            "opt_level = {}, stack limit = {}, num_args = {}",
            self.opt_level(),
            self.stack_limit(),
            self.num_args()
        )
    }
}

impl Config {
    const MAGIC: u64 = 0x802c3c77c7422e70;

    pub fn new(ty: CompilerType, opt: u32) -> Result<Config> {
        Ok(Config {
            opt,
            ty,
            df: None,
            stack: DEFAULT_STACK_LIMIT,
            args: COMPRESSED_ARGS_CAP as u32,
        })
    }

    pub fn from_name(ty: &str, opt: u32) -> Result<Config> {
        // `ty` may carry numeric options after the type, applied with `set_option`
        if let Some((ty, options)) = ty.split_once(';') {
            let mut config = Self::from_name(ty, opt)?;
            for item in options.split(';').filter(|s| !s.trim().is_empty()) {
                let (key, val) = item
                    .split_once('=')
                    .ok_or_else(|| anyhow!("invalid option `{}` (expected key=value)", item))?;
                config.set_option(key.trim(), val.trim())?;
            }
            return Ok(config);
        }

        // the native types of symjit name the one backend here
        let ty = match ty {
            "bytecode" => CompilerType::ByteCode,
            "wasm" | "native" | "amd" | "amd-avx" | "amd-sse" | "arm" | "riscv" | "debug" => {
                CompilerType::Wasm
            }
            _ => {
                if ty.ends_with(".toml") {
                    return Self::from_toml(ty, opt);
                } else {
                    return Err(anyhow!("invalid ty"));
                }
            }
        };
        Self::new(ty, opt)
    }

    pub fn from_toml(path: &str, mut opt: u32) -> Result<Config> {
        let toml = std::fs::read_to_string(path)?;
        let c: ConfigToml = toml::from_str(&toml)?;

        opt &= COMPLEX | SYMBOLICA;
        let mut config = Self::from_name(&c.ty, opt)?;

        config.set_simd(c.options.use_simd);
        config.enable_simd512(c.options.enable_simd512);
        config.set_threads(c.options.use_threads);
        config.set_cse(c.options.cse);
        config.set_fastmath(c.options.fastmath);
        config.set_complex(c.options.complex | config.is_complex());
        config.set_symbolica(c.options.symbolica | config.symbolica());
        config.set_simd_branch(c.options.simd_branch);
        config.set_compact(c.options.compact);
        config.set_compress(c.options.compress);
        config.set_direct(c.options.direct);
        config.set_fast_complex(c.options.fast_complex);
        config.set_huge(c.options.huge);
        config.set_parallel_mul(c.options.parallel_mul);
        config.set_direct_arena(c.options.direct_arena);
        config.set_direct_arena_identity_output(c.options.direct_arena_identity_output);

        config.set_opt_level(c.options.opt_level);
        config.set_stack_limit(c.options.stack_limit);
        config.set_num_args(c.options.num_args);
        config.set_coroutines(c.options.coroutines);
        config.set_stable_math(c.options.stable_math);

        config.set_debug_bytecode(c.debug.bytecode);
        config.set_debug_scalar(c.debug.scalar);
        config.set_debug_simd(c.debug.simd);
        config.set_debug_stats(c.debug.stats);
        config.set_debug_topology(c.debug.topology);
        config.set_debug_instructions(c.debug.instructions);
        config.set_debug_lock(c.debug.lock);

        Ok(config)
    }

    pub fn to_toml(&self, path: &str) {
        let ty = match self.ty {
            CompilerType::ByteCode => "bytecode",
            CompilerType::Wasm => "wasm",
        }
        .into();

        let options: Options = Options {
            use_simd: self.use_simd(),
            enable_simd512: self.is_enabled_simd512(),
            use_threads: self.use_threads(),
            cse: self.cse(),
            fastmath: self.fastmath(),
            complex: self.is_complex(),
            symbolica: self.symbolica(),
            simd_branch: self.simd_branch(),
            compact: self.compact(),
            compress: self.compress(),
            direct: self.direct(),
            fast_complex: self.fast_complex(),
            opt_level: self.opt_level(),
            stack_limit: self.stack_limit(),
            num_args: self.num_args(),
            huge: self.huge(),
            parallel_mul: self.parallel_mul(),
            direct_arena: self.direct_arena(),
            direct_arena_identity_output: self.direct_arena_identity_output(),
            coroutines: self.coroutines(),
            stable_math: self.stable_math(),
        };

        let debug: DebugOptions = DebugOptions {
            bytecode: self.debug_bytecode(),
            scalar: self.debug_scalar(),
            simd: self.debug_simd(),
            stats: self.debug_stats(),
            topology: self.debug_topology(),
            instructions: self.debug_instructions(),
            lock: self.debug_lock(),
        };

        let c: ConfigToml = ConfigToml { ty, options, debug };
        let toml = toml::to_string(&c).unwrap();
        let _ = std::fs::write(path, toml);
    }

    pub fn from_defuns(df: Defuns) -> Result<Config> {
        let mut config = Config::default();
        config.set_defuns(df);
        Ok(config)
    }

    pub fn set_defuns(&mut self, df: Defuns) {
        match self.df {
            None => self.df = Some(Arc::new(df)),
            Some(_) => panic!("Config defuns can only be set once."),
        }
    }

    fn test(&self, mask: u32) -> bool {
        self.opt & mask != 0
    }

    pub fn is_wasm(&self) -> bool {
        matches!(self.ty, CompilerType::Wasm)
    }

    pub fn is_bytecode(&self) -> bool {
        matches!(self.ty, CompilerType::ByteCode)
    }

    pub fn cse(&self) -> bool {
        self.test(CSE)
    }

    // the SIMD, thread and fast-complex options of a saved configuration are kept (and
    // saved again) but have no effect: a module evaluates one point per call
    pub fn use_simd(&self) -> bool {
        false
    }

    pub fn is_enabled_simd512(&self) -> bool {
        self.test(ENABLE_SIMD512)
    }

    pub fn use_simd512(&self) -> bool {
        false
    }

    pub fn simd_branch(&self) -> bool {
        false
    }

    pub fn use_threads(&self) -> bool {
        false
    }

    // WebAssembly has no fused multiply-add, and the reassociations of fastmath would make the
    // module differ from the interpreter's bits
    pub fn fastmath(&self) -> bool {
        false
    }

    pub fn compact(&self) -> bool {
        self.test(COMPACT)
    }

    // the WebAssembly generator does not implement compression mode
    pub fn compress(&self) -> bool {
        self.test(COMPRESS) && !self.is_wasm()
    }

    pub fn coroutines(&self) -> bool {
        self.test(COROUTINES)
    }

    pub fn direct(&self) -> bool {
        self.test(DIRECT)
    }

    pub fn huge(&self) -> bool {
        self.test(HUGE)
    }

    pub fn parallel_mul(&self) -> bool {
        self.test(PARALLEL_MUL)
    }

    /// Range-safe complex arithmetic: complex division (and `recip`), `abs` and `sqrt` scale
    /// their operands by a power of two, so they do not overflow or underflow unless the result
    /// does, and negative integer powers take the reciprocal before the power. The scaling is
    /// exact, so results in the normal range keep the bits of the default code.
    pub fn stable_math(&self) -> bool {
        self.test(STABLE_MATH)
    }

    /// Uses pointer descriptors for scalar inputs to a direct arena application.
    pub fn direct_arena(&self) -> bool {
        self.test(DIRECT_ARENA)
    }

    /// Destination operation used by a direct arena application.
    pub fn direct_arena_operation(&self) -> u8 {
        ((self.opt & DIRECT_ARENA_OPERATION_MASK) >> DIRECT_ARENA_OPERATION_SHIFT) as u8
    }

    /// Stores outputs without complex-scalar scaling.
    pub fn direct_arena_identity_output(&self) -> bool {
        true
        //self.test(DIRECT_ARENA_IDENTITY_OUTPUT)
    }

    pub fn debug_bytecode(&self) -> bool {
        self.test(DEBUG_BYTECODE)
    }

    pub fn debug_scalar(&self) -> bool {
        self.test(DEBUG_SCALAR)
    }

    pub fn debug_simd(&self) -> bool {
        self.test(DEBUG_SIMD)
    }

    pub fn debug_stats(&self) -> bool {
        self.test(DEBUG_STATS)
    }

    pub fn debug_topology(&self) -> bool {
        self.test(DEBUG_TOPOLOGY)
    }

    pub fn debug_instructions(&self) -> bool {
        self.test(DEBUG_INSTRUCTIONS)
    }

    pub fn debug_lock(&self) -> bool {
        self.test(DEBUG_LOCK)
    }

    pub fn opt_level(&self) -> u8 {
        ((self.opt & OPT_LEVEL_MASK) >> OPT_LEVEL_SHIFT) as u8
    }

    pub fn stack_limit(&self) -> usize {
        self.stack
    }

    pub fn num_args(&self) -> u32 {
        self.args
    }

    pub fn compiler_type(&self) -> CompilerType {
        self.ty
    }

    pub fn kernel_type(&self) -> KernelType {
        if self.symbolica() && !self.direct_arena() {
            KernelType::RowFirst
        } else {
            KernelType::ColumnFirst(false)
        }
    }

    // registers are WebAssembly locals: as many as x86's
    pub fn available_registers(&self) -> u8 {
        16
    }

    pub fn count_scratch(&self) -> u8 {
        if !self.is_complex() {
            self.available_registers() - 2
        }
        /*
        else if self.fast_complex() && !self.use_simd() && (self.is_arm64() || self.is_riscv64())
        {
            self.available_registers() - 5
        }
        */
        else {
            (self.available_registers() - 6) / 2
        }
    }

    // user-defined functions are imported and called like the math functions, so only a
    // recursive call takes the kernel convention
    pub fn is_external_func(&self, op: &str) -> bool {
        op == "@self"
    }

    /// `op` is a user-defined function (`Defuns`), real or complex: imported by the module and
    /// called like a math function, with its one or two arguments in registers
    pub fn is_imported_func(&self, op: &str) -> bool {
        match &self.df {
            Some(df) => [op.to_string(), format!("cplx_{}", op)]
                .iter()
                .any(|name| matches!(df.funcs.get(name), Some(Func::Import { .. }))),
            None => false,
        }
    }

    pub fn is_kernel_func(&self, op: &str) -> bool {
        if op == "@self" {
            true
        } else if let Some(df) = &self.df {
            matches!(df.funcs.get(op), Some(Func::Recursive))
        } else {
            false
        }
    }

    pub fn location(&self, arg: u8) -> Loc {
        let k = if self.is_complex() { 2 } else { 1 };
        Loc::Stack(SPILL_AREA as u32 + k * arg as u32)
    }

    pub fn symbolica(&self) -> bool {
        self.test(SYMBOLICA)
    }

    pub fn is_complex(&self) -> bool {
        self.test(COMPLEX)
    }

    pub fn fast_complex(&self) -> bool {
        false
    }

    /// Sets of optimization level. The valid values are 0, 1, 2, which roughly correspond to gcc O0, O1, and O2 levels.
    pub fn set_opt_level(&mut self, opt_level: u8) {
        if !self.debug_lock() {
            self.opt = (self.opt & !OPT_LEVEL_MASK) | ((opt_level as u32) << OPT_LEVEL_SHIFT);
        }
    }

    /// Enables Common-Subexpression-Elimination.
    pub fn set_cse(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !CSE) | if enabled { CSE } else { 0 };
        }
    }

    /// Enables fastmath mode. The main effect is to generate fused-multiply-addition
    /// instructions if possible.
    pub fn set_fastmath(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !FASTMATH) | if enabled { FASTMATH } else { 0 };
        }
    }

    /// Enables SIMD mode.
    pub fn set_simd(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !USE_SIMD) | if enabled { USE_SIMD } else { 0 };
        }
    }

    /// Enables SIMD mode.
    pub fn enable_simd512(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !ENABLE_SIMD512) | if enabled { ENABLE_SIMD512 } else { 0 };
        }
    }

    /// Enables forced SIMD branching mode.
    pub fn set_simd_branch(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !SIMD_BRANCH) | if enabled { SIMD_BRANCH } else { 0 };
        }
    }

    /// Enables Complex Numbers.
    pub fn set_complex(&mut self, enabled: bool) {
        self.opt = (self.opt & !COMPLEX) | if enabled { COMPLEX } else { 0 };
    }

    /// Enables Fast Complex (using SIMD instructions in the scalar code).
    pub fn set_fast_complex(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !FAST_COMPLEX) | if enabled { FAST_COMPLEX } else { 0 };
        }
    }

    /// Enables Multi-threading.
    pub fn set_threads(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !USE_THREADS) | if enabled { USE_THREADS } else { 0 };
        }
    }

    /// Enables Symbolica Mode.
    pub fn set_symbolica(&mut self, enabled: bool) {
        self.opt = (self.opt & !SYMBOLICA) | if enabled { SYMBOLICA } else { 0 };
    }

    /// Compact stack frame.
    pub fn set_compact(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !COMPACT) | if enabled { COMPACT } else { 0 };
        }
    }

    /// Memory-saver mode for very large inputs.
    pub fn set_compress(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !COMPRESS) | if enabled { COMPRESS } else { 0 };
        }
    }

    /// Use co-routines for single-threaded vectorized code.
    pub fn set_coroutines(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !COROUTINES) | if enabled { COROUTINES } else { 0 };
        }
    }

    /// Direct translation from Symbolica IR to Symjit IR.
    /// This is a typo but is kept for compatibility.
    pub fn set_dicect(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !DIRECT) | if enabled { DIRECT } else { 0 };
        }
    }

    /// Direct translation from Symbolica IR to Symjit IR.
    pub fn set_direct(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !DIRECT) | if enabled { DIRECT } else { 0 };
        }
    }

    /// Huge paged to reduce TLB pressure.
    pub fn set_huge(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !HUGE) | if enabled { HUGE } else { 0 };
        }
    }

    /// Merge serial complex multiplications into parallel operation.
    pub fn set_parallel_mul(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !PARALLEL_MUL) | if enabled { PARALLEL_MUL } else { 0 };
        }
    }

    /// Enables range-safe complex arithmetic (see `stable_math`).
    pub fn set_stable_math(&mut self, enabled: bool) {
        if !self.debug_lock() {
            self.opt = (self.opt & !STABLE_MATH) | if enabled { STABLE_MATH } else { 0 };
        }
    }

    /// Enables the direct arena scalar-input ABI.
    pub fn set_direct_arena(&mut self, enabled: bool) {
        self.opt = (self.opt & !DIRECT_ARENA) | if enabled { DIRECT_ARENA } else { 0 };
    }

    /// Sets the direct arena destination operation encoded in portable MIR.
    pub fn set_direct_arena_operation(&mut self, operation: u8) {
        assert!(operation <= 1, "invalid direct arena destination operation");
        self.opt = (self.opt & !DIRECT_ARENA_OPERATION_MASK)
            | ((operation as u32) << DIRECT_ARENA_OPERATION_SHIFT);
    }

    /// Selects identity output scaling independently of the operation.
    pub fn set_direct_arena_identity_output(&mut self, enabled: bool) {
        self.opt = (self.opt & !DIRECT_ARENA_IDENTITY_OUTPUT)
            | if enabled {
                DIRECT_ARENA_IDENTITY_OUTPUT
            } else {
                0
            };
    }

    /// Dump bytecode for debugging
    pub fn set_debug_bytecode(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_BYTECODE) | if enabled { DEBUG_BYTECODE } else { 0 };
    }

    /// Dump scalar binary for debugging
    pub fn set_debug_scalar(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_SCALAR) | if enabled { DEBUG_SCALAR } else { 0 };
    }

    /// Dump simd binary for debugging
    pub fn set_debug_simd(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_SIMD) | if enabled { DEBUG_SIMD } else { 0 };
    }

    /// Print stats for debugging
    pub fn set_debug_stats(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_STATS) | if enabled { DEBUG_STATS } else { 0 };
    }

    /// Print topology information for debugging
    pub fn set_debug_topology(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_TOPOLOGY) | if enabled { DEBUG_TOPOLOGY } else { 0 };
    }

    /// Print instructions for deugging
    pub fn set_debug_instructions(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_INSTRUCTIONS) | if enabled { DEBUG_INSTRUCTIONS } else { 0 };
    }

    /// Print stats for debugging
    pub fn set_debug_lock(&mut self, enabled: bool) {
        self.opt = (self.opt & !DEBUG_LOCK) | if enabled { DEBUG_LOCK } else { 0 };
    }

    pub fn set_stack_limit(&mut self, stack_limit: usize) {
        self.stack = stack_limit.max(DEFAULT_STACK_LIMIT);
    }

    pub fn set_num_args(&mut self, num_args: u32) {
        if num_args > SLICE_CAP as u32 {
            eprintln!("Warning! Max number of arguments is {}.", SLICE_CAP);
        }
        self.args = num_args.clamp(COMPRESSED_ARGS_CAP as u32, SLICE_CAP as u32);
    }

    pub fn fixed(&self) -> u32 {
        if self.is_complex() {
            2 * self.num_args() + SPILL_AREA as u32
        } else {
            self.num_args() + SPILL_AREA as u32
        }
    }

    pub fn max_lanes(&self) -> usize {
        1
    }

    pub fn sizeof(&self) -> usize {
        if self.is_complex() {
            2
        } else {
            1
        }
    }

    pub fn set_option(&mut self, option: &str, val: &str) -> Result<()> {
        match option {
            "use_simd" => {
                self.set_simd(val.parse::<bool>()?);
            }
            "enable_simd512" => {
                self.enable_simd512(val.parse::<bool>()?);
            }
            "use_threads" => {
                self.set_threads(val.parse::<bool>()?);
            }
            "cse" => {
                self.set_cse(val.parse::<bool>()?);
            }
            "fastmath" => {
                self.set_fastmath(val.parse::<bool>()?);
            }
            "complex" => {
                self.set_complex(val.parse::<bool>()?);
            }
            "symbolica" => {
                self.set_symbolica(val.parse::<bool>()?);
            }
            "simd_branch" => {
                self.set_simd_branch(val.parse::<bool>()?);
            }
            "compact" => {
                self.set_compact(val.parse::<bool>()?);
            }
            "compress" => {
                self.set_compress(val.parse::<bool>()?);
            }
            "direct" => {
                self.set_direct(val.parse::<bool>()?);
            }
            "fast_complex" => {
                self.set_fast_complex(val.parse::<bool>()?);
            }
            "huge" => {
                self.set_huge(val.parse::<bool>()?);
            }
            "parallel_mul" => {
                self.set_parallel_mul(val.parse::<bool>()?);
            }
            "coroutines" => {
                self.set_coroutines(val.parse::<bool>()?);
            }
            "stable_math" => {
                self.set_stable_math(val.parse::<bool>()?);
            }
            "opt_level" => {
                self.set_opt_level(val.parse::<u8>()?);
            }
            "stack_limit" => {
                self.set_stack_limit(val.parse::<usize>()?);
            }
            "num_args" => {
                self.set_num_args(val.parse::<u32>()?);
            }
            "debug_bytecode" => {
                self.set_debug_bytecode(val.parse::<bool>()?);
            }
            "debug_scalar" => {
                self.set_debug_scalar(val.parse::<bool>()?);
            }
            "debug_simd" => {
                self.set_debug_simd(val.parse::<bool>()?);
            }
            "debug_stats" => {
                self.set_debug_stats(val.parse::<bool>()?);
            }
            "debug_topology" => {
                self.set_debug_topology(val.parse::<bool>()?);
            }
            _ => return Err(anyhow!("option {} is not recognized.", option)),
        }

        Ok(())
    }

    pub fn can_compile(&self) -> Result<()> {
        Ok(())
    }
}

impl Default for Config {
    fn default() -> Config {
        let toml = match std::env::var("SYMJIT_TOML") {
            Ok(toml) => toml,
            Err(_) => "symjit.toml".into(),
        };

        // (a browser has no file system: `exists` fails there)
        if std::fs::exists(&toml).unwrap_or(false) {
            Self::from_toml(&toml, 0).unwrap()
        } else {
            Config::new(
                CompilerType::Wasm,
                USE_SIMD
                    | CSE
                    | SYMBOLICA
                    | COMPACT
                    | FASTMATH
                    | FAST_COMPLEX
                    // | DIRECT
                    | PARALLEL_MUL
                    // | STABLE_MATH
                    | (2 << OPT_LEVEL_SHIFT)
                    | DIRECT_ARENA_IDENTITY_OUTPUT,
            )
            .unwrap()
            // config.to_toml("symjit.toml");
        }
    }
}

// the list of intrinsic unary ops, i.e., operations that can be implemented directly in
// machine code
const UNARY: &[&str] = &[
    "abs",
    "abs2",
    "not",
    "neg",
    "root",
    "real_root",
    "square",
    "cube",
    "recip",
    "round",
    "floor",
    "ceiling",
    "trunc",
    "frac",
    "_powi_",
    "_call_",
    "real",
    "imaginary",
    "conjugate",
    "iszero",
    "isnotzero",
    "times_i",
    "times_neg_i",
];

// the list of intrinsic binary ops, i.e., operations that can be implemented directly in
// machine code
const BINARY: &[&str] = &[
    "plus",
    "minus",
    "times",
    "divide",
    "rem",
    "gt",
    "geq",
    "lt",
    "leq",
    "eq",
    "neq",
    "and",
    "or",
    "xor",
    "_ifelse_",
    "_powi_mod_",
    "_call_",
    "min",
    "max",
    "heaviside",
    "complex",
];

impl Config {
    pub fn is_intrinsic_unary(&self, op: &Operation) -> bool {
        match op {
            Operation::Op(s) => UNARY.contains(&s.as_str()),
            _ => false,
        }
    }

    pub fn is_intrinsic_binary(&self, op: &Operation) -> bool {
        match op {
            Operation::Plus | Operation::Minus | Operation::Times | Operation::Divide => true,
            Operation::Op(s) => BINARY.contains(&s.as_str()),
        }
    }

    pub fn symbolica_fun(&self, fun: &str, is_real: bool) -> String {
        if fun == "symbolica_sqrt" {
            if is_real {
                "real_root".into()
            } else {
                "root".into()
            }
        } else if let Some(op) = fun.strip_prefix("symbolica_") {
            // Aliases
            let op = match op {
                "log" => "ln", // because of discrepency between real/complex log/ln
                "imag" => "imaginary",
                "conj" => "conjugate",
                "asin" => "arcsin",
                "acos" => "arccos",
                "atan" => "arctan",
                "asinh" => "arcsinh",
                "acosh" => "arccosh",
                "atanh" => "arctanh",
                op => op,
            };

            let oper = Operation::new(op);

            if self.is_intrinsic_unary(&oper)
                || self.is_intrinsic_binary(&oper)
                || (!self.is_complex() && VirtualTable::from_str(op).is_ok())
                || (self.is_complex() && VirtualTable::from_str(&format!("cplx_{}", op)).is_ok())
            {
                op.into()
            } else {
                fun.into()
            }
        } else {
            fun.into()
        }
    }
}

impl Storage for Config {
    fn save(&self, stream: &mut impl Write) -> Result<()> {
        stream.write_all(&Self::MAGIC.to_le_bytes())?;

        // a module is saved as `native` (0): symjit compiles it for the machine that loads it
        let ty: u64 = match self.ty {
            CompilerType::Wasm => 0,
            CompilerType::ByteCode => 6,
        };

        let val: u64 = (self.opt as u64) | (ty << 32);
        stream.write_all(&val.to_le_bytes())?;

        let val: u64 = self.stack_limit() as u64;
        stream.write_all(&val.to_le_bytes())?;

        let val: u64 = self.num_args() as u64;
        stream.write_all(&val.to_le_bytes())?;

        Ok(())
    }

    fn load_with_version(stream: &mut impl Read, config: &Self, version: usize) -> Result<Self> {
        let mut bytes: [u8; 8] = [0; 8];

        stream.read_exact(&mut bytes)?;

        if u64::from_le_bytes(bytes) != Self::MAGIC {
            return Err(anyhow!("invalid magic number (Config)"));
        }

        stream.read_exact(&mut bytes)?;
        let val = u64::from_le_bytes(bytes);
        let opt: u32 = (val & 0xffffffff) as u32;
        let ty: u32 = (val >> 32) as u32;

        // sjb version 3 (symjit 2.22 to 2.26) saves neither the stack limit nor the argument
        // count; version 4 (2.27 on) saves both
        let (stack, args): (usize, u32) = if version == 3 {
            (DEFAULT_STACK_LIMIT, Config::default().num_args())
        } else {
            stream.read_exact(&mut bytes)?;
            let stack = u64::from_le_bytes(bytes) as usize;
            stream.read_exact(&mut bytes)?;
            (stack, u64::from_le_bytes(bytes) as u32)
        };

        // native, amd, amd-avx, amd-sse, arm, riscv, bytecode, debug, wasm (symjit 2.28's
        // `wasm` feature): the functions of any machine compile to WebAssembly here
        let ty: CompilerType = match ty {
            6 => CompilerType::ByteCode,
            0..=8 => CompilerType::Wasm,
            _ => return Err(anyhow!("invalid compiler type value.")),
        };

        Ok(Config {
            opt,
            ty,
            df: config.df.clone(),
            stack,
            args,
        })
    }
}
