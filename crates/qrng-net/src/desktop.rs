use std::collections::BTreeMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use stochcraft_qrng::{Fetch, FetchError, KeyError, KeyStore, Method, Request, Secret, Ticket};

type Answer = Result<Vec<u8>, FetchError>;

/// An HTTPS client for random number sources.
pub struct HttpFetch {
    agent: ureq::Agent,
    next: u64,
    pending: BTreeMap<u64, Receiver<Answer>>,
}

impl HttpFetch {
    /// `user_agent` names the product and its version (`StochCraft/0.2.0`) and nothing about the
    /// person using it.
    pub fn new(user_agent: &str) -> Self {
        // The OS checks each certificate chain itself (it fetches roots it has not stored yet,
        // and trusts what it has been told to), as craft-libs' craft-update does.
        let tls = ureq::tls::TlsConfig::builder()
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .timeout_connect(Some(Duration::from_secs(20)))
            .http_status_as_error(false)
            .max_redirects(0)
            .max_redirects_will_error(false)
            .user_agent(user_agent)
            .build()
            .new_agent();
        Self {
            agent,
            next: 1,
            pending: BTreeMap::new(),
        }
    }
}

/// A ureq failure as one of the fixed reasons.
fn reason(e: &ureq::Error) -> FetchError {
    match e {
        ureq::Error::Timeout(_) => FetchError::Timeout,
        ureq::Error::HostNotFound | ureq::Error::ConnectionFailed | ureq::Error::Io(_) => {
            FetchError::Unreachable
        }
        ureq::Error::Tls(_) | ureq::Error::Rustls(_) | ureq::Error::TlsRequired => FetchError::Tls,
        ureq::Error::BodyExceedsLimit(_) => FetchError::TooLarge,
        ureq::Error::RequireHttpsOnly(_) => FetchError::NotHttps,
        ureq::Error::RedirectFailed | ureq::Error::TooManyRedirects => FetchError::Redirected,
        ureq::Error::StatusCode(s) => FetchError::Status(*s),
        _ => FetchError::Failed,
    }
}

fn get(agent: &ureq::Agent, r: &Request, key: Option<&Secret>) -> Answer {
    let key = match (&r.key_header, key) {
        (Some(h), Some(k)) => {
            if !r.url.starts_with("https://") {
                return Err(FetchError::NotHttps);
            }
            Some((h.as_str(), k.expose()))
        }
        _ => None,
    };
    let timeout = Some(Duration::from_millis(r.timeout_ms.max(1000)));
    let response = match r.method {
        Method::Get => {
            let mut b = agent.get(&r.url).config().timeout_global(timeout).build();
            if let Some((h, k)) = key {
                b = b.header(h, k);
            }
            b.call()
        }
        Method::Post => {
            let mut b = agent.post(&r.url).config().timeout_global(timeout).build();
            if let Some((h, k)) = key {
                b = b.header(h, k);
            }
            b.send_empty()
        }
    };
    let mut response = response.map_err(|e| reason(&e))?;
    let status = response.status().as_u16();
    if (300..400).contains(&status) {
        return Err(FetchError::Redirected);
    }
    if !(200..300).contains(&status) {
        return Err(FetchError::Status(status));
    }
    response
        .body_mut()
        .with_config()
        .limit(r.max_bytes)
        .read_to_vec()
        .map_err(|e| reason(&e))
}

impl Fetch for HttpFetch {
    fn start(&mut self, request: &Request, key: Option<&Secret>) -> Ticket {
        let t = Ticket(self.next);
        self.next += 1;
        let (tx, rx) = mpsc::channel();
        let (agent, request, key) = (self.agent.clone(), request.clone(), key.cloned());
        let spawned = std::thread::Builder::new()
            .name("stochcraft-random-fetch".into())
            .spawn(move || {
                let _ = tx.send(get(&agent, &request, key.as_ref()));
            });
        if spawned.is_err() {
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(Err(FetchError::Failed));
            self.pending.insert(t.0, rx);
        } else {
            self.pending.insert(t.0, rx);
        }
        t
    }

    fn poll(&mut self, ticket: Ticket) -> Option<Answer> {
        let answer = match self.pending.get(&ticket.0)?.try_recv() {
            Ok(a) => a,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err(FetchError::Failed),
        };
        self.pending.remove(&ticket.0);
        Some(answer)
    }

    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// Keys in the OS keychain, under the service name `StochCraft` and the source's key reference.
#[derive(Debug, Default)]
pub struct Keychain;

/// The keychain's service name.
const SERVICE: &str = "StochCraft";

impl KeyStore for Keychain {
    fn get(&self, key_ref: &str) -> Option<Secret> {
        keyring::Entry::new(SERVICE, key_ref)
            .ok()?
            .get_password()
            .ok()
            .map(Secret::new)
    }

    fn set(&mut self, key_ref: &str, key: &Secret) -> Result<(), KeyError> {
        let entry = keyring::Entry::new(SERVICE, key_ref).map_err(|_| KeyError::Unavailable)?;
        entry
            .set_password(key.expose())
            .map_err(|_| KeyError::Refused)
    }

    fn delete(&mut self, key_ref: &str) {
        if let Ok(entry) = keyring::Entry::new(SERVICE, key_ref) {
            let _ = entry.delete_credential();
        }
    }
}
