//! Where tool calls end up: an engine in this process ([`Headless`]), or the running app reached
//! through its control channel ([`Remote`], docs/mcp.md), so a person can watch the agent work.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use stochcraft_engine::{COMMANDS, CommandError, Engine};

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
        let given = params
            .get("path")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(path) = given.or_else(|| self.engine.path().map(str::to_owned)) else {
            // Nowhere to save: the engine says so.
            return Ok(params);
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
    /// Connects to the app's control channel at `addr` (`127.0.0.1:7070`).
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let target = addr
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no such address"))?;
        let stream = TcpStream::connect_timeout(&target, Duration::from_secs(3))?;
        stream.set_nodelay(true)?;
        Ok(Self {
            addr: addr.to_owned(),
            reader: BufReader::new(stream.try_clone()?),
            writer: stream,
            next_id: 1,
        })
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
        if reply["id"] != id {
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
