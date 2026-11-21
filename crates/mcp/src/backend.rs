//! Where tool calls end up: an engine in this process ([`Headless`]), or the running app reached
//! through its control channel ([`Remote`], docs/mcp.md), so a person can watch the agent work.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandError, Engine, ErrorCode};

use crate::roots::{Access, Roots};

/// Something that runs StochCraft's engine commands.
pub trait Backend {
    /// Runs a command; a failure is the engine's error as JSON text (`code`, `message` and any
    /// `details`, such as the model's diagnostics), for the agent to read.
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String>;
    /// Every command with its summary, its parameters as JSON Schema and an example.
    fn commands(&mut self) -> Result<Value, String>;
    /// What the agent is driving, for its instructions.
    fn describe(&self) -> String;
}

/// The catalogue of commands, as `stochcraft-cli schema` prints it.
pub fn catalogue() -> Value {
    Value::Array(COMMANDS.iter().map(|c| c.to_json()).collect())
}

fn engine_error(e: &CommandError) -> String {
    serde_json::to_string_pretty(&e.to_json()).unwrap_or_else(|_| e.to_string())
}

/// An engine in this process, with no window. Files are read and written as `roots` allow.
pub struct Headless {
    pub engine: Engine,
    roots: Roots,
}

impl Headless {
    pub fn new(roots: Roots) -> Self {
        Self {
            engine: Engine::new(),
            roots,
        }
    }

    /// The command's parameters with its file's path checked against the roots (and resolved,
    /// when confined): what `file.open` reads, and what `file.save` writes, given or not.
    fn confine(&self, command: &str, mut params: Value) -> Result<Value, String> {
        let access = match command {
            "file.open" => Access::Read,
            "file.save" => Access::Write,
            _ => return Ok(params),
        };
        if !self.roots.is_confined() {
            return Ok(params);
        }
        let given = match params.get("path") {
            Some(Value::String(s)) => Some(s.clone()),
            None | Some(Value::Null) => None,
            // Not a path: the engine refuses it as invalid parameters, unchanged.
            Some(_) => return Ok(params),
        };
        let path = match (given, access) {
            (Some(path), _) => path,
            // Opening names its file: never the one already open, which would silently reopen it
            // and drop unsaved edits.
            (None, Access::Read) => {
                return Err(engine_error(&CommandError::new(
                    ErrorCode::InvalidParams,
                    "`path` is required: the file to open",
                )));
            }
            // Saving without a path writes the model's own file; with none, the engine says so.
            (None, Access::Write) => match self.engine.path() {
                Some(path) => path.to_owned(),
                None => return Ok(params),
            },
        };
        let resolved = self.roots.check(Path::new(&path), access)?;
        if let Some(o) = params.as_object_mut() {
            o.insert(
                "path".into(),
                Value::String(resolved.to_string_lossy().into_owned()),
            );
        }
        Ok(params)
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new(Roots::default())
    }
}

impl Backend for Headless {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        let params = self.confine(command, params)?;
        self.engine
            .execute(command, params)
            .map_err(|e| engine_error(&e))
    }

    fn commands(&mut self) -> Result<Value, String> {
        Ok(catalogue())
    }

    fn describe(&self) -> String {
        if self.roots.is_confined() {
            "a headless engine (no window), with file access confined to the folders given".into()
        } else {
            "a headless engine (no window)".into()
        }
    }
}

/// Longest reply line accepted from the app.
const MAX_REPLY: u64 = 256 << 20;

