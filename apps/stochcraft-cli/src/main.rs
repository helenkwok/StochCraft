//! `stochcraft-cli`: StochCraft from the command line.
//!
//! Everything goes through the engine's commands (`stochcraft-engine`), as the desktop app and
//! agents do, so a command line session can be replayed as a script and vice versa.

mod fit;
mod output;
mod templates;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use stochcraft_engine::{
    COMMANDS, CommandError, Engine, ErrorCode, Journal, JournalEntry, ReplayMode, wire,
};

use templates::TEMPLATES;

/// Discrete-event simulation of process models in BPMN or XPDL with BPSim data.
#[derive(Parser)]
#[command(name = "stochcraft-cli", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starting model as a new BPMN file with BPSim data.
    New {
        /// The template to start from (`--list` shows them).
        #[arg(long, short, default_value = "mm1")]
        template: String,
        /// List the templates and stop.
        #[arg(long)]
        list: bool,
        /// The file to write (`.bpmn`).
        #[arg(required_unless_present = "list")]
        file: Option<PathBuf>,
        /// Overwrite the file if it exists.
        #[arg(long)]
        force: bool,
    },
    /// Check a model file: what StochCraft noted reading it, and the model's diagnostics.
    /// Exits with 1 if there are errors.
    Validate {
        file: PathBuf,
        /// The BPSim scenario to read (default: the first).
        #[arg(long)]
        scenario: Option<String>,
        #[arg(long, value_enum, default_value_t = CheckFormat::Table)]
        format: CheckFormat,
    },
    /// Without running: each station's arrival rate, mean service time, units and utilisation
    /// worked out from the model's own numbers, marked fine, congested or unstable, the
    /// bottleneck, the units that would bring a station under the target, and the mean wait
    /// where a queueing formula holds exactly (or why none does).
    Explain {
        file: PathBuf,
        /// The BPSim scenario to read (default: the first).
        #[arg(long)]
        scenario: Option<String>,
        /// The utilisation above which a station is congested, between 0 and 1.
        #[arg(long, default_value_t = 0.85, value_parser = target)]
        target: f64,
        #[arg(long, value_enum, default_value_t = CheckFormat::Table)]
        format: CheckFormat,
    },
    /// Balance assembly work elements across stations for a cycle time (the engine's
    /// `line.balance`). Reads a CSV file with header `id,time,after` (`after` lists
    /// predecessors separated by `;`).
    Balance {
        /// A CSV file with header `id,time,after`.
        file: PathBuf,
        /// The cycle time (the longest any station may take), in the same unit as the element
        /// times.
        #[arg(long, value_parser = positive_finite)]
        cycle: f64,
        /// One heuristic to run (default: all three compared).
        #[arg(long, value_enum)]
        heuristic: Option<BalanceHeuristic>,
        #[arg(long, value_enum, default_value_t = CheckFormat::Table)]
        format: CheckFormat,
    },
    /// Run a model's experiment and print the results with their run record. A Monte Carlo model
    /// (a calculation with no process) runs its trials instead: `--reps` is the number of
    /// trials, and `--until` and `--max` run until every output's mean is that precise. A
    /// system dynamics model (an XMILE file) prints each variable's value at the end of the run;
    /// `--format csv` and `--format json` give every variable over time.
    Run {
        file: PathBuf,
        /// The BPSim scenario to run (default: the first).
        #[arg(long)]
        scenario: Option<String>,
        /// Replications (default: the scenario's).
        #[arg(long, short)]
        reps: Option<u32>,
        /// Random seed (default: the scenario's).
        #[arg(long, short)]
        seed: Option<u64>,
        /// Worker threads (default: all). Results do not depend on it.
        #[arg(long, short)]
        threads: Option<u32>,
        /// Warm-up, in the model's time unit (default: the scenario's).
        #[arg(long)]
        warmup: Option<f64>,
        /// Time over which statistics are collected, after the warm-up (default: the
        /// scenario's).
        #[arg(long)]
        duration: Option<f64>,
        /// Run sequentially: after the first replications (`--reps`), add more until every key
        /// measure's 95% half-width is within this share of its mean, such as `5%` or `0.05`
        /// (default: the scenario's setting, else 5%). Any of `--until`, `--until-points` and
        /// `--max` makes the run sequential.
        #[arg(long, value_parser = share_of_mean)]
        until: Option<f64>,
        /// Run sequentially, with a share's (a utilisation's) half-width judged in percentage
        /// points instead (default: the scenario's setting, else 1).
        #[arg(long, value_parser = points)]
        until_points: Option<f64>,
        /// Run sequentially, with at most this many replications (default: the scenario's
        /// setting, else 200).
        #[arg(long, value_parser = clap::value_parser!(u32).range(1..))]
        max: Option<u32>,
        /// Run exactly the replications, even where the scenario runs sequentially.
        #[arg(long, conflicts_with_all = ["until", "until_points", "max"])]
        fixed: bool,
        /// A Monte Carlo model: also work out how much of each output's variation each input
        /// accounts for, alone and with its interactions (Sobol indices; needs independent
        /// inputs, and evaluates the model rows x (inputs + 2) times).
        #[arg(long)]
        sobol: bool,
        /// Rows of each base sample of `--sobol` (default 1000, at least 10).
        #[arg(long, requires = "sobol", value_parser = clap::value_parser!(u64).range(10..))]
        sobol_rows: Option<u64>,
        /// A system dynamics model (an XMILE file): the time step to run at, in place of the
        /// file's, in the model's time unit. `--reps` is then the replications of a model with
        /// a random function (default 100), and `--seed` its seed (default 1).
        #[arg(long, value_parser = time_step)]
        dt: Option<f64>,
        /// A system dynamics model: also run at half the time step and report each stock's largest
        /// relative difference between the two.
        #[arg(long)]
        dt_check: bool,
        /// A Monte Carlo model: how its inputs are drawn in this run (default: the model's own).
        /// `antithetic` makes trials in pairs that use u and 1 - u; `quasi-random` (also
        /// `sobol`) is randomised quasi-Monte Carlo with Sobol points (at most 64 inputs; not
        /// the `--sobol` shares of the variation). Latin hypercube and quasi-random runs are
        /// whole batches of 1,000 trials, antithetic whole pairs.
        #[arg(long, value_enum)]
        sampling: Option<SamplingChoice>,
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
        /// Write the results to this file instead of the terminal. Not the model file or its
        /// scenario's file: the output replaces whatever is there.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Run a process model although StochCraft could not read parts of its file (see
        /// `validate`): each such part is replaced by a stand-in (a processing-time expression
        /// by a time of zero, say), so the results are of an approximation of the model. The
        /// stand-ins are listed on stderr and in the run record. Without this, the run is
        /// refused.
        #[arg(long)]
        allow_approximations: bool,
    },
    /// Run a designed experiment with several factors at once (the engine's
    /// `experiment.design`): every combination of the factors' levels, or a Latin hypercube of
    /// their ranges, each point with the same seed. Prints each result at each point and the
    /// factors' effects.
    Design {
        file: PathBuf,
        /// A factor and its levels, `id.field=1,2,3`, or its range, `id.field=0.5..1.5`. The
        /// field is `capacity`, `quantity`, `queue_capacity` or `mean`. Repeat for each factor.
        #[arg(long, required = true)]
        factor: Vec<String>,
        /// A result to report, `element.metric` (such as `desk.mean_wait`). Repeat for more.
        #[arg(long, required = true)]
        result: Vec<String>,
        #[arg(long, value_enum, default_value_t = DesignKind::FullFactorial)]
        design: DesignKind,
        /// A Latin hypercube's number of points.
        #[arg(long)]
        points: Option<u32>,
        /// The BPSim scenario to run (default: the first).
        #[arg(long)]
        scenario: Option<String>,
        /// Replications at each point (default: the scenario's).
        #[arg(long, short)]
        reps: Option<u32>,
        /// Random seed (default: the scenario's).
        #[arg(long, short)]
        seed: Option<u64>,
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
        /// Write the output to this file instead of the terminal. Not the model file.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Run although StochCraft could not read parts of the file, with stand-ins for them
        /// (as for `run`).
        #[arg(long)]
        allow_approximations: bool,
    },
    /// A Monte Carlo model's input table as CSV (`name,distribution,parameters,unit`): printed,
    /// or with `--import`, read from a CSV file into the model, which is saved.
    Inputs {
        /// The Monte Carlo model file (`.bpmn`).
        file: PathBuf,
        /// A CSV table whose inputs replace those of the same name (with `--replace`, all of
        /// them); the file is then saved.
        #[arg(long)]
        import: Option<PathBuf>,
        /// With `--import`: replace every input with the table's.
        #[arg(long, requires = "import")]
        replace: bool,
        /// Write the table to this file instead of the terminal.
        #[arg(long, short, conflicts_with = "import")]
        out: Option<PathBuf>,
    },
    /// Fit measured values to distributions (the engine's `estimate.fit`): each family by
    /// maximum likelihood, ranked by Anderson–Darling A², with the Kolmogorov–Smirnov distance.
    /// Exits with 1 if no family can be fitted.
    Fit {
        /// A CSV file, or a plain list with one value per line; `-` reads standard input.
        file: PathBuf,
        /// The column: its header or its number from 1 (default: the only column).
        #[arg(long, short)]
        column: Option<String>,
        /// The families to try, separated by commas (default: all): exponential, gamma,
        /// erlang, log_normal, weibull, normal, uniform, triangular.
        #[arg(long, value_delimiter = ',')]
        families: Vec<String>,
        /// The fewest values a fit is offered for (default 20; never fewer than 5).
        #[arg(long)]
        min_count: Option<u32>,
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
    },
    /// Replay engine commands from a JSON script (an array of {"command", "params"}) or from a
    /// journal written by `--journal` (an object with its version and starting state). Prints
    /// each step's result as a line of JSON. A script runs in full; a journal re-applies its
    /// edits and skips saves, runs and experiments unless `--all`, and is refused if the model
    /// is not the one it started from (`--open` the same file) unless `--force`.
    Exec {
        script: PathBuf,
        /// Open this model file before the first step.
        #[arg(long)]
        open: Option<PathBuf>,
        /// Write the journal of what was done to this file, as JSON that `exec` replays.
        #[arg(long)]
        journal: Option<PathBuf>,
        /// Replaying a journal: carry out its saves, runs and experiments too.
        #[arg(long)]
        all: bool,
        /// Replaying a journal: go ahead although the model is not the one it started from.
        #[arg(long)]
        force: bool,
    },
    /// Print the engine's commands with the JSON schema of their parameters.
    Schema {
        /// One command (default: all).
        command: Option<String>,
    },
    /// Serve the engine's commands to an AI agent over MCP (JSON-RPC on stdin and stdout;
    /// docs/mcp.md). Headless by default; `--connect` drives the running app instead.
    Mcp {
        /// The control channel of an app started with `stochcraft --control PORT`, such as
        /// `127.0.0.1:7070`. If it cannot be reached, the server runs headless.
        #[arg(long, conflicts_with_all = ["read_root", "write_root"])]
        connect: Option<String>,
        /// The app's control channel token, for when its token file is not reachable. Default: the
        /// environment variable `STOCHCRAFT_TOKEN`, else the file the app wrote (docs/mcp.md).
        #[arg(long, requires = "connect")]
        token: Option<String>,
        /// Run an engine in this process, with no window (the default).
        #[arg(long, conflicts_with = "connect")]
        headless: bool,
        /// Confine the files the agent opens to this folder (nothing can be opened without it
        /// once either root is given).
        #[arg(long)]
        read_root: Option<PathBuf>,
        /// Confine the files the agent saves to this folder (nothing can be saved without it
        /// once either root is given).
        #[arg(long)]
        write_root: Option<PathBuf>,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum Format {
    Table,
    Json,
    Csv,
}

/// `validate`'s output: its notes and diagnostics are a list, not a table of numbers, so there is
/// no CSV.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum CheckFormat {
    Table,
    Json,
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum BalanceHeuristic {
    #[value(name = "largest_candidate", alias = "largest-candidate")]
    LargestCandidate,
    #[value(name = "kilbridge_wester", alias = "kilbridge-wester")]
    KilbridgeWester,
    #[value(name = "ranked_positional_weight", alias = "ranked-positional-weight")]
    RankedPositionalWeight,
}

impl BalanceHeuristic {
    fn name(self) -> &'static str {
        match self {
            BalanceHeuristic::LargestCandidate => "largest_candidate",
            BalanceHeuristic::KilbridgeWester => "kilbridge_wester",
            BalanceHeuristic::RankedPositionalWeight => "ranked_positional_weight",
        }
    }
}

