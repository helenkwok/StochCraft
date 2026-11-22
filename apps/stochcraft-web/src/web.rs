//! The browser shell: the page's canvas, the file picker, downloads and dropped files, web
//! workers for runs and the service worker for offline use.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_ui_egui::{App, OpenTicket, Platform, Request};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

use crate::{schedule, storage};

const CANVAS_ID: &str = "stochcraft_canvas";
const LOADING_ID: &str = "stochcraft_loading";
/// The page's visually hidden `aria-live` region (index.html), which screen readers announce.
const STATUS_ID: &str = "stochcraft_status";

/// Files read in the background, waiting for the next frame: the ticket the app gave when the
/// read began (it says whether the file is still wanted when it arrives), whether each was
/// dropped (rather than picked after File ▸ Open), its name and contents.
type Inbox = Rc<RefCell<Vec<(OpenTicket, bool, String, Vec<u8>)>>>;

struct Web {
    app: App,
    inbox: Inbox,
    /// The page's `beforeunload` guard reads this: a tab with unsaved changes asks before closing.
    unsaved: Rc<RefCell<bool>>,
    /// The prefix of this page's storage keys: a pull request's preview keeps its own
    /// ([`storage::scope`]).
    scope: Option<String>,
    /// This tab's name in the browser's storage: its recovery copy is kept under it, apart from
    /// other tabs' ([`storage::copy_key`]).
    session: String,
    /// What the live region and the page's `lang` last said, so each is set only on a change.
    announced: String,
    lang: &'static str,
}

/// Where the browser keeps the app's preferences (eframe's storage: the page's local storage).
const PREFS_KEY: &str = "stochcraft.prefs";
/// How often the tab records that it is open, in milliseconds.
const HEARTBEAT_MS: i32 = 10_000;

/// The browser's local storage, if it can be used (it can't in some private windows).
fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok().flatten()
}

/// Records that this tab is open, so other tabs leave its recovery copy alone.
fn heartbeat(scope: Option<&str>, session: &str) {
    if let Some(local) = local_storage() {
        let _ = local.set_item(
            &storage::alive_key(scope, session),
            &js_sys::Date::now().to_string(),
        );
    }
}

/// A name for this tab: the time it started (so the newest sorts last) and a random part.
fn new_session() -> String {
    format!(
        "{:013x}-{:x}",
        js_sys::Date::now() as u64,
        (js_sys::Math::random() * 4_294_967_296.0) as u64
    )
}

/// The recovery copies left by tabs that are gone (and the one shared copy older versions kept),
/// newest first: each is its slot's name and the copy. A tab's copy is left alone while the tab
/// is still open.
fn orphans(local: &web_sys::Storage, scope: Option<&str>, own: &str) -> Vec<(String, String)> {
    let now = js_sys::Date::now();
    let mut slots: Vec<String> = (0..local.length().unwrap_or(0))
        .filter_map(|i| local.key(i).ok().flatten())
        .filter_map(|k| storage::copy_session(scope, &k).map(str::to_owned))
        .filter(|session| session != own)
        .filter(|session| {
            let beat = local
                .get_item(&storage::alive_key(scope, session))
                .ok()
                .flatten()
                .and_then(|v| v.parse::<f64>().ok());
            !storage::session_alive(beat, now)
        })
        .collect();
    // The names start with the time the tab began.
    slots.sort();
    slots.reverse();
    slots.push(storage::LEGACY_SLOT.to_owned());
    slots
        .into_iter()
        .filter_map(|slot| {
            let copy = local
                .get_item(&storage::slot_key(scope, &slot))
                .ok()
                .flatten()
                .filter(|c| !c.is_empty())?;
            Some((slot, copy))
        })
        .collect()
}

/// Keeps the tab's sign of life up to date while it is open, and takes it back when the page
/// goes away (its recovery copy, if it has unsaved changes, stays for the next visit).
fn keep_alive(scope: Option<String>, session: String) {
    let Some(window) = web_sys::window() else {
        return;
    };
    heartbeat(scope.as_deref(), &session);
    let (beat_scope, beat_session) = (scope.clone(), session.clone());
    let beat = Closure::<dyn FnMut()>::new(move || {
        heartbeat(beat_scope.as_deref(), &beat_session);
    });
    let _ = window.set_interval_with_callback_and_timeout_and_arguments_0(
        beat.as_ref().unchecked_ref(),
        HEARTBEAT_MS,
    );
    // Lives as long as the page.
    beat.forget();
    let leave = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        if let Some(local) = local_storage() {
            let _ = local.remove_item(&storage::alive_key(scope.as_deref(), &session));
        }
    });
    let _ = window.add_event_listener_with_callback("pagehide", leave.as_ref().unchecked_ref());
    leave.forget();
}

