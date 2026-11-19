//! Preferences: what the app remembers between sessions, the interface language, the platform's
//! window and menu settings, and the keymap.

use serde_json::Value;

use crate::{App, Appearance, Files, Keyboard, Prefs, RECENT_FILES, commands, form, i18n};

impl App {
    /// What to remember between sessions.
    pub fn prefs(&self) -> Prefs {
        Prefs {
            appearance: Appearance {
                theme: self.theme_choice,
                language: self.language.clone(),
                colors: self.colors.clone(),
                chart_palette: self.chart_palette.clone(),
                custom_themes: self.custom_themes.clone(),
                custom_palettes: self.custom_palettes.clone(),
            },
            files: Files {
                recent: self.recent.clone(),
            },
            keyboard: Keyboard {
                shortcuts: serde_json::to_value(self.keymap.overrides())
                    .ok()
                    .filter(|v| match v {
                        Value::Object(m) => !m.is_empty(),
                        Value::Array(a) => !a.is_empty(),
                        _ => false,
                    }),
            },
        }
    }

    /// Restores what was remembered.
    pub fn set_prefs(&mut self, prefs: &Prefs) {
        self.theme_choice = prefs.appearance.theme;
        self.colors.clone_from(&prefs.appearance.colors);
        self.custom_themes
            .clone_from(&prefs.appearance.custom_themes);
        self.chart_palette
            .clone_from(&prefs.appearance.chart_palette);
        self.custom_palettes
            .clone_from(&prefs.appearance.custom_palettes);
        self.set_language(prefs.appearance.language.as_deref());
        self.recent = prefs.files.recent.clone();
        self.recent.truncate(RECENT_FILES);
        let overrides = prefs
            .keyboard
            .shortcuts
            .clone()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        self.keymap.set_overrides(overrides);
    }

    /// Shows the interface in the language with this code, or the system's (`None`, or a code no
    /// longer embedded).
    pub fn set_language(&mut self, code: Option<&str>) {
        i18n::apply_setting(code);
        self.lang = i18n::lang();
        // Kept as chosen, so a build without that language doesn't forget it.
        self.language = code.map(str::to_owned);
    }

    /// The interface language (the pseudo locale in tests).
    pub fn set_lang(&mut self, lang: craft_i18n::Lang) {
        self.lang = lang;
        i18n::set_lang(lang);
    }

    /// Settings ▸ Keyboard Shortcuts…: craft-keymap's dialog over the app's keymap.
    pub(crate) fn open_shortcuts(&mut self) {
        if self.shortcuts_dialog.is_none() {
            self.shortcuts_dialog = Some(craft_keymap::ui::DialogState::new());
        }
    }

    /// The shell shows the menus in the system's menu bar, so the window doesn't (macOS).
    pub fn set_native_menu(&mut self, on: bool) {
        self.native_menu = on;
    }

    /// Who draws the window frame (craft-window's decision): when it is the app, the header acts
    /// as the title bar, with the window buttons where this desktop puts them.
    pub fn set_window_frame(&mut self, frame: &craft_window::Frame) {
        self.title_bar = frame.layout().cloned();
    }

    /// The platform the menus are laid out for: the browser's own layout (no Quit, the
    /// browser's shortcuts left to it) or this desktop's.
    pub fn menu_target(&self) -> craft_menu::Target {
        if self.platform.can_quit {
            craft_menu::Target::current()
        } else {
            craft_menu::Target::Web
        }
    }

    /// Writes and reads numbers in the fields as `tag`'s region does (`de-CH`, `ar-EG`), in place
    /// of the system's region.
    pub fn set_number_region(&mut self, tag: &str) {
        self.numbers = form::messages(craft_ui::localized::LocaleNumbers::from_tag(tag));
    }

    /// A command's shortcut as this platform writes it ("F5", "⌘O", "Ctrl+O").
    pub fn shortcut(&self, command: &str, ctx: &egui::Context) -> Option<String> {
        commands::shortcut_text(&self.keymap, command, craft_keymap::egui::platform(ctx))
    }
}