/// `run --sampling`: how a Monte Carlo model's inputs are drawn.
#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum SamplingChoice {
    /// Each draw independent.
    Random,
    /// Each input's range cut into as many equally likely strata as there are trials in a batch.
    LatinHypercube,
    /// Trials in pairs that use u and 1 - u for every input.
    Antithetic,
    /// Sobol points with a random digital shift in each batch.
    #[value(name = "quasi-random", alias = "sobol")]
    QuasiRandom,
}

impl SamplingChoice {
    /// Its name in the engine's `mc.run` and `mc.set`.
    fn name(self) -> &'static str {
        match self {
            SamplingChoice::Random => "random",
            SamplingChoice::LatinHypercube => "latin_hypercube",
            SamplingChoice::Antithetic => "antithetic",
            SamplingChoice::QuasiRandom => "sobol",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, ValueEnum)]
enum DesignKind {
    /// Every combination of the levels.
    FullFactorial,
    /// `--points` points, each factor's range cut into that many strata, each used once.
    LatinHypercube,
}

/// A failure to report: the message, and the exit code.
struct Failure(String);

impl From<CommandError> for Failure {
    fn from(e: CommandError) -> Self {
        let mut msg = e.message.clone();
        if let Some(details) = e.details.as_ref().and_then(Value::as_array) {
            for d in details {
                if let Some(m) = d["message"].as_str() {
                    msg.push_str("\n  ");
                    msg.push_str(m);
                }
            }
        }
        Failure(msg)
    }
}