impl eframe::App for Web {
    /// Remembers the appearance between visits, and keeps a recovery copy of unsaved changes
    /// (eframe calls this every half minute and when the page closes).
    fn save(&mut self, store: &mut dyn eframe::Storage) {
        let scope = self.scope.as_deref();
        if let Ok(json) = serde_json::to_string(&self.app.prefs()) {
            store.set_string(&storage::key(scope, PREFS_KEY), json);
        }
        // This tab's copy is in local storage under its own key, written there directly (not
        // through eframe's storage) so a full or blocked storage is noticed and the copy
        // tried again.
        let Some(local) = local_storage() else {
            return;
        };
        let recovery = storage::copy_key(scope, &self.session);
        match self.app.autosave() {
            stochcraft_ui_egui::Autosave::Write(copy) => {
                if let Err(e) = local.set_item(&recovery, &copy) {
                    self.app.autosave_write_failed();
                    self.app.report_error(format!("autosave: {e:?}"));
                }
            }
            stochcraft_ui_egui::Autosave::Clear => {
                if local.remove_item(&recovery).is_err() {
                    self.app.autosave_clear_failed();
                }
            }
            stochcraft_ui_egui::Autosave::Keep => {}
        }
        // Copies of tabs that are gone which this tab recovered or discarded.
        for slot in self.app.take_dropped_copies() {
            let _ = local.remove_item(&storage::slot_key(scope, &slot));
        }
        heartbeat(scope, &self.session);
    }

    /// eframe keeps egui's memory (panel sizes and the like) under one key it chooses, so only
    /// the main site writes it; a preview starts from the main site's and leaves it alone.
    fn persist_egui_memory(&self) -> bool {
        self.scope.is_none()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let files: Vec<_> = self.inbox.borrow_mut().drain(..).collect();
        // A file that arrives late is opened only if it is still wanted: not after another file
        // was asked for, the model was replaced, or (for a picked file) it was edited.
        for (ticket, dropped, name, bytes) in files {
            self.app.finish_open(ticket, &name, bytes, dropped);
        }
        // Files dropped on the page are read in the background, like the picker's.
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let (inbox, ctx) = (Rc::clone(&self.inbox), ctx.clone());
            let ticket = self.app.begin_open();
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.path().to_string_lossy().into_owned();
                if let Ok(bytes) = file.bytes_async().await {
                    inbox.borrow_mut().push((ticket, true, name, bytes));
                    ctx.request_repaint();
                }
            });
        }
        self.app.ui(ui);
        for request in self.app.take_requests() {
            match request {
                Request::PickOpen => {
                    let ticket = self.app.begin_open();
                    pick(&self.inbox, &ctx, ticket);
                }
                // Started, not necessarily kept: the browser doesn't say whether the user saved
                // the file or cancelled, so the model is not marked saved.
                Request::Download { name, bytes } => match download(&name, &bytes) {
                    Ok(()) => self.app.downloaded(&name),
                    Err(e) => self.app.save_failed(format!("{name}: {e:?}")),
                },
                Request::Export { name, bytes } => match download(&name, &bytes) {
                    Ok(()) => self.app.exported(&name),
                    Err(e) => self.app.report_error(format!("{name}: {e:?}")),
                },
                Request::OpenUrl { url } => {
                    ctx.open_url(egui::OpenUrl::new_tab(url));
                }
                // Not made in the browser (Platform::BROWSER has no paths and no Quit).
                Request::PickSave { .. }
                | Request::OpenPath { .. }
                | Request::Write { .. }
                | Request::Quit => {}
            }
        }
        if let Some(title) = self.app.title_if_changed()
            && let Some(document) = web_sys::window().and_then(|w| w.document())
        {
            document.set_title(&title);
        }
        *self.unsaved.borrow_mut() = self.app.has_unsaved_changes();
        self.mirror_for_screen_readers();
    }
}

