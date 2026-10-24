//! The C interface of symjit's Python package (`python/symjit/engine.py`, through `ctypes`),
//! for compiling SymPy models (cargo feature `sympy`): the calls that compile, translate, save,
//! load and inspect a model, and `wasm_size`/`wasm_bytes`, which return its WebAssembly module.
//! There are no calls that run a model: the module's host runs it.

use std::collections::HashSet;
use std::ffi::{c_char, CStr, CString};
use std::fmt::Debug;
use std::str::FromStr;

use crate::{Application, CellModel, Compiler, CompilerType, Config, Defuns, Program, Storage};

#[derive(Debug, Clone, Copy)]
pub enum CompilerStatus {
    Ok,
    Incomplete,
    InvalidUtf8,
    ParseError,
    InvalidCompiler,
    CompilationError,
}

pub struct CompilerResult {
    app: Option<Application>,
    status: CompilerStatus,
    msg: CString,
}

fn error_message<E: Debug>(msg: &str, err: E) -> CString {
    let s = format!("{:?}: {:?}", msg, err);
    CString::from_str(&s).unwrap()
}

/// Compiles a model.
///
/// * model is a json string encoding the model.
/// * ty is `wasm` (or a native type of symjit, which names it) or `bytecode`.
/// * opt: compilation options.
/// * df: user-defined functions.
///
/// # Safety
/// Both model and ty are pointers to null-terminated strings.
/// The output is a raw pointer to a CompilerResults.
///
#[no_mangle]
pub unsafe extern "C" fn compile(
    model: *const c_char,
    ty: *const c_char,
    opt: u32,
    df: *const Defuns,
) -> *const CompilerResult {
    let mut res = CompilerResult {
        app: None,
        status: CompilerStatus::Incomplete,
        msg: CString::from_str("Success").unwrap(),
    };

    let model = unsafe {
        match CStr::from_ptr(model).to_str() {
            Ok(model) => model,
            Err(msg) => {
                res.status = CompilerStatus::InvalidUtf8;
                res.msg = error_message("Invalid encoding", msg);
                return Box::into_raw(Box::new(res)) as *const _;
            }
        }
    };

    let ty = unsafe {
        match CStr::from_ptr(ty).to_str() {
            Ok(ty) => ty,
            Err(msg) => {
                res.status = CompilerStatus::InvalidUtf8;
                res.msg = error_message("Invalid compiler type", msg);
                return Box::into_raw(Box::new(res)) as *const _;
            }
        }
    };

    let ml = match CellModel::load(model) {
        Ok(ml) => ml,
        Err(msg) => {
            res.status = CompilerStatus::ParseError;
            res.msg = error_message("Cannot parse JSON", msg);
            return Box::into_raw(Box::new(res)) as *const _;
        }
    };

    if let Ok(mut config) = Config::from_name(ty, opt) {
        let df: Defuns = unsafe {
            if df.is_null() {
                Defuns::new()
            } else {
                (&*df).clone()
            }
        };

        config.set_defuns(df);

        let prog = match Program::new(&ml, config) {
            Ok(prog) => prog,
            Err(msg) => {
                res.status = CompilerStatus::CompilationError;
                res.msg = error_message("Compilation error (prog)", msg);
                return Box::into_raw(Box::new(res)) as *const _;
            }
        };

        let app = Application::new(prog, HashSet::new());

        match app {
            Ok(app) => {
                res.status = CompilerStatus::Ok;
                res.app = Some(app);
            }
            Err(msg) => {
                res.status = CompilerStatus::CompilationError;
                res.msg = error_message("Compilation error (app)", &msg);
            }
        }
    } else {
        res.status = CompilerStatus::InvalidCompiler;
        // the reason (e.g. an invalid option after the type in `ty`)
        let err = Config::from_name(ty, opt).err().map(|e| e.to_string());
        res.msg = error_message("Config error", err.unwrap_or_default());
    }

    Box::into_raw(Box::new(res)) as *const _
}

