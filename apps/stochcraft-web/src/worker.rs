//! A web worker that runs replications for the page (`"mode": "workers"` in the engine): the page
//! sends it the run (text from `Engine::worker_task`) and then one replication index at a time,
//! and it answers each with the replication's report (`stochcraft_engine::Worker::run`).
//!
//! It says `ready` once it can take messages, since the page's messages sent while its
//! WebAssembly was still loading would otherwise be lost.
//!
//! On other targets this binary only says so, which keeps `cargo build --workspace` working.

#[cfg(target_arch = "wasm32")]
fn main() {
    use std::cell::RefCell;

    use stochcraft_engine::{CommandError, Worker};
    use wasm_bindgen::JsCast as _;
    use wasm_bindgen::prelude::*;

    let scope: web_sys::DedicatedWorkerGlobalScope = js_sys::global().unchecked_into();
    let worker: RefCell<Option<Result<Worker, CommandError>>> = RefCell::new(None);
    let reply = scope.clone();
    let on_message =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
            let data = e.data();
            if let Some(task) = data.as_string() {
                *worker.borrow_mut() = Some(Worker::new(&task));
            } else if let Some(index) = data.as_f64() {
                let index = index as u64;
                let answer = match worker.borrow().as_ref() {
                    Some(Ok(w)) => w.run(index),
                    Some(Err(err)) => {
                        serde_json::json!({ "index": index, "error": err.to_json() }).to_string()
                    }
                    None => serde_json::json!({
                        "index": index,
                        "error": { "code": "internal", "message": "the worker has no run" },
                    })
                    .to_string(),
                };
                let _ = reply.post_message(&JsValue::from_str(&answer));
            }
        });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // Lives as long as the worker.
    on_message.forget();
    let _ = scope.post_message(&JsValue::from_str("ready"));
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("stochcraft-worker runs in a browser's web worker: build it with `trunk build`");
}