fn path_str(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn open(engine: &mut Engine, file: &Path, scenario: Option<&str>) -> Result<Value, Failure> {
    let mut params = json!({ "path": path_str(file) });
    if let Some(s) = scenario {
        params["scenario"] = json!(s);
    }
    Ok(engine.execute("file.open", params)?)
}

/// Whether two paths are the same file: both exist, and are one after resolving links or are
/// hard links to one file.
fn is_same(a: &Path, b: &Path) -> bool {
    same_file::is_same_file(a, b).unwrap_or(false)
}

/// Refuses an output file that is one of the files the command reads: it would replace its own
/// input, and a command that fails part-way would leave the input gone.
fn refuse_overwriting_input(out: Option<&Path>, inputs: &[&Path]) -> Result<(), Failure> {
    match out.and_then(|o| inputs.iter().find(|i| is_same(o, i))) {
        Some(i) => Err(Failure(format!(
            "the output file is the input {}: give another path for the output",
            i.display()
        ))),
        None => Ok(()),
    }
}

/// Says on stderr what a run that was allowed to went ahead with: each error in the file and
/// the stand-in the model has instead, so results are not mistaken for the file's own model.
fn report_approximations(approximations: &Value) {
    let Some(list) = approximations.as_array().filter(|l| !l.is_empty()) else {
        return;
    };
    eprintln!(
        "warning: this run is of an approximation of the model: StochCraft could not read {} part{} of the file and used a stand-in",
        list.len(),
        if list.len() == 1 { "" } else { "s" }
    );
    for a in list {
        let at = a["at"]
            .as_str()
            .map_or_else(String::new, |at| format!(" at `{at}`"));
        eprintln!("  error{at}: {}", a["message"].as_str().unwrap_or(""));
    }
}

fn emit(text: &str, out: Option<&Path>) -> Result<(), Failure> {
    match out {
        Some(p) => stochcraft_engine::write_atomically(p, text.as_bytes())
            .map_err(|e| Failure(format!("cannot write {}: {e}", p.display()))),
        None => {
            print!("{text}");
            Ok(())
        }
    }
}

fn new(template: &str, list: bool, file: Option<&Path>, force: bool) -> Result<u8, Failure> {
    if list {
        for t in TEMPLATES {
            println!("{:<8} {}", t.name, t.summary);
        }
        return Ok(0);
    }
    let Some(file) = file else {
        return Err(Failure("give the file to write".into()));
    };
    let t = TEMPLATES
        .iter()
        .find(|t| t.name == template)
        .ok_or_else(|| {
            let names: Vec<&str> = TEMPLATES.iter().map(|t| t.name).collect();
            Failure(format!(
                "no template `{template}`; the templates are {}",
                names.join(", ")
            ))
        })?;
    if file.extension().is_none_or(|e| e != "bpmn") {
        return Err(Failure(
            "new models are written as BPMN: give a file ending in .bpmn".into(),
        ));
    }
    if file.exists() && !force {
        return Err(Failure(format!(
            "{} exists; use --force to overwrite it",
            file.display()
        )));
    }
    let mut engine = Engine::new();
    for (command, params) in (t.steps)() {
        engine.execute(command, params)?;
    }
    engine.execute("file.save", json!({ "path": path_str(file) }))?;
    eprintln!("wrote {} ({})", file.display(), t.summary);
    Ok(0)
}

fn validate(file: &Path, scenario: Option<&str>, format: CheckFormat) -> Result<u8, Failure> {
    let mut engine = Engine::new();
    let opened = open(&mut engine, file, scenario)?;
    let notes = opened["notes"].as_array().cloned().unwrap_or_default();
    let diagnostics = opened["diagnostics"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let errors = notes
        .iter()
        .chain(&diagnostics)
        .filter(|n| n["severity"] == "error")
        .count();
    match format {
        CheckFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "file": path_str(file),
                "scenario": opened["scenario"],
                "scenarios": opened["scenarios"],
                "notes": notes,
                "diagnostics": diagnostics,
                "errors": errors,
            }))
            .unwrap_or_default()
        ),
        CheckFormat::Table => print!("{}", output::validation(file, &opened, errors)),
    }
    Ok(u8::from(errors > 0))
}

