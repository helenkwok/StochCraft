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
                    Some(Err(err)) => error_answer(index, err.to_json()),
                    None => no_run_answer(index),
                };
                let _ = reply.post_message(&JsValue::from_str(&answer));
            }
        });
    scope.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
    // Lives as long as the worker.
    on_message.forget();
    let _ = scope.post_message(&JsValue::from_str("ready"));
}

/// The answer to replication `index` when the run could not be set up: `error` is the engine's.
#[cfg(any(target_arch = "wasm32", test))]
fn error_answer(index: u64, error: serde_json::Value) -> String {
    serde_json::json!({ "index": index, "error": error }).to_string()
}

/// The answer to a replication asked for before any run was sent.
#[cfg(any(target_arch = "wasm32", test))]
fn no_run_answer(index: u64) -> String {
    let error = serde_json::json!({ "code": "internal", "message": "the worker has no run" });
    error_answer(index, error)
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("stochcraft-worker runs in a browser's web worker: build it with `trunk build`");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(answer: &str) -> serde_json::Value {
        serde_json::from_str(answer).unwrap_or(serde_json::Value::Null)
    }

    #[test]
    fn a_replication_asked_for_before_the_run_is_answered_with_an_error() {
        let answer = parsed(&no_run_answer(7));
        assert_eq!(answer["index"], 7);
        assert_eq!(answer["error"]["code"], "internal");
        assert_eq!(answer["error"]["message"], "the worker has no run");
    }

    #[test]
    fn a_run_that_could_not_be_set_up_answers_every_replication_with_its_error() {
        let error = serde_json::json!({ "code": "invalid_model", "message": "no source" });
        for index in [0, 3] {
            let answer = parsed(&error_answer(index, error.clone()));
            assert_eq!(answer["index"], index);
            assert_eq!(answer["error"], error);
        }
    }
}
