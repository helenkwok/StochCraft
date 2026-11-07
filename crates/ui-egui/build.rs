//! Two optional build inputs, each named by an environment variable and empty when it is unset:
//!
//! - **Fonts** (`CRAFT_FONTS_DIR`, a checkout of https://github.com/storytold/craft-fonts): the
//!   faces for the scripts IBM Plex doesn't cover (Japanese, Korean, Chinese, Arabic, Hebrew,
//!   Thai, Devanagari, Georgian, symbols). `CRAFT_FONTS_REQUIRED=1` makes a missing checkout an
//!   error. Unset, the table is empty and the app still builds.
//!   - Desktop: every face but the CJK ones is embedded. The CJK faces are 5–17 MB each, so
//!     Japanese, Korean and Chinese come from the installed system fonts, as do the scripts
//!     craft-fonts has no face for yet.
//!   - Web: nothing is embedded. The Regular and Bold faces, without the document serif faces
//!     (Shippori Mincho, BIZ UDMincho), are copied into `web-fonts/` at the workspace root
//!     (gitignored) and listed as `ServedFace`s. Trunk copies that directory into the site as
//!     `fonts/` (Trunk.toml), and craft-ui fetches a face when the interface language or a
//!     character needs it. A script with no served face (Bengali, Tamil, Telugu and Lao for now)
//!     is left to the browser's own fonts (egui's web glyph fallback): it draws only where the
//!     visitor's system has a font for it, in that font's style, and as boxes elsewhere.
//!     `STOCHCRAFT_PAGES_PREVIEW=1` builds a pull-request preview, published at `pr/<n>/`: it
//!     fetches the faces from the main site's copy (`../../fonts/`), and nothing is copied into
//!     `web-fonts/` (pages.yml).
//! - **Translations**: StochCraft's own catalogs, `i18n/<locale>.tsv` in this repository, always
//!   embedded (on the web too), in craft-i18n's catalog format. With `CRAFT_I18N_DIR` naming a
//!   checkout of https://github.com/storytold/craft-i18n, each locale's shared `common.tsv` is
//!   embedded beside it, for the words craft-libs' dialogs use (key names, say); StochCraft's own
//!   catalog wins where both have a row. `CRAFT_I18N_REQUIRED=1` makes a missing checkout an
//!   error.

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    fonts();
    translations();
}

/// Writes `craft_fonts.rs`: the craft-fonts faces, as `EmbeddedFace`s on the desktop and as
/// `ServedFace`s on the web.
fn fonts() {
    let written = if craft_fonts_build::target_is_wasm() {
        println!("cargo::rerun-if-env-changed=STOCHCRAFT_PAGES_PREVIEW");
        let preview = std::env::var("STOCHCRAFT_PAGES_PREVIEW").is_ok_and(|v| v == "1");
        let fonts = craft_fonts_build::Embed::new().filter(|face| {
            !matches!(face.family.as_str(), "Shippori Mincho" | "BIZ UDMincho")
                && matches!(face.style.as_str(), "Regular" | "Bold")
        });
        let fonts = if preview {
            // The main site serves the faces; the copy `serve` makes stays in the build directory.
            let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap_or_default());
            fonts.serve(out.join("web-fonts"), "../../fonts/")
        } else {
            let package = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default());
            fonts.serve(package.join("../../web-fonts"), "fonts/")
        };
        fonts.write("craft_fonts.rs")
    } else {
        craft_fonts_build::Embed::new()
            .filter(|face| !face.is_cjk())
            .write("craft_fonts.rs")
    };
    if let Err(e) = written {
        println!("cargo::error={e}");
    }
}

/// Writes `craft_i18n.rs`: `pub static CRAFT_I18N: &craft_i18n::Embedded`.
fn translations() {
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_DIR");
    println!("cargo::rerun-if-env-changed=CRAFT_I18N_REQUIRED");
    let required = std::env::var("CRAFT_I18N_REQUIRED").is_ok_and(|v| v == "1");
    // StochCraft's own catalogs, in the repository: `i18n/<locale>.tsv`.
    let own = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap_or_default())
        .join("../../i18n");
    println!("cargo::rerun-if-changed={}", own.display());
    let shared = match std::env::var_os("CRAFT_I18N_DIR").map(PathBuf::from) {
        Some(dir) if dir.join("locales").is_dir() => Some(dir.join("locales")),
        Some(dir) => {
            complain(
                required,
                &format!(
                    "CRAFT_I18N_DIR ({}) has no locales/; the shared catalogs are left out",
                    dir.display()
                ),
            );
            None
        }
        None if required => {
            complain(true, "CRAFT_I18N_REQUIRED=1 but CRAFT_I18N_DIR is unset");
            None
        }
        None => None,
    };
    let mut entries: Vec<(String, Option<PathBuf>, PathBuf)> = std::fs::read_dir(&own)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|f| {
            let path = f.path();
            let code = path.file_name()?.to_str()?.strip_suffix(".tsv")?.to_owned();
            println!("cargo::rerun-if-changed={}", path.display());
            let common = shared
                .as_ref()
                .map(|l| l.join(&code).join("common.tsv"))
                .filter(|p| p.is_file());
            if let Some(c) = &common {
                println!("cargo::rerun-if-changed={}", c.display());
            }
            Some((code, common, path))
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
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

fn complain(required: bool, message: &str) {
    if required {
        println!("cargo::error={message}");
    } else {
        println!("cargo::warning={message}");
    }
}
