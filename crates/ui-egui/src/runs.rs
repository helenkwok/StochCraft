//! Runs and their results: starting, cancelling and polling a run, the workers it can use, and the
//! results and baseline the window keeps of the last run.

use std::rc::Rc;

use serde_json::{Value, json};

use crate::i18n::{t, tn};
use crate::{App, Running, Workers, baseline, changes, comparable};

impl App {
    /// Results of the last finished run, if the model has not changed since.
    pub fn results(&self) -> Option<&Value> {
        let results = self.results.as_deref()?;
        let revision = self.engine.revision();
        let key = (revision, self.results_serial);
        let mut cache = self.model_cache.borrow_mut();
        let current = match cache.results_current {
            Some((k, current)) if k == key => current,
            _ => {
                let current = self.results_model.as_ref() == Some(&comparable(self.engine.model()));
                cache.results_current = Some((key, current));
                current
            }
        };
        current.then_some(results)
    }

    /// The results on show are of a model that has changed since their run: the Results view
    /// still shows them, marked out of date.
    pub fn results_out_of_date(&self) -> bool {
        self.results.is_some() && self.results().is_none()
    }

    /// Shows `results` of a run of `model` (as [`comparable`] writes it).
    pub(crate) fn set_results(&mut self, results: Value, model: Value) {
        self.results = Some(Rc::new(results));
        self.results_model = Some(model);
        self.results_stale = false;
        self.results_serial += 1;
        self.show_results = true;
    }

    /// The baseline later runs are compared with, if one is kept.
    pub fn baseline(&self) -> Option<&Value> {
        self.baseline.as_ref().map(|b| &b.results)
    }

    /// "Keep as baseline": the results on show become the baseline.
    pub fn keep_baseline(&mut self) {
        if let (Some(results), Some(model)) = (self.results.as_deref(), &self.results_model) {
            self.baseline = Some(baseline::Baseline {
                results: results.clone(),
                model: model.clone(),
                serial: self.results_serial,
            });
            self.comparison = None;
        }
    }

    /// "Clear baseline".
    pub fn clear_baseline(&mut self) {
        self.baseline = None;
        self.comparison = None;
    }

    /// The baseline, the results on show and their comparison, when there is a baseline and the
    /// results are not it. The comparison is the engine's (`results.compare`), worked out once
    /// for each pair of results and again when the interface language changes, as the list of
    /// changes is in its words.
    pub(crate) fn compared(&mut self) -> Option<(Value, Value, Value, Vec<String>)> {
        let b = self.baseline.as_ref()?;
        let now = self.results.as_deref()?;
        if b.serial == self.results_serial {
            return None;
        }
        let key = (b.serial, self.results_serial, self.lang);
        if self.comparison.as_ref().map(|c| c.0) != Some(key) {
            let wanted = baseline::wanted(&b.results, now);
            let params = if wanted.is_empty() {
                json!({ "a": b.results, "b": now })
            } else {
                json!({ "a": b.results, "b": now, "results": wanted })
            };
            let answer = self
                .engine
                .execute("results.compare", params)
                .unwrap_or(Value::Null);
            let changes = self
                .results_model
                .as_ref()
                .map(|m| changes::changes(&b.model, m))
                .unwrap_or_default();
            self.comparison = Some((key, answer, changes));
        }
        let (_, c, changes) = self.comparison.as_ref()?;
        c.is_object()
            .then(|| (b.results.clone(), now.clone(), c.clone(), changes.clone()))
    }

    /// Forgets the results: another model or file replaces the one they came from.
    pub(crate) fn clear_results(&mut self) {
        self.results = None;
        self.results_model = None;
        self.results_stale = false;
        self.clear_baseline();
    }

    /// Runs go to these workers from now on (the browser's web workers).
    pub fn set_workers(&mut self, workers: Box<dyn Workers>) {
        self.workers = Some(workers);
    }

    /// Model ▸ Run: starts the experiment in the background. A model with errors doesn't run:
    /// its next error is shown instead (warnings alone don't stop it).
    pub fn run(&mut self) {
        if self.is_monte_carlo() {
            self.mc_run();
            return;
        }
        if self.running.is_some() || self.go_to_problem() {
            return;
        }
        let on_workers = self.workers.is_some() && self.start_on_workers();
        let status = if on_workers {
            Some(self.engine.poll_run(0, 0))
        } else {
            self.execute("run.start", json!({}))
        };
        if let Some(status) = status {
            self.show_results = true;
            self.run_model = Some(comparable(self.engine.model()));
            self.running = Some(Running {
                done: 0,
                partial: 0.0,
                total: status["total"].as_u64().unwrap_or(0),
                on_workers,
            });
            self.info(t("Running…"));
        }
    }

