//! `screenpeek mcp`: the commands as MCP tools over stdio.

use std::io::{BufRead, Write};
use std::process::Command;

use anyhow::Result;
use serde_json::{json, Map, Value};

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// Required positional text.
    Text,
    /// Optional positional text, after the required ones.
    Extra,
    /// `--name VALUE`.
    Option,
    /// `--name SECONDS`.
    Number,
    /// `--name` when true.
    Flag,
    /// Several positional texts.
    Steps,
}

struct Tool {
    name: &'static str,
    description: &'static str,
    params: &'static [(&'static str, Kind, &'static str)],
}

const TOOLS: &[Tool] = &[
    Tool {
        name: "scan",
        description: "List desktop text and controls as `id text @x,y [states]`. Use instead of a screenshot to find labelled controls.",
        params: &[
            ("grep", Kind::Option, "Only elements containing this text"),
            ("focused", Kind::Flag, "Only the focused window"),
            ("region", Kind::Option, "Only x,y,width,height"),
            ("json", Kind::Flag, "JSON with size, role and states"),
        ],
    },
    Tool {
        name: "click",
        description: "Click an element by its text or scan ID.",
        params: &[
            ("target", Kind::Text, "Text or ID from the last scan"),
            ("fresh", Kind::Flag, "Scan again first; use after the layout changed"),
            ("check", Kind::Flag, "Warn when nothing near the target changes"),
            ("button", Kind::Option, "left, right or middle"),
            ("double", Kind::Flag, "Double click"),
        ],
    },
    Tool {
        name: "fill",
        description: "Click a field and type into it (does not clear it).",
        params: &[
            ("target", Kind::Text, "Field text or ID"),
            ("text", Kind::Text, "Text to type"),
            ("fresh", Kind::Flag, "Scan again first"),
        ],
    },
    Tool {
        name: "type",
        description: "Type text into the focused app.",
        params: &[("text", Kind::Text, "Text to type")],
    },
    Tool {
        name: "key",
        description: "Press a key or combination such as ctrl+s, alt+F4 or enter.",
        params: &[("combination", Kind::Text, "Key combination")],
    },
    Tool {
        name: "wait",
        description: "Wait until an element appears, or disappears with gone.",
        params: &[
            ("target", Kind::Text, "Text or ID"),
            ("timeout", Kind::Number, "Seconds, default 10"),
            ("gone", Kind::Flag, "Wait for it to disappear"),
        ],
    },
    Tool {
        name: "scroll",
        description: "Scroll the wheel, optionally over an element.",
        params: &[
            ("direction", Kind::Text, "up, down, left or right"),
            ("amount", Kind::Extra, "Wheel steps, default 3"),
            ("at", Kind::Option, "Element to scroll over"),
        ],
    },
    Tool {
        name: "drag",
        description: "Drag one element onto another.",
        params: &[
            ("from", Kind::Text, "Element to drag"),
            ("to", Kind::Text, "Where to drop it"),
        ],
    },
    Tool {
        name: "run",
        description: "Run steps in order, stopping at the first failure: `click T`, `fill T with TEXT`, `type TEXT`, `key COMBO`, `wait T`, `scroll down 3`, `drag A to B`.",
        params: &[("steps", Kind::Steps, "Steps in order")],
    },
];

pub fn serve() -> Result<()> {
    let mut out = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(request) => match request.get("id") {
                Some(id) => answer(id.clone(), &request),
                None => continue,
            },
            Err(_) => json!({"jsonrpc": "2.0", "id": null,
                "error": {"code": -32700, "message": "parse error"}}),
        };
        writeln!(out, "{reply}")?;
        out.flush()?;
    }
    Ok(())
}

