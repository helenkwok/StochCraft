//! The browser shell: the page's canvas, the file picker, downloads and dropped files.

use std::cell::RefCell;
use std::rc::Rc;

use stochcraft_ui_egui::{App, Platform, Request};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::prelude::*;

const CANVAS_ID: &str = "stochcraft_canvas";
const LOADING_ID: &str = "stochcraft_loading";

/// Files read in the background, waiting for the next frame: whether each was dropped (rather
/// than picked after File ▸ Open), its name and contents.
type Inbox = Rc<RefCell<Vec<(bool, String, Vec<u8>)>>>;

struct Web {
    app: App,
    inbox: Inbox,
    /// The page's `beforeunload` guard reads this: a tab with unsaved changes asks before closing.
    unsaved: Rc<RefCell<bool>>,
}

impl eframe::App for Web {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let files: Vec<_> = self.inbox.borrow_mut().drain(..).collect();
        for (dropped, name, bytes) in files {
            if dropped {
                self.app.drop_file(&name, bytes);
            } else {
                self.app.open_file(&name, &bytes);
            }
        }
        // Files dropped on the page are read in the background, like the picker's.
        for file in ctx.input(|i| i.raw.dropped_files.clone()) {
            let (inbox, ctx) = (Rc::clone(&self.inbox), ctx.clone());
            wasm_bindgen_futures::spawn_local(async move {
                let name = file.path().to_string_lossy().into_owned();
                if let Ok(bytes) = file.bytes_async().await {
                    inbox.borrow_mut().push((true, name, bytes));
                    ctx.request_repaint();
                }
            });
        }
        self.app.ui(ui);
        for request in self.app.take_requests() {
            match request {
                Request::PickOpen => pick(&self.inbox, &ctx),
                Request::Download { name, bytes } | Request::Write { path: name, bytes } => {
                    match download(&name, &bytes) {
                        Ok(()) => self.app.written(&name, &bytes),
                        Err(e) => self.app.report_error(format!("{name}: {e:?}")),
                    }
                }
                Request::Export { name, bytes } => match download(&name, &bytes) {
                    Ok(()) => self.app.exported(&name),
                    Err(e) => self.app.report_error(format!("{name}: {e:?}")),
                },
                // Not made in the browser (Platform::BROWSER has no paths and no Quit).
                Request::PickSave { .. } | Request::Quit => {}
            }
        }
        if let Some(title) = self.app.title_if_changed()
            && let Some(document) = web_sys::window().and_then(|w| w.document())
        {
            document.set_title(&title);
        }
        *self.unsaved.borrow_mut() = self.app.has_unsaved_changes();
    }
}

/// Shows the browser's file picker; the chosen file is read in the background into `inbox`.
fn pick(inbox: &Inbox, ctx: &egui::Context) {
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
    input.set_accept(".bpmn,.xpdl,.xml");
    let (inbox, ctx, target) = (Rc::clone(inbox), ctx.clone(), input.clone());
    let on_change = Closure::once(move |_: web_sys::Event| {
        let Some(file) = target.files().and_then(|f| f.get(0)) else {
            return;
        };
        wasm_bindgen_futures::spawn_local(async move {
            let name = file.name();
            if let Ok(buffer) = wasm_bindgen_futures::JsFuture::from(file.array_buffer()).await {
                let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
                inbox.borrow_mut().push((false, name, bytes));
                ctx.request_repaint();
            }
        });
    });
    input.set_onchange(Some(on_change.as_ref().unchecked_ref()));
    // The picker calls back at most once; the closure lives until then.
    on_change.forget();
    input.click();
}

/// Offers `bytes` as a download named `name`.
fn download(name: &str, bytes: &[u8]) -> Result<(), JsValue> {
    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| JsValue::from_str("no document"))?;
    let parts = js_sys::Array::of1(&js_sys::Uint8Array::from(bytes));
    let options = web_sys::BlobPropertyBag::new();
    let kind = match name.rsplit('.').next() {
        Some("png") => "image/png",
        Some("csv") => "text/csv",
        _ => "application/xml",
    };
    options.set_type(kind);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob)?;
    let a: web_sys::HtmlAnchorElement = document.create_element("a")?.unchecked_into();
    a.set_href(&url);
    a.set_download(name);
    a.click();
    web_sys::Url::revoke_object_url(&url)?;
    Ok(())
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
                Box::new(move |_cc| {
                    Ok(Box::new(Web {
                        app: App::new(Platform::BROWSER),
                        inbox: Inbox::default(),
                        unsaved,
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
