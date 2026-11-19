// Copyright (c) 2026 StochCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option.

//! `cargo xtask fetch-miwg`: fetches the BPMN Model Interchange Working Group's test suite
//! (https://github.com/bpmn-miwg/bpmn-miwg-test-suite, CC BY 3.0 by its `LICENSE.txt`) at a
//! pinned commit into `target/miwg/suite/`, and only its `Reference/` folder, for the tests in
//! `crates/io/tests/miwg.rs` (ROADMAP.md, D14).
//!
//! The files are never committed, so the repository stays under one licence (the same approach
//! as `fetch-bpsim`). The other folders of the suite are exports from commercial modelling tools
//! and are not used. A git commit id fixes the content, so no further checksum is needed. Uses
//! `git`, present on CI runners.

use std::path::PathBuf;
use std::process::Command;

use crate::root;

const URL: &str = "https://github.com/bpmn-miwg/bpmn-miwg-test-suite.git";
/// The suite's `master` on 2026-11-20.
pub const COMMIT: &str = "ebfa3e545b9765a39ece3fc861b3cdf7b3c15bf8";

pub fn dir() -> PathBuf {
    root().join("target").join("miwg").join("suite")
}

fn git(args: &[&str], dir: &std::path::Path) -> Result<String, String> {
    eprintln!("$ git {}", args.join(" "));
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|e| format!("git: {e} (is it installed?)"))?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    } else {
        Err(format!(
            "git {} failed ({}): {}",
            args.join(" "),
            out.status,
            String::from_utf8_lossy(&out.stderr).trim()
        ))
    }
}

fn present(dir: &std::path::Path) -> bool {
    dir.join("Reference").is_dir()
        && git(&["rev-parse", "HEAD"], dir).is_ok_and(|head| head == COMMIT)
}

pub fn fetch() -> Result<(), String> {
    let dir = dir();
    if present(&dir) {
        eprintln!("miwg: present at {COMMIT}");
        return Ok(());
    }
    if dir.exists() {
        std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    git(&["init", "-q"], &dir)?;
    git(&["remote", "add", "origin", URL], &dir)?;
    // Only the Reference folder is checked out; the other folders' files are never downloaded.
    git(&["sparse-checkout", "set", "--cone", "Reference"], &dir)?;
    git(
        &[
            "fetch",
            "-q",
            "--depth",
            "1",
            "--filter=blob:none",
            "origin",
            COMMIT,
        ],
        &dir,
    )?;
    git(
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "-q",
            "FETCH_HEAD",
        ],
        &dir,
    )?;
    if !present(&dir) {
        return Err(format!(
            "the checkout is not at {COMMIT} with a Reference folder"
        ));
    }
    eprintln!("miwg: fetched {COMMIT}");
    Ok(())
}
