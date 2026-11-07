//! StochCraft in the browser: the shared interface ([`stochcraft_ui_egui::App`]) through eframe's
//! web runner (WebGL2). There is no file system: File ▸ Open uses the browser's file picker,
//! saving downloads the model, and a model file dropped on the page opens.
//!
//! Build with `trunk build --release` from this directory (output in `dist/web`). The GitHub Pages
//! workflow publishes `main` at the site's root and each pull request under `pr/<number>/`.
//!
//! On other targets this binary only says so, which keeps `cargo build --workspace` working.

#[cfg(any(target_arch = "wasm32", test))]
mod storage;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("stochcraft-web runs in a browser: build it with `trunk build` (see Trunk.toml)");
}
