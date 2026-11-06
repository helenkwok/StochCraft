//! Two optional build inputs, each named by an environment variable and empty when it is unset:
//!
//! - **Fonts** (`CRAFT_FONTS_DIR`, a checkout of https://github.com/storytold/craft-fonts): the
//!   faces for the scripts IBM Plex doesn't cover (Arabic, Hebrew, Thai, Devanagari…). Unset, the
//!   desktop app falls back to the installed system fonts. Left out: the CJK faces (tens of
//!   megabytes uncut; cut to the translation catalogs once StochCraft has some), and everything on
//!   the web, whose download has a size budget.
//! - **Translations** (`CRAFT_I18N_DIR`, a checkout of https://github.com/storytold/craft-i18n):
//!   one entry per locale that has StochCraft's own catalog (`locales/<locale>/stochcraft.tsv`),
//!   with the shared `common.tsv` beside it; a locale with only shared strings is left out, so the
//!   app never offers a language it has barely been translated into. Unset, StochCraft is
//!   English only. `CRAFT_I18N_REQUIRED=1` makes a missing checkout an error, for releases. The
//!   recipe is craft-i18n's `docs/integration.md`.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// The app's catalog name in craft-i18n's `locales/<locale>/`.
const APP: &str = "stochcraft";

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = craft_fonts_build::target_is_wasm();
    let embedded = craft_fonts_build::Embed::new()
        .filter(move |face| !wasm && !face.is_cjk())
        .write("craft_fonts.rs");
    if let Err(e) = embedded {
        println!("cargo::error={e}");
    }
    translations();
}

/// Writes `craft_i18n.rs`: `pub static CRAFT_I18N: &craft_i18n::Embedded`.
fn translations() {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v == "1");
    let mut entries = Vec::new();
    match std::env::var_os("CRAFT_I18N_DIR").map(PathBuf::from) {
        Some(dir) if dir.join("locales").is_dir() => {
            let locales = dir.join("locales");
            println!("cargo::rerun-if-changed={}", locales.display());
            entries = catalogs(&locales);
        }
        Some(dir) => complain(
            required,
            &format!(
                "CRAFT_I18N_DIR ({}) has no locales/; English only",
                dir.display()
            ),
        ),
        None if required => complain(true, "CRAFT_I18N_REQUIRED=1 but CRAFT_I18N_DIR is unset"),
        None => {}
    }
    let mut src = String::from(
        "/// (locale, shared catalog, StochCraft's catalog), one entry per locale (build.rs).\n\
         pub static CRAFT_I18N: &craft_i18n::Embedded = &[\n",
    );
    for (code, common, app) in &entries {
        let common = common.as_ref().map_or_else(
            || "\"\"".to_owned(),
            |p| format!("include_str!({:?})", p.display().to_string()),
        );
        let _ = writeln!(
            src,
            "    ({code:?}, {common}, include_str!({:?})),",
            app.display().to_string()
        );
    }
    src.push_str("];\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default()).join("craft_i18n.rs");
    if let Err(e) = std::fs::write(&out, src) {
        println!("cargo::error=writing {}: {e}", out.display());
    }
}

/// The locales with StochCraft's catalog, sorted by code: (code, common.tsv if any, app catalog).
fn catalogs(locales: &Path) -> Vec<(String, Option<PathBuf>, PathBuf)> {
    let Ok(dirs) = std::fs::read_dir(locales) else {
        return Vec::new();
    };
    let mut entries: Vec<_> = dirs
        .flatten()
        .filter_map(|d| {
            let code = d.file_name().to_str()?.to_owned();
            let app = d.path().join(format!("{APP}.tsv"));
            if !app.is_file() {
                return None;
            }
            let common = Some(d.path().join("common.tsv")).filter(|p| p.is_file());
            for p in [Some(&app), common.as_ref()].into_iter().flatten() {
                println!("cargo::rerun-if-changed={}", p.display());
            }
            Some((code, common, app))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

fn complain(required: bool, message: &str) {
    if required {
        println!("cargo::error={message}");
    } else {
        println!("cargo::warning={message}");
    }
}
