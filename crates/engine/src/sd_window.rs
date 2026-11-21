//! What the window asks of the open system dynamics model besides its commands: whether the
//! document is one, and the name it was opened under. (The model itself is read with `sd.get`,
//! and run with `sd.run`.)

use crate::Engine;

impl Engine {
    /// Whether the document is a system dynamics model: an XMILE file opened with `file.open`
    /// or `sd.open`. Such a document has no process model (the engine holds an empty one), and
    /// `save_bytes` would write that empty model, so a caller that saves must check this first.
    pub fn has_system_dynamics(&self) -> bool {
        self.sd.is_some()
    }

    /// The name the system dynamics model was opened under (its path, for a file).
    pub fn system_dynamics_path(&self) -> Option<&str> {
        self.sd.as_ref().map(|doc| doc.path.as_str())
    }
}