impl Web {
    /// The canvas shows no text a screen reader can read, so the status bar's message goes into
    /// the page's live region, and the page's `lang` follows the interface language.
    fn mirror_for_screen_readers(&mut self) {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let status = self.app.status_announcement();
        if status != self.announced {
            if let Some(region) = document.get_element_by_id(STATUS_ID) {
                region.set_text_content(Some(&status));
            }
            self.announced = status;
        }
        let lang = self.app.language_code();
        if lang != self.lang {
            if let Some(root) = document.document_element() {
                let _ = root.set_attribute("lang", lang);
            }
            self.lang = lang;
        }
    }
}

/// Shows the browser's file picker; the chosen file is read in the background into `inbox`, with
/// the `ticket` from when the picker was asked for.
fn pick(inbox: &Inbox, ctx: &egui::Context, ticket: OpenTicket) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let Ok(input) = document
        .create_element("input")
        .map(|e| e.unchecked_into::<web_sys::HtmlInputElement>())
    else {
        return;
    };
    input.set_type("file");
    input.set_accept(".bpmn,.xpdl,.xml,.xmile,.stmx");
    let (inbox, ctx, target) = (Rc::clone(inbox), ctx.clone(), input.clone());
    let on_change = Closure::once(move |_: web_sys::Event| {
        let Some(file) = target.files().and_then(|f| f.get(0)) else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let name = file.name();
            if let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                inbox.borrow_mut().push((ticket, false, name, bytes));
                ctx.request_repaint();
            }
        });
    });
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    // The picker calls back at most once; the closure lives until then.
    on_change.forget();
    input.click();
}

/// Offers `bytes` as a download named `name` (craft-platform's browser download).
fn download(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
    craft_platform::browser::download_bytes(
        name,
        schedule::download_mime(name),
        bytes,
        craft_platform::browser::DownloadOptions::default(),
    )
}

/// Asks before the tab closes or reloads while there are unsaved changes.
fn guard_unload(unsaved: &Rc<RefCell<bool>>) {
    let unsaved = Rc::clone(unsaved);
    let guard = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
        move |e: web_sys::BeforeUnloadEvent| {
            if *unsaved.borrow() {
                e.prevent_default();
            }
        },
    );
    if let Some(window) = web_sys::window() {
        let _ =
            window.add_event_listener_with_callback("beforeunload", guard.as_ref().unchecked_ref());
    }
    // Lives as long as the page.
    guard.forget();
}

pub fn start() {
    let unsaved = Rc::new(RefCell::new(false));
    guard_unload(&unsaved);
    let scope = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .and_then(|path| storage::scope(&path));
    let session = new_session();
    keep_alive(scope.clone(), session.clone());
    wasm_bindgen_futures::spawn_local(async move {
        let Some(document) = web_sys::window().and_then(|w| w.document()) else {
            return;
        };
        let Some(canvas) = document
            .get_element_by_id(CANVAS_ID)
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok())
        else {
            return;
        };
        let result = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(move |cc| {
                    let mut app = App::new(Platform::BROWSER);
                    app.set_workers(Box::new(crate::pool::Pool::new(cc.egui_ctx.clone())));
                    let key = |k| storage::key(scope.as_deref(), k);
                    if let Some(prefs) = cc
                        .storage
                        .and_then(|s| s.get_string(&key(PREFS_KEY)))
                        .and_then(|j| serde_json::from_str(&j).ok())
                    {
                        app.set_prefs(&prefs);
                    }
                    if let Some(local) = local_storage() {
                        for (slot, copy) in orphans(&local, scope.as_deref(), &session) {
                            app.offer_recovery(&slot, &copy);
                        }
                    }
                    Ok(Box::new(Web {
                        app,
                        inbox: Inbox::default(),
                        unsaved,
                        scope,
                        session,
                        announced: String::new(),
                        lang: "",
                    }))
                }),
            )
            .await;
        if let Some(loading) = document.get_element_by_id(LOADING_ID) {
            match result {
                Ok(()) => loading.remove(),
                Err(e) => loading.set_text_content(Some(&format!(
                    "StochCraft could not start: {}",
                    e.as_string().unwrap_or_else(|| format!("{e:?}"))
                ))),
            }
        }
    });
}