/// Translates a Symbolica model.
///
/// * json is a json string encoding the output of export_instructions.
/// * ty is `wasm` (or a native type of symjit, which names it) or `bytecode`.
/// * opt: compilation options.
/// * df: user-defined functions (currently ignored).
///
/// # Safety
/// Both model and ty are pointers to null-terminated strings.
/// The output is a raw pointer to a CompilerResults.
///
#[no_mangle]
pub unsafe extern "C" fn translate(
    json: *const c_char,
    ty: *const c_char,
    opt: u32,
    df: *mut Defuns,
    num_params: usize,
) -> *const CompilerResult {
    let mut res = CompilerResult {
        app: None,
        status: CompilerStatus::Incomplete,
        msg: CString::from_str("Success").unwrap(),
    };

    let json = unsafe {
        match CStr::from_ptr(json).to_str() {
            Ok(json) => json,
            Err(msg) => {
                res.status = CompilerStatus::InvalidUtf8;
                res.msg = error_message("Invalid encoding", msg);
                return Box::into_raw(Box::new(res)) as *const _;
            }
        }
    };

    let ty = unsafe {
        match CStr::from_ptr(ty).to_str() {
            Ok(ty) => ty,
            Err(msg) => {
                res.status = CompilerStatus::InvalidUtf8;
                res.msg = error_message("Invalid compiler type", msg);
                return Box::into_raw(Box::new(res)) as *const _;
            }
        }
    };

    if let Ok(mut config) = Config::from_name(ty, opt) {
        let df: Defuns = unsafe {
            if df.is_null() {
                Defuns::new()
            } else {
                (&*df).clone()
            }
        };

        config.set_defuns(df);
        let mut comp = Compiler::with_config(config);
        let app = comp.translate(json.to_string(), num_params);

        match app {
            Ok(app) => {
                res.app = Some(app);
                res.status = CompilerStatus::Ok;
            }
            Err(msg) => {
                res.status = CompilerStatus::InvalidCompiler;
                res.msg = error_message("Compilation error", msg);
            }
        }
    } else {
        res.status = CompilerStatus::InvalidCompiler;
        // the reason (e.g. an invalid option after the type in `ty`)
        let err = Config::from_name(ty, opt).err().map(|e| e.to_string());
        res.msg = error_message("Config error", err.unwrap_or_default());
    }

    Box::into_raw(Box::new(res)) as *const _
}

/// Checks the status of a CompilerResult.
///
/// Returns a null-terminated string representing the status message.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn check_status(q: *const CompilerResult) -> *const c_char {
    let q: &CompilerResult = unsafe { &*q };
    q.msg.as_ptr() as *const _
}

/// Checks the status of a CompilerResult.
///
/// Returns a null-terminated string representing the status message.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn save(q: *const CompilerResult, file: *const c_char) -> bool {
    let q: &CompilerResult = unsafe { &*q };
    let file = unsafe {
        match CStr::from_ptr(file).to_str() {
            Ok(file) => file,
            Err(_) => return false,
        }
    };

    if let Some(app) = &q.app {
        if let Ok(mut fs) = std::fs::File::create(file) {
            app.save(&mut fs).is_ok()
        } else {
            false
        }
    } else {
        false
    }
}

/// Checks the status of a CompilerResult.
///
/// Returns a null-terminated string representing the status message.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn load(file: *const c_char, df: *mut Defuns) -> *const CompilerResult {
    let mut res = CompilerResult {
        app: None,
        status: CompilerStatus::Incomplete,
        msg: CString::from_str("Success").unwrap(),
    };

    let df: Defuns = unsafe {
        if df.is_null() {
            Defuns::new()
        } else {
            (&*df).clone()
        }
    };

    let file = unsafe {
        match CStr::from_ptr(file).to_str() {
            Ok(file) => file,
            Err(_) => return Box::into_raw(Box::new(res)) as *const _,
        }
    };

    let fs = std::fs::File::open(file);

    match fs {
        Ok(mut fs) => match Application::load(&mut fs, &Config::from_defuns(df).unwrap()) {
            Ok(app) => {
                res.app = Some(app);
                res.status = CompilerStatus::Ok;
            }
            Err(err) => {
                res.status = CompilerStatus::ParseError;
                res.msg = error_message("File parse error", &err);
            }
        },
        Err(err) => {
            res.msg = error_message("File I/O error", &err);
        }
    }

    Box::into_raw(Box::new(res)) as *const _
}

