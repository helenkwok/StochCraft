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

/// What a web page can send to a local port without a preflight (a `text/plain` POST) carries a
/// request in its body: refused at its first line, and the body is never run.
#[test]
fn an_http_post_with_a_request_in_its_body_runs_nothing() {
    use std::io::{BufRead, BufReader, Write};
    let (addr, calls) = control::listen_with_token(0, TOKEN, || {}).unwrap();
    let stream = std::net::TcpStream::connect(addr).unwrap();
    let mut w = stream.try_clone().unwrap();
    let body = r#"{"id":1,"method":"engine.execute","params":{"command":"file.save","params":{"path":"pwned.bpmn"}}}"#;
    let request = format!(
        "POST / HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://example.com\r\n\
         Content-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}\n",
        body.len() + 1
    );
    // All at once, as a browser sends it; the channel may reset the connection while the rest is
    // unread, so its answer may not arrive, but if it does it is a refusal.
    let _ = w.write_all(request.as_bytes());
    let mut reply = String::new();
    if BufReader::new(stream).read_line(&mut reply).is_ok() && !reply.is_empty() {
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["error"]["code"], "unauthorized", "{reply}");
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(calls.try_recv().is_err(), "the body was run");
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
    assert!(matches!(e, stochcraft_mcp::ConnectError::Refused(_)), "{e}");
    // Not a reason to fall back to headless: the agent would not be in the window.
    let refused = stochcraft_mcp::backend(Options {
        connect: Some(addr),
        token: Some("wrong".into()),
        ..Options::default()
    });
    assert!(refused.err().unwrap().contains("refused the connection"));
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
    // No token given anywhere (the environment's is passed in as none): the file's is found.
    let (backend, note) = stochcraft_mcp::backend_with_environment(
        Options {
            connect: Some(channel.addr.to_string()),
            token_file: Some(path.clone()),
            ..Options::default()
        },
        None,
    )
    .unwrap();
    assert!(note.is_none(), "{note:?}");
    let mut s = Session::new(backend);
    assert!(
        !s.ok("list_commands", json!({}))
            .as_array()
            .unwrap()
            .is_empty()
    );
    // Without the file and without a token the app is not running: a headless session, with a
    // note naming the file.
    std::fs::remove_file(&path).unwrap();
    let (_, note) = stochcraft_mcp::backend_with_environment(
        Options {
            connect: Some(channel.addr.to_string()),
            token_file: Some(path.clone()),
            ..Options::default()
        },
        None,
    )
    .unwrap();
    let note = note.unwrap_or_default();
    assert!(
        note.contains("control.json") && note.contains("headless"),
        "{note}"
    );
}

