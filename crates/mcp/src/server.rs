//! JSON-RPC 2.0 framing and the MCP lifecycle, tools and resources methods.
//!
//! Adapted from FieldCraft's `crates/mcp/src/server.rs` (MIT OR Apache-2.0), see ATTRIBUTION.md.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::backend::Backend;
use crate::tools::{call_tool, check_arguments, tool_definitions};

/// The newest protocol revision spoken; offered when the client asks for one not known.
pub const PROTOCOL_VERSION: &str = "2025-06-18";
pub const SUPPORTED_VERSIONS: &[&str] = &[PROTOCOL_VERSION, "2025-03-26", "2024-11-05"];

/// Longest message line accepted (the app's control channel's limit too).
pub const MAX_LINE: usize = 16 << 20;
/// Most messages in one JSON-RPC batch (protocol revisions before 2025-06-18).
pub const MAX_BATCH_MESSAGES: usize = 100;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const INTERNAL_ERROR: i64 = -32603;
const RESOURCE_NOT_FOUND: i64 = -32002;

const INSTRUCTIONS: &str = "StochCraft simulates processes: entities arrive at sources, wait for \
and take service at servers (or shared tasks that need units from pools), take time in delays, are \
routed by exclusive gateways (probability or condition) and parallel gateways, and leave at sinks. A \
model is BPMN or XPDL with BPSim data. A typical session: `open` a .bpmn or .xpdl file, or build a \
model with `execute`/`batch` (block.add {kind, id, ...}, connect {from, to}, flow.set, resource.add, \
calendar.add, experiment.set {time_unit, warmup, duration, replications, seed}; list_commands shows \
each command's parameters and an example); `check_model`; `run` for the results, each a mean across \
replications with its 95% confidence interval; change a parameter and run again to compare; `save`. \
Every failure leaves the model unchanged and says why; `undo` takes back an edit.";

pub struct Server {
    backend: Box<dyn Backend>,
    initialized: bool,
}

