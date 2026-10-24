// symjit.mjs: runs the WebAssembly modules of symjit-wasm (`Application::wasm`) in a browser
// or in Node. No dependencies; see docs/WASM.md for the module interface.
//
//     import { SymjitModel } from "./symjit.mjs";
//     const model = await SymjitModel.compile(bytes);      // bytes: the module
//     const y = model.evaluate([1.0, 2.0]);                 // a Symbolica/Composer model
//     const z = model.call([0.5, 0.25]);                    // a model of states (SymPy, Expr)
//
// A module imports its math functions, and the user-defined functions it was compiled with
// (`Defuns`), from the import module "symjit"; MATH supplies the math functions, and `host`
// the others: {name: (x[, y]) => number} for real ones, {cplx_name: (re, im[, re2, im2]) =>
// [re, im]} for complex ones.

export const IMPORT_MODULE = "symjit";

const recip = (f) => (x) => 1 / f(x);

// symjit function names -> JavaScript implementations (symjit's `log` is `ln`, as in
// symjit 2.28 and later; `log10` is the decimal logarithm)
const REAL = {
    sin: Math.sin, cos: Math.cos, tan: Math.tan,
    csc: recip(Math.sin), sec: recip(Math.cos), cot: recip(Math.tan),
    sinh: Math.sinh, cosh: Math.cosh, tanh: Math.tanh,
    csch: recip(Math.sinh), sech: recip(Math.cosh), coth: recip(Math.tanh),
    arcsin: Math.asin, arccos: Math.acos, arctan: Math.atan,
    arcsinh: Math.asinh, arccosh: Math.acosh, arctanh: Math.atanh,
    sinc: (x) => (x === 0 ? 1 : Math.sin(x) / x),
    exp: Math.exp, expm1: Math.expm1, exp2: (x) => 2 ** x,
    ln: Math.log, log: Math.log, log10: Math.log10, log1p: Math.log1p, log2: Math.log2,
    // C99 pow: pow(1, y) = 1 and pow(-1, +-inf) = 1 (JavaScript's Math.pow gives NaN)
    power: (x, y) => (x === 1 || (x === -1 && Math.abs(y) === Infinity) ? 1 : Math.pow(x, y)),
    cbrt: Math.cbrt, atan2: Math.atan2,
    arg: (x) => (x < 0 ? Math.PI : 0),
    random: (x) => Math.random() * x,
};

// complex functions take (re, im[, re, im]) and return [re, im] (principal branches, as in
// Rust's num_complex)
const c = {
    add: ([a, b], [x, y]) => [a + x, b + y],
    sub: ([a, b], [x, y]) => [a - x, b - y],
    mul: ([a, b], [x, y]) => [a * x - b * y, a * y + b * x],
    div: ([a, b], [x, y]) => { const d = x * x + y * y; return [(a * x + b * y) / d, (b * x - a * y) / d]; },
    exp: ([a, b]) => { const e = Math.exp(a); return [e * Math.cos(b), e * Math.sin(b)]; },
    ln: ([a, b]) => [Math.log(Math.hypot(a, b)), Math.atan2(b, a)],
    sqrt: ([a, b]) => {
        if (a === 0 && b === 0) return [0, b];
        const t = Math.sqrt((Math.hypot(a, b) + Math.abs(a)) / 2);
        return a >= 0 ? [t, b / (2 * t)] : [Math.abs(b) / (2 * t), Math.sign(b) * t || t];
    },
    sin: ([a, b]) => [Math.sin(a) * Math.cosh(b), Math.cos(a) * Math.sinh(b)],
    cos: ([a, b]) => [Math.cos(a) * Math.cosh(b), -Math.sin(a) * Math.sinh(b)],
    sinh: ([a, b]) => [Math.sinh(a) * Math.cos(b), Math.cosh(a) * Math.sin(b)],
    cosh: ([a, b]) => [Math.cosh(a) * Math.cos(b), Math.sinh(a) * Math.sin(b)],
};
c.recip = (z) => c.div([1, 0], z);
c.tan = (z) => c.div(c.sin(z), c.cos(z));
c.tanh = (z) => c.div(c.sinh(z), c.cosh(z));
c.pow = (z, w) => (z[0] === 0 && z[1] === 0 ? [0, 0] : c.exp(c.mul(w, c.ln(z))));
const I = [0, 1], ONE = [1, 0];
c.asin = (z) => c.mul([0, -1], c.ln(c.add(c.mul(I, z), c.sqrt(c.sub(ONE, c.mul(z, z))))));
c.atan = (z) => c.mul([0, 0.5], c.ln(c.div(c.add(I, z), c.sub(I, z))));
c.asinh = (z) => c.ln(c.add(z, c.sqrt(c.add(c.mul(z, z), ONE))));
c.acosh = (z) => c.ln(c.add(z, c.mul(c.sqrt(c.add(z, ONE)), c.sqrt(c.sub(z, ONE)))));
c.atanh = (z) => c.mul([0.5, 0], c.ln(c.div(c.add(ONE, z), c.sub(ONE, z))));

