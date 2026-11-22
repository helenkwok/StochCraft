//! StochCraft's interface on egui, shared by the desktop app (`apps/stochcraft`) and the browser
//! app (`apps/stochcraft-web`).
//!
//! [`App`] draws the window: menus, the model outline, the results of the last run and a status
//! bar. Every change it makes to the model goes through the engine's commands (ROADMAP principle
//! 2), so undo, redo and the journal work as they do from the command line.
//!
//! What differs between platforms stays in the shells. The app never touches the file system or
//! shows a native dialog: it hands the shell a [`Request`] (pick a file to open, pick where to
//! save, write these bytes, offer them as a download) through [`App::take_requests`], and the shell
//! answers by calling [`App::open_file`], [`App::save_to`], [`App::written`],
//! [`App::save_failed`] or [`App::report_error`].

mod autosave;
mod baseline;
mod calendars;
mod canvas;
mod capacity;
mod changes;
mod chart_editor;
pub mod chart_palette;
mod charts;
mod chrome;
pub mod commands;
mod dashboard;
mod decide;
mod documents;
pub use autosave::Autosave;
pub use documents::OpenTicket;
mod editor;
pub mod form;
mod guide;
mod help;
pub mod i18n;
mod layout_view;
mod live;
mod montecarlo;
mod pools;
mod prefs;
mod results;
mod runs;
mod sd;
mod start;
mod tabs;
pub mod theme;
mod theme_editor;
mod view3d;
mod views;

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::Value;
use stochcraft_engine::Engine;

use crate::i18n::{t, tf};

