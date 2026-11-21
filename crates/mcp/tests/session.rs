//! The MCP server driven as an agent drives it: a session that opens, builds, checks, runs and
//! saves models only through MCP messages, hostile input, folders the agent is kept to, and the
//! same tools bridged to an app through its control channel.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)] // A failed step fails the test.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use stochcraft_engine::Engine;
use stochcraft_mcp::{Backend, Headless, Options, Remote, Roots, Server, control};

fn models() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/models")
}

/// A folder of its own for one test, removed afterwards.
struct Scratch(PathBuf);

impl Scratch {
    fn new(tag: &str) -> Self {
        let d = std::env::temp_dir().join(format!("stochcraft-mcp-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(std::fs::canonicalize(&d).unwrap())
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Session {
    server: Server,
    next: u64,
}

impl Session {
    fn new(backend: Box<dyn Backend>) -> Self {
        let mut s = Self {
            server: Server::new(backend),
            next: 1,
        };
        let init = s.request(
            "initialize",
            json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}),
        );
        assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
        assert_eq!(init["result"]["serverInfo"]["name"], "stochcraft");
        assert!(
            s.server
                .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
                .is_none()
        );
        assert!(s.server.is_initialized());
        s
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let reply = self.server.handle_line(&line.to_string()).expect("a reply");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    /// A tool call's text, parsed as JSON where it is, and whether it was an error.
    fn call(&mut self, tool: &str, arguments: Value) -> (Value, bool) {
        let r = self.request("tools/call", json!({"name": tool, "arguments": arguments}));
        let result = &r["result"];
        assert!(result.is_object(), "{tool}: {r}");
        let text = result["content"][0]["text"].as_str().unwrap_or_default();
        let value = serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_owned()));
        (value, result["isError"] == true)
    }

    fn ok(&mut self, tool: &str, arguments: Value) -> Value {
        let (v, error) = self.call(tool, arguments);
        assert!(!error, "{tool}: {v}");
        v
    }
}

#[test]
fn an_agent_opens_builds_checks_runs_and_saves() {
    let mut s = Session::new(Box::new(Headless::default()));
    let tools = s.request("tools/list", json!({}));
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|t| t["name"].as_str())
        .collect();
    for t in [
        "list_commands",
        "execute",
        "batch",
        "inspect_model",
        "check_model",
        "open",
        "save",
        "run",
        "undo",
        "redo",
    ] {
        assert!(names.contains(&t), "{t} in {names:?}");
    }
    let commands = s.ok("list_commands", json!({"filter": "RESOURCE"}));
    let commands = commands.as_array().unwrap();
    assert!(!commands.is_empty());
    assert!(commands.iter().all(|c| {
        c["name"].as_str().unwrap().contains("resource")
            || c["summary"]
                .as_str()
                .unwrap()
                .to_lowercase()
                .contains("resource")
    }));

    // A model opened from a file and run.
    let opened = s.ok(
        "open",
        json!({"path": models().join("mm1.bpmn").to_string_lossy()}),
    );
    assert_eq!(opened["format"], "bpmn");
    let results = s.ok("run", json!({"replications": 3, "seed": 5}));
    assert_eq!(results["replications"], 3);
    assert_eq!(results["record"]["seed"], 5);
    // The warm-up suggestion, without the values over the run that it comes from, which
    // `execute` gives.
    assert_eq!(
        results["warmup_suggestion"]["method"], "mser-5",
        "{results}"
    );
    assert!(results.get("series").is_none());
    let full = s.ok(
        "execute",
        json!({"command": "run", "params": {"replications": 3, "seed": 5}}),
    );
    assert!(full["series"]["times"].is_array());

