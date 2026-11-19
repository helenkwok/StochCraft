//! The parts of the worker pool (src/pool.rs) and of the downloads (src/web.rs) that need no
//! browser: how many workers a run gets, which message a worker is sent next, and the answer
//! standing in for a worker that stopped. Kept apart so they can be tested natively.

/// No more workers than this, however many cores: a run rarely has more replications.
pub const MAX_WORKERS: usize = 16;

/// How many workers a run of `replications` gets on a machine with `cores` logical cores (the
/// page keeps one): at least one, at most [`MAX_WORKERS`], and no more than there are replications.
pub fn worker_count(cores: f64, replications: u64) -> usize {
    let wanted = (cores as usize).saturating_sub(1).clamp(1, MAX_WORKERS);
    wanted.min(usize::try_from(replications).unwrap_or(usize::MAX))
}

/// The media type a download named `name` is offered as, from its extension.
pub fn download_mime(name: &str) -> &'static str {
    match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("csv") => "text/csv",
        _ => "application/xml",
    }
}

/// What the workers' handlers share with the pool.
#[derive(Default)]
pub struct Shared {
    task: String,
    next: u64,
    total: u64,
    answers: Vec<String>,
}

/// What a worker's message makes the pool do.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reaction {
    /// Send the worker the run (it said `ready`).
    pub task: Option<String>,
    /// Send the worker this replication's index.
    pub index: Option<u64>,
    /// An answer arrived: the window has something new to show.
    pub repaint: bool,
}

impl Shared {
    /// A run of `total` replications of `task`.
    pub fn new(task: &str, total: u64) -> Self {
        Self {
            task: task.to_owned(),
            next: 0,
            total,
            answers: Vec::new(),
        }
    }

    /// A worker said `ready` or answered a replication with `text`; it is given the next
    /// replication, if any is left.
    pub fn on_message(&mut self, text: String) -> Reaction {
        let mut reaction = Reaction::default();
        if text == "ready" {
            reaction.task = Some(self.task.clone());
        } else {
            self.answers.push(text);
            reaction.repaint = true;
        }
        if self.next < self.total {
            reaction.index = Some(self.next);
            self.next += 1;
        }
        reaction
    }

    /// A worker failed to load or crashed: the run fails with what it said, rather than wait
    /// for answers that will not come.
    pub fn worker_failed(&mut self, message: &str) {
        let error = serde_json::json!({
            "code": "internal",
            "message": format!("a web worker stopped: {message}"),
        });
        let answer = serde_json::json!({ "index": 0, "error": error });
        self.answers.push(answer.to_string());
    }

    /// The answers that came in since the last call.
    pub fn take_answers(&mut self) -> Vec<String> {
        std::mem::take(&mut self.answers)
    }

    /// The run is stopped: no more replications are handed out, and answers still to come are
    /// dropped.
    pub fn stop(&mut self) {
        self.next = self.total;
        self.answers.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_gets_a_worker_per_core_but_one() {
        assert_eq!(worker_count(8.0, 100), 7);
        assert_eq!(worker_count(2.0, 100), 1);
        // One core, an unknown count (the page asks for none), or nonsense: still one worker.
        assert_eq!(worker_count(1.0, 100), 1);
        assert_eq!(worker_count(0.0, 100), 1);
        assert_eq!(worker_count(f64::NAN, 100), 1);
        assert_eq!(worker_count(-3.0, 100), 1);
    }

    #[test]
    fn a_run_never_gets_more_workers_than_the_cap_or_than_replications() {
        assert_eq!(worker_count(128.0, 1000), MAX_WORKERS);
        assert_eq!(worker_count(128.0, 3), 3);
        assert_eq!(worker_count(8.0, 1), 1);
        assert_eq!(worker_count(8.0, u64::MAX), 7);
        // No replications, no workers.
        assert_eq!(worker_count(8.0, 0), 0);
    }

    #[test]
    fn a_download_is_offered_as_what_its_extension_says() {
        assert_eq!(download_mime("mm1-charts.png"), "image/png");
        assert_eq!(download_mime("mm1-results.csv"), "text/csv");
        assert_eq!(download_mime("mm1.bpmn"), "application/xml");
        assert_eq!(download_mime("mm1.xpdl"), "application/xml");
        assert_eq!(download_mime("no-extension"), "application/xml");
        assert_eq!(download_mime(""), "application/xml");
        // Only the last extension counts.
        assert_eq!(download_mime("a.png.bpmn"), "application/xml");
    }

    #[test]
    fn a_worker_that_is_ready_gets_the_run_and_a_first_replication() {
        let mut s = Shared::new("the run", 3);
        assert_eq!(
            s.on_message("ready".into()),
            Reaction {
                task: Some("the run".into()),
                index: Some(0),
                repaint: false
            }
        );
        // A second worker, ready at once, gets the next.
        assert_eq!(s.on_message("ready".into()).index, Some(1));
        assert!(s.take_answers().is_empty());
    }

    #[test]
    fn an_answer_is_kept_and_the_next_replication_handed_out_until_none_are_left() {
        let mut s = Shared::new("run", 2);
        assert_eq!(s.on_message("ready".into()).index, Some(0));
        let r = s.on_message("answer 0".into());
        assert_eq!(
            r,
            Reaction {
                task: None,
                index: Some(1),
                repaint: true
            }
        );
        // All handed out: answers still come in, and nothing more is sent.
        let r = s.on_message("answer 1".into());
        assert_eq!(
            r,
            Reaction {
                task: None,
                index: None,
                repaint: true
            }
        );
        assert_eq!(s.take_answers(), vec!["answer 0", "answer 1"]);
        assert!(s.take_answers().is_empty(), "taken once");
    }

    #[test]
    fn a_stopped_run_hands_out_nothing_and_drops_late_answers() {
        let mut s = Shared::new("run", 10);
        s.on_message("ready".into());
        s.on_message("answer 0".into());
        s.stop();
        assert!(s.take_answers().is_empty());
        let r = s.on_message("late answer".into());
        assert_eq!(r.index, None);
        // A new run starts afresh.
        let mut s = Shared::new("again", 1);
        assert_eq!(s.on_message("ready".into()).index, Some(0));
    }

    #[test]
    fn a_worker_that_stopped_fails_the_run_with_what_it_said() {
        let mut s = Shared::default();
        s.worker_failed("Uncaught RuntimeError: unreachable");
        let answers = s.take_answers();
        assert_eq!(answers.len(), 1);
        let answer: serde_json::Value =
            serde_json::from_str(&answers[0]).unwrap_or(serde_json::Value::Null);
        assert_eq!(answer["index"], 0);
        assert_eq!(answer["error"]["code"], "internal");
        assert_eq!(
            answer["error"]["message"],
            "a web worker stopped: Uncaught RuntimeError: unreachable"
        );
    }
}
