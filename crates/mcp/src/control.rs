//! The app's side of the control channel (`stochcraft --control PORT`), which [`crate::Remote`]
//! connects to: a listener on the loopback interface only, one JSON request per line, each
//! answered by one line.
//!
//! - `{"id", "method": "engine.commands"}`: the command catalogue, answered here.
//! - `{"id", "method": "engine.execute", "params": {"command", "params"}}`: handed to the app as
//!   a [`Call`] (the app owns the engine, on its UI thread), and answered with what the app
//!   replies: `{"id", "result"}` or `{"id", "error": {"code", "message", "details"?}}`.
//!
//! Every connection first presents the channel's token ([`crate::token`]) as one line,
//! `{"token": "..."}`; the channel answers it with `{"id": null, "result": {"authenticated":
//! true}}`, or, if the token is missing or wrong, with one `unauthorized` error line, and closes
//! the connection without running anything. The token is in a file only the app's user can read,
//! so a program under another account on this computer cannot drive the app; nothing is
//! reachable from other computers.

use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use stochcraft_engine::{CommandError, ErrorCode};

use crate::backend::catalogue;
use crate::server::{MAX_LINE, read_line};
use crate::token::{self, TokenFile, constant_time_eq};

/// How long a new connection has, from being accepted, to send its whole token line before it is
/// closed.
const AUTH_TIMEOUT: Duration = Duration::from_secs(2);
/// The longest token line read: a token is 64 digits, so anything near this is not one.
const AUTH_MAX_LINE: usize = 1024;

/// A command for the app to run, from a client of the control channel.
pub struct Call {
    pub command: String,
    pub params: Value,
    reply: Sender<Result<Value, Value>>,
}

impl Call {
    /// Sends the app's answer back to the client.
    pub fn answer(self, result: Result<Value, CommandError>) {
        let _ = self.reply.send(result.map_err(|e| e.to_json()));
    }
}

/// The open control channel: the address it listens on, the calls arriving, and the token file,
/// which is removed when this is dropped.
pub struct Channel {
    pub addr: SocketAddr,
    pub calls: Receiver<Call>,
    _file: TokenFile,
}

/// Listens on `127.0.0.1:port` (0 picks a free port) for clients that present a new random
/// token, and writes the port and token to [`token::default_path`] for them to find. Calls
/// arrive on `calls`; `wake` is called after each, so an idle window looks at them. If another
/// running app owns the token file, this fails (`AddrInUse`) rather than hide that app's token.
pub fn listen(port: u16, wake: impl Fn() + Send + Sync + 'static) -> std::io::Result<Channel> {
    let path = token::default_path().ok_or_else(|| {
        std::io::Error::other("this system has no folder for the control channel's token file")
    })?;
    listen_at(port, &path, wake)
}

/// [`listen`], with the token file at `path`.
pub fn listen_at(
    port: u16,
    path: &Path,
    wake: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<Channel> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let token = token::generate()?;
    let file = TokenFile::publish(path, listener.local_addr()?.port(), &token)?;
    let (addr, calls) = serve_listener(listener, token, wake)?;
    Ok(Channel {
        addr,
        calls,
        _file: file,
    })
}

/// Listens on `127.0.0.1:port` for clients that present `token`, writing no file.
pub fn listen_with_token(
    port: u16,
    token: &str,
    wake: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<(SocketAddr, Receiver<Call>)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    serve_listener(listener, token.to_owned(), wake)
}

fn serve_listener(
    listener: TcpListener,
    token: String,
    wake: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<(SocketAddr, Receiver<Call>)> {
    let addr = listener.local_addr()?;
    let (calls, receiver) = channel();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let token: Arc<str> = Arc::from(token);
    let (open, refusing) = (Arc::new(AtomicUsize::new(0)), Arc::new(AtomicUsize::new(0)));
    std::thread::Builder::new()
        .name("stochcraft-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let deadline = Instant::now() + AUTH_TIMEOUT;
                let Some(slot) = Slot::take(&open) else {
                    // Refusals are capped too, so clients that never finish a line can't pile up
                    // threads; beyond that the connection is just closed.
                    if let Some(slot) = Slot::take(&refusing) {
                        let _ = std::thread::Builder::new()
                            .name("stochcraft-control-refuse".into())
                            .spawn(move || {
                                let _slot = slot;
                                refuse(stream, deadline);
                            });
                    }
                    continue;
                };
                let (calls, wake, token) = (calls.clone(), Arc::clone(&wake), Arc::clone(&token));
                let _ = std::thread::Builder::new()
                    .name("stochcraft-control-client".into())
                    .spawn(move || {
                        let _slot = slot;
                        serve(stream, &token, deadline, &calls, wake.as_ref());
                    });
            }
        })?;
    Ok((addr, receiver))
}

/// The most clients connected at once. An agent keeps its connection for its whole session, idle
/// between calls, so connections are not timed out; this cap is what stops them piling up.
pub const MAX_CLIENTS: usize = 8;

