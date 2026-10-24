// For tests/wasm.rs (`compiler_runs_in_webassembly`): the compiler compiled to WebAssembly
// (argv[2], symjit_wasm.wasm) compiles models in Node, and their modules run.
//
// Reads a JSON array from stdin: {"name", "text", "opt", "points"} (a Symbolica model, three
// inputs) or {"name", "json", "opt", "points", "functions", "host"} (a SymPy model, its
// user-defined functions declared to the compiler and supplied by the host, as in
// tests/runner.mjs). Prints {"version": ..., "results": [{"name", "obs": [[...], ...]}, ...]}
// (numbers as in tests/runner.mjs).

import { readFileSync } from "node:fs";
import { SymjitCompiler } from "../js/compiler.mjs";
import { SymjitModel } from "../js/symjit.mjs";

const encode = (v) => (Number.isFinite(v) && !Object.is(v, -0) ? v : Object.is(v, -0) ? "-0" : String(v));
const decode = (v) => (typeof v === "string" ? (v === "-0" ? -0 : Number(v)) : v);

const compiler = await SymjitCompiler.load(readFileSync(process.argv[2]));
const results = [];
for (const req of JSON.parse(readFileSync(0, "utf8"))) {
    const host = {};
    for (const [name, src] of Object.entries(req.host ?? {})) host[name] = new Function(`return ${src}`)();
    const sympy = req.json !== undefined;
    const bytes = sympy
        ? compiler.compile(req.json, req.opt, req.functions ?? {})
        : compiler.translate(req.text, 3, req.opt);
    const model = await SymjitModel.compile(bytes, host);
    const obs = req.points.map((p) => {
        const x = p.map(decode);
        return Array.from(sympy ? model.call(x) : model.evaluate(x)).map(encode);
    });
    results.push({ name: req.name, obs });
}
console.log(JSON.stringify({ version: compiler.version, results }));
