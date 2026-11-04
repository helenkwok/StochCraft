// Runs a wasm32-wasip1 program (a test binary built by cargo) under Node's built-in WASI, so
// golden results can be checked on wasm without installing a separate runtime. Used as cargo's
// runner by `cargo xtask golden`. Usage: node wasi-run.mjs program.wasm [args...]
import { readFile } from "node:fs/promises";
import { argv, env, exit } from "node:process";
import { WASI } from "node:wasi";

const [program, ...args] = argv.slice(2);
const wasi = new WASI({
  version: "preview1",
  args: [program, ...args],
  env,
  returnOnExit: true,
});
const module = await WebAssembly.compile(await readFile(program));
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
exit(wasi.start(instance));
