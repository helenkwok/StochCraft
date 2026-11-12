// Runs a wasm32-wasip1 program (a test binary built by cargo) under Node's built-in WASI, so
// golden results can be checked on wasm without installing a separate runtime. Used as cargo's
// runner by `cargo xtask golden`. Usage: node wasi-run.mjs program.wasm [args...]
import { readFile } from "node:fs/promises";
import { argv, env, exit, versions } from "node:process";
import { setFlagsFromString } from "node:v8";
import { WASI } from "node:wasi";

// Node 22 crashes (SIGSEGV) when a WASI write triggers a garbage collection inside V8's fast API
// call for it (#102, nodejs/node#51303). Without fast API calls it runs correctly; set before any
// code is compiled. Node 24 and later have not crashed.
if (Number(versions.node.split(".")[0]) < 24) setFlagsFromString("--no-turbo-fast-api-calls");

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