/// One of the [`MAX_CLIENTS`] places (for clients, or for refusals being answered), given back
/// when its thread ends or never starts.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>) -> Option<Self> {
        // Counted first, then given back if that went over the limit (`fetch_update` is
        // deprecated on newer toolchains, and its replacement is not on 1.95).
        if open.fetch_add(1, Ordering::AcqRel) < MAX_CLIENTS {
            Some(Slot(Arc::clone(open)))
        } else {
            open.fetch_sub(1, Ordering::AcqRel);
            None
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Reads one line of at most [`AUTH_MAX_LINE`] bytes, all of it by `deadline`: every read waits
/// only for what is left of the time, and a line that grows past the limit stops the reading at
/// once (nothing more is taken in). `None` for a line too long, a closed connection or the
/// deadline passing.
fn read_first_line(reader: &mut BufReader<TcpStream>, deadline: Instant) -> Option<Vec<u8>> {
    let mut line = Vec::new();
    loop {
        let left = deadline
            .checked_duration_since(Instant::now())
            .filter(|d| !d.is_zero())?;
        reader.get_ref().set_read_timeout(Some(left)).ok()?;
        let (done, used) = {
            let available = match reader.fill_buf() {
                Ok([]) => return None,
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            match available.iter().position(|b| *b == b'\n') {
                Some(i) => {
                    line.extend_from_slice(available.get(..i).unwrap_or(&[]));
                    (true, i + 1)
                }
                None => {
                    line.extend_from_slice(available);
                    (false, available.len())
                }
            }
        };
        reader.consume(used);
        if line.len() > AUTH_MAX_LINE {
            return None;
        }
        if done {
            return Some(line);
        }
    }
}

/// Answers a client over [`MAX_CLIENTS`]: its first line (its token line, waited for until
/// `deadline`, and not checked: nothing is run) gets a `conflict` error naming the limit, and the
/// connection is closed.
fn refuse(stream: TcpStream, deadline: Instant) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    let id = match read_first_line(&mut reader, deadline) {
        Some(line) => serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|r| r.get("id").cloned())
            .unwrap_or(Value::Null),
        None => Value::Null,
    };
    let message = format!(
        "the app already has {MAX_CLIENTS} control connections, the most it takes; close one and \
         connect again"
    );
    let _ = reply(&mut writer, id, Err(failure(ErrorCode::Conflict, message)));
    let _ = writer.shutdown(Shutdown::Write);
}

fn failure(code: ErrorCode, message: impl Into<String>) -> Value {
    CommandError::new(code, message).to_json()
}

/// Reads the client's first line and checks it is `{"token": ...}` with the right token, in
/// constant time. A client that has not sent the whole line by `deadline` (however slowly it
/// trickles), sends too long a line, or anything else gets one `unauthorized` error line and the
/// connection is closed. On success the client is answered and may send requests, with no time
/// limit.
fn authenticate(
    reader: &mut BufReader<TcpStream>,
    writer: &mut TcpStream,
    token: &str,
    deadline: Instant,
) -> bool {
    let presented = read_first_line(reader, deadline).and_then(|line| {
        serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|v| v.get("token").and_then(Value::as_str).map(str::to_owned))
    });
    let accepted = presented.is_some_and(|t| constant_time_eq(t.as_bytes(), token.as_bytes()));
    if accepted {
        let _ = reader.get_ref().set_read_timeout(None);
        return reply(writer, Value::Null, Ok(json!({ "authenticated": true }))).is_ok();
    }
    let error = json!({
        "code": "unauthorized",
        "message": "the first line must be {\"token\": ...} with the app's token (docs/mcp.md)",
    });
    let _ = reply(writer, Value::Null, Err(error));
    let _ = writer.shutdown(Shutdown::Both);
    false
}

/// Answers one client until it disconnects or the app has gone. Nothing is run before the
/// client has presented the token.
fn serve(
    stream: TcpStream,
    token: &str,
    deadline: Instant,
    calls: &Sender<Call>,
    wake: &(dyn Fn() + Send + Sync),
) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
    if !authenticate(&mut reader, &mut writer, token, deadline) {
        return;
    }
    loop {
        let (id, outcome) = match read_line(&mut reader, MAX_LINE) {
            Ok(None) => return,
            Err(e) if e.kind() == std::io::ErrorKind::InvalidData => (
                Value::Null,
                Err(failure(ErrorCode::InvalidParams, "request too large")),
            ),
            Err(_) => return,
            Ok(Some(line)) => match serde_json::from_slice::<Value>(&line) {
                Err(e) => (
                    Value::Null,
                    Err(failure(ErrorCode::InvalidParams, format!("not JSON: {e}"))),
                ),
                Ok(request) => {
                    let id = request.get("id").cloned().unwrap_or(Value::Null);
                    match request["method"].as_str() {
                        Some("engine.commands") => (id, Ok(catalogue())),
                        Some("engine.execute") => {
                            let Some(command) = request["params"]["command"].as_str() else {
                                let e = failure(ErrorCode::InvalidParams, "`command` is required");
                                if reply(&mut writer, id, Err(e)).is_err() {
                                    return;
                                }
                                continue;
                            };
                            let params = request["params"]
                                .get("params")
                                .cloned()
                                .filter(Value::is_object)
                                .unwrap_or_else(|| json!({}));
                            let (answer, answered) = channel();
                            let call = Call {
                                command: command.to_owned(),
                                params,
                                reply: answer,
                            };
                            if calls.send(call).is_err() {
                                return; // The app has closed.
                            }
                            wake();
                            match answered.recv() {
                                Ok(outcome) => (id, outcome),
                                Err(_) => return,
                            }
                        }
                        other => (
                            id,
                            Err(failure(
                                ErrorCode::UnknownCommand,
                                format!(
                                    "unknown method `{}`",
                                    other.unwrap_or("").chars().take(100).collect::<String>()
                                ),
                            )),
                        ),
                    }
                }
            },
        };
        if reply(&mut writer, id, outcome).is_err() {
            return;
        }
    }
}

fn reply(writer: &mut TcpStream, id: Value, outcome: Result<Value, Value>) -> std::io::Result<()> {
    let line = match outcome {
        Ok(result) => json!({ "id": id, "result": result }),
        Err(error) => json!({ "id": id, "error": error }),
    };
    writeln!(writer, "{line}")?;
    writer.flush()
}
