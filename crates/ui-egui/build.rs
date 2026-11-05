//! Embeds craft-fonts' faces for the scripts IBM Plex doesn't cover (Arabic, Hebrew, Thai,
//! Devanagari…) when `CRAFT_FONTS_DIR` names a checkout of https://github.com/storytold/craft-fonts.
//! Unset, the table is empty and the desktop app falls back to the installed system fonts.
//!
//! Left out: the CJK faces (tens of megabytes uncut; cut to the translation catalogs once
//! StochCraft has some), and everything on the web, whose download has a size budget.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    let wasm = craft_fonts_build::target_is_wasm();
    let embedded = craft_fonts_build::Embed::new()
        .filter(move |face| !wasm && !face.is_cjk())
        .write("craft_fonts.rs");
    if let Err(e) = embedded {
        println!("cargo::error={e}");
    }
}