/// Checks the status of a CompilerResult.
///
/// Returns a null-terminated string representing the status message.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn get_config(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };

    match &q.app {
        Some(app) => {
            let config = app.prog.config();

            // as saved in `.sjb` files: a module is `native` (0)
            let ty: u64 = match config.ty {
                CompilerType::Wasm => 0,
                CompilerType::ByteCode => 6,
            };

            ((config.opt as u64) | (ty << 32)) as usize
        }
        None => 0,
    }
}

/// Returns the number of state variables.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn count_states(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    if let Some(app) = &q.app {
        app.count_states
    } else {
        0
    }
}

/// Returns the number of parameters.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn count_params(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    if let Some(app) = &q.app {
        app.count_params
    } else {
        0
    }
}

/// Returns the number of observables (output).
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn count_obs(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    if let Some(app) = &q.app {
        app.count_obs
    } else {
        0
    }
}

/// Returns the number of differential equations.
///
/// Generally, it should be the same as the number of states.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn count_diffs(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    if let Some(app) = &q.app {
        app.count_diffs
    } else {
        0
    }
}

/// Dumps the compiled binary code to a file (name).
///
/// This function is useful for debugging but is not necessary for
/// normal operations.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn dump(
    q: *mut CompilerResult,
    name: *const c_char,
    what: *const c_char,
) -> bool {
    let q: &mut CompilerResult = unsafe { &mut *q };
    if let Some(app) = &mut q.app {
        let name = unsafe { CStr::from_ptr(name).to_str().unwrap() };
        let what = unsafe { CStr::from_ptr(what).to_str().unwrap() };
        app.dump(name, what)
    } else {
        false
    }
}

/// Returns the size in bytes of the model's WebAssembly module (0 if there is none).
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn wasm_size(q: *const CompilerResult) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    match q.app.as_ref().and_then(|app| app.wasm()) {
        Some(wasm) => wasm.len(),
        None => 0,
    }
}

/// Returns a pointer to the model's WebAssembly module (`wasm_size` bytes), or NULL.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult. The bytes belong to it: they are valid until `finalize(q)`.
///
#[no_mangle]
pub unsafe extern "C" fn wasm_bytes(q: *const CompilerResult) -> *const u8 {
    let q: &CompilerResult = unsafe { &*q };
    match q.app.as_ref().and_then(|app| app.wasm()) {
        Some(wasm) => wasm.as_ptr(),
        None => std::ptr::null(),
    }
}

/// Returns diagnostic measurements.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points
/// to a valid CompilerResult.
///
#[no_mangle]
pub unsafe extern "C" fn measure(q: *mut CompilerResult, what: *const c_char) -> usize {
    let q: &CompilerResult = unsafe { &*q };
    if let Some(app) = &q.app {
        let what = unsafe { CStr::from_ptr(what).to_str().unwrap() };
        app.measure(what)
    } else {
        0
    }
}

/// Deallocates the CompilerResult pointed by q.
///
/// # Safety
/// It is the responsibility of the calling function to ensure that q points to a
/// valid CompilerResult and that after calling this function, q is invalid and
/// should not be used anymore.
///
#[no_mangle]
pub unsafe extern "C" fn finalize(q: *mut CompilerResult) {
    if !q.is_null() {
        let _ = unsafe { Box::from_raw(q) };
    }
}

