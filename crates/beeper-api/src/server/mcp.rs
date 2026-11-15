//! Desktop's MCP server at `/v0/mcp`: stateless streamable HTTP, with the `beeper_desktop_api_api` tools. A tool
//! calls its API route with the arguments, like the Stainless client of Desktop.

use super::{empty, json_with, query_values, respond, Reply, Response};
use crate::codec::url_encode;
use crate::http::{parse_query, HttpMessage, StatusCode};
use beeper_core::cloud::Cloud;
use beeper_core::store::SharedStore;
use serde_json::{json, Value};
use std::sync::LazyLock;

/// The tools of Desktop in its order, with the API route of each, from `scripts/mcp_tools.mjs`.
static TOOLS: LazyLock<Vec<Value>> =
    LazyLock::new(|| serde_json::from_str(include_str!("../../assets/mcp_tools.json")).expect("MCP tools JSON"));
const VERSIONS: [&str; 4] = ["2025-06-18", "2025-03-26", "2024-11-05", "2024-10-07"];
const INSTRUCTIONS: &str = "Access to all chats and messages across networks using Beeper Desktop. Can be used to find, get, send, and manage messages and chats.";

fn rpc_error(status: u16, code: i64, message: &str) -> Reply {
    Ok(json_with(
        StatusCode(status),
        &json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null }),
    ))
}

/// The tools that the `tool` and `no_tool` query parameters of the MCP URL keep, like Desktop's `pE`. A `tool` that
/// names no tool is an error.
fn tools(msg: &HttpMessage) -> Result<Vec<&'static Value>, String> {
    let query = msg.start_line.split(' ').nth(1).and_then(|t| t.split_once('?')).map_or("", |(_, q)| q);
    let query = parse_query(query);
    let (include, exclude) = (query_values(&query, "tool"), query_values(&query, "no_tool"));
    let unknown: Vec<String> = include
        .iter()
        .filter(|n| !TOOLS.iter().any(|t| t["tool"]["name"] == **n))
        .map(|n| format!("tool={n}"))
        .collect();
    if !unknown.is_empty() {
        return Err(format!("The following filters did not match any endpoints: {}", unknown.join(", ")));
    }
    let name = |t: &Value| t["tool"]["name"].as_str().expect("a tool name").to_string();
    let kept = TOOLS.iter().filter(|t| {
        (include.is_empty() || include.contains(&name(t).as_str())) && !exclude.contains(&name(t).as_str())
    });
    Ok(kept.collect())
}

/// Whether a JSON-RPC message is a request, a notification, or a response, like the schema of the MCP SDK.
fn jsonrpc(m: &Value) -> bool {
    let id = m.get("id").is_some_and(|id| id.is_string() || id.is_i64() || id.is_u64());
    m["jsonrpc"] == "2.0"
        && match m.get("method") {
            Some(method) => method.is_string() && (m.get("id").is_none() || id),
            None => id && (m.get("result").is_some() || m["error"].is_object()),
        }
}

pub(super) fn handle(
    store: &SharedStore,
    cloud: &Cloud,
    method: &str,
    msg: &HttpMessage,
    token: Option<&str>,
) -> Reply {
    match method {
        "OPTIONS" => return empty(StatusCode::NO_CONTENT, Vec::new()),
        "POST" => {}
        _ => return rpc_error(405, -32000, "Method not supported"),
    }
    let accept = msg.header("accept").unwrap_or_default();
    if !accept.contains("application/json") || !accept.contains("text/event-stream") {
        return rpc_error(
            406,
            -32000,
            "Not Acceptable: Client must accept both application/json and text/event-stream",
        );
    }
    if !msg.header("content-type").is_some_and(|c| c.contains("application/json")) {
        return rpc_error(415, -32000, "Unsupported Media Type: Content-Type must be application/json");
    }
    let tools = match tools(msg) {
        Ok(tools) => tools,
        Err(e) => return rpc_error(400, -32000, &format!("Invalid request: {e}")),
    };
    let Ok(body) = serde_json::from_slice::<Value>(&msg.body) else { return rpc_error(400, -32700, "Parse error") };
    let batch = match body {
        Value::Array(all) => all,
        one => vec![one],
    };
    if let Some(bad) = batch.iter().find(|m| !jsonrpc(m)) {
        let data = format!("Invalid JSON-RPC message: {bad}");
        let body = json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": "Parse error", "data": data }, "id": null });
        return Ok(json_with(StatusCode::BAD_REQUEST, &body));
    }
    let initialize = batch.iter().any(|m| m["method"] == "initialize");
    if initialize && batch.len() > 1 {
        return rpc_error(400, -32600, "Invalid Request: Only one initialization request is allowed");
    }
    if !initialize && !VERSIONS.contains(&msg.header("mcp-protocol-version").unwrap_or(VERSIONS[1])) {
        let supported =
            format!("Bad Request: Unsupported protocol version (supported versions: {})", VERSIONS.join(", "));
        return rpc_error(400, -32000, &supported);
    }
    let requests = batch.iter().filter(|m| m.get("id").is_some() && m["method"].is_string());
    let events: String =
        requests.map(|m| format!("event: message\ndata: {}\n\n", answer(store, cloud, &tools, m, token))).collect();
    if events.is_empty() {
        return empty(StatusCode(202), Vec::new());
    }
    let headers = vec![("content-type", "text/event-stream".into()), ("cache-control", "no-cache".into())];
    Ok(Response { status: StatusCode::OK, headers, body: events.into_bytes() })
}

