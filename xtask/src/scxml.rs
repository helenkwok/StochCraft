// Copyright (c) 2026 StochCraft contributors.
// Licensed under the MIT License or the Apache License, Version 2.0, at your option.

//! `cargo xtask fetch-scxml`: downloads W3C's XML Schema for SCXML 1.0 (its driver, the modules
//! it includes and the schema of the XML namespace it imports) into `target/scxml/`, for the test
//! that every state chart StochCraft writes is valid SCXML (`crates/io/tests/scxml.rs`, ROADMAP
//! M4.3). W3C's notice allows copying the schema, but like the BPSim files it is fetched rather
//! than committed, and checked against pinned SHA-256 checksums, so a changed file is refused.

use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::root;

/// (URL, file name, SHA-256) of each download.
pub const FILES: [(&str, &str, &str); 8] = [
    (
        "https://www.w3.org/2011/04/SCXML/scxml.xsd",
        "scxml.xsd",
        "98e14d0e03d5f9bb49b03fe4a2c24d72c165789d68fe93eee24fc403e4b23b48",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-module-core.xsd",
        "scxml-module-core.xsd",
        "cbe0cde7a2cc165b84263e0dad45abd74296ed99a9196611e4b0c98b494222e4",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-module-data.xsd",
        "scxml-module-data.xsd",
        "c34438431aa5d9815a63a64415e3127b832abd451ca509ff79a89d529dddba47",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-module-external.xsd",
        "scxml-module-external.xsd",
        "899b13488332a6b37700ea53766e16d39b1b4f38fb829d0d3b12f0955a08057d",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-datatypes.xsd",
        "scxml-datatypes.xsd",
        "51036cac697db9be6f92ad853215dc3166990ca292786fed1612d0666932163b",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-attribs.xsd",
        "scxml-attribs.xsd",
        "b74de05e9e8d86123a4e8cca6655761fae78009e8d63b95cc27ae38222a2de01",
    ),
    (
        "https://www.w3.org/2011/04/SCXML/scxml-contentmodels.xsd",
        "scxml-contentmodels.xsd",
        "704baaf6e19c480fa76039390dc0a0e045f5544b5f68ec294c47fe51d40d0053",
    ),
    // The schema of the XML namespace, which SCXML's imports.
    (
        "https://www.w3.org/2001/xml.xsd",
        "xml.xsd",
        "61960fb3131e38022caad5360e2f33a3382578ab3c80cd58bd74320ede61b20c",
    ),
];

pub fn dir() -> PathBuf {
    root().join("target").join("scxml")
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
    for (url, name, want) in FILES {
        let path = dir.join(name);
        if path.exists() && sha256(&path)? == want {
            eprintln!("{name}: present, checksum matches");
            continue;
        }
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
            .arg(url)
            .status()
            .map_err(|e| format!("curl: {e} (is it installed?)"))?;
        if !status.success() {
            return Err(format!("curl failed ({status})"));
        }
        let got = sha256(&partial)?;
        if got != want {
            let _ = std::fs::remove_file(&partial);
            return Err(format!(
                "{name}: SHA-256 is {got}, expected {want}. W3C may have changed the file; check it and update xtask/src/scxml.rs"
            ));
        }
        std::fs::rename(&partial, &path)
            .map_err(|e| format!("rename {}: {e}", partial.display()))?;
        eprintln!("{name}: downloaded, checksum matches");
    }
    Ok(())
}