fn answer(id: Value, request: &Value) -> Value {
    let params = &request["params"];
    let result = match request["method"].as_str().unwrap_or_default() {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or("2025-06-18"),
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "screenpeek", "version": env!("CARGO_PKG_VERSION")},
        }),
        "ping" => json!({}),
        "tools/list" => json!({"tools": TOOLS.iter().map(schema).collect::<Vec<_>>()}),
        "tools/call" => call(params),
        method => {
            return json!({"jsonrpc": "2.0", "id": id,
                "error": {"code": -32601, "message": format!("unknown method {method}")}})
        }
    };
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn schema(tool: &Tool) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for (name, kind, description) in tool.params {
        let kind = *kind;
        let schema = match kind {
            Kind::Flag => json!({"type": "boolean", "description": description}),
            Kind::Number => json!({"type": "number", "description": description}),
            Kind::Steps => {
                json!({"type": "array", "items": {"type": "string"}, "description": description})
            }
            _ => json!({"type": "string", "description": description}),
        };
        properties.insert(name.to_string(), schema);
        if matches!(kind, Kind::Text | Kind::Steps) {
            required.push(*name);
        }
    }
    json!({
        "name": tool.name,
        "description": tool.description,
        "inputSchema": {"type": "object", "properties": properties, "required": required},
    })
}

/// Command-line arguments for a tool call; positionals follow `--` so that
/// text starting with a dash is never read as a flag.
fn arguments(tool: &Tool, given: &Value) -> Result<Vec<String>, String> {
    let mut flags = vec![tool.name.to_string()];
    let mut positional = Vec::new();
    for (name, kind, _) in tool.params {
        let value = &given[name];
        let flag = format!("--{name}");
        match (kind, value) {
            (_, Value::Null) if matches!(kind, Kind::Text | Kind::Steps) => {
                return Err(format!("{name} is required"))
            }
            (_, Value::Null) | (Kind::Flag, Value::Bool(false)) => {}
            (Kind::Flag, Value::Bool(true)) => flags.push(flag),
            (Kind::Option | Kind::Number, value) => flags.extend([flag, text(value)]),
            (Kind::Steps, Value::Array(steps)) => positional.extend(steps.iter().map(text)),
            (Kind::Text | Kind::Extra, value) => positional.push(text(value)),
            _ => return Err(format!("{name} has the wrong type")),
        }
    }
    flags.push("--".into());
    flags.extend(positional);
    Ok(flags)
}

fn text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn call(params: &Value) -> Value {
    let name = params["name"].as_str().unwrap_or_default();
    let outcome = TOOLS
        .iter()
        .find(|tool| tool.name == name)
        .ok_or_else(|| format!("unknown tool {name}"))
        .and_then(|tool| arguments(tool, &params["arguments"]))
        .and_then(|args| {
            let exe = std::env::current_exe().map_err(|error| error.to_string())?;
            Command::new(exe)
                .args(args)
                .output()
                .map_err(|error| error.to_string())
        });
    let (text, failed) = match outcome {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            text.push_str(&String::from_utf8_lossy(&output.stderr));
            (text, !output.status.success())
        }
        Err(error) => (error, true),
    };
    let text = match text.trim_end() {
        "" => "(no output: nothing matched)",
        text => text,
    };
    json!({"content": [{"type": "text", "text": text}], "isError": failed})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str) -> &'static Tool {
        TOOLS.iter().find(|tool| tool.name == name).unwrap()
    }

    #[test]
    fn calls_become_safe_argument_lists() {
        let args = arguments(
            tool("click"),
            &json!({"target": "-rf", "fresh": true, "double": false}),
        );
        assert_eq!(args.unwrap(), ["click", "--fresh", "--", "-rf"]);
        let args = arguments(
            tool("scroll"),
            &json!({"direction": "down", "amount": 5, "at": "List"}),
        );
        assert_eq!(args.unwrap(), ["scroll", "--at", "List", "--", "down", "5"]);
        let args = arguments(tool("run"), &json!({"steps": ["click File", "key enter"]}));
        assert_eq!(args.unwrap(), ["run", "--", "click File", "key enter"]);
        assert!(arguments(tool("click"), &json!({})).is_err());
    }

    #[test]
    fn every_tool_lists_its_required_parameters() {
        let listed = answer(json!(1), &json!({"method": "tools/list"}));
        let tools = listed["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), TOOLS.len());
        let fill = tools.iter().find(|tool| tool["name"] == "fill").unwrap();
        assert_eq!(fill["inputSchema"]["required"], json!(["target", "text"]));
    }
}