/// A target utilisation: a number strictly between 0 and 1 (not NaN or infinite).
fn target(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if v > 0.0 && v < 1.0 {
        Ok(v)
    } else {
        Err(format!("the target must be between 0 and 1, not {s}"))
    }
}

fn explain(
    file: &Path,
    scenario: Option<&str>,
    target: f64,
    format: CheckFormat,
) -> Result<u8, Failure> {
    let mut engine = Engine::new();
    let opened = open(&mut engine, file, scenario)?;
    // A model with errors has no table worth trusting: say what they are, as `run` does.
    let errors: Vec<String> = opened["notes"]
        .as_array()
        .into_iter()
        .chain(opened["diagnostics"].as_array())
        .flatten()
        .filter(|n| n["severity"] == "error")
        .map(|n| n["message"].as_str().unwrap_or("").to_owned())
        .collect();
    if !errors.is_empty() {
        eprintln!(
            "{} cannot be explained: it has errors (stochcraft-cli validate lists them):",
            file.display()
        );
        for e in &errors {
            eprintln!("  {e}");
        }
        return Ok(1);
    }
    let table = engine.execute("model.capacity", json!({ "target": target }))?;
    match format {
        CheckFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&table).unwrap_or_default()
        ),
        CheckFormat::Table => print!("{}", output::capacity(file, &table)),
    }
    Ok(0)
}

/// A half-width as a share of the mean: `5%`, or a fraction such as `0.05`.
fn share_of_mean(s: &str) -> Result<f64, String> {
    let bad = || format!("give the share of the mean as `5%` or `0.05`, not `{s}`");
    let v = match s.trim().strip_suffix('%') {
        Some(p) => p.trim().parse::<f64>().map_err(|_| bad())? / 100.0,
        None => s.trim().parse::<f64>().map_err(|_| bad())?,
    };
    if v.is_finite() && v > 0.0 && v < 1.0 {
        Ok(v)
    } else {
        Err(bad())
    }
}

/// A half-width in percentage points, above 0.
fn points(s: &str) -> Result<f64, String> {
    match s.trim().parse::<f64>() {
        Ok(v) if v.is_finite() && v > 0.0 => Ok(v),
        _ => Err(format!(
            "give the half-width in percentage points as a number above 0, not `{s}`"
        )),
    }
}

/// A positive, finite number (for `--cycle`).
fn time_step(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(format!(
            "the time step must be positive and finite, not {s}"
        ))
    }
}

fn positive_finite(s: &str) -> Result<f64, String> {
    let v: f64 = s.parse().map_err(|e| format!("{e}"))?;
    if v.is_finite() && v > 0.0 {
        Ok(v)
    } else {
        Err(format!(
            "the cycle time must be positive and finite, not {s}"
        ))
    }
}

struct CsvElement {
    id: String,
    time: f64,
    after: Vec<String>,
}

fn csv_row(line: &str, line_no: usize) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => out.push(std::mem::take(&mut cell).trim().to_owned()),
            c => cell.push(c),
        }
    }
    if quoted {
        return Err(format!("line {line_no}: unclosed double quote"));
    }
    out.push(cell.trim().to_owned());
    Ok(out)
}

fn read_balance_csv(text: &str) -> Result<Vec<CsvElement>, String> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut rows = text
        .lines()
        .enumerate()
        .map(|(i, line)| (i + 1, line))
        .filter(|(_, line)| !line.trim().is_empty());
    let Some((header_line, header_text)) = rows.next() else {
        return Err("the CSV file is empty (expected header `id,time,after`)".into());
    };
    let header = csv_row(header_text, header_line)?;
    let [h0, h1, h2] = header.as_slice() else {
        return Err(format!(
            "line {header_line}: expected header `id,time,after`, got `{}`",
            header_text.trim()
        ));
    };
    if h0 != "id" || h1 != "time" || h2 != "after" {
        return Err(format!(
            "line {header_line}: expected header `id,time,after`, got `{}`",
            header_text.trim()
        ));
    }
    let mut elements = Vec::new();
    for (line_no, line) in rows {
        let fields = csv_row(line, line_no)?;
        let [id, time_str, after_str] = fields.as_slice() else {
            return Err(format!(
                "line {line_no}: expected 3 fields (`id,time,after`), found {}",
                fields.len()
            ));
        };
        if id.is_empty() {
            return Err(format!("line {line_no}: missing work element `id`"));
        }
        let time: f64 = time_str
            .parse()
            .map_err(|_| format!("line {line_no}: `{time_str}` is not a valid time"))?;
        if !(time.is_finite() && time > 0.0) {
            return Err(format!(
                "line {line_no}: time must be positive and finite, not `{time_str}`"
            ));
        }
        let after = if after_str.is_empty() {
            Vec::new()
        } else {
            after_str
                .split(';')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect()
        };
        elements.push(CsvElement {
            id: id.clone(),
            time,
            after,
        });
    }
    if elements.is_empty() {
        return Err(format!("line {header_line}: no work elements after header"));
    }
    Ok(elements)
}

