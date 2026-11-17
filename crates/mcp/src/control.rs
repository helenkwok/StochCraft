//! The app's side of the control channel (`stochcraft --control PORT`), which [`crate::Remote`]
//! connects to: a listener on the loopback interface only, one JSON request per line, each
//! answered by one line.
//!
//! - `{"id", "method": "engine.commands"}`: the command catalogue, answered here.
//! - `{"id", "method": "engine.execute", "params": {"command", "params"}}`: handed to the app as
//!   a [`Call`] (the app owns the engine, on its UI thread), and answered with what the app
//!   replies: `{"id", "result"}` or `{"id", "error": {"code", "message", "details"?}}`.
//!
//! Anyone on this computer who can reach the port can drive the app, as a person at its window
//! can; nothing is reachable from other computers.

use std::io::{BufReader, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::Duration;

use serde_json::{Value, json};
use stochcraft_engine::{CommandError, ErrorCode};

use crate::backend::catalogue;
use crate::server::{MAX_LINE, read_line};

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

/// Listens on `127.0.0.1:port` (0 picks a free port). Calls arrive on the receiver; `wake` is
/// called after each, so an idle window looks at them.
pub fn listen(
    port: u16,
    wake: impl Fn() + Send + Sync + 'static,
) -> std::io::Result<(SocketAddr, Receiver<Call>)> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let addr = listener.local_addr()?;
    let (calls, receiver) = channel();
    let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(wake);
    let open = Arc::new(AtomicUsize::new(0));
    std::thread::Builder::new()
        .name("stochcraft-control".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let Some(slot) = Slot::take(&open) else {
                    let _ = std::thread::Builder::new()
                        .name("stochcraft-control-refuse".into())
                        .spawn(move || refuse(stream));
                    continue;
                };
                let (calls, wake) = (calls.clone(), Arc::clone(&wake));
                let _ = std::thread::Builder::new()
                    .name("stochcraft-control-client".into())
                    .spawn(move || {
                        let _slot = slot;
                        serve(stream, &calls, wake.as_ref());
                    });
            }
        })?;
    Ok((addr, receiver))
}

/// The most clients connected at once. An agent keeps its connection for its whole session, idle
/// between calls, so connections are not timed out; this cap is what stops them piling up.
pub const MAX_CLIENTS: usize = 8;

/// One of the [`MAX_CLIENTS`] places, given back when the client's thread ends (or never starts).
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(open: &Arc<AtomicUsize>) -> Option<Self> {
        open.fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
            (n < MAX_CLIENTS).then_some(n + 1)
        })
        .ok()
        .map(|_| Slot(Arc::clone(open)))
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Answers a client over [`MAX_CLIENTS`]: its first request (waited for briefly) gets a
/// `conflict` error naming the limit, and the connection is closed.
fn refuse(stream: TcpStream) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut reader = BufReader::new(stream);
    let id = match read_line(&mut reader, MAX_LINE) {
        Ok(Some(line)) => serde_json::from_slice::<Value>(&line)
            .ok()
            .and_then(|r| r.get("id").cloned())
            .unwrap_or(Value::Null),
        _ => Value::Null,
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

/// Answers one client until it disconnects or the app has gone.
fn serve(stream: TcpStream, calls: &Sender<Call>, wake: &(dyn Fn() + Send + Sync)) {
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = BufReader::new(stream);
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
