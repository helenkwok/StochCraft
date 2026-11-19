//! The page's web workers for runs, one per core but one (the page keeps that one): each loads
//! `stochcraft-worker` (src/worker.rs), gets the run, then one replication at a time, and is
//! given the next as soon as it answers, so faster workers take more.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_ui_egui::Workers;
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::schedule::{Shared, worker_count};

/// What trunk names the worker's loader (index.html's `data-loader-shim`), next to the page.
const LOADER: &str = "./stochcraft-worker_loader.js";

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
            .unwrap_or(1.0);
        worker_count(cores, replications)
    }

    fn spawn(&self) -> Result<Running, JsValue> {
        let worker = web_sys::Worker::new(LOADER)?;
        let (shared, ctx, me) = (Rc::clone(&self.shared), self.ctx.clone(), worker.clone());
        let on_message =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |e: web_sys::MessageEvent| {
                let Some(text) = e.data().as_string() else {
                    return;
                };
                let reaction = shared.borrow_mut().on_message(text);
                if let Some(task) = reaction.task {
                    let _ = me.post_message(&JsValue::from_str(&task));
                }
                if reaction.repaint {
                    ctx.request_repaint();
                }
                // The next replication, if any is left.
                if let Some(index) = reaction.index {
                    let _ = me.post_message(&JsValue::from_f64(index as f64));
                }
            });
        let (shared, ctx) = (Rc::clone(&self.shared), self.ctx.clone());
        let on_error =
            Closure::<dyn FnMut(web_sys::ErrorEvent)>::new(move |e: web_sys::ErrorEvent| {
                // A worker that failed to load or crashed.
                shared.borrow_mut().worker_failed(&e.message());
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
        *self.shared.borrow_mut() = Shared::new(task, replications);
        for _ in 0..Self::count(replications) {
            match self.spawn() {
                Ok(r) => self.running.push(r),
                Err(_) => break,
            }
        }
        !self.running.is_empty()
    }

    fn answers(&mut self) -> Vec<String> {
        self.shared.borrow_mut().take_answers()
    }

    fn stop(&mut self) {
        for r in self.running.drain(..) {
            r.worker.set_onmessage(None);
            r.worker.set_onerror(None);
            r.worker.terminate();
        }
        self.shared.borrow_mut().stop();
    }
}
