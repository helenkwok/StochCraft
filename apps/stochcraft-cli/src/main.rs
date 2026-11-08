//! `stochcraft-cli`: StochCraft from the command line.
//!
//! Everything goes through the engine's commands (`stochcraft-engine`), as the desktop app and
//! agents do, so a command line session can be replayed as a script and vice versa.

mod output;
mod templates;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandError, Engine, JournalEntry};

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
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
    },
    /// Run a model's experiment and print the results with their run record.
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
        #[arg(long, value_enum, default_value_t = Format::Table)]
        format: Format,
        /// Write the results to this file instead of the terminal.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Replay engine commands from a JSON script: an array of {"command", "params"}, as an
    /// engine journal is written. Prints each step's result as a line of JSON.
    Exec {
        script: PathBuf,
        /// Open this model file before the first step.
        #[arg(long)]
        open: Option<PathBuf>,
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

fn emit(text: &str, out: Option<&Path>) -> Result<(), Failure> {
    match out {
        Some(p) => std::fs::write(p, text)
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

fn validate(file: &Path, scenario: Option<&str>, format: Format) -> Result<u8, Failure> {
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
        Format::Json => println!(
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
        Format::Table | Format::Csv => print!("{}", output::validation(file, &opened, errors)),
    }
    Ok(u8::from(errors > 0))
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
    format: Format,
    out: Option<&Path>,
) -> Result<u8, Failure> {
    let mut engine = Engine::new();
    open(&mut engine, file, scenario)?;
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
    let results = engine.execute("run", params)?;
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

fn exec(script: &Path, open_file: Option<&Path>) -> Result<u8, Failure> {
    let text = std::fs::read_to_string(script)
        .map_err(|e| Failure(format!("cannot read {}: {e}", script.display())))?;
    let steps: Vec<JournalEntry> = serde_json::from_str(&text).map_err(|e| {
        Failure(format!(
            "{}: not a script (a JSON array of {{\"command\", \"params\"}}): {e}",
            script.display()
        ))
    })?;
    let mut engine = Engine::new();
    if let Some(f) = open_file {
        open(&mut engine, f, None)?;
    }
    for (i, step) in steps.iter().enumerate() {
        match engine.execute(&step.command, step.params.clone()) {
            Ok(result) => println!(
                "{}",
                json!({ "step": i, "command": step.command, "result": result })
            ),
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
fn mcp(connect: Option<String>, read: Option<&Path>, write: Option<&Path>) -> Result<u8, Failure> {
    let roots = if read.is_some() || write.is_some() {
        stochcraft_mcp::Roots::new(read, write).map_err(Failure)?
    } else {
        stochcraft_mcp::Roots::default()
    };
    let (backend, note) =
        stochcraft_mcp::backend(stochcraft_mcp::Options { connect, roots }).map_err(Failure)?;
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
        Command::Run {
            file,
            scenario,
            reps,
            seed,
            threads,
            warmup,
            duration,
            format,
            out,
        } => run(
            &file,
            scenario.as_deref(),
            reps,
            seed,
            threads,
            warmup,
            duration,
            format,
            out.as_deref(),
        ),
        Command::Exec { script, open } => exec(&script, open.as_deref()),
        Command::Schema { command } => schema(command.as_deref()),
        Command::Mcp {
            connect,
            headless: _,
            read_root,
            write_root,
        } => mcp(connect, read_root.as_deref(), write_root.as_deref()),
    };
    match outcome {
        Ok(code) => ExitCode::from(code),
        Err(Failure(message)) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}
