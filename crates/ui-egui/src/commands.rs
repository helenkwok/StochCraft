//! The app's commands, their default shortcuts and its menus, on craft-keymap and craft-menu.
//!
//! Every menu item and shortcut runs a [`CommandRef`] through [`App::run_command`], so the
//! in-window menu bar, the macOS menu bar (the desktop shell's [`craft_menu::NativeMenu`]) and the
//! keyboard all do the same thing. The keymap holds the default bindings; the layouts put About,
//! Settings and Quit where each platform keeps them, and the web layout leaves the browser's own
//! shortcuts (Ctrl+N, Ctrl+W…) to the browser.

use craft_keymap::{Bindable, Binding, Keymap, Platform, Scope, Scopes};
use craft_menu::{
    Item, ItemRole, ItemState, LayoutOptions, Menu, MenuBar, MenuRole, Node, Platforms, Target,
};
use serde_json::json;

pub use craft_keymap::CommandRef;

use crate::{App, EXAMPLES, Replace, ThemeChoice};

pub const NEW: &str = "file.new";
pub const OPEN: &str = "file.open";
pub const OPEN_EXAMPLE: &str = "file.open_example";
pub const OPEN_RECENT: &str = "file.open_recent";
pub const CLEAR_RECENT: &str = "file.clear_recent";
pub const SAVE: &str = "file.save";
pub const SAVE_AS: &str = "file.save_as";
pub const QUIT: &str = "app.quit";
pub const UNDO: &str = "edit.undo";
pub const REDO: &str = "edit.redo";
pub const RUN: &str = "model.run";
pub const CANCEL_RUN: &str = "model.cancel_run";
pub const THEME: &str = "view.theme";
pub const ABOUT: &str = "help.about";

/// The commands that can have shortcuts, with their defaults (`Cmd` is Ctrl outside macOS).
fn catalog() -> Vec<Bindable> {
    let app = Scope::app();
    let b = |id: &str, label: &str, keys: &[&str]| {
        keys.iter().filter_map(|k| Binding::parse(k).ok()).fold(
            Bindable::new(id, label, app.clone()),
            Bindable::with_default,
        )
    };
    vec![
        b(NEW, "New", &["Cmd+N"]),
        b(OPEN, "Open…", &["Cmd+O"]),
        b(SAVE, "Save", &["Cmd+S"]),
        b(SAVE_AS, "Save As…", &["Cmd+Shift+S"]),
        b(QUIT, "Quit", &["Cmd+Q"]),
        b(UNDO, "Undo", &["Cmd+Z"]),
        b(REDO, "Redo", &["Cmd+Shift+Z", "Cmd+Y"]),
        b(RUN, "Run", &["F5"]),
        b(CANCEL_RUN, "Cancel Run", &[]),
        b(ABOUT, "About StochCraft", &[]),
    ]
}

/// The keymap with the default bindings, for `platform`.
pub fn keymap(platform: Platform) -> Keymap {
    Keymap::new(catalog(), Scopes::new(), platform)
}

/// The command's shortcut as the platform writes it ("F5", "⌘O", "Ctrl+O"), if it has one.
pub fn shortcut_text(keymap: &Keymap, id: &str, platform: Platform) -> Option<String> {
    keymap
        .bindings(&CommandRef::new(id))
        .first()
        .map(|b| b.trigger.display(platform))
}

fn item(id: &str, label: &str) -> Node {
    Node::Item(Item::new(id, label))
}

