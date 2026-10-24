// Runs symjit-wasm modules in Node through js/symjit.mjs, for tests/wasm.rs.
//
// Reads a JSON array of requests from stdin:
//   {"module": path, "host": {name: "JavaScript function source"},
//    "kind": "call" | "evaluate" | "matrix" | "fast",
//    "points": [[...], ...], "params": [...]}
// "call" evaluates a model of states at each point (with "params"), "evaluate" a Symbolica or
// Composer model at each point (its inputs), "matrix" the same with one evaluateMatrix call,
// "fast" calls the `fast` export. Prints, per request, {"obs": [[...], ...], "shape": {...},
// "imports": [...]} or {"error": message}.
//
// Numbers travel exactly: NaN, Infinity, -Infinity and -0 (which JSON cannot represent) as the
// strings "NaN", "Infinity", "-Infinity", "-0".

import { readFileSync } from "node:fs";
import { SymjitModel } from "../js/symjit.mjs";

const decode = (v) => (typeof v === "string" ? (v === "-0" ? -0 : Number(v)) : v);
const encode = (v) => (Number.isFinite(v) && !Object.is(v, -0) ? v : Object.is(v, -0) ? "-0" : String(v));

function run(req) {
    const points = (req.points ?? []).map((p) => p.map(decode));
    const params = (req.params ?? []).map(decode);
    const host = {};
    for (const [name, src] of Object.entries(req.host ?? {})) host[name] = new Function(`return ${src}`)();

    const module = new WebAssembly.Module(readFileSync(req.module));
    const model = SymjitModel.fromModule(module, host);
    let obs;
    switch (req.kind) {
        case "call": obs = points.map((p) => Array.from(model.call(p, params))); break;
        case "evaluate": obs = points.map((p) => Array.from(model.evaluate(p))); break;
        case "matrix": {
            const out = model.evaluateMatrix(new Float64Array(points.flat()), points.length);
            const m = model.countObs;
            obs = points.map((_, i) => Array.from(out.subarray(i * m, (i + 1) * m)));
            break;
        }
        case "fast": obs = points.map((p) => [model.fast(...p)]); break;
        default: throw new Error(`unknown kind ${req.kind}`);
    }
    return {
        obs: obs.map((row) => row.map(encode)),
        shape: model.shape,
        imports: WebAssembly.Module.imports(module).map((i) => i.name),
    };
}

const reqs = JSON.parse(readFileSync(0, "utf8"));
console.log(JSON.stringify(reqs.map((r) => {
    try {
        return run(r);
    } catch (e) {
        return { error: String(e) };
    }
})));
