//! The tools: their definitions (JSON schemas) and what they run. Every tool is a thin wrapper
//! over engine commands; the same list serves the headless and the connected backend.

use serde_json::{Map, Value, json};

use crate::backend::Backend;

/// Most commands in one `batch` call.
pub const MAX_STEPS: usize = 10_000;

fn obj(props: Value, required: &[&str]) -> Value {
    let mut o = json!({"type": "object", "properties": props, "additionalProperties": false});
    if !required.is_empty() {
        o["required"] = json!(required);
    }
    o
}

fn tool(name: &str, title: &str, desc: &str, schema: Value, read_only: bool) -> Value {
    json!({
        "name": name,
        "title": title,
        "description": desc,
        "inputSchema": schema,
        "annotations": {
            "title": title,
            "readOnlyHint": read_only,
            "destructiveHint": false,
            "idempotentHint": read_only,
            "openWorldHint": false,
        },
    })
}

fn string(desc: &str) -> Value {
    json!({"type": "string", "description": desc})
}

fn integer(desc: &str) -> Value {
    json!({"type": "integer", "minimum": 0, "description": desc})
}

/// Every tool, with its JSON schema.
pub fn tool_definitions() -> Vec<Value> {
    vec![
        tool(
            "list_commands",
            "List commands",
            "Every engine command with its summary, its parameters as JSON Schema and an example. \
             Names are dotted and lower-case (block.add, connect, flow.set, resource.add, \
             experiment.set, run, file.save). Optional case-insensitive substring `filter` over \
             the name and summary.",
            obj(
                json!({"filter": string("Text to match, e.g. \"block\", \"resource\", \"run\"")}),
                &[],
            ),
            true,
        ),
        tool(
            "execute",
            "Execute a command",
            "Run one engine command with JSON parameters, exactly as the app, scripts and the \
             command line do; it never opens a dialog. Examples: block.add {kind: \"source\", id: \
             \"arrivals\", interarrival: {type: \"exponential\", mean: 2}}; block.add {kind: \
             \"server\", id: \"desk\", service: {type: \"exponential\", mean: 1.5}}; block.add \
             {kind: \"sink\", id: \"done\"}; connect {from: \"arrivals\", to: \"desk\"}; \
             experiment.set {replications: 20, duration: 480}. Returns the command's result; a \
             failure leaves the model unchanged and says why.",
            obj(
                json!({
                    "command": string("Command name (see list_commands)"),
                    "params": {"type": "object", "description": "The command's parameters"},
                }),
                &["command"],
            ),
            false,
        ),
        tool(
            "batch",
            "Run several commands",
            "Run commands in order, stopping at the first error. Each step is {command, params}. \
             The reply has every result and, on an error, the failing step (index from 0) and its \
             error; the steps before it stay applied (`undo` takes them back one by one).",
            obj(
                json!({
                    "commands": {
                        "type": "array",
                        "description": "Steps: [{\"command\": name, \"params\": {...}}, ...]",
                        "items": {
                            "type": "object",
                            "properties": {
                                "command": {"type": "string"},
                                "params": {"type": "object"},
                            },
                            "required": ["command"],
                        },
                    },
                }),
                &["commands"],
            ),
            false,
        ),
        tool(
            "inspect_model",
            "Inspect the model",
            "The model (model.get: its elements with their kinds and parameters, connections, \
             shared pools, calendars and experiment) and its edit history (history: what undo \
             and redo would do, and whether there are unsaved changes).",
            obj(json!({}), &[]),
            true,
        ),
        tool(
            "check_model",
            "Check the model",
            "Every problem with the model (model.validate), errors first, each with the element it \
             is at, a stable code and a message. A model with errors does not run.",
            obj(json!({}), &[]),
            true,
        ),
        tool(
            "open",
            "Open a model",
            "Open a BPMN (.bpmn) or XPDL (.xpdl) file with BPSim data (file.open), replacing the \
             model and clearing the history. The reply lists the file's scenarios, what \
             StochCraft noted reading it, and the model's problems.",
            obj(
                json!({
                    "path": string("Path to a .bpmn or .xpdl file"),
                    "scenario": string("The BPSim scenario to read (default: the first)"),
                }),
                &["path"],
            ),
            false,
        ),
        tool(
            "save",
            "Save the model",
            "Save the model (file.save): into the file it came from, keeping everything StochCraft \
             does not understand, or to `path`. `as_new` writes a new BPMN file from the model \
             alone, for an edit the opened file cannot take.",
            obj(
                json!({
                    "path": string("Where to save (default: the model's file)"),
                    "as_new": {"type": "boolean", "description": "Write a new BPMN file from the model alone"},
                }),
                &[],
            ),
            false,
        ),
        tool(
            "run",
            "Run the experiment",
            "Run the model's experiment now (run) and return the results: for each element and \
             metric (entered, left, mean queue length, mean wait, utilisation, mean time in \
             system, …) the mean across replications with its 95% confidence interval \
             (half_width), and the run record (seed, replications, warm-up, duration, model \
             digest) that reproduces them.",
            obj(
                json!({
                    "replications": integer("Replications (default: the experiment's)"),
                    "seed": integer("Random seed (default: the experiment's)"),
                }),
                &[],
            ),
            false,
        ),
        tool(
            "undo",
            "Undo",
            "Undo the last edit.",
            obj(json!({}), &[]),
            false,
        ),
        tool(
            "redo",
            "Redo",
            "Redo the last undone edit.",
            obj(json!({}), &[]),
            false,
        ),
    ]
}

fn type_matches(t: &str, v: &Value) -> bool {
    match t {
        "string" => v.is_string(),
        "number" => v.is_number(),
        "integer" => v.is_i64() || v.is_u64(),
        "boolean" => v.is_boolean(),
        "array" => v.is_array(),
        "object" => v.is_object(),
        "null" => v.is_null(),
        _ => true,
    }
}

