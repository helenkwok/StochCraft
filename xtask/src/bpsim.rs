// Copyright (c) 2026 StochCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option.

//! `cargo xtask fetch-bpsim`: downloads the BPSim reference files from bpsim.org into
//! `target/bpsim/` for the tests (ROADMAP.md, M1.3).
//!
//! The example models and the XML Schema carry no licence of their own, so they are never
//! committed: they are downloaded, checked against pinned SHA-256 checksums (a changed file is
//! refused, so tests never silently run on different data) and unpacked. Downloads use `curl`
//! and unpacking uses `unzip`, both present on CI runners, Linux and macOS.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::root;

/// (URL, file name, SHA-256) of each download.
pub const FILES: [(&str, &str, &str); 2] = [
    (
        "https://www.bpsim.org/implementers/2.0/BPSim%20Implementer's%20Guide%20v2.0.zip",
        "implementers-guide-2.0.zip",
        "2d82a8763e7c3356c215ae0b237c64a672400d5e2c339e7421c5e8c790aa7b00",
    ),
    (
        "https://www.bpsim.org/schemas/2.0/BPSim-2.0.xsd",
        "BPSim-2.0.xsd",
        "35999c20096c2cba9e6012d8cbb1b37d2deaac152d8da2ea4f95eb5a990ffcd2",
    ),
];

pub fn dir() -> PathBuf {
    root().join("target").join("bpsim")
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

fn run(mut cmd: Command, what: &str) -> Result<(), String> {
    let status = cmd
        .status()
        .map_err(|e| format!("{what}: {e} (is it installed?)"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{what} failed ({status})"))
    }
}

pub fn fetch() -> Result<(), String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    for (url, name, want) in FILES {
        let path = dir.join(name);
        if path.exists() && sha256(&path)? == want {
            eprintln!("{name}: present, checksum matches");
            continue;
        }
        let partial = dir.join(format!("{name}.part"));
        eprintln!("$ curl {url}");
        let mut curl = Command::new("curl");
        curl.args([
            "--fail",
            "--silent",
            "--show-error",
            "--location",
            "--retry",
            "3",
            "--output",
        ])
        .arg(&partial)
        .arg(url);
        run(curl, "curl")?;
        let got = sha256(&partial)?;
        if got != want {
            let _ = std::fs::remove_file(&partial);
            return Err(format!(
                "{name}: SHA-256 is {got}, expected {want}. bpsim.org may have changed the file; check it and update xtask/src/bpsim.rs"
            ));
        }
        std::fs::rename(&partial, &path)
            .map_err(|e| format!("rename {}: {e}", partial.display()))?;
        eprintln!("{name}: downloaded, checksum matches");
    }
    let unpacked = dir.join("guide");
    if examples().is_none() {
        let _ = std::fs::remove_dir_all(&unpacked);
        let mut unzip = Command::new("unzip");
        unzip
            .args(["-q", "-o"])
            .arg(dir.join(FILES[0].1))
            .arg("-d")
            .arg(&unpacked);
        run(unzip, "unzip")?;
    }
    match examples() {
        Some(e) => {
            eprintln!("examples: {}", e.display());
            Ok(())
        }
        None => Err("the guide's zip has no Examples folder".into()),
    }
}

/// The unpacked `Examples` folder, if present.
pub fn examples() -> Option<PathBuf> {
    fn find(dir: &Path, depth: u32) -> Option<PathBuf> {
        for entry in std::fs::read_dir(dir).ok()?.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if !path.is_dir() || name == "__MACOSX" {
                continue;
            }
            if name == "Examples" {
                return Some(path);
            }
            if depth > 0
                && let Some(found) = find(&path, depth - 1)
            {
                return Some(found);
            }
        }
        None
    }
    find(&dir().join("guide"), 2)
}