fn balance(
    file: &Path,
    cycle: f64,
    heuristic: Option<BalanceHeuristic>,
    format: CheckFormat,
) -> Result<u8, Failure> {
    let text = std::fs::read_to_string(file)
        .map_err(|e| Failure(format!("cannot read {}: {e}", file.display())))?;
    let elements = match read_balance_csv(&text) {
        Ok(els) => els,
        Err(msg) => {
            eprintln!("error: {msg}");
            return Ok(2);
        }
    };
    let mut params = json!({
        "elements": elements
            .iter()
            .map(|e| json!({ "id": e.id, "time": e.time, "after": e.after }))
            .collect::<Vec<_>>(),
        "cycle": cycle,
    });
    if let Some(h) = heuristic {
        params["heuristic"] = json!(h.name());
    }
    let answer = match Engine::new().execute("line.balance", params) {
        Ok(v) => v,
        Err(err) => {
            eprintln!("error: {}", err.message);
            return Ok(2);
        }
    };
    match format {
        CheckFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&answer).unwrap_or_default()
        ),
        CheckFormat::Table => print!("{}", output::balance(file, &answer)),
    }
    Ok(0)
}

/// `run`'s options for sequential replications.
struct Until {
    /// `--until`: the half-width as a share of the mean.
    relative: Option<f64>,
    /// `--until-points`: a share's half-width in percentage points.
    points: Option<f64>,
    /// `--max`: the most replications.
    max: Option<u32>,
    /// `--fixed`: exactly the replications, whatever the scenario says.
    fixed: bool,
}

impl Until {
    /// The `sequential` parameter of `run`: `null` for a fixed run, the scenario's setting
    /// (else the defaults) with the options given, or nothing to keep the scenario's.
    fn param(&self, engine: &Engine) -> Option<Value> {
        if self.fixed {
            return Some(Value::Null);
        }
        if self.relative.is_none() && self.points.is_none() && self.max.is_none() {
            return None;
        }
        let mut q = engine
            .model()
            .experiment
            .sequential
            .as_ref()
            .map_or_else(|| json!({}), |x| json!(wire::SequentialSpec::from(x)));
        if let Some(r) = self.relative {
            q["relative"] = json!(r);
        }
        if let Some(p) = self.points {
            q["points"] = json!(p);
        }
        if let Some(m) = self.max {
            q["max"] = json!(m);
        }
        Some(q)
    }
}

/// `run`'s options for a system dynamics model's time step.
struct StepOptions {
    /// `--dt`.
    dt: Option<f64>,
    /// `--dt-check`.
    check: bool,
}

/// `run`'s options for a Monte Carlo model's Sobol indices.
struct Shares {
    /// `--sobol`: work them out.
    wanted: bool,
    /// `--sobol-rows`: the rows of each base sample.
    rows: Option<u64>,
}

#[allow(clippy::too_many_arguments)] // One per command-line option.
fn run(
    file: &Path,
    scenario: Option<&str>,
    reps: Option<u32>,
    seed: Option<u64>,
    threads: Option<u32>,
    warmup: Option<f64>,
    duration: Option<f64>,
    until: &Until,
    shares: &Shares,
    steps: &StepOptions,
    sampling: Option<SamplingChoice>,
    format: Format,
    out: Option<&Path>,
    allow_approximations: bool,
) -> Result<u8, Failure> {
    refuse_overwriting_input(out, &[file])?;
    let mut engine = Engine::new();
    let opened = open(&mut engine, file, scenario)?;
    if opened["kind"] == "system_dynamics" {
        if warmup.is_some()
            || duration.is_some()
            || until.relative.is_some()
            || until.points.is_some()
            || until.max.is_some()
            || until.fixed
            || shares.wanted
            || sampling.is_some()
        {
            return Err(Failure(
                "a system dynamics model has no warm-up, duration, sequential stop, sampling or Sobol shares: use --reps (replications, for a model with a random function), --seed, --dt and --dt-check"
                    .into(),
            ));
        }
        return run_system_dynamics(&mut engine, reps, seed, steps, format, out);
    }
    if steps.dt.is_some() || steps.check {
        return Err(Failure(
            "--dt and --dt-check are for system dynamics models (XMILE files)".into(),
        ));
    }
    if opened["kind"] == "monte_carlo" {
        if warmup.is_some() || duration.is_some() || until.points.is_some() {
            return Err(Failure(
                "a Monte Carlo model has no warm-up, duration or shares in points: use --reps (trials), --until and --max".into(),
            ));
        }
        return run_monte_carlo(
            &mut engine,
            reps,
            seed,
            threads,
            until,
            shares,
            sampling,
            format,
            out,
        );
    }
    if sampling.is_some() {
        return Err(Failure(
            "--sampling is for Monte Carlo models: a process model has no inputs that are distributions"
                .into(),
        ));
    }
    if shares.wanted {
        return Err(Failure(
            "--sobol is for Monte Carlo models: a process model has no inputs that are distributions"
                .into(),
        ));
    }
    let mut set = json!({});
    if let Some(w) = warmup {
        set["warmup"] = json!(w);
    }
    if let Some(d) = duration {
        set["duration"] = json!(d);
    }
    if set.as_object().is_some_and(|o| !o.is_empty()) {
        engine.execute("experiment.set", set)?;
    }
    let mut params = json!({});
    if let Some(r) = reps {
        params["replications"] = json!(r);
    }
    if let Some(s) = seed {
        params["seed"] = json!(s);
    }
    if let Some(t) = threads {
        params["threads"] = json!(t);
    }
    if let Some(q) = until.param(&engine) {
        params["sequential"] = q;
    }
    if allow_approximations {
        params["allow_approximations"] = json!(true);
    }
    let results = engine.execute("run", params)?;
    report_approximations(&results["record"]["approximations"]);
    let text = match format {
        Format::Json => format!(
            "{}\n",
            serde_json::to_string_pretty(&results).unwrap_or_default()
        ),
        Format::Csv => output::csv(&results),
        Format::Table => output::table(&engine.model().name, &results),
    };
    emit(&text, out)?;
    Ok(0)
}

