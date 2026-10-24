// compiler.mjs: symjit-wasm's compiler, itself compiled to WebAssembly
// (`cargo build --release --target wasm32-unknown-unknown` -> symjit_wasm.wasm), driven from
// JavaScript through its C interface (rust/sympy.rs). It turns models into modules, which
// symjit.mjs runs:
//
//     import { SymjitCompiler, options } from "./compiler.mjs";
//     import { SymjitModel } from "./symjit.mjs";
//     const compiler = await SymjitCompiler.load(await (await fetch("symjit_wasm.wasm")).arrayBuffer());
//     const bytes = compiler.translate(instructions, 2, options({ optLevel: 2 }));
//     const model = await SymjitModel.compile(bytes);
//
// `instructions` is a Symbolica evaluator's instructions, in the text form symjit's parser
// reads; `compile` takes a SymPy model as symjit's Python package encodes it (JSON).

// option bits (rust/config.rs)
export const CSE = 0x4, COMPLEX = 0x20, SYMBOLICA = 0x40, COMPACT = 0x1000, DIRECT = 0x4000,
    PARALLEL_MUL = 0x20_0000, STABLE_MATH = 0x0800_0000;

/**
 * The option word of `compile`/`translate`. `direct` translates Symbolica instructions one by
 * one instead of through SSA form: needed for loops (backward gotos) whose temporaries carry
 * values from one iteration to the next.
 */
export function options({ optLevel = 2, cse = true, complex = false, direct = false,
                          stableMath = false } = {}) {
    return (cse ? CSE : 0) | (complex ? COMPLEX : 0) | (direct ? DIRECT : 0) | SYMBOLICA | COMPACT
        | PARALLEL_MUL | (stableMath ? STABLE_MATH : 0) | ((optLevel & 0xf) << 8);
}

export class SymjitCompiler {
    /** Instantiates the compiler's module (its bytes, or a compiled `WebAssembly.Module`). */
    static async load(module) {
        const { instance } = module instanceof WebAssembly.Module
            ? { instance: await WebAssembly.instantiate(module, {}) }
            : await WebAssembly.instantiate(module, {});
        return new SymjitCompiler(instance);
    }

    constructor(instance) {
        this.exports = instance.exports;
    }

    /** The crate's version. */
    get version() {
        return this.#string(this.exports.info());
    }

    /**
     * Compiles a SymPy model (JSON) into a module. `functions` declares the user-defined
     * functions the model calls, {name: arity} (real) or {name: [arity, "complex"]}.
     */
    compile(json, opt = options(), functions = {}) {
        return this.#withDefuns(functions, (df) =>
            this.#withStrings([json, "wasm"], ([m, ty]) => this.exports.compile(m, ty, opt, df)));
    }

    /** Translates a Symbolica evaluator (`numParams` inputs) into a module. */
    translate(instructions, numParams, opt = options(), functions = {}) {
        return this.#withDefuns(functions, (df) =>
            this.#withStrings([instructions, "wasm"],
                ([m, ty]) => this.exports.translate(m, ty, opt, df, numParams)));
    }

    // the module of a CompilerResult, which is then released
    #result(q) {
        const e = this.exports;
        try {
            const msg = this.#string(e.check_status(q));
            const size = e.wasm_size(q);
            if (msg !== "Success" || size === 0) throw new Error(`symjit: ${msg}`);
            return new Uint8Array(e.memory.buffer, e.wasm_bytes(q), size).slice();
        } finally {
            e.finalize(q);
        }
    }

    #withDefuns(functions, f) {
        const e = this.exports;
        const df = e.create_defuns();
        try {
            for (const [name, spec] of Object.entries(functions)) {
                const [arity, kind] = Array.isArray(spec) ? spec : [spec, "real"];
                const ok = this.#withStrings([name], ([n]) => kind === "complex"
                    ? e.add_func_complex(df, n, arity)
                    : e.add_func(df, n, 0, arity));
                if (!ok) throw new Error(`symjit: cannot declare function ${name}`);
            }
            return this.#result(f(df));
        } finally {
            e.finalize_defuns(df);
        }
    }

    // calls f with the addresses of NUL-terminated copies of `strings` in the module's memory
    #withStrings(strings, f) {
        const e = this.exports;
        const blocks = strings.map((s) => {
            const bytes = new TextEncoder().encode(s);
            const p = e.symjit_alloc(bytes.length + 1);
            const view = new Uint8Array(e.memory.buffer, p, bytes.length + 1);
            view.set(bytes);
            view[bytes.length] = 0;
            return [p, bytes.length + 1];
        });
        try {
            return f(blocks.map(([p]) => p));
        } finally {
            for (const [p, n] of blocks) e.symjit_free(p, n);
        }
    }

    #string(p) {
        const mem = new Uint8Array(this.exports.memory.buffer);
        let end = p;
        while (mem[end] !== 0) end++;
        return new TextDecoder().decode(mem.subarray(p, end));
    }
}
