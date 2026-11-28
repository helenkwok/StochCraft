//! The desktop apps' side of StochCraft's random number sources: [`HttpFetch`], an HTTPS client
//! that runs each request on a thread of its own and answers through a channel, so a window
//! never waits; and [`Keychain`], the OS keychain for keys.
//!
//! Errors are reduced to [`stochcraft_qrng::FetchError`]'s fixed reasons: nothing ureq says
//! (which can hold an address) reaches a message. A key is sent only over HTTPS, no redirect is
//! followed, no cookie is kept, and an answer is capped at the request's `max_bytes`.
//!
//! In the browser this crate is empty: the web app fetches with the browser's own `fetch`.

#[cfg(not(target_arch = "wasm32"))]
mod desktop;

#[cfg(not(target_arch = "wasm32"))]
pub use desktop::{HttpFetch, Keychain};
