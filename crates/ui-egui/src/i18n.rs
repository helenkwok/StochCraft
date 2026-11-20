//! The interface's words, through craft-i18n: English source text looked up in the catalogs that
//! build.rs embeds: StochCraft's own (`i18n/`, always), and craft-i18n's shared `common.tsv`
//! beside them when `CRAFT_I18N_DIR` is set.
//!
//! The language is per thread, as egui draws on one: [`t`] and [`tf`] look up in it, and
//! [`apply_setting`] sets it from the user's choice (`None`: the system's). Numbers follow the
//! region, not the language (`App::set_number_region`).

use std::cell::Cell;
use std::sync::LazyLock;

use craft_i18n::{Direction, I18n, Lang};
use serde_json::Value;

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

/// The templates the window words differently from the engine: the engine says "replication"
/// (as the command line, JSON and files do), the window "run". [`engine_message`] has the
/// window's English for each; every other template is the engine's
/// ([`stochcraft_model::messages::TEMPLATES`]). (The window never shows `no_warmup_at` and
/// `no_warmup_late`: the results say it in "How sure are these numbers?".)
pub const WINDOW_TEMPLATE_KEYS: &[&str] = &[
    "bad_experiment_replications",
    "stop_with_warmup",
    "ended_in_warmup_all",
    "ended_in_warmup_some",
    "one_replication",
    "behaviour_runs",
];

/// One part of an engine message, from the catalog's wording for its `key`, filled with `args`.
/// `None` for a key the window does not know.
fn message_part(key: &str, args: &[(&str, &str)]) -> Option<String> {
    Some(match key {
        "bad_experiment_replications" => tf(
            "Set the number of runs of the experiment to 1 or more.",
            args,
        ),
        "stop_with_warmup" => tf(
            "The stop condition at {block} can be met before the warm-up ends. A run that stops during the warm-up has no statistics after it and is left out of the results. Shorten the warm-up, or stop later.",
            args,
        ),
        "ended_in_warmup_all" => tf(
            "Every run stopped before the warm-up ended, so no metric is reported. Shorten the warm-up, or stop later.",
            args,
        ),
        "ended_in_warmup_some" => tf(
            "{replications} of {of} runs stopped before the warm-up ended, so their statistics are left out of every metric. Shorten the warm-up, or stop later.",
            args,
        ),
        "one_replication" => tf(
            "The experiment has one run, so its results cannot be told from chance. Set the number of runs to at least 2, and usually 10 or more.",
            args,
        ),
        "behaviour_runs" => tf(
            "This was found in {replications} of {of} runs, and the furthest off is shown.",
            args,
        ),
        _ => tf(stochcraft_model::messages::template(key)?, args),
    })
}

/// A diagnostic from the engine in the interface language, without its severity word (the
/// window shows that as a label): each part of its message is built from the catalog's wording
/// for the part's key, filled with the part's arguments (labels of the window, in the
/// arguments named in [`stochcraft_model::messages::LABEL_ARGS`], translated first). A
/// diagnostic without parts, or with a key the window does not know, shows the engine's English.
pub fn engine_message(d: &Value) -> String {
    let english = || {
        let m = d["message"].as_str().unwrap_or("");
        ["error: ", "warning: "]
            .iter()
            .find_map(|head| m.strip_prefix(head))
            .unwrap_or(m)
            .to_owned()
    };
    let Some(parts) = d["parts"].as_array().filter(|p| !p.is_empty()) else {
        return english();
    };
    let mut out: Vec<String> = Vec::new();
    for part in parts {
        let args: Vec<(String, String)> = part["args"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(name, value)| {
                let value = value.as_str().unwrap_or("");
                let value = if stochcraft_model::messages::LABEL_ARGS.contains(&name.as_str()) {
                    td(value)
                } else {
                    value.to_owned()
                };
                (name.clone(), value)
            })
            .collect();
        let args: Vec<(&str, &str)> = args
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
            .collect();
        match part["key"]
            .as_str()
            .and_then(|key| message_part(key, &args))
        {
            Some(text) => out.push(text),
            None => return english(),
        }
    }
    out.join(" ")
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

/// A language's name in English, which View ▸ Language shows on the web until the font that
/// draws the name in its own script has been fetched (so opening the menu fetches no fonts). The
/// code itself for a locale this table doesn't know (a test keeps it complete).
pub fn english_name(lang: Lang) -> &'static str {
    match code(lang) {
        "en" => "English",
        "bg" => "Bulgarian",
        "bn" => "Bengali",
        "bs" => "Bosnian",
        "ca" => "Catalan",
        "cs" => "Czech",
        "da" => "Danish",
        "de" => "German",
        "el" => "Greek",
        "es" => "Spanish",
        "et" => "Estonian",
        "fi" => "Finnish",
        "fr" => "French",
        "gl" => "Galician",
        "hi" => "Hindi",
        "hr" => "Croatian",
        "hu" => "Hungarian",
        "id" => "Indonesian",
        "it" => "Italian",
        "ja" => "Japanese",
        "ka" => "Georgian",
        "kk" => "Kazakh",
        "ko" => "Korean",
        "lo" => "Lao",
        "lt" => "Lithuanian",
        "nb" => "Norwegian Bokmål",
        "nl" => "Dutch",
        "pl" => "Polish",
        "pt-br" => "Portuguese (Brazil)",
        "ro" => "Romanian",
        "ru" => "Russian",
        "sk" => "Slovak",
        "sl" => "Slovenian",
        "sq" => "Albanian",
        "sr-latn" => "Serbian (Latin)",
        "sr" => "Serbian (Cyrillic)",
        "sv" => "Swedish",
        "ta" => "Tamil",
        "te" => "Telugu",
        "th" => "Thai",
        "tr" => "Turkish",
        "uk" => "Ukrainian",
        "vi" => "Vietnamese",
        "zh-hans" => "Chinese (Simplified)",
        "zh-hant" => "Chinese (Traditional)",
        other => other,
    }
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
        assert_eq!(tn(1, "{n} run", "{n} runs"), "1 run");
        assert_eq!(languages().first().map(|l| l.0), Some(Lang::EN));
    }

    #[test]
    fn every_language_has_an_english_name() {
        for (lang, _) in languages() {
            assert_ne!(english_name(lang), code(lang), "no English name");
        }
        assert_eq!(english_name(Lang::EN), "English");
    }

    #[test]
    fn the_pseudo_locale_marks_what_was_looked_up() {
        set_lang(Lang::PSEUDO);
        assert!(t("Open…").contains('⟦'), "{}", t("Open…"));
        set_lang(Lang::EN);
    }
}
