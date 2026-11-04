//! The golden models give the pinned results on this target (native, or wasm32-wasip1 under
//! `cargo xtask golden`).

use stochcraft_testkit::{digests, pinned};

const HEADER: &str = "\
# Digests of the golden models' results (stochcraft-testkit), identical on every target.
# Rewrite with STOCHCRAFT_BLESS=1 cargo test -p stochcraft-testkit when a change is meant to
# alter results, and say why in the commit.
";

#[test]
fn golden_models_give_the_pinned_results() {
    let got = digests().unwrap_or_else(|e| format!("error: {e}\n"));
    if std::env::var("STOCHCRAFT_BLESS").is_ok_and(|v| v == "1")
        && cfg!(not(target_family = "wasm"))
    {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/golden/results.txt"
        );
        if let Err(e) = std::fs::write(path, format!("{HEADER}{got}")) {
            eprintln!("cannot write {path}: {e}");
        }
        return;
    }
    assert_eq!(
        got,
        pinned(),
        "results changed on {}. If that is intended, rewrite the digests (STOCHCRAFT_BLESS=1, natively)",
        std::env::consts::ARCH
    );
}

#[test]
fn runs_are_repeatable_within_a_target() {
    assert_eq!(digests(), digests());
}