fn answer(store: &SharedStore, cloud: &Cloud, tools: &[&Value], m: &Value, token: Option<&str>) -> Value {
    let result = match m["method"].as_str().expect("a request method") {
        "initialize" => {
            let asked = m["params"]["protocolVersion"].as_str().filter(|v| VERSIONS.contains(v));
            Ok(json!({
                "protocolVersion": asked.unwrap_or(VERSIONS[0]),
                "capabilities": { "tools": {}, "logging": {} },
                "serverInfo": { "name": "beeper_desktop_api_api", "version": "4.2.2" },
                "instructions": INSTRUCTIONS
            }))
        }
        "ping" | "logging/setLevel" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools.iter().map(|t| &t["tool"]).collect::<Vec<_>>() })),
        "tools/call" => call(store, cloud, tools, &m["params"], token).map_err(|e| (-32603, e)),
        _ => Err((-32601, "Method not found".into())),
    };
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": m["id"], "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": m["id"], "error": { "code": code, "message": message } })
        }
    }
}

/// A path or query value.
fn param(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        v => v.to_string(),
    }
}

/// Calls the route with the token of the MCP client, so the route checks its scopes.
fn call(
    store: &SharedStore,
    cloud: &Cloud,
    tools: &[&Value],
    params: &Value,
    token: Option<&str>,
) -> Result<Value, String> {
    let name = params["name"].as_str().unwrap_or_default();
    let t = tools.iter().find(|t| t["tool"]["name"] == name).ok_or_else(|| format!("Unknown tool: {name}"))?;
    let mut args = params["arguments"].as_object().cloned().unwrap_or_default();
    let mut path = t["path"].as_str().expect("a tool path").to_string();
    while let Some(start) = path.find('{') {
        let end = start + path[start..].find('}').expect("a closed path parameter");
        let value = args.remove(&path[start + 1..end]).map(|v| param(&v)).unwrap_or_default();
        path.replace_range(start..=end, &url_encode(&value));
    }
    let method = t["method"].as_str().expect("a tool method");
    // The `repeat` array format of the Stainless client: `k=a&k=b`.
    let pair = |k: &String, v: &Value| format!("{}={}", url_encode(k), url_encode(&param(v)));
    let query: Vec<String> = args
        .iter()
        .flat_map(|(k, v)| match v {
            Value::Array(items) => items.iter().map(|i| pair(k, i)).collect(),
            v => vec![pair(k, v)],
        })
        .collect();
    let (target, body) = match method {
        "GET" | "DELETE" if query.is_empty() => (path, Vec::new()),
        "GET" | "DELETE" => (format!("{path}?{}", query.join("&")), Vec::new()),
        _ => (path, serde_json::to_vec(&args).expect("serialize tool arguments")),
    };
    let (status, bytes) = if target.starts_with("https://") {
        (200, tokio::runtime::Handle::current().block_on(beeper_core::cloud::http_bytes(&target))?)
    } else {
        let headers = [("content-type", "application/json"), ("accept", "application/json")];
        let mut headers: std::collections::HashMap<_, _> =
            headers.into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        headers.extend(token.map(|t| ("authorization".to_string(), format!("Bearer {t}"))));
        let msg = HttpMessage { start_line: format!("{method} {target} HTTP/1.1"), headers, body };
        let resp = respond(store, cloud, msg).unwrap_or_else(|e| e);
        (resp.status.0, resp.body)
    };
    let text = String::from_utf8_lossy(&bytes);
    if status >= 300 {
        return Err(format!("{status} {text}"));
    }
    let value = match t["text"] == true {
        true => Value::String(text.into()),
        false => serde_json::from_slice(&bytes).map_err(|e| format!("{target} did not answer JSON: {e}"))?,
    };
    let text = serde_json::to_string_pretty(&value).expect("serialize the tool result");
    Ok(json!({ "content": [{ "type": "text", "text": text }] }))
}
