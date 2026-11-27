//! Random number sources in the browser: fetching with the page's own `fetch` (no redirect
//! followed, no cookies sent, a key only over HTTPS), answered between frames; and keys in the
//! page's local storage, which the window warns about.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use stochcraft_qrng::{Fetch, FetchError, KeyError, KeyStore, Method, Request, Secret, Ticket};
use wasm_bindgen::JsCast as _;
use wasm_bindgen_futures::JsFuture;

type Answer = Result<Vec<u8>, FetchError>;

/// Fetching through the browser.
#[derive(Default)]
pub struct WebFetch {
    next: u64,
    answers: Arc<Mutex<BTreeMap<u64, Option<Answer>>>>,
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
    let headers = web_sys::Headers::new().map_err(|_| FetchError::Failed)?;
    if let (Some(h), Some(k)) = (&r.key_header, &key) {
        headers.set(h, k).map_err(|_| FetchError::Failed)?;
    }
    init.set_headers(&headers);
    let request =
        web_sys::Request::new_with_str_and_init(&r.url, &init).map_err(|_| FetchError::Failed)?;
    let window = web_sys::window().ok_or(FetchError::Unavailable)?;
    // The browser says only that it failed: no network, or a site that allows no requests from
    // other sites (CORS), look the same from here.
    let response = JsFuture::from(window.fetch_with_request(&request))
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
    let buffer = JsFuture::from(response.array_buffer().map_err(|_| FetchError::Failed)?)
        .await
        .map_err(|_| FetchError::Failed)?;
    let bytes = js_sys::Uint8Array::new(&buffer).to_vec();
    if bytes.len() as u64 > r.max_bytes {
        return Err(FetchError::TooLarge);
    }
    Ok(bytes)
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