/// `run` on a system dynamics model: every variable over time, once or in replications.
fn run_system_dynamics(
    engine: &mut Engine,
    reps: Option<u32>,
    seed: Option<u64>,
    steps: &StepOptions,
    format: Format,
    out: Option<&Path>,
) -> Result<u8, Failure> {
    let mut params = json!({});
    if let Some(r) = reps {
        params["replications"] = json!(r);
    }
    if let Some(s) = seed {
        params["seed"] = json!(s);
    }
    if let Some(d) = steps.dt {
        params["dt"] = json!(d);
    }
    if steps.check {
        params["dt_check"] = json!(true);
    }
    let results = engine.execute("sd.run", params)?;
    let text = match format {
        Format::Json => format!(
            "{}
",
            serde_json::to_string_pretty(&results).unwrap_or_default()
        ),
        Format::Csv => output::sd_csv(&results),
        Format::Table => output::sd_table("", &results),
    };
    emit(&text, out)?;
    Ok(0)
}

/// `run` on a Monte Carlo model: its trials (`--reps`), seed, threads and stop, and with
/// `--sobol` the shares of each output's variation.
#[allow(clippy::too_many_arguments)] // One per command-line option.
fn run_monte_carlo(
    engine: &mut Engine,
    trials: Option<u32>,
    seed: Option<u64>,
    threads: Option<u32>,
    until: &Until,
    shares: &Shares,
    sampling: Option<SamplingChoice>,
    format: Format,
    out: Option<&Path>,
) -> Result<u8, Failure> {
    let mut params = json!({});
    if let Some(s) = sampling {
        params["sampling"] = json!(s.name());
    }
    if let Some(t) = trials {
        params["trials"] = json!(t);
    }
    if let Some(s) = seed {
        params["seed"] = json!(s);
    }
    if let Some(t) = threads {
        params["threads"] = json!(t);
    }
    if until.fixed {
        params["stop"] = Value::Null;
    } else if until.relative.is_some() || until.max.is_some() {
        let own = engine
            .model()
            .monte_carlo
            .as_ref()
            .and_then(|m| m.stop)
            .unwrap_or_default();
        params["stop"] = json!({
            "relative": until.relative.unwrap_or(own.relative),
            "max": until.max.unwrap_or(own.max),
        });
    }
    let mut results = engine.execute("mc.run", params)?;
    let shares_text = if shares.wanted {
        Some(sobol_shares(engine, seed, shares.rows, &mut results)?)
    } else {
        None
    };
    let text = match format {
        Format::Json => format!(
            "{}
",
            serde_json::to_string_pretty(&results).unwrap_or_default()
        ),
        Format::Csv => {
            // The table of outputs stays as the app exports it: the shares go to stderr.
            if let Some(more) = &shares_text {
                eprint!("{}", more.trim_start());
            }
            output::mc_csv(&results)
        }
        Format::Table => {
            let mut table = output::mc_table(&engine.model().name, &results);
            if let Some(more) = &shares_text {
                table.push_str(more);
            }
            table
        }
    };
    emit(&text, out)?;
    Ok(0)
}

/// `--sobol`: works out the Sobol indices with the run's seed, puts them in `results` as its
/// `sobol` field, and returns their text. A model that cannot have them (correlated inputs) is
/// not a failure: the reason is given in their place, and the run's results stand. A number that
/// cannot be used is one.
fn sobol_shares(
    engine: &mut Engine,
    seed: Option<u64>,
    rows: Option<u64>,
    results: &mut Value,
) -> Result<String, Failure> {
    let mut params = json!({});
    if let Some(s) = seed {
        params["seed"] = json!(s);
    }
    if let Some(r) = rows {
        params["rows"] = json!(r);
    }
    match engine.execute("mc.sobol", params) {
        Ok(v) => {
            let text = output::sobol_lines(&v);
            results["sobol"] = v;
            Ok(text)
        }
        Err(e) if e.code == ErrorCode::InvalidModel => {
            results["sobol"] = json!({ "unavailable": e.message.clone() });
            Ok(format!("\nshares of the variation: {}\n", e.message))
        }
        Err(e) => Err(e.into()),
    }
}

/// `inputs`: a Monte Carlo model's input table, printed or imported.
fn inputs(
    file: &Path,
    import: Option<&Path>,
    replace: bool,
    out: Option<&Path>,
) -> Result<u8, Failure> {
    // `--out` writes the table of the model: not over the model.
    refuse_overwriting_input(out, &[file])?;
    let mut engine = Engine::new();
    let opened = open(&mut engine, file, None)?;
    match import {
        None => {
            if opened["kind"] != "monte_carlo" {
                return Err(Failure(format!(
                    "{} is not a Monte Carlo model: it has no inputs",
                    file.display()
                )));
            }
            let table = engine.execute("mc.inputs.export", json!({}))?;
            emit(table["csv"].as_str().unwrap_or(""), out)?;
        }
        Some(csv) => {
            let text = std::fs::read_to_string(csv)
                .map_err(|e| Failure(format!("cannot read {}: {e}", csv.display())))?;
            let r = engine.execute(
                "mc.inputs.import",
                json!({ "csv": text, "replace": replace }),
            )?;
            engine.execute("file.save", json!({ "path": path_str(file) }))?;
            eprintln!("{} inputs read into {}", r["inputs"], file.display());
        }
    }
    Ok(0)
}

