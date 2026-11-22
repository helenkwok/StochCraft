//! Which keys the page keeps in the browser's storage.
//!
//! Every page of the site shares one origin, so one local storage: the GitHub Pages workflow
//! publishes `main` at the site's root and each pull request's preview under `pr/<number>/`. A
//! preview's keys carry its `pr/<number>/` so it neither reads nor overwrites the main site's
//! preferences and recovery copy; the main site (and `trunk serve`) keeps the bare keys it has
//! always used.

/// The prefix for this page's keys, from its URL path: `pr/<number>/` for a pull request's
/// preview, none otherwise.
pub fn scope(pathname: &str) -> Option<String> {
    let segments = pathname.split('/');
    segments
        .clone()
        .zip(segments.skip(1))
        .find_map(|(this, next)| {
            (this == "pr" && !next.is_empty() && next.bytes().all(|b| b.is_ascii_digit()))
                .then(|| format!("pr/{next}/"))
        })
}

/// `key` within `scope`.
pub fn key(scope: Option<&str>, key: &str) -> String {
    format!("{}{key}", scope.unwrap_or(""))
}

/// Where the browser keeps the recovery copy older versions kept, one for every tab.
pub const LEGACY_RECOVERY_KEY: &str = "stochcraft.recovery";
/// The slot name of that copy.
pub const LEGACY_SLOT: &str = "legacy";
/// How long a tab's last sign of life counts: a tab in the background has its timers slowed to
/// one a minute, so a few minutes without one means the tab is gone.
pub const ALIVE_FOR_MS: f64 = 180_000.0;

const COPY_PREFIX: &str = "stochcraft.recovery.";
const ALIVE_PREFIX: &str = "stochcraft.alive.";

/// Where the tab `session` keeps its recovery copy: each tab has its own, so two tabs never
/// replace or remove each other's.
pub fn copy_key(scope: Option<&str>, session: &str) -> String {
    key(scope, &format!("{COPY_PREFIX}{session}"))
}

/// Where the tab `session` records that it is open (the time, in milliseconds).
pub fn alive_key(scope: Option<&str>, session: &str) -> String {
    key(scope, &format!("{ALIVE_PREFIX}{session}"))
}

/// The key of the recovery copy in `slot` (a session's name, or [`LEGACY_SLOT`]).
pub fn slot_key(scope: Option<&str>, slot: &str) -> String {
    if slot == LEGACY_SLOT {
        key(scope, LEGACY_RECOVERY_KEY)
    } else {
        copy_key(scope, slot)
    }
}

/// The session whose recovery copy is kept under `storage_key`, if it is one of this page's
/// (a preview's keys carry its scope, so the main site does not see them, nor they its).
pub fn copy_session<'a>(scope: Option<&str>, storage_key: &'a str) -> Option<&'a str> {
    let rest = storage_key.strip_prefix(scope.unwrap_or(""))?;
    rest.strip_prefix(COPY_PREFIX).filter(|s| !s.is_empty())
}

/// Whether a tab whose last sign of life was at `beat` (none: it never gave one, or left) is
/// still open at `now`.
pub fn session_alive(beat: Option<f64>, now: f64) -> bool {
    beat.is_some_and(|beat| now - beat < ALIVE_FOR_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_main_site_keeps_its_keys() {
        for path in ["/", "/StochCraft/", "/StochCraft/index.html", ""] {
            assert_eq!(scope(path), None, "{path}");
        }
        assert_eq!(key(None, "stochcraft.prefs"), "stochcraft.prefs");
    }

    #[test]
    fn a_preview_has_its_own_keys() {
        for path in [
            "/StochCraft/pr/43/",
            "/StochCraft/pr/43/index.html",
            "/StochCraft/pr/43",
        ] {
            assert_eq!(scope(path).as_deref(), Some("pr/43/"), "{path}");
        }
        assert_eq!(
            key(Some("pr/43/"), "stochcraft.recovery"),
            "pr/43/stochcraft.recovery"
        );
        // Only a pull request's number makes a preview.
        for path in ["/StochCraft/pr/", "/StochCraft/pr/x1/", "/pr"] {
            assert_eq!(scope(path), None, "{path}");
        }
    }

    #[test]
    fn each_tab_has_its_own_recovery_key() {
        assert_eq!(copy_key(None, "18f-2a"), "stochcraft.recovery.18f-2a");
        assert_eq!(
            copy_key(Some("pr/43/"), "18f-2a"),
            "pr/43/stochcraft.recovery.18f-2a"
        );
        assert_ne!(copy_key(None, "1-a"), copy_key(None, "1-b"));
        assert_eq!(slot_key(None, LEGACY_SLOT), "stochcraft.recovery");
        assert_eq!(slot_key(None, "1-a"), copy_key(None, "1-a"));
        assert_eq!(alive_key(None, "1-a"), "stochcraft.alive.1-a");
    }

    #[test]
    fn a_page_finds_only_its_own_tabs_copies() {
        let main = copy_key(None, "1-a");
        let preview = copy_key(Some("pr/43/"), "1-a");
        assert_eq!(copy_session(None, &main), Some("1-a"));
        assert_eq!(copy_session(Some("pr/43/"), &preview), Some("1-a"));
        // Neither sees the other's.
        assert_eq!(copy_session(None, &preview), None);
        assert_eq!(copy_session(Some("pr/43/"), &main), None);
        assert_eq!(copy_session(Some("pr/44/"), &preview), None);
        // The old shared key, the preferences and the alive marks are not copies.
        for other in [
            LEGACY_RECOVERY_KEY,
            "stochcraft.prefs",
            "stochcraft.alive.1-a",
        ] {
            assert_eq!(copy_session(None, other), None, "{other}");
        }
        assert_eq!(copy_session(None, "stochcraft.recovery."), None);
    }

    #[test]
    fn a_tab_is_open_while_its_sign_of_life_is_recent() {
        let now = 1_000_000.0;
        assert!(session_alive(Some(now - 10_000.0), now));
        assert!(
            session_alive(Some(now + 5.0), now),
            "clocks differ a little"
        );
        assert!(!session_alive(Some(now - ALIVE_FOR_MS - 1.0), now));
        assert!(!session_alive(None, now), "it left, or never began");
    }
}
