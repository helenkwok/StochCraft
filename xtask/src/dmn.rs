// Copyright (c) 2026 StochCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option.

//! `cargo xtask fetch-dmn`: downloads OMG's XML Schema for DMN 1.6 (its model schema, the
//! diagram interchange schema it imports and the two that one imports) into `target/dmn/`, for
//! the test that every `.dmn` file StochCraft writes is valid DMN 1.6
//! (`crates/io/tests/dmn_schema.rs`, decision D30). The files are the DMN task force's, at the
//! commit tagged `v1.6`; they carry no licence of their own, so like the BPSim files they are
//! fetched rather than committed, and checked against pinned SHA-256 checksums, so a changed file
//! is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::root;

/// The commit of `omg-dmn-taskforce/omg-dmn-spec` tagged `v1.6`.
const COMMIT: &str = "1231abef4e3e68061194034bc3c420d4b2da98be";

/// (file name, SHA-256) of each download, from the repository's `xsd/` folder.
pub const FILES: [(&str, &str); 4] = [
    (
        "DMN16.xsd",
        "861483990d3df4c771c16e8516bc36a5120ee2051decc1146fcb18e4f0a4a318",
    ),
    (
        "DMNDI15.xsd",
        "6e78fe9621d7360c96609523afef3da5ba4fee6a2ad53ec4cd12d126c7795c50",
    ),
    (
        "DC.xsd",
        "9f57b8e99b00163a65dfaf5de2a3e3ea4be041a1bb23228044c50693334d59e6",
    ),
    (
        "DI.xsd",
        "73a5908426f12021da85c9cce74ae70db5ddac1ebe9c9d28d8495f34bea60f07",
    ),
];

pub fn dir() -> PathBuf {
    root().join("target").join("dmn")
}

fn sha256(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(Sha256::digest(&bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

pub fn fetch() -> Result<(), String> {
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    for (name, want) in FILES {
        let path = dir.join(name);
        if path.exists() && sha256(&path)? == want {
            eprintln!("{name}: present, checksum matches");
            continue;
        }
        let url = format!(
            "https://raw.githubusercontent.com/omg-dmn-taskforce/omg-dmn-spec/{COMMIT}/xsd/{name}"
        );
        let partial = dir.join(format!("{name}.part"));
        eprintln!("$ curl {url}");
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "3",
                "--output",
            ])
            .arg(&partial)
            .arg(&url)
            .status()
            .map_err(|e| format!("curl: {e} (is it installed?)"))?;
        if !status.success() {
            return Err(format!("curl failed ({status})"));
        }
        let got = sha256(&partial)?;
        if got != want {
            let _ = std::fs::remove_file(&partial);
            return Err(format!(
                "{name}: SHA-256 is {got}, expected {want}. The file may have changed; check it and update xtask/src/dmn.rs"
            ));
        }
        std::fs::rename(&partial, &path)
            .map_err(|e| format!("rename {}: {e}", partial.display()))?;
        eprintln!("{name}: downloaded, checksum matches");
    }
    Ok(())
}