    // A model built from blocks in one batch, checked, run and undone.
    s.ok("execute", json!({"command": "model.new"}));
    let built = s.ok(
        "batch",
        json!({"commands": [
            {"command": "block.add", "params": {"kind": "source", "id": "arrivals", "interarrival": {"type": "exponential", "mean": 2.0}}},
            {"command": "block.add", "params": {"kind": "server", "id": "desk", "service": {"type": "exponential", "mean": 1.5}}},
            {"command": "block.add", "params": {"kind": "sink", "id": "done"}},
            {"command": "connect", "params": {"from": "arrivals", "to": "desk"}},
            {"command": "connect", "params": {"from": "desk", "to": "done"}},
            {"command": "experiment.set", "params": {"replications": 4, "duration": 500.0}},
        ]}),
    );
    assert_eq!(built["completed"], 6);
    assert_eq!(s.ok("check_model", json!({})), json!({"diagnostics": []}));
    let model = s.ok("inspect_model", json!({}));
    assert_eq!(model["model"]["elements"].as_array().unwrap().len(), 3);
    let results = s.ok("run", json!({}));
    let desk = results["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["id"] == "desk")
        .unwrap();
    let rho = desk["metrics"]["utilisation"]["mean"].as_f64().unwrap();
    assert!((0.6..0.9).contains(&rho), "utilisation {rho}, theory 0.75");
    // A second desk, run again with the same seed, and the two runs compared in pairs.
    s.ok(
        "execute",
        json!({"command": "block.set", "params": {"id": "desk", "capacity": 2}}),
    );
    s.ok("run", json!({}));
    let compared = s.ok(
        "execute",
        json!({"command": "results.compare", "params": {
            "results": [{"element": "desk", "metric": "mean_wait"}]}}),
    );
    assert_eq!(compared["paired"], true, "{compared}");
    let wait = &compared["comparisons"][0];
    assert!(wait["difference"].as_f64().unwrap() < 0.0, "{wait}");
    assert_eq!(wait["significant"], true, "{wait}");
    s.ok("undo", json!({}));
    s.ok("undo", json!({}));
    assert_eq!(
        s.ok("inspect_model", json!({}))["model"]["experiment"]["replications"],
        10
    );
    s.ok("redo", json!({}));

    // A failing step stops the batch and says which.
    let (failed, error) = s.call(
        "batch",
        json!({"commands": [
            {"command": "block.rename", "params": {"id": "desk", "name": "Front desk"}},
            {"command": "connect", "params": {"from": "nowhere", "to": "desk"}},
            {"command": "block.rename", "params": {"id": "done", "name": "Out"}},
        ]}),
    );
    assert!(error);
    assert_eq!(failed["failed"]["index"], 1);
    assert_eq!(failed["completed"], 1);

    // An engine failure is a tool error with the engine's code, not a protocol error.
    let (e, error) = s.call(
        "execute",
        json!({"command": "block.set", "params": {"id": "ghost"}}),
    );
    assert!(error);
    assert_eq!(e["code"], "not_found", "{e}");

    // Saved as a new file, and read back.
    let dir = Scratch::new("save");
    let file = dir.0.join("desk.bpmn");
    s.ok("save", json!({"path": file.to_string_lossy()}));
    let mut back = Engine::new();
    back.execute("file.open", json!({"path": file.to_string_lossy()}))
        .unwrap();
    assert_eq!(back.model().elements.len(), 3);

    // Resources.
    let listed = s.request("resources/list", json!({}));
    assert_eq!(listed["result"]["resources"].as_array().unwrap().len(), 2);
    let model = s.request("resources/read", json!({"uri": "stochcraft://model"}));
    let text = model["result"]["contents"][0]["text"].as_str().unwrap();
    assert!(text.contains("\"desk\""));
    let missing = s.request("resources/read", json!({"uri": "stochcraft://nothing"}));
    assert_eq!(missing["error"]["code"], -32002);
}

#[test]
fn hostile_input_gets_errors_and_the_server_carries_on() {
    let mut s = Session::new(Box::new(Headless::default()));
    let mut line = |l: &str| -> Value {
        serde_json::from_str(&s.server.handle_line(l).expect("a reply")).unwrap()
    };
    assert_eq!(line("not json")["error"]["code"], -32700);
    assert_eq!(line("[]")["error"]["code"], -32600);
    assert_eq!(line("42")["error"]["code"], -32600);
    assert_eq!(
        line(r#"{"jsonrpc":"2.0","id":{"x":1},"method":"ping"}"#)["error"]["code"],
        -32600
    );
    assert_eq!(line(r#"{"jsonrpc":"2.0","id":1}"#)["error"]["code"], -32600);
    assert_eq!(
        line(r#"{"jsonrpc":"2.0","id":1,"method":"nope"}"#)["error"]["code"],
        -32601
    );
    let batch: Vec<Value> = (0..101)
        .map(|i| json!({"jsonrpc": "2.0", "id": i, "method": "ping"}))
        .collect();
    assert_eq!(
        line(&Value::Array(batch).to_string())["error"]["code"],
        -32600
    );
    let two = line(
        r#"[{"jsonrpc":"2.0","id":1,"method":"ping"},{"jsonrpc":"2.0","method":"notifications/x"}]"#,
    );
    assert_eq!(
        two.as_array().unwrap().len(),
        1,
        "notifications get no reply"
    );

    // Arguments are checked against the tool's schema before anything runs.
    for (tool, args) in [
        ("no_such_tool", json!({})),
        ("open", json!({})),
        ("open", json!({"path": 3})),
        ("run", json!({"replications": "many"})),
        ("undo", json!({"extra": true})),
        ("execute", json!("block.add")),
    ] {
        let r = s.request("tools/call", json!({"name": tool, "arguments": args}));
        assert_eq!(r["error"]["code"], -32602, "{tool} {r}");
    }
    let r = s.request("tools/call", json!({"arguments": {}}));
    assert_eq!(r["error"]["code"], -32602);

    let too_long: Vec<Value> = (0..=stochcraft_mcp::tools::MAX_STEPS)
        .map(|_| json!({"command": "history"}))
        .collect();
    let (e, error) = s.call("batch", json!({"commands": too_long}));
    assert!(
        error && e.as_str().unwrap().contains("batch too long"),
        "{e}"
    );

    // An over-long line is refused and the stream goes on.
    let mut input = vec![b'x'; stochcraft_mcp::MAX_LINE + 10];
    input.extend(b"\n{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"ping\"}\n");
    let mut out = Vec::new();
    s.server.serve(input.as_slice(), &mut out).unwrap();
    let replies: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0]["error"]["code"], -32600);
    assert_eq!(replies[1]["id"], 7);
}

#[test]
fn the_agent_is_kept_to_its_folders() {
    let dir = Scratch::new("roots");
    let (read, write) = (dir.0.join("in"), dir.0.join("out"));
    std::fs::create_dir_all(&read).unwrap();
    std::fs::create_dir_all(&write).unwrap();
    std::fs::copy(models().join("mm1.bpmn"), read.join("mm1.bpmn")).unwrap();
    let roots = Roots::new(Some(&read), Some(&write)).unwrap();
    let mut s = Session::new(Box::new(Headless::new(roots)));

    let (e, error) = s.call(
        "open",
        json!({"path": models().join("mm1.bpmn").to_string_lossy()}),
    );
    assert!(
        error && e.as_str().unwrap().contains("outside the read root"),
        "{e}"
    );
    s.ok(
        "open",
        json!({"path": read.join("mm1.bpmn").to_string_lossy()}),
    );
    // Saving into the file it came from writes outside the write root.
    let (e, error) = s.call("save", json!({}));
    assert!(
        error && e.as_str().unwrap().contains("outside the write root"),
        "{e}"
    );
    let (e, error) = s.call(
        "execute",
        json!({"command": "file.save", "params": {"path": dir.0.join("x.bpmn").to_string_lossy()}}),
    );
    assert!(
        error && e.as_str().unwrap().contains("outside the write root"),
        "{e}"
    );
    s.ok(
        "save",
        json!({"path": write.join("copy.bpmn").to_string_lossy()}),
    );
    assert!(write.join("copy.bpmn").is_file());

    // Opening with no path is an error, not a reopening of the open file (which would drop
    // its unsaved edits).
    s.ok(
        "execute",
        json!({"command": "model.rename", "params": {"name": "unsaved"}}),
    );
    for params in [json!({}), json!({"path": 5})] {
        let (e, error) = s.call("execute", json!({"command": "file.open", "params": params}));
        assert!(error && e["code"] == "invalid_params", "{e}");
    }
    let model = s.ok("inspect_model", json!({}));
    assert!(model.to_string().contains("unsaved"), "{model}");

    // Confined roots and a connection to the app do not go together.
    let refused = stochcraft_mcp::backend(Options {
        connect: Some("127.0.0.1:1".into()),
        roots: Roots::new(Some(&read), None).unwrap(),
        ..Options::default()
    });
    assert!(refused.is_err());
}

const TOKEN: &str = "0123456789abcdef";

/// An app's side of the control channel: an engine on its own thread, answering calls, for
/// clients that present [`TOKEN`].
fn app_with_control() -> std::net::SocketAddr {
    let (addr, calls) = control::listen_with_token(0, TOKEN, || {}).unwrap();
    std::thread::spawn(move || {
        let mut engine = Engine::new();
        for call in calls {
            let result = engine.execute(&call.command, call.params.clone());
            call.answer(result);
        }
    });
    addr
}

#[test]
fn the_same_tools_drive_an_app_through_its_control_channel() {
    let addr = app_with_control().to_string();
    let (backend, note) = stochcraft_mcp::backend(Options {
        connect: Some(addr.clone()),
        token: Some(TOKEN.into()),
        ..Options::default()
    })
    .unwrap();
    assert!(note.is_none(), "{note:?}");
    let mut s = Session::new(backend);
    let init = s.request("initialize", json!({}));
    assert!(
        init["result"]["instructions"]
            .as_str()
            .unwrap()
            .contains(&addr)
    );
    s.ok(
        "open",
        json!({"path": models().join("tandem.xpdl").to_string_lossy()}),
    );
    let results = s.ok("run", json!({"replications": 2}));
    assert_eq!(results["replications"], 2);
    let (e, error) = s.call(
        "execute",
        json!({"command": "block.set", "params": {"id": "ghost"}}),
    );
    assert!(error);
    assert_eq!(e["code"], "not_found");
    assert!(
        !s.ok("list_commands", json!({}))
            .as_array()
            .unwrap()
            .is_empty()
    );

    // The raw channel: unknown methods and junk are answered, and the connection stays usable.
    let mut remote = Remote::connect(&addr, TOKEN).unwrap();
    assert!(remote.execute("history", json!({})).is_ok());
    use std::io::{BufRead, BufReader, Write};
    let stream = std::net::TcpStream::connect(&addr).unwrap();
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut w = stream;
    let mut ask = |line: &str| -> Value {
        writeln!(w, "{line}").unwrap();
        let mut reply = String::new();
        reader.read_line(&mut reply).unwrap();
        serde_json::from_str(&reply).unwrap()
    };
    assert_eq!(
        ask(&json!({ "token": TOKEN }).to_string())["result"]["authenticated"],
        true
    );
    assert_eq!(ask("junk")["error"]["code"], "invalid_params");
    assert_eq!(
        ask(r#"{"id":1,"method":"ui.nothing"}"#)["error"]["code"],
        "unknown_command"
    );
    assert_eq!(
        ask(r#"{"id":2,"method":"engine.execute","params":{}}"#)["error"]["code"],
        "invalid_params"
    );
    assert_eq!(
        ask(r#"{"id":3,"method":"engine.execute","params":{"command":"history"}}"#)["id"],
        3
    );

    // An app that cannot be reached: headless instead, with a note.
    let (_, note) = stochcraft_mcp::backend(Options {
        connect: Some("127.0.0.1:1".into()),
        token: Some(TOKEN.into()),
        ..Options::default()
    })
    .unwrap();
    assert!(note.is_some_and(|n| n.contains("running headless")));
}

/// What a connection that does not present the token gets: one `unauthorized` line, then
/// nothing more, and the command it sent next has not run.
fn assert_refused(first: Option<&str>, then_execute: bool) {
    use std::io::{BufRead, BufReader, Write};
    let (addr, calls) = control::listen_with_token(0, TOKEN, || {}).unwrap();
    let stream = std::net::TcpStream::connect(addr).unwrap();
    let mut w = stream.try_clone().unwrap();
    let mut reader = BufReader::new(stream);
    if let Some(line) = first {
        writeln!(w, "{line}").unwrap();
    }
    let mut reply = String::new();
    reader.read_line(&mut reply).unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply["error"]["code"], "unauthorized", "{reply}");
    if then_execute {
        // Sent after the refusal, to a connection that is closing: it may not even be sent.
        let _ = writeln!(
            w,
            r#"{{"id":1,"method":"engine.execute","params":{{"command":"history"}}}}"#
        );
    }
    // Closed: no second line, and the app was handed nothing.
    let mut rest = String::new();
    assert_eq!(reader.read_line(&mut rest).unwrap_or(0), 0, "{rest}");
    assert!(calls.try_recv().is_err());
}

#[test]
fn a_connection_without_the_token_runs_nothing_and_is_closed() {
    // A request where the token should be.
    assert_refused(
        Some(r#"{"id":1,"method":"engine.execute","params":{"command":"history"}}"#),
        true,
    );
    // A wrong token, one that is a prefix of the right one, and one that is not text.
    assert_refused(Some(r#"{"token":"0123456789abcdee"}"#), true);
    assert_refused(Some(r#"{"token":"0123456789abcde"}"#), true);
    assert_refused(Some(r#"{"token":7}"#), true);
    assert_refused(Some("junk"), true);
    // Nothing sent at all: closed after the short wait for the first line.
    assert_refused(None, false);
}

#[test]
fn the_right_token_works_and_the_cli_side_is_told_when_it_is_wrong() {
    let addr = app_with_control().to_string();
    assert!(
        Remote::connect(&addr, TOKEN)
            .unwrap()
            .execute("history", json!({}))
            .is_ok()
    );
    let e = Remote::connect(&addr, "wrong").err().unwrap();
    assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);
    // Not a reason to fall back to headless: the agent would not be in the window.
    let refused = stochcraft_mcp::backend(Options {
        connect: Some(addr),
        token: Some("wrong".into()),
        ..Options::default()
    });
    assert!(refused.err().unwrap().contains("refused the token"));
}

#[test]
fn connect_finds_the_token_in_the_file_the_app_wrote() {
    let dir = Scratch::new("tokenfile");
    let path = dir.0.join("stochcraft").join("control.json");
    let channel = control::listen_at(0, &path, || {}).unwrap();
    let calls = channel.calls;
    std::thread::spawn(move || {
        let mut engine = Engine::new();
        for call in calls {
            let result = engine.execute(&call.command, call.params.clone());
            call.answer(result);
        }
    });
    let (port, token) = stochcraft_mcp::token::read(&path).unwrap();
    assert_eq!(port, channel.addr.port());
    assert_eq!(token.len(), 64);
    // No token given anywhere (so, unless STOCHCRAFT_TOKEN is set, the file's): it is found.
    let (backend, note) = stochcraft_mcp::backend(Options {
        connect: Some(channel.addr.to_string()),
        token_file: Some(path.clone()),
        ..Options::default()
    })
    .unwrap();
    assert!(note.is_none(), "{note:?}");
    let mut s = Session::new(backend);
    assert!(
        !s.ok("list_commands", json!({}))
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Without the file and without a token, the error names the file.
    std::fs::remove_file(&path).unwrap();
    let missing = stochcraft_mcp::backend(Options {
        connect: Some(channel.addr.to_string()),
        token_file: Some(path.clone()),
        ..Options::default()
    });
    let message = missing.err().unwrap();
    assert!(
        message.contains("control.json") || std::env::var("STOCHCRAFT_TOKEN").is_ok(),
        "{message}"
    );
}

#[test]
fn the_control_channel_takes_a_limited_number_of_clients() {
    use std::io::{BufRead, BufReader, Write};
    let addr = app_with_control();
    // A client that sends its token line, then the request; the answer to the token is read
    // first (and is the refusal itself for a client over the limit, which has no id).
    let ask = |stream: &std::net::TcpStream, id: u64| -> Option<Value> {
        let mut w = stream.try_clone().unwrap();
        writeln!(w, r#"{{"token":"{TOKEN}"}}"#).ok()?;
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut reply = String::new();
        reader.read_line(&mut reply).ok()?;
        let first: Value = serde_json::from_str(&reply).ok()?;
        if first.get("error").is_some() {
            return Some(first);
        }
        writeln!(w, r#"{{"id":{id},"method":"engine.commands"}}"#).ok()?;
        reply.clear();
        reader.read_line(&mut reply).ok()?;
        serde_json::from_str(&reply).ok()
    };
    // Each answered, so the app has taken all of them.
    let mut open: Vec<std::net::TcpStream> = (0..control::MAX_CLIENTS)
        .map(|_| std::net::TcpStream::connect(addr).unwrap())
        .collect();
    for (i, s) in open.iter().enumerate() {
        assert!(ask(s, i as u64).unwrap()["result"].is_array());
    }
    // One more is told why, under its own id, and closed.
    let extra = std::net::TcpStream::connect(addr).unwrap();
    let refused = ask(&extra, 99).unwrap();
    assert!(refused["id"].is_null());
    assert_eq!(refused["error"]["code"], "conflict");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains(&control::MAX_CLIENTS.to_string())
    );
    let mut rest = String::new();
    assert_eq!(
        BufReader::new(extra).read_line(&mut rest).unwrap_or(0),
        0,
        "{rest}"
    );
    // Once one closes, a new client is taken (its place is given back as its thread ends).
    drop(open.pop());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let s = std::net::TcpStream::connect(addr).unwrap();
        if ask(&s, 1).is_some_and(|r| r["result"].is_array()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no place came free");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}
