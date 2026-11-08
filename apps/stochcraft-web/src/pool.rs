//! The page's web workers for runs, one per core but one (the page keeps that one): each loads
//! `stochcraft-worker` (src/worker.rs), gets the run, then one replication at a time, and is
//! given the next as soon as it answers, so faster workers take more.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_ui_egui::Workers;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

/// What trunk names the worker's loader (index.html's `data-loader-shim`), next to the page.
const LOADER: &str = "./stochcraft-worker_loader.js";
/// No more workers than this, however many cores: a run rarely has more replications.
const MAX_WORKERS: usize = 16;

/// What the workers' handlers share with the pool.
#[derive(Default)]
struct Shared {
    task: String,
    next: u64,
    total: u64,
    answers: Vec<String>,
}

struct Running {
    worker: web_sys::Worker,
    // Kept alive as long as the worker.
    _on_message: Closure<dyn FnMut(web_sys::MessageEvent)>,
    _on_error: Closure<dyn FnMut(web_sys::ErrorEvent)>,
}

pub struct Pool {
    shared: Rc<RefCell<Shared>>,
    running: Vec<Running>,
    ctx: egui::Context,
}

impl Pool {
    pub fn new(ctx: egui::Context) -> Self {
        Self {
            shared: Rc::default(),
            running: Vec::new(),
            ctx,
        }
    }

    /// How many workers a run of `replications` gets.
    fn count(replications: u64) -> usize {
        let cores = web_sys::window()
            .map(|w| w.navigator().hardware_concurrency())
            .unwrap_or(1.0) as usize;
        let wanted = cores.saturating_sub(1).clamp(1, MAX_WORKERS);
        wanted.min(usize::try_from(replications).unwrap_or(usize::MAX))
    }

    fn spawn(&self) -> Result<Running, JsValue> {
        let worker = web_sys::Worker::new(LOADER)?;
        let (shared, ctx, me) = (Rc::clone(&self.shared), self.ctx.clone(), worker.clone());
        let on_message =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                let Some(text) = e.data().as_string() else {
                    return;
                };
                let mut s = shared.borrow_mut();
                if text == "ready" {
                    let _ = me.post_message(&JsValue::from_str(&s.task));
                } else {
                    s.answers.push(text);
                    ctx.request_repaint();
                }
                // The next replication, if any is left.
                if s.next < s.total {
                    let _ = me.post_message(&JsValue::from_f64(s.next as f64));
                    s.next += 1;
                }
            });
        let (shared, ctx) = (Rc::clone(&self.shared), self.ctx.clone());
        let on_error =
            Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |e: web_sys::ErrorEvent| {
                // A worker that failed to load or crashed: the run fails with what it said,
                // rather than wait for answers that will not come.
                let error = serde_json::json!({
                    "code": "internal",
                    "message": format!("a web worker stopped: {}", e.message()),
                });
                let answer = serde_json::json!({ "index": 0, "error": error });
                shared.borrow_mut().answers.push(answer.to_string());
                ctx.request_repaint();
            });
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(Running {
            worker,
            _on_message: on_message,
            _on_error: on_error,
        })
    }
}

impl Workers for Pool {
    fn start(&mut self, task: &str, replications: u64) -> bool {
        self.stop();
        *self.shared.borrow_mut() = Shared {
            task: task.to_owned(),
            next: 0,
            total: replications,
            answers: Vec::new(),
        };
        for _ in 0..Self::count(replications) {
            match self.spawn() {
                Ok(r) => self.running.push(r),
                Err(_) => break,
            }
        }
        !self.running.is_empty()
    }

    fn answers(&mut self) -> Vec<String> {
        std::mem::take(&mut self.shared.borrow_mut().answers)
    }

    fn stop(&mut self) {
        for r in self.running.drain(..) {
            r.worker.set_onmessage(None);
            r.worker.set_onerror(None);
            r.worker.terminate();
        }
        let mut s = self.shared.borrow_mut();
        s.next = s.total;
        s.answers.clear();
    }
}