/// `id.field=1,2,3` or `id.field=min..max` as a factor of `experiment.design`.
fn factor(text: &str) -> Result<Value, Failure> {
    let bad = || {
        Failure(format!(
            "`{text}`: give a factor as `id.field=1,2,3` (its levels) or `id.field=min..max` \
             (its range)"
        ))
    };
    let (name, values) = text.split_once('=').ok_or_else(bad)?;
    // Ids may contain dots; the field is after the last.
    let (id, field) = name.rsplit_once('.').ok_or_else(bad)?;
    let num = |v: &str| v.trim().parse::<f64>().map_err(|_| bad());
    let mut f = json!({ "id": id, "field": field });
    if let Some((lo, hi)) = values.split_once("..") {
        f["min"] = json!(num(lo)?);
        f["max"] = json!(num(hi)?);
    } else {
        let levels = values.split(',').map(num).collect::<Result<Vec<_>, _>>()?;
        f["levels"] = json!(levels);
    }
    Ok(f)
}

/// `element.metric` as a result of `experiment.design`.
fn result(text: &str) -> Result<Value, Failure> {
    let (element, metric) = text.rsplit_once('.').ok_or_else(|| {
        Failure(format!(
            "`{text}`: give a result as `element.metric`, such as `desk.mean_wait`"
        ))
    })?;
    Ok(json!({ "element": element, "metric": metric }))
}

struct DesignArgs<'a> {
    file: &'a Path,
    factors: &'a [String],
    results: &'a [String],
    design: DesignKind,
    points: Option<u32>,
    scenario: Option<&'a str>,
    reps: Option<u32>,
    seed: Option<u64>,
    format: Format,
    out: Option<&'a Path>,
    allow_approximations: bool,
}

fn design(a: &DesignArgs) -> Result<u8, Failure> {
    refuse_overwriting_input(a.out, &[a.file])?;
    let mut engine = Engine::new();
    open(&mut engine, a.file, a.scenario)?;
    let mut params = json!({
        "factors": a.factors.iter().map(|f| factor(f)).collect::<Result<Vec<_>, _>>()?,
        "results": a.results.iter().map(|r| result(r)).collect::<Result<Vec<_>, _>>()?,
        "design": match a.design {
            DesignKind::FullFactorial => "full_factorial",
            DesignKind::LatinHypercube => "latin_hypercube",
        },
    });
    if let Some(n) = a.points {
        params["points"] = json!(n);
    }
    if let Some(r) = a.reps {
        params["replications"] = json!(r);
    }
    if let Some(s) = a.seed {
        params["seed"] = json!(s);
    }
    if a.allow_approximations {
        params["allow_approximations"] = json!(true);
    }
    let answer = engine.execute("experiment.design", params)?;
    report_approximations(&answer["approximations"]);
    let text = match a.format {
        Format::Json => format!(
            "{}\n",
            serde_json::to_string_pretty(&answer).unwrap_or_default()
        ),
        Format::Csv => output::design_csv(&answer),
        Format::Table => output::design_table(&engine.model().name, &answer),
    };
    emit(&text, a.out)?;
    Ok(0)
}

/// What `exec` replays: a bare script, or a journal with the state it started from.
enum Script {
    Steps(Vec<JournalEntry>),
    Journal(Box<Journal>),
}

fn exec(
    script: &Path,
    open_file: Option<&Path>,
    journal_out: Option<&Path>,
    all: bool,
    force: bool,
) -> Result<u8, Failure> {
    // A journal written over the script or the model it replays would replace it, and a script
    // that failed part-way would leave only the journal of how far it got.
    let inputs: Vec<&Path> = std::iter::once(script).chain(open_file).collect();
    refuse_overwriting_input(journal_out, &inputs)?;
    let text = std::fs::read_to_string(script)
        .map_err(|e| Failure(format!("cannot read {}: {e}", script.display())))?;
    let not_a_script = |e: serde_json::Error| {
        Failure(format!(
            "{}: not a script (a JSON array of {{\"command\", \"params\"}}) or a journal (an object with `start` and `entries`): {e}",
            script.display()
        ))
    };
    let value: Value = serde_json::from_str(&text).map_err(not_a_script)?;
    let script = if value.is_array() {
        Script::Steps(serde_json::from_value(value).map_err(not_a_script)?)
    } else {
        Script::Journal(Box::new(
            serde_json::from_value(value).map_err(not_a_script)?,
        ))
    };
    let mut engine = Engine::new();
    if let Some(f) = open_file {
        // A journal opens the scenario it was recorded from, so its starting model (and its
        // digest) is the one the journal began with; a bare script reads the first scenario.
        let scenario = match &script {
            Script::Journal(journal) => journal.start.scenario.as_deref(),
            Script::Steps(_) => None,
        };
        match open(&mut engine, f, scenario) {
            Ok(_) => {}
            // Forced onto a file without that scenario: replay onto its first one, as --force
            // replays onto a model that is not the one the journal started from.
            Err(_) if force && scenario.is_some() => {
                open(&mut engine, f, None)?;
            }
            Err(e) => return Err(e),
        }
    }
    // Every file the steps open or save, not only the one open at the end.
    let mut touched: Vec<PathBuf> = Vec::new();
    let ran = exec_steps(&mut engine, &script, all, force, &mut touched);
    // The journal of what was done is written even when a step failed: it says how far it got.
    let written = journal_out.map(|out| {
        // The script may have opened or saved other files than the one it was given: not those
        // either, nor the one open at the end.
        touched.extend(engine.path().map(PathBuf::from));
        let touched: Vec<&Path> = touched.iter().map(PathBuf::as_path).collect();
        refuse_overwriting_input(Some(out), &touched)?;
        let text = serde_json::to_string_pretty(&engine.journal_document()).unwrap_or_default();
        stochcraft_engine::write_atomically(out, (text + "\n").as_bytes())
            .map_err(|e| Failure(format!("cannot write {}: {e}", out.display())))
    });
    let code = ran?;
    written.transpose()?;
    Ok(code)
}

