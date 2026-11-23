//! StochCraft's test tooling: the golden models and their pinned results.
//!
//! Each golden model is run through the engine (as the command line and the app run it) with a
//! fixed number of replications, and its results are reduced to a digest. The digests are
//! pinned in `tests/golden/results.txt`; the `golden` test checks them on whatever target it is
//! compiled for, natively and as `wasm32-wasip1` under a WASI runtime (`cargo xtask golden`),
//! so a seeded model gives exactly the same numbers everywhere.
//!
//! When a change is meant to alter results, `STOCHCRAFT_BLESS=1 cargo test -p
//! stochcraft-testkit` rewrites the pinned digests (natively), and the diff shows which models
//! changed.

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use stochcraft_engine::Engine;

/// A golden model: its file name under `tests/models/` and its bytes, built into the test so it
/// needs no file system (a WASI runtime need not grant one), with the files it names beside it
/// (an agent-based model's state charts), by their references.
pub struct Golden {
    pub name: &'static str,
    pub bytes: &'static [u8],
    pub files: &'static [(&'static str, &'static [u8])],
}

pub static GOLDEN: &[Golden] = &[
    Golden {
        name: "mm1.bpmn",
        bytes: include_bytes!("../../../tests/models/mm1.bpmn"),
        files: &[],
    },
    Golden {
        name: "tandem.xpdl",
        bytes: include_bytes!("../../../tests/models/tandem.xpdl"),
        files: &[],
    },
    Golden {
        name: "distributions.bpmn",
        bytes: include_bytes!("../../../tests/models/distributions.bpmn"),
        files: &[],
    },
    // Gateways with conditions, a parallel split and join, entity properties, a shared pool and
    // a finite buffer.
    Golden {
        name: "repair-shop.bpmn",
        bytes: include_bytes!("../../../tests/models/repair-shop.bpmn"),
        files: &[],
    },
    // A task whose two pools serve in times of their own (a service time by resource), with
    // whole-minute times, so units come free together and arrivals meet them.
    Golden {
        name: "able-baker.bpmn",
        bytes: include_bytes!("../../../tests/models/able-baker.bpmn"),
        files: &[],
    },
    // An epidemic among 100 people as agents (ROADMAP M4.3): a population whose SCXML state
    // chart sends messages to random others at a rate, with counts over time.
    Golden {
        name: "sir-agents.bpmn",
        bytes: include_bytes!("../../../tests/models/sir-agents.bpmn"),
        files: &[(
            "sir-person.scxml",
            include_bytes!("../../../tests/models/sir-person.scxml"),
        )],
    },
];

/// Replications per golden run: enough to exercise the experiment, few enough to be quick on a
/// slow runtime.
pub const REPLICATIONS: u32 = 4;

/// The pinned digests, as `tests/golden/results.txt` holds them.
pub const PINNED: &str = include_str!("../../../tests/golden/results.txt");

/// Runs a golden model with its own scenario (seed, warm-up, duration) and
/// [`REPLICATIONS`] replications, and returns the results: `run`'s for a process model,
/// `agents.run`'s for an agent-based one.
pub fn run(g: &Golden) -> Result<Value, String> {
    let mut engine = Engine::new();
    let opened = engine
        .open_bytes_with(g.name, g.bytes, None, &mut |href: &str| {
            g.files
                .iter()
                .find(|(name, _)| *name == href)
                .map(|(_, bytes)| bytes.to_vec())
                .ok_or_else(|| format!("`{href}` is not among the golden model's files"))
        })
        .map_err(|e| format!("{}: {e}", g.name))?;
    let command = if opened["kind"] == "agents" {
        "agents.run"
    } else {
        "run"
    };
    engine
        .execute(command, json!({ "replications": REPLICATIONS }))
        .map_err(|e| format!("{}: {e}", g.name))
}

/// The digest of a run's results: every element's statistics, exactly (JSON prints each number
/// so that it reads back to the same bits), or for an agent-based model the times, every
/// statistic over time and every population's counts. The run record is left out: it names the
/// StochCraft version, which changes without the results changing.
pub fn digest(results: &Value) -> String {
    let part = match results.get("elements") {
        Some(elements) => elements.clone(),
        None => json!({
            "times": results["times"],
            "statistics": results["statistics"],
            "populations": results["populations"],
        }),
    };
    let text = serde_json::to_string(&part).unwrap_or_default();
    Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Every golden model's digest, as lines of `name digest`, sorted by name.
pub fn digests() -> Result<String, String> {
    let mut lines = Vec::new();
    for g in GOLDEN {
        lines.push(format!("{} {}", g.name, digest(&run(g)?)));
    }
    lines.sort();
    Ok(lines.join("\n") + "\n")
}

/// The pinned digests without comments and blank lines.
pub fn pinned() -> String {
    let lines: Vec<&str> = PINNED
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    lines.join("\n") + "\n"
}
