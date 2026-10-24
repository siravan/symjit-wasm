// Runs a wasm32-wasip1 program (e.g. the crate's unit tests:
// `cargo test --lib --target wasm32-wasip1 --no-run`) in Node:
//     node tests/wasi_run.mjs PROGRAM.wasm [ARGS...]
// exits with the program's status.

import { readFileSync } from "node:fs";
import { WASI } from "node:wasi";

const [program, ...args] = process.argv.slice(2);
const wasi = new WASI({ version: "preview1", args: [program, ...args], env: {}, returnOnExit: true });
const module = await WebAssembly.compile(readFileSync(program));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exitCode = wasi.start(instance);