fn exec_steps(
    engine: &mut Engine,
    script: &Script,
    all: bool,
    force: bool,
    touched: &mut Vec<PathBuf>,
) -> Result<u8, Failure> {
    // A bare script runs in full; a journal re-applies its edits unless `--all`.
    let (steps, mode) = match script {
        Script::Steps(steps) => (steps.as_slice(), ReplayMode::All),
        Script::Journal(journal) => {
            engine
                .begin_replay(journal, force)
                .map_err(|e| Failure(e.to_string()))?;
            let mode = if all {
                ReplayMode::All
            } else {
                ReplayMode::EditsOnly
            };
            // On stderr, so the steps on stdout stay one JSON line each.
            eprintln!("{}", stochcraft_engine::replay_summary(journal, mode));
            (journal.entries.as_slice(), mode)
        }
    };
    for (i, step) in steps.iter().enumerate() {
        if !mode.applies(&step.command) {
            println!(
                "{}",
                json!({ "step": i, "command": step.command, "skipped": true })
            );
            continue;
        }
        if matches!(step.command.as_str(), "file.open" | "file.save") {
            touched.extend(step.params["path"].as_str().map(PathBuf::from));
        }
        match engine.execute(&step.command, step.params.clone()) {
            Ok(result) => {
                // A started run is finished here, as a replay has no one to poll it.
                engine.finish_started_run(&step.command);
                println!(
                    "{}",
                    json!({ "step": i, "command": step.command, "result": result })
                );
            }
            Err(e) => {
                println!(
                    "{}",
                    json!({ "step": i, "command": step.command, "error": e.to_json() })
                );
                return Err(Failure(format!(
                    "step {i} ({}): {}",
                    step.command, e.message
                )));
            }
        }
    }
    Ok(0)
}

fn schema(command: Option<&str>) -> Result<u8, Failure> {
    let value = match command {
        None => Value::Array(COMMANDS.iter().map(|c| c.to_json()).collect()),
        Some(name) => COMMANDS
            .iter()
            .find(|c| c.name == name)
            .map(|c| c.to_json())
            .ok_or_else(|| Failure(format!("no command `{name}`")))?,
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    );
    Ok(0)
}

/// `stochcraft-cli mcp`: serves until stdin closes. Only protocol messages go to stdout.
fn mcp(
    connect: Option<String>,
    token: Option<String>,
    read: Option<&Path>,
    write: Option<&Path>,
) -> Result<u8, Failure> {
    let roots = if read.is_some() || write.is_some() {
        stochcraft_mcp::Roots::new(read, write).map_err(Failure)?
    } else {
        stochcraft_mcp::Roots::default()
    };
    let (backend, note) = stochcraft_mcp::backend(stochcraft_mcp::Options {
        connect,
        token,
        token_file: None,
        roots,
    })
    .map_err(Failure)?;
    if let Some(note) = note {
        eprintln!("stochcraft-cli mcp: {note}");
    }
    let mut server = stochcraft_mcp::Server::new(backend);
    server
        .serve(std::io::stdin().lock(), std::io::stdout().lock())
        .map_err(|e| Failure(format!("mcp: {e}")))?;
    Ok(0)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::New {
            template,
            list,
            file,
            force,
        } => new(&template, list, file.as_deref(), force),
        Command::Validate {
            file,
            scenario,
            format,
        } => validate(&file, scenario.as_deref(), format),
        Command::Explain {
            file,
            scenario,
            target,
            format,
        } => explain(&file, scenario.as_deref(), target, format),
        Command::Balance {
            file,
            cycle,
            heuristic,
            format,
        } => balance(&file, cycle, heuristic, format),
        Command::Run {
            file,
            scenario,
            reps,
            seed,
            threads,
            warmup,
            duration,
            until,
            until_points,
            max,
            fixed,
            sobol,
            sobol_rows,
            dt,
            dt_check,
            sampling,
            format,
            out,
            allow_approximations,
        } => run(
            &file,
            scenario.as_deref(),
            reps,
            seed,
            threads,
            warmup,
            duration,
            &Until {
                relative: until,
                points: until_points,
                max,
                fixed,
            },
            &Shares {
                wanted: sobol,
                rows: sobol_rows,
            },
            &StepOptions {
                dt,
                check: dt_check,
            },
            sampling,
            format,
            out.as_deref(),
            allow_approximations,
        ),
        Command::Design {
            file,
            factor,
            result,
            design: kind,
            points,
            scenario,
            reps,
            seed,
            format,
            out,
            allow_approximations,
        } => design(&DesignArgs {
            file: &file,
            factors: &factor,
            results: &result,
            design: kind,
            points,
            scenario: scenario.as_deref(),
            reps,
            seed,
            format,
            out: out.as_deref(),
            allow_approximations,
        }),
        Command::Inputs {
            file,
            import,
            replace,
            out,
        } => inputs(&file, import.as_deref(), replace, out.as_deref()),
        Command::Fit {
            file,
            column,
            families,
            min_count,
            format,
        } => fit::fit(&fit::FitArgs {
            file: &file,
            column: column.as_deref(),
            families: &families,
            min_count,
            format,
        }),
        Command::Exec {
            script,
            open,
            journal,
            all,
            force,
        } => exec(&script, open.as_deref(), journal.as_deref(), all, force),
        Command::Schema { command } => schema(command.as_deref()),
        Command::Mcp {
            connect,
            token,
            headless: _,
            read_root,
            write_root,
        } => mcp(connect, token, read_root.as_deref(), write_root.as_deref()),
    };
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(Failure(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}
