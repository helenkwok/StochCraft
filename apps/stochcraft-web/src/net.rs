//! Random number sources in the browser: fetching with the page's own `fetch` (no redirect
//! followed, no cookies sent, a key only over HTTPS, the body read in pieces under its cap, the
//! whole request stopped at its timeout), answered between frames; and keys in the
//! page's local storage, which the window warns about.

use std::cell::Cell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use stochcraft_qrng::{Fetch, FetchError, KeyError, KeyStore, Method, Request, Secret, Ticket};
use wasm_bindgen::JsCast as _;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::JsFuture;

type Answer = Result<Vec<u8>, FetchError>;

/// Fetching through the browser.
#[derive(Default)]
pub struct WebFetch {
    next: u64,
    answers: Arc<Mutex<BTreeMap<u64, Option<Answer>>>>,
}

/// The timer that stops a request once its time is up: it aborts the request (the fetch and the
/// reading of its body) and notes that it fired. Dropped, it is cleared, so it never fires after
/// the request is over.
struct Timer {
    window: web_sys::Window,
    handle: i32,
    fired: Rc<Cell<bool>>,
    _callback: Closure<dyn FnMut()>,
}

impl Timer {
    fn start(
        window: &web_sys::Window,
        controller: &web_sys::AbortController,
        timeout_ms: u64,
    ) -> Result<Self, FetchError> {
        let fired = Rc::new(Cell::new(false));
        let (flag, abort) = (Rc::clone(&fired), controller.clone());
        let callback = Closure::<dyn FnMut()>::new(move || {
            flag.set(true);
            abort.abort();
        });
        let handle = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                callback.as_ref().unchecked_ref(),
                crate::body::delay_ms(timeout_ms),
            )
            .map_err(|_| FetchError::Failed)?;
        Ok(Self {
            window: window.clone(),
            handle,
            fired,
            _callback: callback,
        })
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.window.clear_timeout_with_handle(self.handle);
    }
}

async fn get(r: Request, key: Option<String>) -> Answer {
    if key.is_some() && !r.url.starts_with("https://") {
        return Err(FetchError::NotHttps);
    }
    let init = web_sys::RequestInit::new();
    init.set_method(match r.method {
        Method::Get => "GET",
        Method::Post => "POST",
    });
    init.set_redirect(web_sys::RequestRedirect::Error);
    init.set_credentials(web_sys::RequestCredentials::Omit);
    let controller = web_sys::AbortController::new().map_err(|_| FetchError::Failed)?;
    init.set_signal(Some(&controller.signal()));
    let headers = web_sys::Headers::new().map_err(|_| FetchError::Failed)?;
    if let (Some(h), Some(k)) = (&r.key_header, &key) {
        headers.set(h, k).map_err(|_| FetchError::Failed)?;
    }
    init.set_headers(&headers);
    let request =
        web_sys::Request::new_with_str_and_init(&r.url, &init).map_err(|_| FetchError::Failed)?;
    let window = web_sys::window().ok_or(FetchError::Unavailable)?;
    // `timeout_ms` covers the whole request: the answer's headers and all of its body.
    let timer = Timer::start(&window, &controller, r.timeout_ms)?;
    let answer = fetch_body(&window, &request, r.max_bytes).await;
    if answer.is_err() {
        // A request stopped early is aborted, so nothing more of it is fetched.
        controller.abort();
    }
    match answer {
        Err(_) if timer.fired.get() => Err(FetchError::Timeout),
        other => other,
    }
}

/// The answer to `request`, its body read piece by piece and given up as soon as it passes
/// `max_bytes`, so a larger one is never held whole.
async fn fetch_body(
    window: &web_sys::Window,
    request: &web_sys::Request,
    max_bytes: u64,
) -> Answer {
    // The browser says only that it failed: no network, or a site that allows no requests from
    // other sites (CORS), look the same from here.
    let response = JsFuture::from(window.fetch_with_request(request))
        .await
        .map_err(|_| FetchError::Blocked)?;
    let response: web_sys::Response = response.dyn_into().map_err(|_| FetchError::Failed)?;
    let status = response.status();
    if response.redirected() || (300..400).contains(&status) {
        return Err(FetchError::Redirected);
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Status(status));
    }
    let Some(stream) = response.body() else {
        return Ok(Vec::new());
    };
    let reader: web_sys::ReadableStreamDefaultReader = stream
        .get_reader()
        .dyn_into()
        .map_err(|_| FetchError::Failed)?;
    let mut body = crate::body::Capped::new(max_bytes);
    loop {
        let read: web_sys::ReadableStreamReadResult = JsFuture::from(reader.read())
            .await
            .map_err(|_| FetchError::Failed)?
            .unchecked_into();
        if read.get_done().unwrap_or(true) {
            return Ok(body.into_bytes());
        }
        let piece: js_sys::Uint8Array = read
            .get_value()
            .dyn_into()
            .map_err(|_| FetchError::Failed)?;
        if !body.fits(u64::from(piece.length())) || !body.push(&piece.to_vec()) {
            // The rest is not read: the stream is cancelled (and the request aborted).
            let _ = reader.cancel();
            return Err(FetchError::TooLarge);
        }
    }
}

impl Fetch for WebFetch {
    fn start(&mut self, request: &Request, key: Option<&Secret>) -> Ticket {
        self.next += 1;
        let t = self.next;
        let answers = Arc::clone(&self.answers);
        if let Ok(mut a) = answers.lock() {
            a.insert(t, None);
        }
        let (request, key) = (request.clone(), key.map(|k| k.expose().to_owned()));
        wasm_bindgen_futures::spawn_local(async move {
            let answer = get(request, key).await;
            if let Ok(mut a) = answers.lock() {
                a.insert(t, Some(answer));
            }
        });
        Ticket(t)
    }

    fn poll(&mut self, ticket: Ticket) -> Option<Answer> {
        let mut a = self.answers.lock().ok()?;
        if matches!(a.get(&ticket.0), Some(Some(_))) {
            return a.remove(&ticket.0).flatten();
        }
        if a.contains_key(&ticket.0) {
            None
        } else {
            Some(Err(FetchError::Failed))
        }
    }

    fn now_ms(&self) -> u64 {
        js_sys::Date::now().max(0.0) as u64
    }
}

/// Keys in the page's local storage.
#[derive(Debug, Default)]
pub struct LocalKeys;

const PREFIX: &str = "stochcraft.random-key.";

fn storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

impl KeyStore for LocalKeys {
    fn get(&self, key_ref: &str) -> Option<Secret> {
        storage()?
            .get_item(&format!("{PREFIX}{key_ref}"))
            .ok()?
            .map(Secret::new)
    }

    fn set(&mut self, key_ref: &str, key: &Secret) -> Result<(), KeyError> {
        storage()
            .ok_or(KeyError::Unavailable)?
            .set_item(&format!("{PREFIX}{key_ref}"), key.expose())
            .map_err(|_| KeyError::Refused)
    }

    fn delete(&mut self, key_ref: &str) {
        if let Some(s) = storage() {
            let _ = s.remove_item(&format!("{PREFIX}{key_ref}"));
        }
    }

    fn warning(&self) -> Option<&'static str> {
        Some(
            "In the browser, keys are kept in this site's local storage, unencrypted: anything that runs on this page, and anyone who uses this browser profile, can read them. Use a key you can revoke, or the desktop app, which keeps keys in the system's keychain.",
        )
    }
}