fn response(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn error(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// One line of at most `max` bytes (without the newline). A longer line is read to its end and
/// dropped, so the stream stays in step: `Err(InvalidData)` reports it.
pub fn read_line(r: &mut impl BufRead, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    let mut too_long = false;
    let too_large = || std::io::Error::new(std::io::ErrorKind::InvalidData, "request too large");
    loop {
        let (done, used) = {
            let available = r.fill_buf()?;
            if available.is_empty() {
                if too_long {
                    return Err(too_large());
                }
                return Ok((!line.is_empty()).then_some(line));
            }
            let (part, done, used) = match available.iter().position(|b| *b == b'\n') {
                Some(i) => (available.get(..i).unwrap_or(&[]), true, i + 1),
                None => (available, false, available.len()),
            };
            if !too_long {
                line.extend_from_slice(part);
                if line.len() > max {
                    too_long = true;
                    line = Vec::new();
                }
            }
            (done, used)
        };
        r.consume(used);
        if done {
            if too_long {
                return Err(too_large());
            }
            return Ok(Some(line));
        }
    }
}

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self {
            backend,
            initialized: false,
        }
    }

    pub fn backend(&mut self) -> &mut dyn Backend {
        self.backend.as_mut()
    }

    /// Whether the client sent `notifications/initialized`.
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Serves newline-delimited JSON-RPC until `input` closes. Only protocol messages go to
    /// `output`; diagnostics belong on stderr.
    pub fn serve(
        &mut self,
        mut input: impl BufRead,
        mut output: impl Write,
    ) -> std::io::Result<()> {
        loop {
            let reply = match read_line(&mut input, MAX_LINE) {
                Ok(None) => return Ok(()),
                Ok(Some(bytes)) => match String::from_utf8(bytes) {
                    Ok(line) => self.handle_line(&line),
                    Err(_) => Some(
                        error(Value::Null, PARSE_ERROR, "parse error: input is not UTF-8")
                            .to_string(),
                    ),
                },
                Err(e) if e.kind() == std::io::ErrorKind::InvalidData => Some(
                    error(
                        Value::Null,
                        INVALID_REQUEST,
                        format!("request too large (at most {MAX_LINE} bytes a line)"),
                    )
                    .to_string(),
                ),
                Err(e) => return Err(e),
            };
            if let Some(reply) = reply {
                output.write_all(reply.as_bytes())?;
                output.write_all(b"\n")?;
                output.flush()?;
            }
        }
    }

    /// One line in, at most one line out (notifications get no reply).
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        let line = line.trim();
        if line.is_empty() {
            return None;
        }
        if line.len() > MAX_LINE {
            return Some(error(Value::Null, INVALID_REQUEST, "request too large").to_string());
        }
        let reply = match serde_json::from_str::<Value>(line) {
            Ok(Value::Array(batch)) if batch.is_empty() => {
                Some(error(Value::Null, INVALID_REQUEST, "empty batch"))
            }
            Ok(Value::Array(batch)) if batch.len() > MAX_BATCH_MESSAGES => Some(error(
                Value::Null,
                INVALID_REQUEST,
                format!("batch too large (at most {MAX_BATCH_MESSAGES} messages)"),
            )),
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> =
                    batch.into_iter().filter_map(|m| self.handle(m)).collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => self.handle(msg),
            Err(e) => Some(error(Value::Null, PARSE_ERROR, format!("parse error: {e}"))),
        };
        reply.map(|r| r.to_string())
    }

    /// One message in, its reply out (none for notifications and responses).
    pub fn handle(&mut self, msg: Value) -> Option<Value> {
        let Value::Object(o) = &msg else {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "a message must be an object",
            ));
        };
        let id = o.get("id").cloned();
        if let Some(id) = &id
            && !(id.is_string() || id.is_i64() || id.is_u64())
        {
            return Some(error(
                Value::Null,
                INVALID_REQUEST,
                "`id` must be a string or an integer",
            ));
        }
        let Some(method) = o.get("method").and_then(Value::as_str) else {
            // A response to a request of ours (none are sent) or junk.
            if o.contains_key("result") || o.contains_key("error") {
                return None;
            }
            return Some(error(
                id.unwrap_or(Value::Null),
                INVALID_REQUEST,
                "missing `method`",
            ));
        };
        let params = o.get("params").cloned().unwrap_or(Value::Null);
        let Some(id) = id else {
            if method == "notifications/initialized" {
                self.initialized = true;
            }
            return None;
        };
        Some(match self.request(method, &params) {
            Ok(r) => response(id, r),
            Err((code, m)) => error(id, code, m),
        })
    }

    fn request(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params
                    .get("protocolVersion")
                    .and_then(Value::as_str)
                    .unwrap_or(PROTOCOL_VERSION);
                let version = if SUPPORTED_VERSIONS.contains(&asked) {
                    asked
                } else {
                    PROTOCOL_VERSION
                };
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {
                        "tools": {"listChanged": false},
                        "resources": {"listChanged": false},
                    },
                    "serverInfo": {
                        "name": "stochcraft",
                        "title": "StochCraft",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                    "instructions": format!("{INSTRUCTIONS} You are driving {}.", self.backend.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tool_definitions()})),
            "tools/call" => {
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or((INVALID_PARAMS, "missing tool `name`".to_owned()))?;
                let args = match params.get("arguments") {
                    None | Some(Value::Null) => json!({}),
                    Some(a) => a.clone(),
                };
                check_arguments(name, &args).map_err(|e| (INVALID_PARAMS, e))?;
                Ok(call_tool(self.backend.as_mut(), name, &args))
            }
            "resources/list" => Ok(json!({"resources": [
                {"uri": "stochcraft://model", "name": "Model", "description": "model.get: the elements, connections, pools, calendars and experiment", "mimeType": "application/json"},
                {"uri": "stochcraft://commands", "name": "Command catalogue", "description": "every command with its summary, parameter schema and an example", "mimeType": "application/json"},
            ]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates": []})),
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).unwrap_or("");
                let v = match uri {
                    "stochcraft://model" => self.backend.execute("model.get", json!({})),
                    "stochcraft://commands" => self.backend.commands(),
                    _ => {
                        return Err((
                            RESOURCE_NOT_FOUND,
                            format!(
                                "unknown resource `{}`",
                                uri.chars().take(200).collect::<String>()
                            ),
                        ));
                    }
                }
                .map_err(|e| (INTERNAL_ERROR, e))?;
                Ok(
                    json!({"contents": [{"uri": uri, "mimeType": "application/json", "text": v.to_string()}]}),
                )
            }
            _ => Err((
                METHOD_NOT_FOUND,
                format!(
                    "method `{}` not found",
                    method.chars().take(200).collect::<String>()
                ),
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    #[test]
    fn lines_are_capped_and_the_stream_stays_in_step() {
        let mut input = Vec::new();
        input.extend(vec![b'x'; 100]);
        input.extend(b"\nshort\n");
        input.extend(vec![b'y'; 30]);
        let mut r = std::io::BufReader::with_capacity(8, input.as_slice());
        assert_eq!(
            read_line(&mut r, 10).unwrap_err().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(read_line(&mut r, 10).unwrap().unwrap(), b"short");
        assert!(
            read_line(&mut r, 10).is_err(),
            "a last line without newline that is too long"
        );
        assert!(read_line(&mut r, 10).unwrap().is_none());
    }
}