/// A connection to the first of `targets` that answers, else the last one's error.
fn first_reachable(targets: impl IntoIterator<Item = SocketAddr>) -> std::io::Result<TcpStream> {
    let mut last = std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "the address resolves to nothing",
    );
    for target in targets {
        match TcpStream::connect_timeout(&target, Duration::from_secs(3)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Why [`Remote::connect`] failed.
#[derive(Debug)]
pub enum ConnectError {
    /// No connection could be made (refused, timed out, unreachable, nothing to resolve).
    Unreachable(std::io::Error),
    /// A connection was made but the app's answer to the token was an error, or none.
    Refused(String),
    /// A connection was made, so the app is there, but setting it up here failed.
    Broken(std::io::Error),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(e) => write!(f, "{e}"),
            ConnectError::Refused(m) => write!(f, "{m}"),
            ConnectError::Broken(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ConnectError {}

/// The running app's control channel (`stochcraft --control PORT`): one JSON request per line,
/// `{"id", "method": "engine.execute", "params": {"command", "params"}}` or
/// `{"id", "method": "engine.commands"}`, answered by one line, `{"id", "result"}` or
/// `{"id", "error": {"code", "message", "details"?}}`.
pub struct Remote {
    addr: String,
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    next_id: u64,
}

impl Remote {
    /// Connects to the app's control channel at `addr` (`127.0.0.1:7070`) and presents `token`
    /// (the channel's first line). Each address the name resolves to is tried in turn:
    /// `localhost` may give `::1` first, while the app listens on `127.0.0.1` only. A channel
    /// that answers with an error (a wrong token, too many clients) or breaks the handshake is
    /// [`ConnectError::Refused`], and a failure setting up a connection that was made is
    /// [`ConnectError::Broken`]: either way the app is there, so the caller must not use another
    /// engine. Only [`ConnectError::Unreachable`] means no connection was made.
    pub fn connect(addr: &str, token: &str) -> Result<Self, ConnectError> {
        let stream = addr
            .to_socket_addrs()
            .and_then(first_reachable)
            .map_err(ConnectError::Unreachable)?;
        stream.set_nodelay(true).map_err(ConnectError::Broken)?;
        let mut remote = Self {
            addr: addr.to_owned(),
            reader: BufReader::new(stream.try_clone().map_err(ConnectError::Broken)?),
            writer: stream,
            next_id: 1,
        };
        remote.authenticate(token).map_err(ConnectError::Refused)?;
        Ok(remote)
    }

    /// Sends the token and reads the channel's answer to it; the failure is what to tell the user.
    fn authenticate(&mut self, token: &str) -> Result<(), String> {
        let broken = |e: std::io::Error| format!("the handshake failed: {e}");
        self.writer
            .set_read_timeout(Some(Duration::from_secs(10)))
            .map_err(broken)?;
        writeln!(self.writer, "{}", json!({ "token": token })).map_err(broken)?;
        self.writer.flush().map_err(broken)?;
        let mut line = String::new();
        let read = (&mut self.reader)
            .take(MAX_REPLY)
            .read_line(&mut line)
            .map_err(broken)?;
        self.writer.set_read_timeout(None).map_err(broken)?;
        if read == 0 {
            return Err("the app closed the connection during the handshake".into());
        }
        let reply: Value = serde_json::from_str(&line)
            .map_err(|e| format!("the app's answer to the token is not JSON: {e}"))?;
        match reply.get("error") {
            None => Ok(()),
            Some(error) => Err(error["message"].as_str().unwrap_or("refused").to_owned()),
        }
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "id": id, "method": method, "params": params });
        let lost = |e: std::io::Error| format!("the app at {} cannot be reached: {e}", self.addr);
        writeln!(self.writer, "{request}").map_err(lost)?;
        self.writer.flush().map_err(lost)?;
        let mut line = String::new();
        let read = (&mut self.reader)
            .take(MAX_REPLY)
            .read_line(&mut line)
            .map_err(lost)?;
        if read == 0 {
            return Err(format!("the app at {} closed the connection", self.addr));
        }
        let reply: Value =
            serde_json::from_str(&line).map_err(|e| format!("the app's reply is not JSON: {e}"))?;
        // A request the app could not read, or a connection it refused, is answered with no id.
        if reply["id"] != id && !(reply["id"].is_null() && reply.get("error").is_some()) {
            return Err("the app's reply is for another request".into());
        }
        if let Some(e) = reply.get("error") {
            return Err(serde_json::to_string_pretty(e).unwrap_or_default());
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }
}

impl Backend for Remote {
    fn execute(&mut self, command: &str, params: Value) -> Result<Value, String> {
        self.call(
            "engine.execute",
            json!({ "command": command, "params": params }),
        )
    }

    fn commands(&mut self) -> Result<Value, String> {
        self.call("engine.commands", json!({}))
    }

    fn describe(&self) -> String {
        format!(
            "the StochCraft app at {}: the person using it sees every change",
            self.addr
        )
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn every_address_is_tried_in_turn() {
        // As `localhost` may resolve: `::1` first, while the app listens on 127.0.0.1 only.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let targets: Vec<SocketAddr> = vec![
            format!("[::1]:{port}").parse().unwrap(),
            format!("127.0.0.1:{port}").parse().unwrap(),
        ];
        let stream = first_reachable(targets).unwrap();
        assert_eq!(stream.peer_addr().unwrap().port(), port);
        assert!(first_reachable(Vec::new()).is_err());
    }
}