/// The models offered on the start page and under File ▸ Open Example, built in so the browser
/// app has something to open: the golden models of `stochcraft-testkit` and the published
/// benchmarks whose answers the engine's tests check.
pub const EXAMPLES: &[Example] = &[
    Example {
        title: "M/M/1 queue",
        about: "One clerk, random arrivals: the textbook queue, checked against theory.",
        file: "mm1.bpmn",
        bytes: include_bytes!("../../../tests/models/mm1.bpmn"),
        card: start::Card {
            question: "How long do customers wait at one busy clerk?",
            industry: start::Industry::Services,
            topics: &[start::Topic::Staffing],
            teaches: "Why a clerk busy three quarters of the time already means a queue.",
            minutes: 2,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Tandem line",
        about: "Two stations in series, read from an XPDL file.",
        file: "tandem.xpdl",
        bytes: include_bytes!("../../../tests/models/tandem.xpdl"),
        card: start::Card {
            question: "How do two stations in a row share the waiting?",
            industry: start::Industry::Reference,
            topics: &[],
            teaches: "Reading a process diagram from another file format.",
            minutes: 2,
            proof: None,
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Every distribution",
        about: "One task per BPSim distribution, to compare their spread.",
        file: "distributions.bpmn",
        bytes: include_bytes!("../../../tests/models/distributions.bpmn"),
        card: start::Card {
            question: "How do the time distributions differ?",
            industry: start::Industry::Reference,
            topics: &[],
            teaches: "One task for each distribution, to compare their spread.",
            minutes: 3,
            proof: None,
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Task,
                start::Glyph::Task,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Same mean, different spread",
        about: "Three desks with the same mean service time of 1 minute: fixed, random and very uneven.",
        file: "spread.bpmn",
        bytes: include_bytes!("../../../tests/models/spread.bpmn"),
        card: start::Card {
            question: "Does it matter how much service times vary when the average is the same?",
            industry: start::Industry::Reference,
            topics: &[start::Topic::Change],
            teaches: "Variability makes queues: the same average time gives waits of 2, 4 and 10 minutes.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Four-machine line with weekend breaks (Schmitt et al. 2026, Case A)",
        about: "Four machines in series with failures, small buffers, weekend breaks and power per machine.",
        file: "case-a.bpmn",
        bytes: include_bytes!("../../../tests/models/case-a.bpmn"),
        card: start::Card {
            question: "What do failures and weekend breaks do to a line's output and energy per part?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck, start::Topic::Breakdown],
            teaches: "Failures, small buffers, shifts and energy per machine state.",
            minutes: 8,
            proof: Some(start::Proof::Published),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Seven-machine line with two presses (Schmitt et al. 2026, Case B)",
        about: "Two presses in parallel fed by the shorter queue, rejects at quality, weekend breaks.",
        file: "case-b.bpmn",
        bytes: include_bytes!("../../../tests/models/case-b.bpmn"),
        card: start::Card {
            question: "Which machine holds back a line with two presses in parallel?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck],
            teaches: "Parallel machines fed by the shorter queue, and parts rejected at quality.",
            minutes: 8,
            proof: Some(start::Proof::Published),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Decision,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Decision,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Coffee counter",
        about: "One barista busy 80% of the time, and what a second would do.",
        file: "coffee.bpmn",
        bytes: include_bytes!("../../../tests/models/coffee.bpmn"),
        card: start::Card {
            question: "One barista or two at the morning rush?",
            industry: start::Industry::Services,
            topics: &[start::Topic::Staffing, start::Topic::Change],
            teaches: "Why a barista busy 80% of the time already means a four-minute wait.",
            minutes: 2,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Bank with an ATM and cashiers",
        about: "Half the customers use the ATM first; five cashiers share one queue (Borshchev and Grigoryev, The Big Book of Simulation Modeling, Example 2.2).",
        file: "bank.bpmn",
        bytes: include_bytes!("../../../tests/models/bank.bpmn"),
        card: start::Card {
            question: "Are five cashiers enough when half the customers use the ATM first?",
            industry: start::Industry::Services,
            topics: &[start::Topic::Staffing, start::Topic::Bottleneck],
            teaches: "Routing by chance and a shared queue: cashiers busy 91% of the time, and what a sixth would do.",
            minutes: 4,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Decision,
                start::Glyph::Resource,
                start::Glyph::Decision,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Polling place",
        about: "Voters arrive all day and wait for a free voting machine, with a long or a short ballot (Allen, Introduction to Discrete Event Simulation and Agent-based Modeling, ch. 6 and 7).",
        file: "polling.bpmn",
        bytes: include_bytes!("../../../tests/models/polling.bpmn"),
        card: start::Card {
            question: "How many voting machines does a polling place need?",
            industry: start::Industry::PublicServices,
            topics: &[start::Topic::Staffing],
            teaches: "Machines in proportion to voters and ballot length: 16 instead of 18 makes the wait six times longer. The election-day runs show the direction only.",
            minutes: 4,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Emergency triage",
        about: "Three priority classes sharing five nurses.",
        file: "triage.bpmn",
        bytes: include_bytes!("../../../tests/models/triage.bpmn"),
        card: start::Card {
            question: "Who waits how long when urgent patients go first?",
            industry: start::Industry::Healthcare,
            topics: &[start::Topic::Staffing],
            teaches: "Priorities on a shared team: the urgent wait least, the minor wait longest.",
            minutes: 5,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Decision,
                start::Glyph::Shared,
                start::Glyph::Shared,
                start::Glyph::Shared,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Hospital beds",
        about: "Patients who find every bed taken are turned away; stays vary widely around five days.",
        file: "beds.bpmn",
        bytes: include_bytes!("../../../tests/models/beds.bpmn"),
        card: start::Card {
            question: "How many beds so that fewer than 1 in 50 patients are turned away?",
            industry: start::Industry::Healthcare,
            topics: &[start::Topic::Staffing],
            teaches: "Beds turn patients away rather than queue them, and only the average stay matters.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Mass vaccination hub",
        about: "Entrance, registration, assessment and vaccination, with each dose prepared alongside, then about 20 minutes in an observation seat (Hanly et al., BMC Health Services Research, 2022).",
        file: "vaccination-hub.bpmn",
        bytes: include_bytes!("../../../tests/models/vaccination-hub.bpmn"),
        card: start::Card {
            question: "How many doses a day can a vaccination hub give?",
            industry: start::Industry::Healthcare,
            topics: &[start::Topic::Staffing, start::Topic::Bottleneck],
            teaches: "Each station's staff are as busy as the flow of patients says: all under 80%, and about 470 doses in an 8-hour day with 21 staff. The time inside is not checked.",
            minutes: 4,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Split,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Split,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Call centre",
        about: "Three agents and callers who hang up when they wait too long.",
        file: "call-centre.bpmn",
        bytes: include_bytes!("../../../tests/models/call-centre.bpmn"),
        card: start::Card {
            question: "How many callers hang up before an agent answers?",
            industry: start::Industry::ContactCentre,
            topics: &[start::Topic::Staffing],
            teaches: "Callers who give up: the share lost and the wait of those who stay.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Weld and paint line",
        about: "Two stations with small buffers, paint breaking down now and then.",
        file: "weld-paint.bpmn",
        bytes: include_bytes!("../../../tests/models/weld-paint.bpmn"),
        card: start::Card {
            question: "Where is the bottleneck, and what do breakdowns at paint cost?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck, start::Topic::Breakdown],
            teaches: "Blocking, small buffers and breakdowns: parts turned away at the start of the line.",
            minutes: 4,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Machine repair crew",
        about: "Six machines that break down now and then, and the fitters who repair them.",
        file: "machine-repair.bpmn",
        bytes: include_bytes!("../../../tests/models/machine-repair.bpmn"),
        card: start::Card {
            question: "How many fitters keep six machines running?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Staffing, start::Topic::Breakdown],
            teaches: "A closed loop: a machine waiting for repair cannot break down again, so the queue stays short.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Task,
                start::Glyph::Resource,
            ],
            featured: true,
        },
    },
    Example {
        title: "Inspection with rework",
        about: "Half the parts fail inspection, are reworked and are inspected again.",
        file: "rework.bpmn",
        bytes: include_bytes!("../../../tests/models/rework.bpmn"),
        card: start::Card {
            question: "How much longer do parts take when half of them need rework?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck, start::Topic::Change],
            teaches: "A loop back: rework doubles the load on inspection, and less rework cuts the time far more than in proportion.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Decision,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Penny Fab",
        about: "Four machines in a line, 2 hours a job each, with a fixed number of jobs going round (Hopp and Spearman, Factory Physics).",
        file: "penny-fab.bpmn",
        bytes: include_bytes!("../../../tests/models/penny-fab.bpmn"),
        card: start::Card {
            question: "How much work in process does a line need?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck],
            teaches: "Little's law and the critical work in process: 4 jobs give 0.29 jobs an hour with random times and 0.5 with fixed ones.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
                start::Glyph::Resource,
            ],
            featured: false,
        },
    },
    Example {
        title: "Web service with health checks",
        about: "Three app servers, taken out for a restart when too many requests wait.",
        file: "it-service.bpmn",
        bytes: include_bytes!("../../../tests/models/it-service.bpmn"),
        card: start::Card {
            question: "Do health checks that restart busy servers make the queue worse?",
            industry: start::Industry::ItOperations,
            topics: &[start::Topic::Breakdown, start::Topic::Change],
            teaches: "Health checks that take busy servers out send their requests back to the queue.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: true,
        },
    },
    Example {
        title: "Project finish date",
        about: "Three activities in parallel, each ten days on average; the project ends when all three are done.",
        file: "project.bpmn",
        bytes: include_bytes!("../../../tests/models/project.bpmn"),
        card: start::Card {
            question: "When will the project finish?",
            industry: start::Industry::Projects,
            topics: &[],
            teaches: "Why three ten-day activities in parallel take more than 18 days on average.",
            minutes: 3,
            proof: Some(start::Proof::Probability),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Split,
                start::Glyph::Task,
                start::Glyph::Split,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Prison population",
        about: "Petty and serious offenders serve their sentences, and some offend again and come back; the numbers scaled down 100 times (Brailsford, Churilov and Dangerfield, ch. 8).",
        file: "prison.bpmn",
        bytes: include_bytes!("../../../tests/models/prison.bpmn"),
        card: start::Card {
            question: "How many prison places are needed?",
            industry: start::Industry::PublicServices,
            topics: &[start::Topic::Change],
            teaches: "Places are arrivals times the length of stay, whatever the sentences: 500 petty and 260 serious offenders inside, and what fewer reoffending would do.",
            minutes: 3,
            proof: Some(start::Proof::Theory),
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Task,
                start::Glyph::Decision,
                start::Glyph::Task,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Parcel sorter",
        about: "A parcel sorter: a loop, four lanes by product, diverts that send parcels round when a lane is full",
        file: "sorter.bpmn",
        bytes: include_bytes!("../../../tests/models/sorter.bpmn"),
        card: start::Card {
            question: "How many parcels go round the loop because their lane is full?",
            industry: start::Industry::Manufacturing,
            topics: &[start::Topic::Bottleneck],
            teaches: "Conveyors and diverts: parcels sent round the loop when their lane is full.",
            minutes: 4,
            proof: None,
            flow: &[
                start::Glyph::Arrivals,
                start::Glyph::Decision,
                start::Glyph::Resource,
                start::Glyph::Departures,
            ],
            featured: false,
        },
    },
    Example {
        title: "Product adoption",
        about: "A new product spreads by word of mouth: 10,000 potential customers, some buying on their own and the rest after meeting a customer.",
        file: "adoption.xmile",
        bytes: include_bytes!("../../../tests/models/adoption.xmile"),
        card: start::Card {
            question: "How fast does a new product spread?",
            industry: start::Industry::Services,
            topics: &[start::Topic::Change],
            teaches: "Stocks and flows: word of mouth makes sales speed up and then slow down in an S-shaped curve, which matches its exact formula.",
            minutes: 3,
            proof: Some(start::Proof::Formula),
            // Stocks and flows, not a process: no blocks to show.
            flow: &[],
            featured: false,
        },
    },
    Example {
        title: "Project budget",
        about: "A Monte Carlo model: uncertain durations, day rate and materials, and the chance the cost or the duration runs over.",
        file: "budget.bpmn",
        bytes: include_bytes!("../../../tests/models/budget.bpmn"),
        card: start::Card {
            question: "Will the project come in under budget?",
            industry: start::Industry::Projects,
            topics: &[],
            teaches: "Uncertain inputs instead of single guesses: the chance of an overrun, not one total.",
            minutes: 3,
            proof: Some(start::Proof::Probability),
            // A calculation, not a process: no blocks to show.
            flow: &[],
            featured: true,
        },
    },
];

/// A built-in example model.
pub struct Example {
    pub title: &'static str,
    /// One line on what the example shows.
    pub about: &'static str,
    pub file: &'static str,
    pub bytes: &'static [u8],
    /// Its card on the start page.
    pub card: start::Card,
}

/// Something only the shell can do, asked for by the app.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Ask for a model file; answer with [`App::open_file`] (or nothing, if the user cancels).
    PickOpen,
    /// Read the file at `path` (File ▸ Open Recent); answer with [`App::open_file`], or
    /// [`App::report_error`] if it can't be read.
    OpenPath { path: String },
    /// Ask where to save, suggesting `name`; answer with [`App::save_to`], with
    /// [`App::save_cancelled`] if the user closes the dialog, or with [`App::save_failed`] if it
    /// can't be shown. BPMN is offered, and with `xpdl` XPDL too: the model came from an XPDL
    /// file, which it can be saved into. A `name` ending in `.xmile` is a system dynamics model
    /// (offer XMILE only). The app's menus and shortcuts wait until it is answered,
    /// and an answer that comes after the model was replaced saves nothing.
    PickSave { name: String, xpdl: bool },
    /// Write `bytes` to `path`; answer with [`App::written`] or [`App::save_failed`].
    Write { path: String, bytes: Vec<u8> },
    /// Offer `bytes` as a download named `name` (where there is no file system); answer with
    /// [`App::downloaded`] once the download has started, or [`App::save_failed`].
    Download { name: String, bytes: Vec<u8> },
    /// Save an exported file (results as CSV, charts as PNG) named `name`: ask where (or
    /// download it); answer with [`App::exported`] or [`App::report_error`].
    Export { name: String, bytes: Vec<u8> },
    /// Close the window: the user confirmed, or there was nothing unsaved.
    Quit,
    /// Open `url` in the web browser (Help's links).
    OpenUrl { url: String },
}

/// What the platform offers the app.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Platform {
    /// Files can be written to a path (desktop). Without it, saving downloads the file.
    pub file_system: bool,
    /// The app can close its own window (desktop; a browser tab closes itself).
    pub can_quit: bool,
}

impl Platform {
    pub const DESKTOP: Self = Self {
        file_system: true,
        can_quit: true,
    };
    pub const BROWSER: Self = Self {
        file_system: false,
        can_quit: false,
    };
}

/// An action that replaces or closes the model, held while the user decides what to do with
/// unsaved changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Replace {
    New,
    /// A new, empty Monte Carlo model (File ▸ New Monte Carlo Model).
    NewMonteCarlo,
    /// A new, empty model to build on the canvas (the start page's Start from a blank model),
    /// with Getting started beside it for Build a first model.
    Build {
        getting_started: bool,
    },
    Open,
    Example(usize),
    /// File ▸ Open Recent: the recent file at this index.
    Recent(usize),
    /// A file dropped on the window (held in `App::dropped`).
    Dropped,
    Quit,
}

/// What the views read of the model, each with the engine's revision it was worked out at
/// ([`App::wire_model`], [`App::model_digest`], [`App::results`]).
#[derive(Default)]
struct ModelCache {
    wire: Option<(u64, Rc<Value>)>,
    digest: Option<(u64, String)>,
    /// Whether the results are of the model, by revision and results serial.
    results_current: Option<((u64, u64), bool)>,
}

/// A save that needs a new BPMN file, written from the model alone, held while the user decides.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NewFile {
    /// Why the model can't be saved into the opened file; none when the user chose BPMN for an
    /// XPDL file.
    reason: Option<String>,
    /// The file chosen in Save As. Without one, the user is asked where (desktop), or the
    /// download is named for a BPMN file (browser).
    path: Option<String>,
    /// The document asked about (`doc_gen` then): an answer is not applied to another one.
    for_doc: u64,
}

/// A charts export waiting for its screenshot: what the frame that asked for it showed, which
/// the screenshot is cropped by. The web captures a later frame than the one that asked, so
/// every frame until the image arrives is checked against it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct PendingPng {
    /// Where the charts were, in points.
    area: egui::Rect,
    /// The scale, pixels per point.
    pixels_per_point: f32,
    /// The results on show (their `results_serial`).
    results: u64,
    /// A menu or dialog was over the window.
    covered: bool,
    /// A frame since showed something else (another view, place, scale or results, or a menu
    /// or dialog over the charts): the screenshot is not of the charts, so it is not exported.
    spoiled: bool,
}

/// The appearance the user chose: the system's, or always light or dark.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeChoice {
    pub fn key(self) -> &'static str {
        match self {
            ThemeChoice::System => "system",
            ThemeChoice::Light => "light",
            ThemeChoice::Dark => "dark",
        }
    }

    pub fn from_key(key: &str) -> Option<Self> {
        [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark]
            .into_iter()
            .find(|t| t.key() == key)
    }

    fn preference(self) -> egui::ThemePreference {
        match self {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        }
    }
}

/// What the app remembers between sessions: kept by the shell (craft-settings on the desktop,
/// the browser's storage on the web).
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub appearance: Appearance,
    pub files: Files,
    pub keyboard: Keyboard,
}

/// The user's own shortcuts: craft-keymap's overrides of the defaults (`{"file.save": "Ctrl+S"}`,
/// `""` for none), as Settings ▸ Keyboard Shortcuts… saves them.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Keyboard {
    /// Left out when the user changed nothing (TOML has no null).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shortcuts: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// The interface language's code (`de`, `pt-br`); left out to follow the system.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    /// The colour theme in use, by name: a built-in one, or one of `custom_themes`. A name that
    /// matches neither shows StochCraft's theme, and the name is kept as it was saved.
    pub colors: String,
    /// The categorical palette series use, by name: a built-in one, or one of `custom_palettes`.
    /// A name that matches none, or a palette that isn't categorical, shows the default.
    pub chart_palette: String,
    /// The user's own colour themes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_themes: Vec<theme::Theme>,
    /// The user's own chart palettes.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_palettes: Vec<chart_palette::ChartPalette>,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::default(),
            language: None,
            colors: theme::DEFAULT.to_owned(),
            chart_palette: chart_palette::DEFAULT.to_owned(),
            custom_themes: Vec::new(),
            custom_palettes: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Files {
    /// Recently opened or saved model files, newest first.
    pub recent: Vec<String>,
}

/// How many files File ▸ Open Recent lists.
pub const RECENT_FILES: usize = 10;

/// A run in the background: its last reported status.
struct Running {
    done: u64,
    /// How far the replication under way has got (0 to 1), for a run in slices.
    partial: f64,
    total: u64,
    /// The shell's [`Workers`] run it, rather than the page between frames.
    on_workers: bool,
}

/// Workers the shell offers for runs: in the browser, web workers, one per core. The app hands a
/// run's replications to them (`"mode": "workers"`, see [`stochcraft_engine::Worker`]) instead of
/// running them on the page in slices of events between frames. The results are the same either
/// way.
pub trait Workers {
    /// Starts running `task` (from [`Engine::worker_task`]), whose replications are numbered
    /// from 0 to `replications - 1`: each worker makes a [`stochcraft_engine::Worker`] from it
    /// and runs the replications it is given. False if no worker could start; the app then runs
    /// the replications itself.
    fn start(&mut self, task: &str, replications: u64) -> bool;
    /// The run now has `replications` replications in all, numbered from 0, more than before: a
    /// sequential run that looked at its results and went on. The workers run the new ones too,
    /// those that had run out of replications included.
    fn extend(&mut self, replications: u64);
    /// The workers' answers that have arrived since the last call, for [`Engine::deliver`].
    fn answers(&mut self) -> Vec<String>;
    /// Stops every worker: the run finished, failed or was cancelled.
    fn stop(&mut self);
}

/// The engine's comparison of the baseline with the results on show, keyed by their serials and
/// the interface language its list of changes is written in.
type Comparison = ((u64, u64, craft_i18n::Lang), Value, Vec<String>);

/// The application: one model document and what the window shows of it.
pub struct App {
    engine: Engine,
    platform: Platform,
    /// Results of the last finished run. They stay after the model changes, marked out of date
    /// ([`App::results_out_of_date`]), until another run, file or model replaces them.
    /// Shared, so a view can hold them while it changes the app.
    results: Option<Rc<Value>>,
    /// The model the results came from, as [`comparable`] writes it.
    results_model: Option<Value>,
    /// The model as the views read it, and whether the results are of it, worked out once per
    /// change of the model (by the engine's revision) rather than every frame.
    model_cache: RefCell<ModelCache>,
    /// The model as the run in progress started with it, as [`comparable`] writes it.
    run_model: Option<Value>,
    /// The model has changed since the results' run (worked out once a frame).
    results_stale: bool,
    /// Counts the results shown, so a baseline knows whether it is the results on show.
    results_serial: u64,
    /// Results kept to compare later runs with ("Keep as baseline").
    baseline: Option<baseline::Baseline>,
    /// The engine's comparison of the baseline with the results on show, and what changed in
    /// the model between them, by their serials and the interface language the changes are
    /// written in.
    comparison: Option<Comparison>,
    running: Option<Running>,
    /// The shell's workers for runs, if it has any.
    workers: Option<Box<dyn Workers>>,
    /// The last message for the status bar: what happened, or what went wrong.
    status: String,
    /// The status is an error.
    status_error: bool,
    /// The block the status-bar error concerns, which clicking it shows.
    status_at: Option<String>,
    /// The error Run or the problems chip showed last (by its place in the list), so the next
    /// click shows the one after it.
    problem_shown: Option<usize>,
    /// Waiting for the user to save or discard changes before this.
    confirm: Option<Replace>,
    /// Do this once the save in progress has been written.
    after_save: Option<Replace>,
    /// Waiting for the user to agree to saving as a new BPMN file.
    new_file: Option<NewFile>,
    /// The user agreed to a new BPMN file and is choosing where it goes.
    as_new: bool,
    /// Counts the documents the window has shown: one more each time the model is replaced
    /// (opened, new, recovered), so an answer asked for one document is not applied to the next.
    doc_gen: u64,
    /// The Save dialog is open ([`Request::PickSave`]), for the document `doc_gen` counted then.
    pick_for: Option<u64>,
    about: bool,
    /// A file dropped on the window, waiting on the unsaved-changes question.
    dropped: Option<(String, Vec<u8>)>,
    requests: Vec<Request>,
    /// The title last shown, so the shell sets it only when it changes.
    title: String,
    /// The colour theme last installed or applied to the context (none before the first frame).
    styled: Option<theme::Theme>,
    /// The language whose font stack is installed: the fonts are installed again when the
    /// interface language changes, so its faces lead (and, on the web, are fetched).
    fonts_lang: Option<craft_i18n::Lang>,
    /// The languages View ▸ Language names in English for now, because the font for their own
    /// script hasn't been fetched (the web; see [`theme::names_waiting_for_fonts`]).
    names_in_english: Vec<craft_i18n::Lang>,
    /// The block the inspector shows; none shows the model overview (or, with several blocks
    /// selected on the canvas, what can be done to them).
    selected: Option<String>,
    /// The flowchart's view, its multiple selection and its clipboard.
    canvas: canvas::Canvas,
    /// A run being watched on the canvas (M3.5).
    watching: Option<live::Watching>,
    /// The engine revision of the model last validated, and its problems, so a frame validates
    /// at most once.
    problems_of: Option<(u64, Vec<editor::Problem>)>,
    /// Text being typed into form fields, until it commits, and numbers a field refused, until
    /// they are fixed or another block is shown.
    drafts: std::collections::HashMap<String, String>,
    /// The block the inspector showed last (`None`: the overview), whose drafts these are.
    drafts_of: Option<String>,
    /// The field of the inspector that had the keyboard when it was last drawn, and the engine
    /// revision of the model then.
    inspector_focus: Option<(egui::Id, u64)>,
    /// The inspector shows `drafts_of` for one more frame, while its field commits, though
    /// another block is selected.
    drafts_held: bool,
    /// The connections table's choices, before Connect.
    connect_from: Option<String>,
    connect_to: Option<String>,
    /// The work area shows the results rather than the model.
    show_results: bool,
    /// The Dashboard panel (View ▸ Dashboard).
    show_dashboard: bool,
    /// The work area shows the Analysis view (sweeps, sensitivity, sizing), unless the results
    /// are shown.
    show_analysis: bool,
    /// The work area shows the Capacity view (the table worked out from the model, no run).
    show_capacity: bool,
    /// The work area shows the 3D view of the model (M5.3).
    show_3d: bool,
    /// The 3D view's camera and the scene of the model's revision.
    view3d: view3d::View3d,
    /// The Layout view (M5.5), shown on the 3D tab's flag when `layout_view.on`.
    layout_view: layout_view::LayoutView,
    /// The Capacity view's target and its table for the model's revision.
    capacity: capacity::State,
    /// The M2.8 screens' choices and last answers.
    decide: decide::Decide,
    /// The Help panel.
    help: help::Help,
    /// The guided first model: "Show me" in Getting started.
    guide: guide::Guide,
    /// The start page's filters.
    start: start::Start,
    /// An empty model is being built (from the start page), so the model view shows rather than
    /// the start page.
    building: bool,
    /// The Monte Carlo screens' state, and a Monte Carlo run in progress.
    mc: montecarlo::Mc,
    /// The system dynamics screens' state (an XMILE document).
    sd: sd::Sd,
    /// Where the charts are drawn this frame (none until the work area draws them), for
    /// exporting them.
    charts_rect: Option<egui::Rect>,
    /// A screenshot asked for to export the charts, filled in by the renderer.
    screenshot: std::sync::Arc<std::sync::Mutex<Option<std::sync::Arc<egui::ColorImage>>>>,
    /// The charts export waiting for its screenshot.
    pending_png: Option<PendingPng>,
    /// Frames left to bring the charts into view before exporting them: the screenshot holds
    /// only what is on screen, and the charts sit below the key measures.
    png_scroll: u8,
    /// Commands and their shortcuts (craft-keymap), and the frame's key dispatch.
    keymap: craft_keymap::Keymap,
    dispatcher: craft_keymap::egui::Dispatcher,
    /// Recently opened or saved files, newest first (desktop only).
    recent: Vec<String>,
    theme_choice: ThemeChoice,
    /// The colour theme in use, by name, and the user's own colour themes (see [`Appearance`]).
    colors: String,
    custom_themes: Vec<theme::Theme>,
    /// The Themes window, while it is open.
    themes: Option<theme_editor::Editor>,
    /// The categorical palette series use, by name, and the user's own chart palettes (see
    /// [`Appearance`]), and the Chart colours window while it is open.
    chart_palette: String,
    custom_palettes: Vec<chart_palette::ChartPalette>,
    chart_colours: Option<chart_editor::Editor>,
    /// The interface language as the user chose it (`None`: the system's), and as shown.
    language: Option<String>,
    lang: craft_i18n::Lang,
    /// The theme last applied to the context.
    applied_theme: Option<ThemeChoice>,
    /// The shell shows the menus in the system's menu bar (macOS), not in the window.
    native_menu: bool,
    /// The digest of the model last handed out as this session's recovery copy.
    autosaved: Option<String>,
    /// This session's own recovery copy can go (it was handed out and nothing is unsaved now,
    /// but the shell could not remove it).
    autosave_clear_pending: bool,
    /// Recovery copies left by sessions that are gone, found at start, waiting for the user's
    /// answer: each is its slot's name (the shell's) and the copy. Asked about one at a time.
    recovery: std::collections::VecDeque<(String, Value)>,
    /// Slots of recovery copies this session recovered, kept until its own copy of the recovered
    /// model is written (or it has nothing unsaved any more), so a crash in between loses nothing.
    recovery_held: Vec<String>,
    /// Slots the shell can remove now (recovered and now kept by this session, discarded, or
    /// damaged): [`App::take_dropped_copies`].
    recovery_drops: Vec<String>,
    /// The slots of `recovery_held` that the last [`Autosave::Write`] released, put back if its
    /// write fails.
    recovery_released: Vec<String>,
    /// Counts the files being opened ([`App::begin_open`]): a newer one makes older ones stale.
    open_serial: u64,
    /// The user chose to quit (discarding any changes): no recovery copy is kept.
    quitting: bool,
    /// Settings ▸ Keyboard Shortcuts…, while it is open (craft-keymap's dialog).
    shortcuts_dialog: Option<craft_keymap::ui::DialogState>,
    /// The app draws the window frame (craft-window chose it): the header is the title bar and
    /// shows these window buttons.
    title_bar: Option<craft_window::Layout>,
    /// How numbers are written and read in the fields: the user's region (craft-i18n).
    numbers: craft_ui::localized::LocaleNumbers,
}

impl App {
    /// The app with a new, empty model.
    pub fn new(platform: Platform) -> Self {
        Self {
            engine: Engine::new(),
            platform,
            results: None,
            results_model: None,
            model_cache: RefCell::default(),
            run_model: None,
            results_stale: false,
            results_serial: 0,
            baseline: None,
            comparison: None,
            running: None,
            workers: None,
            status: String::new(),
            status_error: false,
            status_at: None,
            problem_shown: None,
            confirm: None,
            after_save: None,
            new_file: None,
            as_new: false,
            doc_gen: 0,
            pick_for: None,
            about: false,
            dropped: None,
            requests: Vec::new(),
            title: String::new(),
            styled: None,
            fonts_lang: None,
            names_in_english: Vec::new(),
            selected: None,
            canvas: canvas::Canvas::default(),
            watching: None,
            problems_of: None,
            drafts: std::collections::HashMap::new(),
            drafts_of: None,
            inspector_focus: None,
            drafts_held: false,
            connect_from: None,
            connect_to: None,
            show_results: false,
            show_dashboard: false,
            show_analysis: false,
            show_capacity: false,
            show_3d: false,
            view3d: view3d::View3d::default(),
            layout_view: layout_view::LayoutView::default(),
            capacity: capacity::State::default(),
            decide: decide::Decide::default(),
            help: help::Help::default(),
            guide: guide::Guide::default(),
            start: start::Start::default(),
            building: false,
            mc: montecarlo::Mc::default(),
            sd: sd::Sd::default(),
            charts_rect: None,
            screenshot: std::sync::Arc::default(),
            pending_png: None,
            png_scroll: 0,
            keymap: commands::keymap(craft_keymap::Platform::current()),
            dispatcher: craft_keymap::egui::Dispatcher::new(),
            recent: Vec::new(),
            theme_choice: ThemeChoice::System,
            colors: theme::DEFAULT.to_owned(),
            custom_themes: Vec::new(),
            themes: None,
            chart_palette: chart_palette::DEFAULT.to_owned(),
            custom_palettes: Vec::new(),
            chart_colours: None,
            language: None,
            lang: i18n::I18N.system_lang(),
            applied_theme: None,
            native_menu: false,
            shortcuts_dialog: None,
            autosaved: None,
            autosave_clear_pending: false,
            recovery: std::collections::VecDeque::new(),
            recovery_held: Vec::new(),
            recovery_drops: Vec::new(),
            recovery_released: Vec::new(),
            open_serial: 0,
            quitting: false,
            title_bar: None,
            numbers: form::messages(craft_ui::localized::LocaleNumbers::new(
                std::sync::Arc::new(craft_i18n::Format::new(
                    &craft_i18n::Region::system(),
                    craft_i18n::Digits::Locale,
                )),
            )),
        }
    }

    /// The engine, for shells and tests that read the model.
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// A run is in progress.
    pub fn is_running(&self) -> bool {
        self.running.is_some()
    }

    /// Requests made since the last call, in order.
    pub fn take_requests(&mut self) -> Vec<Request> {
        std::mem::take(&mut self.requests)
    }

    /// The status-bar message.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// The status-bar message as a screen reader should hear it: an error says so in words, as
    /// the bar shows "Error" beside it. Empty when there is nothing to say.
    pub fn status_announcement(&self) -> String {
        if self.status_error && !self.status.is_empty() {
            tf("Error: {message}", &[("message", &self.status)])
        } else {
            self.status.clone()
        }
    }

    /// The interface language's code (`de`, `pt-br`), for the web page's `lang`.
    pub fn language_code(&self) -> &'static str {
        i18n::code(self.lang)
    }

    /// The model as the window reads it ([`stochcraft_engine::wire::model`]), written once per
    /// change of the model however many views read it in a frame.
    pub(crate) fn wire_model(&self) -> Rc<Value> {
        let revision = self.engine.revision();
        let mut cache = self.model_cache.borrow_mut();
        match &cache.wire {
            Some((r, m)) if *r == revision => m.clone(),
            _ => {
                let m = Rc::new(stochcraft_engine::wire::model(self.engine.model()));
                cache.wire = Some((revision, m.clone()));
                m
            }
        }
    }

    /// The model's digest ([`stochcraft_engine::model_digest`]), worked out once per change.
    pub(crate) fn model_digest(&self) -> String {
        let revision = self.engine.revision();
        let mut cache = self.model_cache.borrow_mut();
        match &cache.digest {
            Some((r, d)) if *r == revision => d.clone(),
            _ => {
                let d = stochcraft_engine::model_digest(self.engine.model());
                cache.digest = Some((revision, d.clone()));
                d
            }
        }
    }
}

/// What a block of `kind` is, in a few lowercase words ("task with a resource").
pub(crate) fn kind_text(kind: &str) -> &str {
    match kind {
        "source" => t("arrivals"),
        "server" => t("task with a resource"),
        "shared_task" => t("task with shared resources"),
        "delay" => t("task"),
        "sink" => t("departures"),
        "exclusive_gateway" => t("decision: one way"),
        "parallel_gateway" => t("split or join: all ways"),
        "batch" => t("batch"),
        "unbatch" => t("split batches"),
        "conveyor" => t("conveyor"),
        "transport" => t("transport"),
        other => other,
    }
}

/// The model's time unit as written in results ("min", "h", …).
fn time_unit(engine: &Engine) -> String {
    serde_json::to_value(stochcraft_engine::wire::TimeUnitSpec::from(
        engine.model().experiment.time_unit,
    ))
    .ok()
    .and_then(|v| v.as_str().map(|u| results::unit_label(u).to_owned()))
    .unwrap_or_default()
}

/// The model's length unit as written after a number ("m", "cm", "ft"): the unit's own symbol.
fn length_unit(engine: &Engine) -> String {
    serde_json::to_value(stochcraft_engine::wire::LengthUnitSpec::from(
        engine.model().experiment.length_unit,
    ))
    .ok()
    .and_then(|v| v.as_str().map(str::to_owned))
    .unwrap_or_default()
}

/// How the Edit menu and the status bar name a change, from the command that made it.
/// `model` as results are kept against it: its wire form without where blocks are drawn, so
/// moving a block leaves the results current while any other change marks them out of date.
fn comparable(model: &stochcraft_model::Model) -> Value {
    let mut m = stochcraft_engine::wire::model(model);
    for e in m["elements"].as_array_mut().into_iter().flatten() {
        if let Some(e) = e.as_object_mut() {
            e.remove("bounds");
        }
    }
    m
}

fn change_label(command: &str) -> &str {
    match command {
        "model.set" => "Replace Model",
        "model.rename" => "Rename Model",
        "block.add" => "Add Block",
        "block.set" => "Change Block",
        "block.rename" => "Rename Block",
        "block.remove" => "Remove Block",
        "connect" => "Connect",
        "flow.set" => "Change Flow",
        "disconnect" => "Disconnect",
        "experiment.set" => "Experiment Settings",
        "experiment.size" => "Size Resources",
        "property.set" => "Set Property",
        "property.remove" => "Remove Property",
        "estimate.set" => "Change Estimate",
        "estimate.remove" => "Remove Estimate",
        "resource.add" => "Add Pool",
        "resource.set" => "Change Pool",
        "resource.remove" => "Remove Pool",
        "calendar.add" => "Add Calendar",
        "calendar.set" => "Change Calendar",
        "calendar.remove" => "Remove Calendar",
        "mc.input.set" => "Change Input",
        "mc.inputs.import" => "Import Inputs",
        "mc.input.remove" => "Remove Input",
        "mc.output.set" => "Change Output",
        "mc.output.remove" => "Remove Output",
        "mc.correlation.set" => "Change Correlation",
        "mc.set" => "Monte Carlo Settings",
        "layout.set" => "Move",
        "batch" => "Changes",
        "paste" => "Paste",
        "delete" => "Delete",
        "align" => "Align",
        "duplicate" => "Duplicate",
        other => other,
    }
}

/// The last component of a path, either separator.
fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// A file's extension in lower case, without the dot (empty if it has none).
fn extension(path: &str) -> String {
    file_name(path)
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use stochcraft_engine::{COMMANDS, CommandKind};

    /// Every command that goes on the undo history is named in the Edit menu and the status bar
    /// by its own label, not its command name (`change_label` returns the name when it has none).
    #[test]
    fn every_undoable_command_has_a_label() {
        for c in COMMANDS.iter().filter(|c| c.kind == CommandKind::Edit) {
            assert_ne!(
                change_label(c.name),
                c.name,
                "`{}` has no undo label",
                c.name
            );
        }
    }

    /// The views read one written model until the model changes, and then the new one.
    #[test]
    fn the_model_is_written_once_per_change() {
        let mut app = App::new(Platform::DESKTOP);
        let a = app.wire_model();
        assert!(Rc::ptr_eq(&a, &app.wire_model()), "unchanged: the same");
        assert!(
            app.execute("model.rename", json!({ "name": "x" }))
                .is_some()
        );
        let b = app.wire_model();
        assert!(!Rc::ptr_eq(&a, &b));
        assert_eq!(b["name"], "x");
        assert_eq!(
            app.model_digest(),
            stochcraft_engine::model_digest(app.engine.model())
        );
        app.undo();
        assert_eq!(*app.wire_model(), *a);
        assert_eq!(
            app.model_digest(),
            stochcraft_engine::model_digest(app.engine.model())
        );
    }
}
