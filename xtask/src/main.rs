// Copyright (c) 2026 StochCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option.
// The command layout follows FieldCraft's xtask (https://github.com/tangency-labs/fieldcraft,
// MIT OR Apache-2.0); see ATTRIBUTION.md.

//! Repository tooling. Run `cargo xtask help`.

mod affected;
mod layers;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
cargo xtask <command>

  layers          check the crate dependency layering (docs/architecture.md)
  affected --base REV [--head REV] [--github-output]
                  the crates a change since REV touches, plus the crates that depend on them,
                  and the CI flags: everything, heavy (#[ignore]d tests affected), wasm
  wasm            cargo check of every layered crate for wasm32-unknown-unknown
  ci [--affected REV]
                  fmt --check, layers, clippy -D warnings and tests; with --affected REV only
                  for the crates `affected --base REV` names (everything when it says so)
  help            this text
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let result = match args.first().copied() {
        Some("layers") => cmd_layers(),
        Some("affected") => affected::run(&args[1..]),
        Some("wasm") => cmd_wasm(),
        Some("ci") => match &args[1..] {
            [] => cmd_ci(None),
            ["--affected", base] => cmd_ci(Some(base)),
            _ => Err(format!(
                "ci: expected no arguments or `--affected REV`\n\n{USAGE}"
            )),
        },
        Some("help") | None => {
            print!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

pub fn root() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent().unwrap_or(here).to_path_buf()
}

fn cargo() -> Command {
    let mut c = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()));
    c.current_dir(root());
    c
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    eprintln!("$ {what}");
    let status = cmd.status().map_err(|e| format!("{what}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

pub fn metadata() -> Result<serde_json::Value, String> {
    let out = cargo()
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("cargo metadata: bad JSON: {e}"))
}

/// The crates the wasm check builds: every layered crate (apps, tooling and the testkit are
/// native-only).
pub fn wasm_crate(pkg: &str) -> bool {
    matches!(
        layers::classify(pkg),
        Some(layers::Class::Layer(_) | layers::Class::Standalone)
    )
}

fn cmd_layers() -> Result<(), String> {
    let crates = layers::from_metadata(&metadata()?)?;
    println!("{:<28} {:<14} workspace deps", "crate", "layer");
    for c in &crates {
        let ws: Vec<String> = c
            .deps
            .iter()
            .filter(|d| d.workspace)
            .map(|d| {
                let k = match d.kind {
                    layers::DepKind::Normal => "",
                    layers::DepKind::Dev => " (dev)",
                    layers::DepKind::Build => " (build)",
                };
                format!("{}{k}", layers::short_name(&d.name))
            })
            .collect();
        println!(
            "{:<28} {:<14} {}",
            c.name,
            layers::describe(layers::classify(&c.name)),
            ws.join(", ")
        );
    }
    let violations = layers::check(&crates);
    if violations.is_empty() {
        println!("\nlayers: ok");
        return Ok(());
    }
    for v in &violations {
        eprintln!("  {v}");
    }
    Err(format!("layers: {} violation(s)", violations.len()))
}

fn wasm_crates() -> Result<Vec<String>, String> {
    let crates = layers::from_metadata(&metadata()?)?;
    Ok(crates
        .into_iter()
        .map(|c| c.name)
        .filter(|n| wasm_crate(n))
        .collect())
}

fn cmd_wasm() -> Result<(), String> {
    let set = wasm_crates()?;
    let mut c = cargo();
    c.args(["check", "--locked", "--target", "wasm32-unknown-unknown"]);
    for p in &set {
        c.args(["-p", p]);
    }
    run(
        c,
        &format!(
            "cargo check --target wasm32-unknown-unknown ({} crate(s))",
            set.len()
        ),
    )
}

fn cmd_ci(affected_base: Option<&str>) -> Result<(), String> {
    let packages: Option<Vec<String>> = match affected_base {
        None => None,
        Some(base) => {
            let (plan, files) = affected::plan_since(base, None)?;
            affected::print(&plan, base, files);
            if plan.everything {
                None
            } else {
                Some(plan.crates.into_iter().collect())
            }
        }
    };
    let mut fmt = cargo();
    fmt.args(["fmt", "--all", "--check"]);
    run(fmt, "cargo fmt --all --check")?;
    cmd_layers()?;
    let select = |c: &mut Command| match &packages {
        None => {
            c.arg("--workspace");
        }
        Some(p) => {
            for name in p {
                c.args(["-p", name]);
            }
        }
    };
    if packages.as_ref().is_some_and(|p| p.is_empty()) {
        eprintln!("no crates affected: skipping clippy and tests");
        return Ok(());
    }
    let mut clippy = cargo();
    clippy.args(["clippy", "--locked", "--all-targets"]);
    select(&mut clippy);
    clippy.args(["--", "-D", "warnings"]);
    run(clippy, "cargo clippy -D warnings")?;
    let mut test = cargo();
    test.args(["test", "--locked"]);
    select(&mut test);
    run(test, "cargo test")
}
