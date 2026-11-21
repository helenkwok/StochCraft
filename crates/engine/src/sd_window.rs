//! What the window asks of the open system dynamics model besides its commands: whether the
//! document is one, the name it was opened under, and whether its last results are out of date.
//! (The model itself is read with `sd.get`, edited with the `sd.*` commands and run with
//! `sd.run`.)

use crate::Engine;

impl Engine {
    /// Whether the document is a system dynamics model: an XMILE file opened with `file.open`
    /// or `sd.open`. Such a document has no process model (the engine holds an empty one), and
    /// `save_bytes` writes the XMILE model rather than that empty one.
    pub fn has_system_dynamics(&self) -> bool {
        self.sd.is_some()
    }

    /// The name the system dynamics model was opened under (its path, for a file).
    pub fn system_dynamics_path(&self) -> Option<&str> {
        self.sd.as_ref().map(|doc| doc.path.as_str())
    }

    /// Whether the system dynamics model has results from a run and has been changed since (what
    /// `sd.get` gives as `results_stale`), without describing the model again.
    pub fn system_dynamics_results_stale(&self) -> bool {
        self.sd
            .as_ref()
            .is_some_and(|doc| doc.last.is_some() && doc.last_stale)
    }
}
