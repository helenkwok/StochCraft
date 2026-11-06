//! The interface's words, through craft-i18n: English source text looked up in the catalogs that
//! build.rs embeds from `CRAFT_I18N_DIR` (none without it: English only).
//!
//! The language is per thread, as egui draws on one: [`t`] and [`tf`] look up in it, and
//! [`apply_setting`] sets it from the user's choice (`None`: the system's). Numbers follow the
//! region, not the language (`App::set_number_region`).

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::{Direction, I18n, Lang};

include!(concat!(env!("OUT_DIR"), "/craft_i18n.rs"));

/// The embedded catalogs. StochCraft offers every left-to-right locale that has its own catalog;
/// right-to-left ones wait for a mirrored layout (they still serve as fallbacks).
pub static I18N: LazyLock<I18n> = LazyLock::new(|| {
    let offered: Vec<&str> = CRAFT_I18N
        .iter()
        .map(|(code, _, _)| *code)
        .filter(|code| craft_i18n::direction(code) == Direction::Ltr)
        .collect();
    // Unusable locales come back as problems; `catalogs_are_clean` lists them in a test.
    I18n::new(CRAFT_I18N, &offered).0
});

thread_local! {
    static CURRENT: Cell<Lang> = const { Cell::new(Lang::EN) };
}

/// The interface language on this thread.
pub fn lang() -> Lang {
    CURRENT.get()
}

/// Shows the interface in `lang` on this thread.
pub fn set_lang(lang: Lang) {
    CURRENT.set(lang);
}

/// Sets the language from the user's setting: a locale code, or `None` for the system's (a code
/// no longer embedded falls back to the system's too).
pub fn apply_setting(code: Option<&str>) {
    let lang = code
        .and_then(|c| I18N.from_code(c))
        .unwrap_or_else(|| I18N.system_lang());
    set_lang(lang);
}

/// `source` (English) in the interface language.
pub fn t(source: &'static str) -> &'static str {
    I18N.t(lang(), source)
}

/// Text known only at run time (an engine command's label, an example's title) in the interface
/// language.
pub fn td(source: &str) -> String {
    I18N.t(lang(), source).to_owned()
}

/// `source` with a context (`menu`, `@msg`…) in the interface language.
pub fn tc(context: &str, source: &'static str) -> &'static str {
    I18N.tc(lang(), context, source)
}

/// A template with `{name}` placeholders, translated and then filled (values isolated where the
/// text mixes directions).
pub fn tf(template: &'static str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(t(template), args)
}

/// A count in words: `one` and `other` are the English forms, `{n}` the count.
pub fn tn(n: u64, one: &'static str, other: &'static str) -> String {
    I18N.trn(lang(), n, one, other)
}

/// [`tn`] with more `{name}` placeholders filled after the count.
pub fn tnf(n: u64, one: &'static str, other: &'static str, args: &[(&str, &str)]) -> String {
    craft_i18n::fmt(&tn(n, one, other), args)
}

/// The languages to choose from, with their names in their own language, English first. Only
/// English without catalogs.
pub fn languages() -> Vec<(Lang, String)> {
    I18N.languages()
        .into_iter()
        .map(|l| (l, I18N.name(l).to_owned()))
        .collect()
}

/// A language's code, as saved in the preferences.
pub fn code(lang: Lang) -> &'static str {
    I18N.code(lang)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalogs_are_clean() {
        assert_eq!(I18N.check_all(), Vec::<String>::new());
    }

    #[test]
    fn english_is_the_source_text() {
        set_lang(Lang::EN);
        assert_eq!(t("Open…"), "Open…");
        assert_eq!(
            tf("Opened {file}", &[("file", "mm1.bpmn")]),
            "Opened mm1.bpmn"
        );
        assert_eq!(
            tn(1, "{n} replication", "{n} replications"),
            "1 replication"
        );
        assert_eq!(languages().first().map(|l| l.0), Some(Lang::EN));
    }

    #[test]
    fn the_pseudo_locale_marks_what_was_looked_up() {
        set_lang(Lang::PSEUDO);
        assert!(t("Open…").contains('⟦'), "{}", t("Open…"));
        set_lang(Lang::EN);
    }
}