/// Returns a null-terminated string representing the version.
///
/// Used for debugging.
///
/// # Safety
/// The return value is a null-terminated string that should not be freed.
///
#[no_mangle]
pub unsafe extern "C" fn info() -> *const c_char {
    // let msg = c"symjit 1.3.3";
    let msg = CString::new(env!("CARGO_PKG_VERSION")).unwrap();
    msg.into_raw() as *const _
}

/************************************************/

/// Creates an empty Defun (a list of user-defined functions).
///
/// Defuns declare the user-defined functions of a model, which its WebAssembly
/// module imports from its host.
///
/// # Safety
/// It returns a pointer to the allocated Defun, which needs to be
/// deallocated eventually.
///
#[no_mangle]
pub unsafe extern "C" fn create_defuns() -> *const Defuns {
    let df = Defuns::new();
    Box::into_raw(Box::new(df)) as *const Defuns
}

/// Finalizes (deallocates) a Defun.
///
/// # Safety
/// df should point to a valid Defun object created by create_defuns.
/// After finalize_defun is called, df is invalid.
///
#[no_mangle]
pub unsafe extern "C" fn finalize_defuns(df: *mut Defuns) {
    if !df.is_null() {
        let _ = unsafe { Box::from_raw(df) };
    }
}

/// Declares a function of `num_args` (1 or 2) double arguments, which the module imports
/// as `symjit.<name>`. Returns false if it cannot be declared.
///
/// # Safety
/// df should point to a valid Defun object created by create_defun.
/// name should be a valid utf8 string.
/// p is ignored (a module cannot call a function pointer of the compiling process); it is
/// kept for the Python package's signature.
///
#[no_mangle]
pub unsafe extern "C" fn add_func(
    df: *mut Defuns,
    name: *const c_char,
    _p: *const usize,
    num_args: usize,
) -> bool {
    let df: &mut Defuns = unsafe { &mut *df };
    match unsafe { CStr::from_ptr(name).to_str() } {
        Ok(name) => df.add_func(name, num_args).is_ok(),
        Err(_) => false,
    }
}

/// Adds self (recursive) to a Defun.
///
/// # Safety
/// df should point to a valid Defun object created by create_defun.
/// name should be a valid utf8 string.
///
#[no_mangle]
pub unsafe extern "C" fn add_self(df: *mut Defuns, name: *const c_char) {
    let df: &mut Defuns = unsafe { &mut *df };
    let name = unsafe { CStr::from_ptr(name).to_str().unwrap() };
    df.add_self(name);
}

/// Declares a complex function of `num_args` (1 or 2) complex arguments, which a complex
/// model's module imports as `symjit.cplx_<name>`. Returns false if it cannot be declared.
///
/// # Safety
/// df should point to a valid Defun object created by create_defun.
/// name should be a valid utf8 string.
///
#[no_mangle]
pub unsafe extern "C" fn add_func_complex(
    df: *mut Defuns,
    name: *const c_char,
    num_args: usize,
) -> bool {
    let df: &mut Defuns = unsafe { &mut *df };
    match unsafe { CStr::from_ptr(name).to_str() } {
        Ok(name) => df.add_func_complex(name, num_args).is_ok(),
        Err(_) => false,
    }
}

/************************************************/

/// Allocates `size` bytes for a caller that cannot allocate in this module's memory (a
/// JavaScript host of the crate compiled to WebAssembly passes its strings in them).
///
/// # Safety
/// The block must be released with `symjit_free(p, size)`.
///
#[no_mangle]
pub unsafe extern "C" fn symjit_alloc(size: usize) -> *mut u8 {
    let mut v: Vec<u8> = Vec::with_capacity(size.max(1));
    let p = v.as_mut_ptr();
    std::mem::forget(v);
    p
}

/// Releases a block of `symjit_alloc`.
///
/// # Safety
/// `p` and `size` should be those of a `symjit_alloc` call, released once.
///
#[no_mangle]
pub unsafe extern "C" fn symjit_free(p: *mut u8, size: usize) {
    if !p.is_null() {
        drop(unsafe { Vec::from_raw_parts(p, 0, size.max(1)) });
    }
}