    /// Starts the run on the shell's workers; false (with nothing left running) if it could not
    /// start there, so the app runs it itself.
    fn start_on_workers(&mut self) -> bool {
        let Ok(status) = self
            .engine
            .execute("run.start", json!({ "mode": "workers" }))
        else {
            return false;
        };
        let total = status["total"].as_u64().unwrap_or(0);
        let started = match (self.engine.worker_task(), self.workers.as_mut()) {
            (Some(task), Some(workers)) => workers.start(task, total),
            _ => false,
        };
        if !started {
            let _ = self.engine.execute("run.cancel", json!({}));
        }
        started
    }

    /// Stops the run in progress.
    pub fn cancel_run(&mut self) {
        if self.running.is_some() {
            // A Monte Carlo run is the window's own: dropping it stops it.
            self.mc.job = None;
            self.stop_workers();
            let status = self.execute("run.cancel", json!({}));
            self.running = None;
            if status.is_some() {
                self.info(t("Run cancelled"));
            }
        }
    }

    /// Advances and collects the background run. Called every frame; true while it runs.
    pub fn poll(&mut self) -> bool {
        if self.running.is_none() {
            return false;
        }
        if self.mc.job.is_some() {
            return self.mc_poll();
        }
        // Answers from the shell's workers; otherwise, in the browser, a run advances a slice of
        // events per frame (a long replication over several frames), so the page stays
        // responsive and Cancel acts within a frame.
        if self.running.as_ref().is_some_and(|r| r.on_workers)
            && let Some(workers) = self.workers.as_mut()
        {
            let refused = workers
                .answers()
                .iter()
                .find_map(|answer| self.engine.deliver(answer).err());
            // Not expected: the workers run this same engine. The run stops rather than wait for
            // an answer that will not come.
            if let Some(e) = refused {
                self.stop_workers();
                let _ = self.engine.execute("run.cancel", json!({}));
                self.running = None;
                self.report_error(e.message);
                return false;
            }
        }
        let status = self.engine.poll_run(1, stochcraft_engine::SLICE_EVENTS);
        if status["state"] != "running" {
            self.stop_workers();
        }
        match status["state"].as_str() {
            Some("running") => {
                if let Some(r) = &mut self.running {
                    r.done = status["done"].as_u64().unwrap_or(r.done);
                    r.partial = status["partial"].as_f64().unwrap_or(0.0);
                    // A sequential run that went on after a look: more replications, which the
                    // shell's workers are handed too.
                    let total = status["total"].as_u64().unwrap_or(r.total);
                    if total > r.total {
                        r.total = total;
                        if r.on_workers
                            && let Some(workers) = self.workers.as_mut()
                        {
                            workers.extend(total);
                        }
                    }
                }
                true
            }
            Some("finished") => {
                self.running = None;
                // The model as the run started: it may have been edited while it ran.
                let model = self
                    .run_model
                    .take()
                    .unwrap_or_else(|| comparable(self.engine.model()));
                self.set_results(status["result"].clone(), model);
                let n = status["total"].as_u64().unwrap_or(0);
                self.info(tn(n, "Finished {n} run", "Finished {n} runs"));
                false
            }
            _ => {
                self.running = None;
                let message = status["error"]["message"]
                    .as_str()
                    .unwrap_or(t("the run stopped"))
                    .to_owned();
                self.report_error(message);
                self.status_at = self.error_block(&status["error"]["details"]);
                false
            }
        }
    }

    pub(crate) fn stop_workers(&mut self) {
        if self.running.as_ref().is_some_and(|r| r.on_workers)
            && let Some(workers) = self.workers.as_mut()
        {
            workers.stop();
        }
    }

    /// Results go out of date when the model changes (moving a block does not count); they stay
    /// on show, marked, and are fresh again if the change is undone.
    pub(crate) fn mark_stale_results(&mut self) {
        self.results_stale = self.results_out_of_date();
    }
}