impl App {
    /// The app's menus for the current state, with the keymap's shortcuts and each item's
    /// enabled and checked state. Lay it out with [`App::menu_layout`].
    pub fn menu_bar(&mut self) -> MenuBar {
        let save = if self.platform.file_system {
            "Save"
        } else {
            "Download"
        };
        let mut recent: Vec<Node> = self
            .recent
            .iter()
            .enumerate()
            .map(|(i, path)| {
                Node::Item(Item::new(
                    CommandRef::with_params(OPEN_RECENT, json!({ "index": i })),
                    crate::file_name(path),
                ))
            })
            .collect();
        if !recent.is_empty() {
            recent.push(Node::Separator);
        }
        recent.push(item(CLEAR_RECENT, "Clear Menu"));
        let examples = EXAMPLES
            .iter()
            .enumerate()
            .map(|(i, e)| {
                Node::Item(Item::new(
                    CommandRef::with_params(OPEN_EXAMPLE, json!({ "index": i })),
                    e.title,
                ))
            })
            .collect();
        let file = Menu::new(
            "File",
            vec![
                item(NEW, "New"),
                item(OPEN, "Open…"),
                Node::submenu("Open Recent", recent)
                    .platforms(Platforms::ALL.without(Platforms::WEB)),
                Node::submenu("Open Example", examples),
                Node::Separator,
                item(SAVE, save),
                Node::Item(
                    Item::new(SAVE_AS, "Save As…")
                        .platforms(Platforms::ALL.without(Platforms::WEB)),
                ),
                Node::Separator,
                Node::Item(Item::new(QUIT, "Quit").role(ItemRole::Quit)),
            ],
        )
        .role(MenuRole::File);

        let (undo, redo) = self.engine_history();
        let edit = Menu::new(
            "Edit",
            vec![
                Node::Item(Item::new(
                    UNDO,
                    undo.as_deref()
                        .map_or("Undo".into(), |l| format!("Undo {l}")),
                )),
                Node::Item(Item::new(
                    REDO,
                    redo.as_deref()
                        .map_or("Redo".into(), |l| format!("Redo {l}")),
                )),
            ],
        )
        .role(MenuRole::Edit);

        let theme = self.theme_choice;
        let appearance = Node::submenu(
            "Appearance",
            [
                (ThemeChoice::System, "Use System Setting"),
                (ThemeChoice::Light, "Light"),
                (ThemeChoice::Dark, "Dark"),
            ]
            .into_iter()
            .map(|(t, label)| {
                Node::Item(
                    Item::new(CommandRef::with_params(THEME, json!(t.key())), label)
                        .radio(theme == t),
                )
            })
            .collect(),
        )
        .role(ItemRole::Appearance);
        let view = Menu::new("View", vec![appearance]).role(MenuRole::View);

        let model = Menu::new(
            "Model",
            vec![item(RUN, "Run"), item(CANCEL_RUN, "Cancel Run")],
        );
        let help = Menu::new(
            "Help",
            vec![Node::Item(
                Item::new(ABOUT, "About StochCraft").role(ItemRole::About),
            )],
        )
        .role(MenuRole::Help);

        let mut bar = MenuBar {
            menus: vec![file, edit, view, model, help],
            ..MenuBar::default()
        };
        craft_menu::apply_keymap(&mut bar, &self.keymap);
        let can_undo = undo.is_some();
        let can_redo = redo.is_some();
        craft_menu::apply_state(&mut bar, |c| ItemState {
            enabled: self.enabled(c, can_undo, can_redo),
            ..ItemState::default()
        });
        bar
    }

    /// Whether a command can run now (with what Undo and Redo would do).
    fn enabled(&self, command: &CommandRef, can_undo: bool, can_redo: bool) -> bool {
        if self.modal_open() {
            return false;
        }
        let running = self.running.is_some();
        match command.id.as_str() {
            UNDO => can_undo,
            REDO => can_redo,
            RUN => !running && !self.engine.model().elements.is_empty(),
            CANCEL_RUN => running,
            CLEAR_RECENT => !self.recent.is_empty(),
            _ => true,
        }
    }

    /// The menus laid out for `target`: About, Settings and Quit where the platform keeps them,
    /// and on the web without the shortcuts the browser keeps for itself.
    pub fn menu_layout(&mut self, target: Target) -> craft_menu::Layout {
        let bar = self.menu_bar();
        craft_menu::layout(&bar, target, &LayoutOptions::new("StochCraft"))
    }

    /// A dialog is open: menu commands wait.
    pub(crate) fn modal_open(&self) -> bool {
        self.confirm.is_some()
    }

    /// Runs a command from a menu, the macOS menu bar or a shortcut.
    pub fn run_command(&mut self, command: &CommandRef) {
        let index = || {
            command.params["index"]
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
        };
        match command.id.as_str() {
            NEW => self.replace(Replace::New),
            OPEN => self.replace(Replace::Open),
            OPEN_EXAMPLE => {
                if let Some(i) = index() {
                    self.replace(Replace::Example(i));
                }
            }
            OPEN_RECENT => {
                if let Some(i) = index().filter(|i| *i < self.recent.len()) {
                    self.replace(Replace::Recent(i));
                }
            }
            CLEAR_RECENT => self.recent.clear(),
            SAVE => self.save(),
            SAVE_AS => self.save_as(),
            QUIT => {
                if self.platform.can_quit {
                    self.replace(Replace::Quit);
                }
            }
            UNDO => self.undo(),
            REDO => self.redo(),
            RUN => self.run(),
            CANCEL_RUN => self.cancel_run(),
            THEME => {
                if let Some(t) = command.params.as_str().and_then(ThemeChoice::from_key) {
                    self.theme_choice = t;
                }
            }
            ABOUT => self.about = true,
            _ => {}
        }
    }

    /// Runs the shortcuts pressed this frame (not while a dialog is open). A text field keeps
    /// its own keys (⌘Z undoes typing, not the model).
    pub(crate) fn dispatch_shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal_open() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let active = craft_keymap::Active {
            typing,
            ..craft_keymap::Active::NONE
        };
        let hits = ctx.input_mut(|input| self.dispatcher.dispatch(input, &self.keymap, &active));
        for hit in hits {
            self.run_command(&hit.command);
        }
    }
}
