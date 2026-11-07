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
}
