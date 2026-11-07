// SPDX-License-Identifier: Apache-2.0

//! `lampl mcp` advertises the schema enums and surfaces every failure as a tool error.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use lampl_core::{LinkType, MemoryType};
use serde_json::{Value, json};

fn session(dir: &Path, calls: &[(&str, Value)]) -> Vec<Value> {
    let mut lines = vec![
        json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}),
        json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
    ];
    for (i, (method, params)) in calls.iter().enumerate() {
        lines.push(json!({"jsonrpc":"2.0","id":i + 1,"method":method,"params":params}));
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .arg("mcp")
        .env("LAMPL_CONFIG_DIR", dir)
        .env("LAMPL_DATA_DIR", dir)
        .env_remove("LAMPL_SERVER")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn lampl mcp");
    let mut stdin = child.stdin.take().unwrap();
    for l in &lines {
        writeln!(stdin, "{l}").unwrap();
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| panic!("stdout line not JSON ({e}): {l}"))
        })
        .collect()
}

fn reply(msgs: &[Value], id: u64) -> &Value {
    msgs.iter()
        .find(|m| m["id"] == id)
        .unwrap_or_else(|| panic!("no reply with id {id}: {msgs:?}"))
}

fn tool_call(name: &str, arguments: &Value) -> (&'static str, Value) {
    ("tools/call", json!({"name": name, "arguments": arguments}))
}

fn error_text(msgs: &[Value], id: u64) -> String {
    let result = &reply(msgs, id)["result"];
    assert_eq!(result["isError"], true, "expected a tool error: {result}");
    result["content"][0]["text"].as_str().unwrap().to_owned()
}

fn names(v: &Value) -> Vec<&str> {
    v.as_array()
        .unwrap()
        .iter()
        .map(|e| e.as_str().unwrap())
        .collect()
}

#[test]
fn tools_list_advertises_enums_generated_from_core() {
    let d = tempfile::tempdir().unwrap();
    let msgs = session(d.path(), &[("tools/list", json!({}))]);
    let tools = reply(&msgs, 1)["result"]["tools"].as_array().unwrap();
    let tool = |n: &str| {
        tools
            .iter()
            .find(|t| t["name"] == n)
            .unwrap_or_else(|| panic!("no tool {n}"))
    };
    let mut listed: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    listed.sort_unstable();
    assert_eq!(listed, ["get", "link", "recall", "related", "store"]);

    let memory_types: Vec<&str> = MemoryType::ALL.iter().map(|t| t.as_str()).collect();
    let link_types: Vec<&str> = LinkType::ALL.iter().map(|t| t.as_str()).collect();
    let store_enum = &tool("store")["inputSchema"]["properties"]["type"]["enum"];
    assert_eq!(names(store_enum), memory_types);
    let link_enum = &tool("link")["inputSchema"]["properties"]["type"]["enum"];
    assert_eq!(names(link_enum), link_types);
}

#[test]
fn unconfigured_client_returns_tool_errors_not_empty_results() {
    let d = tempfile::tempdir().unwrap();
    let id = "00000000-0000-0000-0000-000000000001";
    let msgs = session(
        d.path(),
        &[
            tool_call("recall", &json!({"query": "anything"})),
            tool_call("store", &json!({"title": "t", "body": "b"})),
            tool_call("get", &json!({"id": id})),
            tool_call("related", &json!({"id": id})),
            tool_call("link", &json!({"from": id, "to": id, "type": "SOLVES"})),
        ],
    );
    for call in 1..=5 {
        assert!(error_text(&msgs, call).contains("run `lampl init`"));
    }
    assert!(std::fs::read_dir(d.path()).unwrap().next().is_none());
}

#[test]
fn server_down_fails_closed_through_the_tools() {
    let d = tempfile::tempdir().unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["init", "--server", "http://127.0.0.1:1"])
        .env("LAMPL_CONFIG_DIR", d.path())
        .env("LAMPL_DATA_DIR", d.path())
        .status()
        .unwrap();
    assert!(init.success());
    let msgs = session(
        d.path(),
        &[
            tool_call("recall", &json!({"query": "anything"})),
            tool_call("store", &json!({"title": "t", "body": "b"})),
        ],
    );
    assert!(error_text(&msgs, 1).contains("not falling back"));
    assert!(error_text(&msgs, 2).contains("not falling back"));
}

#[test]
fn bad_arguments_and_unknown_tools_are_tool_errors_naming_the_valid_set() {
    let d = tempfile::tempdir().unwrap();
    let id = "00000000-0000-0000-0000-000000000001";
    let msgs = session(
        d.path(),
        &[
            tool_call(
                "store",
                &json!({"title": "t", "body": "b", "type": "bogus"}),
            ),
            tool_call("store", &json!({"title": "t"})),
            tool_call("link", &json!({"from": id, "to": id, "type": "NOPE"})),
            tool_call("recall", &json!({"query": "q", "surprise": 1})),
            tool_call("nonexistent", &json!({})),
        ],
    );
    let bad_memory_type = error_text(&msgs, 1);
    for t in MemoryType::ALL {
        assert!(bad_memory_type.contains(t.as_str()), "{bad_memory_type}");
    }
    assert!(error_text(&msgs, 2).contains("body"));
    let bad_link_type = error_text(&msgs, 3);
    for t in LinkType::ALL {
        assert!(bad_link_type.contains(t.as_str()), "{bad_link_type}");
    }
    assert!(error_text(&msgs, 4).contains("surprise"));
    assert!(error_text(&msgs, 5).contains("nonexistent"));
}

#[test]
fn local_mode_get_link_related_say_they_need_a_server() {
    let d = tempfile::tempdir().unwrap();
    let init = Command::new(env!("CARGO_BIN_EXE_lampl"))
        .args(["init", "--local"])
        .env("LAMPL_CONFIG_DIR", d.path())
        .env("LAMPL_DATA_DIR", d.path())
        .status()
        .unwrap();
    assert!(init.success());
    let id = "00000000-0000-0000-0000-000000000001";
    let msgs = session(d.path(), &[tool_call("get", &json!({"id": id}))]);
    assert!(error_text(&msgs, 1).contains("needs a server"));
}