/// A client that trickles bytes with no newline is refused when the deadline from its connecting
/// passes, not a read timeout after each byte; a line over the limit is refused at once.
#[test]
fn a_slow_or_endless_first_line_is_refused() {
    use std::io::{BufRead, BufReader, Write};
    use std::time::{Duration, Instant};
    let (addr, calls) = control::listen_with_token(0, TOKEN, || {}).unwrap();
    let refusal = |stream: std::net::TcpStream| -> (Value, Duration) {
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        let started = Instant::now();
        let mut reply = String::new();
        BufReader::new(stream).read_line(&mut reply).unwrap();
        (serde_json::from_str(&reply).unwrap(), started.elapsed())
    };

    // One byte every 300 ms, never a newline.
    let stream = std::net::TcpStream::connect(addr).unwrap();
    let mut w = stream.try_clone().unwrap();
    let trickle = std::thread::spawn(move || {
        for _ in 0..40 {
            if w.write_all(b" ").is_err() {
                return;
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    });
    let (reply, took) = refusal(stream);
    assert_eq!(reply["error"]["code"], "unauthorized");
    assert!(
        took > Duration::from_millis(1500) && took < Duration::from_secs(5),
        "{took:?}"
    );
    trickle.join().unwrap();

    // Over the limit with no newline: well before the deadline.
    let stream = std::net::TcpStream::connect(addr).unwrap();
    let mut w = stream.try_clone().unwrap();
    w.write_all(&[b'a'; 2000]).unwrap();
    let (reply, took) = refusal(stream);
    assert_eq!(reply["error"]["code"], "unauthorized");
    assert!(took < Duration::from_millis(1000), "{took:?}");
    assert!(calls.try_recv().is_err());
}

#[test]
fn the_control_channel_takes_a_limited_number_of_clients() {
    use std::io::{BufRead, BufReader, Write};
    let addr = app_with_control();
    // A client sends its token line as soon as it connects, as a real one does (the channel waits
    // only a short time for it); `ask` then reads the answer to the token (the refusal itself for
    // a client over the limit, which has no id) and sends the request.
    let connect = || {
        let s = std::net::TcpStream::connect(addr).unwrap();
        writeln!(&s, r#"{{"token":"{TOKEN}"}}"#).unwrap();
        s
    };
    let ask = |stream: &std::net::TcpStream, id: u64| -> Option<Value> {
        let mut w = stream.try_clone().unwrap();
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
    let mut open: Vec<std::net::TcpStream> = (0..control::MAX_CLIENTS).map(|_| connect()).collect();
    for (i, s) in open.iter().enumerate() {
        assert!(ask(s, i as u64).unwrap()["result"].is_array());
    }
    // One more is told why, under its own id, and closed.
    let extra = connect();
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
    // The command line side is told, not given a headless engine in its place.
    let full = stochcraft_mcp::backend_with_environment(
        Options {
            connect: Some(addr.to_string()),
            token: Some(TOKEN.into()),
            ..Options::default()
        },
        None,
    );
    let message = full
        .err()
        .expect("all places taken is an error, not headless");
    assert!(message.contains("refused the connection"), "{message}");
    assert!(
        message.contains(&control::MAX_CLIENTS.to_string()),
        "{message}"
    );
    // Once one closes, a new client is taken (its place is given back as its thread ends).
    drop(open.pop());
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let s = connect();
        if ask(&s, 1).is_some_and(|r| r["result"].is_array()) {
            break;
        }
        assert!(std::time::Instant::now() < deadline, "no place came free");
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
}

#[test]
fn a_file_read_with_errors_does_not_run_for_an_agent_unless_it_allows_it() {
    let dir = Scratch::new("errors");
    let text = std::fs::read_to_string(models().join("mm1.bpmn")).unwrap();
    // A processing-time expression, which the reader cannot use: it reads as a time of zero.
    let altered = text.replacen(
        "<bpsim:NegativeExponentialDistribution mean=\"1\"/>",
        "<bpsim:ExpressionParameter value=\"1 + 9\"/>",
        1,
    );
    assert_ne!(text, altered, "the fixture changed");
    let file = dir.0.join("expression.bpmn");
    std::fs::write(&file, altered).unwrap();

    let mut s = Session::new(Box::new(Headless::default()));
    let opened = s.ok("open", json!({"path": file.to_string_lossy()}));
    assert!(
        opened["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["severity"] == "error"),
        "{opened}"
    );
    let (refused, error) = s.call("run", json!({"replications": 2}));
    assert!(error, "{refused}");
    assert_eq!(refused["code"], "invalid_model", "{refused}");
    assert!(
        refused["message"]
            .as_str()
            .unwrap()
            .contains("allow_approximations")
    );
    let allowed = s.ok(
        "execute",
        json!({"command": "run", "params": {"replications": 2, "allow_approximations": true}}),
    );
    assert!(
        !allowed["record"]["approximations"]
            .as_array()
            .unwrap()
            .is_empty(),
        "{}",
        allowed["record"]
    );
}

/// An outside optimiser's evaluation is offered as every command is: listed with its schema, and
/// carried out through `execute`, giving what the engine gives.
#[test]
fn experiment_evaluate_is_listed_and_runs() {
    let mut s = Session::new(Box::new(Headless::default()));
    let listed = s.ok("list_commands", json!({"filter": "experiment.evaluate"}));
    let listed = listed.as_array().unwrap();
    assert!(
        listed
            .iter()
            .any(|c| c["name"] == "experiment.evaluate" && c["params"].is_object()),
        "{listed:?}"
    );
    let path = models().join("mm1.bpmn");
    s.ok("open", json!({"path": path.to_string_lossy()}));
    let params = json!({
        "decisions": [{"id": "serve", "field": "mean", "value": 0.8}],
        "results": [{"element": "serve", "metric": "mean_wait"}],
        "first": 2,
        "count": 3,
        "seed": 11,
    });
    let r = s.ok(
        "execute",
        json!({"command": "experiment.evaluate", "params": params.clone()}),
    );
    assert_eq!(r["first"], 2);
    assert_eq!(r["results"][0]["values"].as_array().map(Vec::len), Some(3));
    let mut e = Engine::new();
    e.execute("file.open", json!({"path": path.to_string_lossy()}))
        .unwrap();
    let direct = e.execute("experiment.evaluate", params).unwrap();
    assert_eq!(r["results"], direct["results"]);
    let (err, error) = s.call(
        "execute",
        json!({"command": "experiment.evaluate", "params": {
            "decisions": [{"id": "ghost", "field": "mean", "value": 1.0}],
            "results": [{"element": "serve", "metric": "mean_wait"}],
        }}),
    );
    assert!(error);
    assert!(err.to_string().contains("ghost.mean"), "{err}");
}