const unary = (f) => (re, im) => f([re, im]);
const COMPLEX = {
    cplx_sin: unary(c.sin), cplx_cos: unary(c.cos), cplx_tan: unary(c.tan),
    cplx_csc: unary((z) => c.recip(c.sin(z))), cplx_sec: unary((z) => c.recip(c.cos(z))),
    cplx_cot: unary((z) => c.recip(c.tan(z))),
    cplx_sinh: unary(c.sinh), cplx_cosh: unary(c.cosh), cplx_tanh: unary(c.tanh),
    cplx_csch: unary((z) => c.recip(c.sinh(z))), cplx_sech: unary((z) => c.recip(c.cosh(z))),
    cplx_coth: unary((z) => c.recip(c.tanh(z))),
    cplx_sinc: unary((z) => (z[0] === 0 && z[1] === 0 ? [1, 0] : c.div(c.sin(z), z))),
    cplx_arcsin: unary(c.asin), cplx_arccos: unary((z) => c.sub([Math.PI / 2, 0], c.asin(z))),
    cplx_arctan: unary(c.atan), cplx_arcsinh: unary(c.asinh), cplx_arccosh: unary(c.acosh),
    cplx_arctanh: unary(c.atanh),
    cplx_root: unary(c.sqrt), cplx_cbrt: unary((z) => c.pow(z, [1 / 3, 0])),
    cplx_exp: unary(c.exp), cplx_ln: unary(c.ln), cplx_log: unary(c.ln),
    cplx_log10: unary((z) => c.ln(z).map((v) => v / Math.LN10)),
    cplx_arg: unary(([a, b]) => [Math.atan2(b, a), 0]),
    cplx_power: (a, b, x, y) => c.pow([a, b], [x, y]),
    cplx_random: (a, b) => c.mul([Math.random(), Math.random()], [a, b]),
};

/** The math functions a module may import, by symjit name. */
export const MATH = Object.freeze({ ...REAL, ...COMPLEX });

/** The call shape a module describes in its custom section `symjit` (Application::wasm). */
export function shapeOf(module) {
    const sections = WebAssembly.Module.customSections(module, "symjit");
    if (sections.length !== 1) {
        throw new Error("not a symjit-wasm module (no `symjit` section)");
    }
    return JSON.parse(new TextDecoder().decode(sections[0]));
}

/** The functions `module` imports that neither MATH nor `host` supplies. */
export function missingImports(module, host = {}) {
    return WebAssembly.Module.imports(module)
        .filter((i) => i.kind === "function" && !(i.name in host) && !(i.name in MATH))
        .map((i) => i.name);
}

/** The import object of a module: MATH, then `host`. */
export function importObject(module, host = {}) {
    const missing = missingImports(module, host);
    if (missing.length > 0) {
        throw new Error(`no host implementation for: ${missing.join(", ")}`);
    }
    return { [IMPORT_MODULE]: { ...MATH, ...host } };
}

/**
 * A compiled model: an instance of its module and a block of the instance's memory for the
 * inputs and outputs of a call. Values are f64; a complex value is two (re, im).
 */