/// Checks `args` against the tool's schema (top level): unknown tool, missing required
/// arguments, unknown arguments and wrong types.
pub(crate) fn check_arguments(name: &str, args: &Value) -> Result<(), String> {
    let defs = tool_definitions();
    let shown: String = name.chars().take(100).collect();
    let def = defs
        .iter()
        .find(|d| d["name"] == name)
        .ok_or_else(|| format!("unknown tool `{shown}`"))?;
    let schema = &def["inputSchema"];
    let Some(a) = args.as_object() else {
        return Err(format!("{name}: arguments must be an object"));
    };
    for r in schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        if a.get(r).is_none_or(Value::is_null) {
            return Err(format!("{name}: missing required argument `{r}`"));
        }
    }
    let props = schema["properties"].as_object();
    for (k, v) in a {
        let Some(p) = props.and_then(|p| p.get(k)) else {
            return Err(format!(
                "{name}: unknown argument `{}`",
                k.chars().take(100).collect::<String>()
            ));
        };
        if v.is_null() {
            continue;
        }
        if let Some(t) = p["type"].as_str()
            && !type_matches(t, v)
        {
            return Err(format!("{name}: argument `{k}` must be of type {t}"));
        }
    }
    Ok(())
}

/// A tool result: one text block and the error flag.
fn text_result(v: &Value, is_error: bool) -> Value {
    let text = match v {
        Value::String(s) => s.clone(),
        v => serde_json::to_string_pretty(v).unwrap_or_default(),
    };
    let mut r = json!({"content": [{"type": "text", "text": text}]});
    if is_error {
        r["isError"] = json!(true);
    }
    r
}

/// Runs a tool. A failure is a tool result with `isError: true` (the agent reads the message and
/// corrects); nothing here panics on hostile arguments.
pub fn call_tool(b: &mut dyn Backend, name: &str, args: &Value) -> Value {
    let empty = Map::new();
    let a = args.as_object().unwrap_or(&empty);
    if name == "batch" {
        return batch(b, a);
    }
    match dispatch(b, name, a) {
        Ok(v) => text_result(&v, false),
        Err(e) => text_result(&Value::String(e), true),
    }
}

/// The arguments named in `keys` that were given, as a command's parameters.
fn pick(a: &Map<String, Value>, keys: &[&str]) -> Value {
    Value::Object(
        keys.iter()
            .filter_map(|k| {
                a.get(*k)
                    .filter(|v| !v.is_null())
                    .map(|v| ((*k).to_owned(), v.clone()))
            })
            .collect(),
    )
}

fn dispatch(b: &mut dyn Backend, name: &str, a: &Map<String, Value>) -> Result<Value, String> {
    match name {
        "list_commands" => {
            let all = b.commands()?;
            let Some(f) = a
                .get("filter")
                .and_then(Value::as_str)
                .map(str::to_lowercase)
                .filter(|f| !f.is_empty())
            else {
                return Ok(all);
            };
            let hit = |c: &Value| {
                ["name", "summary"].iter().any(|k| {
                    c[*k]
                        .as_str()
                        .is_some_and(|s| s.to_lowercase().contains(&f))
                })
            };
            Ok(Value::Array(
                all.as_array()
                    .map(|v| v.iter().filter(|c| hit(c)).cloned().collect())
                    .unwrap_or_default(),
            ))
        }
        "execute" => {
            let command = a
                .get("command")
                .and_then(Value::as_str)
                .ok_or("`command` is required")?;
            let params = a
                .get("params")
                .cloned()
                .filter(Value::is_object)
                .unwrap_or_else(|| json!({}));
            b.execute(command, params)
        }
        "inspect_model" => Ok(json!({
            "model": b.execute("model.get", json!({}))?,
            "history": b.execute("history", json!({}))?,
        })),
        "check_model" => b.execute("model.validate", json!({})),
        "open" => b.execute("file.open", pick(a, &["path", "scenario"])),
        "save" => b.execute("file.save", pick(a, &["path", "as_new"])),
        "run" => b.execute("run", pick(a, &["replications", "seed"])),
        "undo" => b.execute("undo", json!({})),
        "redo" => b.execute("redo", json!({})),
        _ => Err(format!(
            "unknown tool `{}`",
            name.chars().take(100).collect::<String>()
        )),
    }
}

fn batch(b: &mut dyn Backend, a: &Map<String, Value>) -> Value {
    let Some(steps) = a.get("commands").and_then(Value::as_array) else {
        return text_result(
            &json!("`commands` must be an array of {command, params}"),
            true,
        );
    };
    if steps.len() > MAX_STEPS {
        return text_result(
            &json!(format!(
                "batch too long ({} steps, at most {MAX_STEPS})",
                steps.len()
            )),
            true,
        );
    }
    let mut results = Vec::with_capacity(steps.len());
    for (i, step) in steps.iter().enumerate() {
        let command = step.get("command").and_then(Value::as_str);
        let r = match command {
            Some(name) => b.execute(
                name,
                step.get("params")
                    .cloned()
                    .filter(Value::is_object)
                    .unwrap_or_else(|| json!({})),
            ),
            None => Err("missing `command`".to_owned()),
        };
        match r {
            Ok(v) => results.push(v),
            Err(e) => {
                return text_result(
                    &json!({
                        "ok": false,
                        "completed": i,
                        "failed": {"index": i, "command": command, "error": e},
                        "results": results,
                    }),
                    true,
                );
            }
        }
    }
    text_result(
        &json!({"ok": true, "completed": results.len(), "results": results}),
        false,
    )
}