export class SymjitModel {
    /** Compiles and instantiates `bytes` (asynchronously: browsers limit synchronous
     * compilation on the main thread to small modules). */
    static async compile(bytes, host = {}) {
        const module = await WebAssembly.compile(bytes);
        const instance = await WebAssembly.instantiate(module, importObject(module, host));
        return new SymjitModel(module, instance);
    }

    /** Instantiates a compiled `WebAssembly.Module` synchronously. */
    static fromModule(module, host = {}) {
        return new SymjitModel(module, new WebAssembly.Instance(module, importObject(module, host)));
    }

    constructor(module, instance) {
        this.module = module;
        this.instance = instance;
        this.shape = shapeOf(module);
        const { run, memory, __heap_base } = instance.exports;
        this.#run = run;
        this.#memory = memory;

        // the block above `__heap_base`: params, then the model memory
        const s = this.shape;
        this.#params = __heap_base.value;
        this.#mem = this.#params + 8 * Math.max(1, s.params);
        const top = this.#mem + 8 * Math.max(1, s.mem);
        const pages = Math.ceil(top / 65536) - memory.buffer.byteLength / 65536;
        if (pages > 0) memory.grow(pages);
    }

    #run; #memory; #params; #mem;

    get countStates() { return this.shape.states; }
    get countObs() { return this.shape.obs; }
    get countParams() { return this.shape.params; }

    #view() {
        // a fresh view: `memory.grow` detaches the old buffer
        return new Float64Array(this.#memory.buffer);
    }

    #check(status) {
        if (status !== 0) throw new Error("symjit: the module's stack is exhausted (recursion too deep)");
    }

    /**
     * Evaluates a model of states (SymPy and `Expr` models) at one point: `states` (and
     * `params`) in, the observables (then the diffs of an ODE model) out.
     */
    call(states, params = []) {
        const s = this.shape;
        if (states.length !== s.states) throw new Error(`expected ${s.states} states, got ${states.length}`);
        if (params.length !== s.params) throw new Error(`expected ${s.params} params, got ${params.length}`);
        let f64 = this.#view();
        const mem = this.#mem / 8;
        f64.fill(0, mem, mem + s.mem);
        f64.set(states, mem);
        f64.set(params, this.#params / 8);
        this.#check(this.#run(this.#mem, 0, 0, this.#params));
        f64 = this.#view();
        return f64.slice(mem + s.states, mem + s.states + s.obs + s.diffs);
    }

    /**
     * Evaluates a Symbolica or `Composer` model, whose inputs are its parameters, at one
     * point: `params` in, the outputs out.
     */
    evaluate(params) {
        const s = this.shape;
        if (params.length !== s.params) throw new Error(`expected ${s.params} inputs, got ${params.length}`);
        let f64 = this.#view();
        const mem = this.#mem / 8;
        f64.fill(0, mem, mem + s.mem);
        f64.set(params, this.#params / 8);
        this.#check(this.#run(this.#mem, 0, 0, this.#params));
        f64 = this.#view();
        return f64.slice(mem + s.states, mem + s.states + s.obs);
    }

    /**
     * `evaluate` at `n` points: `args` holds n rows of `countParams` values, the result n rows
     * of `countObs` values (the layout of symjit's `evaluate_matrix`).
     */
    evaluateMatrix(args, n) {
        const s = this.shape;
        if (args.length !== n * s.params) throw new Error(`expected ${n * s.params} values, got ${args.length}`);
        const out = new Float64Array(n * s.obs);
        const mem = this.#mem / 8, params = this.#params / 8;
        for (let i = 0; i < n; i++) {
            const f64 = this.#view();
            f64.fill(0, mem, mem + s.mem);
            f64.set(args.subarray ? args.subarray(i * s.params, (i + 1) * s.params)
                                  : args.slice(i * s.params, (i + 1) * s.params), params);
            this.#check(this.#run(this.#mem, 0, 0, this.#params));
            out.set(this.#view().subarray(mem + s.states, mem + s.states + s.obs), i * s.obs);
        }
        return out;
    }

    /** The `fast` export (one observable, no parameters): the states as arguments. */
    fast(...states) {
        const f = this.instance.exports.fast;
        if (!f) throw new Error("the module has no `fast` export");
        return f(...states);
    }
}
