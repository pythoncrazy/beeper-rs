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

/// An error of the MCP SDK's transport, which writes the JSON with `writeHead`, so without a `Content-Type`.
fn rpc_error(status: u16, code: i64, message: &str) -> Reply {
    transport_error(status, &json!({ "jsonrpc": "2.0", "error": { "code": code, "message": message }, "id": null }))
}

fn transport_error(status: u16, body: &Value) -> Reply {
    let body = serde_json::to_vec(body).expect("serialize a JSON-RPC error");
    Ok(Response { status: StatusCode(status), headers: Vec::new(), body })
}

/// An error of Desktop's Stainless MCP routes, which answer with `res.json` and no `id`.
fn route_error(status: u16, message: &str) -> Reply {
    Ok(json_with(StatusCode(status), &json!({ "jsonrpc": "2.0", "error": { "code": -32000, "message": message } })))
}

/// Desktop's MCP client capabilities (`uh`), which the `capability` and `no_capability` parameters of the MCP URL
/// change. A missing capability makes Desktop rewrite the tools (`ux`) or the arguments (`sx`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Caps {
    top_level_unions: bool,
    valid_json: bool,
    refs: bool,
    unions: bool,
    formats: bool,
    name_len: Option<usize>,
}

const CAPS: Caps =
    Caps { top_level_unions: true, valid_json: true, refs: true, unions: true, formats: true, name_len: None };
const CAPABILITIES: [&str; 6] = ["top-level-unions", "valid-json", "refs", "unions", "formats", "tool-name-length"];
const CLIENTS: [&str; 5] = ["openai-agents", "claude", "claude-code", "cursor", "infer"];

/// The zod enum issue of a query value, as `zod-validation-error` words it.
fn enum_issue(path: String, allowed: &[&str], got: &str) -> String {
    let expected = allowed.iter().map(|a| format!("'{a}'")).collect::<Vec<_>>().join(" | ");
    format!("Invalid enum value. Expected {expected}, received '{got}' at \"{path}\"")
}

/// The tools of the MCP URL and the capabilities of its client, like Desktop's `fx`, `pE` and `u2`.
///
/// - The `resource`, `operation`, `tag` and `tool` filters (and their `no_` forms) pick the API tools. When every
///   filter excludes, a tool stays unless one matches; otherwise it goes unless one includes it. The last matching
///   filter wins, and the excludes come after the includes. A `tool` or `resource` that matches nothing is an error.
/// - `search_docs` is not filtered: `no_tools=docs` drops it and `tools=docs` keeps it.
/// - `client` only has to be a known client. Desktop infers the capabilities of a client from the `clientInfo` of
///   `initialize`, which a stateless request never has with `tools/list` or `tools/call`.
fn tools(msg: &HttpMessage) -> Result<(Vec<Value>, Caps), String> {
    let query = msg.start_line.split(' ').nth(1).and_then(|t| t.split_once('?')).map_or("", |(_, q)| q);
    let query = parse_query(query);
    let values = |k: &str| query_values(&query, k);
    // The zod issues in the order of Desktop's schema (`dx`).
    let check = |key: &str, allowed: &[&str]| -> Vec<String> {
        let values = values(key).into_iter().enumerate();
        values
            .filter(|(_, v)| !allowed.contains(v))
            .map(|(i, v)| enum_issue(format!("{key}[{i}]"), allowed, v))
            .collect()
    };
    let client = query.get("client").and_then(Value::as_str).filter(|c| !CLIENTS.contains(c));
    let issues: Vec<String> =
        [check("tools", &["docs"]), check("no_tools", &["docs"]), check("operation", &["read", "write"])]
            .into_iter()
            .chain([check("no_operation", &["read", "write"])])
            .chain([client.map(|c| enum_issue("client".into(), &CLIENTS, c)).into_iter().collect()])
            .chain([check("no_capability", &CAPABILITIES)])
            .flatten()
            .collect();
    if !issues.is_empty() {
        return Err(format!("Validation error: {}", issues.join("; ")));
    }
    let mut caps = CAPS;
    for c in values("capability") {
        match c {
            "top-level-unions" => caps.top_level_unions = true,
            "valid-json" => caps.valid_json = true,
            "refs" => caps.refs = true,
            "unions" => caps.unions = true,
            "formats" => caps.formats = true,
            _ => match c.strip_prefix("tool-name-length=").map(|n| (n, n.parse::<i64>())) {
                Some((_, Ok(n))) => caps.name_len = usize::try_from(n).ok().filter(|n| *n > 0),
                Some((n, Err(_))) if !n.contains('=') => {
                    return Err(format!("Invalid tool-name-length value: {n}. Expected a number."));
                }
                Some(_) => return Err("Invalid format for tool-name-length. Expected tool-name-length=N.".into()),
                None => {
                    let valid = CAPABILITIES.join(", ");
                    return Err(format!("Unknown capability: {c}. Valid capabilities are: {valid}"));
                }
            },
        }
    }
    for c in values("no_capability") {
        match c {
            "top-level-unions" => caps.top_level_unions = false,
            "valid-json" => caps.valid_json = false,
            "refs" => caps.refs = false,
            "unions" => caps.unions = false,
            "formats" => caps.formats = false,
            _ => caps.name_len = None,
        }
    }
    let kinds = ["resource", "operation", "tag", "tool"];
    let filters: Vec<(&str, bool, &str)> = [true, false]
        .into_iter()
        .flat_map(|include| {
            let values = &values;
            kinds.into_iter().flat_map(move |kind| {
                let key = if include { kind.to_string() } else { format!("no_{kind}") };
                values(&key).into_iter().map(move |v| (kind, include, v))
            })
        })
        .collect();
    let simple = |s: &str| -> String {
        s.to_lowercase().chars().filter(|c| c.is_ascii_lowercase() || ".*-_".contains(*c)).collect()
    };
    let matches = |(kind, _, value): &(&str, bool, &str), t: &Value| match *kind {
        "resource" => glob(&simple(value), &simple(t["resource"].as_str().unwrap_or_default())),
        "operation" => t["operation"] == *value,
        "tag" => t["tags"].as_array().is_some_and(|tags| tags.iter().any(|g| g == value)),
        _ => t["tool"]["name"] == *value,
    };
    let endpoints = TOOLS.iter().filter(|t| t["docs"] != true);
    let mut kept = Vec::new();
    let mut used = vec![false; filters.len()];
    for t in endpoints {
        let mut keep = filters.iter().all(|f| !f.1);
        for (i, f) in filters.iter().enumerate().filter(|(_, f)| matches(f, t)) {
            used[i] = true;
            keep = f.1;
        }
        if keep {
            kept.push(t.clone());
        }
    }
    let unknown: Vec<String> = filters
        .iter()
        .zip(&used)
        .filter(|(f, used)| !**used && (f.0 == "tool" || f.0 == "resource"))
        .map(|(f, _)| format!("{}={}", f.0, f.2))
        .collect();
    if !unknown.is_empty() {
        return Err(format!("The following filters did not match any endpoints: {}", unknown.join(", ")));
    }
    let docs = values("no_tools").is_empty() || values("tools").contains(&"docs");
    kept.extend(TOOLS.iter().filter(|t| docs && t["docs"] == true).cloned());
    Ok((adapt(kept, caps), caps))
}

/// Whether `text` matches `pattern`, whose `*` stands for any run of characters (Desktop's `^…$` regex of a resource
/// filter, whose `.` matches any character too).
fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let mut ok = vec![vec![false; t.len() + 1]; p.len() + 1];
    ok[0][0] = true;
    for i in 1..=p.len() {
        for j in 0..=t.len() {
            ok[i][j] = match p[i - 1] {
                '*' => ok[i - 1][j] || (j > 0 && ok[i][j - 1]),
                '.' => j > 0 && ok[i - 1][j - 1],
                c => j > 0 && ok[i - 1][j - 1] && t[j - 1] == c,
            };
        }
    }
    ok[p.len()][t.len()]
}

/// Desktop's `ux`: the tools for a client without some capabilities.
fn adapt(tools: Vec<Value>, caps: Caps) -> Vec<Value> {
    let mut tools: Vec<Value> = match caps.top_level_unions {
        true => tools,
        false => tools.into_iter().flat_map(split_union).collect(),
    };
    if let Some(len) = caps.name_len {
        let names: Vec<String> = tools.iter().map(|t| t["tool"]["name"].as_str().unwrap_or_default().into()).collect();
        let short = short_names(&names, len);
        for t in &mut tools {
            if let Some(name) = t["tool"]["name"].as_str().and_then(|n| short.get(n)).cloned() {
                t["tool"]["name"] = name.into();
            }
        }
    }
    for t in &mut tools {
        let schema = &mut t["tool"]["inputSchema"];
        if !caps.refs {
            *schema = inline_refs(schema);
        }
        if !caps.unions {
            *schema = first_union(schema);
        }
        if !caps.formats {
            *schema = formats_in_descriptions(schema);
        }
    }
    tools
}

/// Desktop's `ox`: a tool whose input is a top-level `anyOf` becomes a tool per variant, named after its title.
fn split_union(t: Value) -> Vec<Value> {
    let schema = &t["tool"]["inputSchema"];
    let Some(variants) = schema["anyOf"].as_array().filter(|v| !v.is_empty()) else { return vec![t] };
    let defs = schema["$defs"].as_object().cloned().unwrap_or_default();
    // Desktop's `lx`: runs of spaces become `_`, and so does each lower-to-upper case change.
    let snake = |s: &str| {
        let mut out = String::new();
        let mut prev: Option<char> = None;
        for c in s.chars() {
            match c.is_whitespace() {
                true if prev.is_some_and(char::is_whitespace) => {}
                true => out.push('_'),
                false => {
                    if prev.is_some_and(|p| p.is_ascii_lowercase()) && c.is_ascii_uppercase() {
                        out.push('_');
                    }
                    out.push(c);
                }
            }
            prev = Some(c);
        }
        out.to_lowercase()
    };
    variants
        .iter()
        .enumerate()
        .map(|(i, r)| {
            let mut u = schema.as_object().cloned().unwrap_or_default();
            u.extend(r.as_object().cloned().unwrap_or_default());
            u.insert("type".into(), "object".into());
            let mut props = schema["properties"].as_object().cloned().unwrap_or_default();
            props.extend(r["properties"].as_object().cloned().unwrap_or_default());
            u.insert("properties".into(), props.into());
            u.remove("anyOf");
            if !u.get("description").is_some_and(|d| d.as_str().is_some_and(|d| !d.is_empty())) {
                u.insert("description".into(), t["tool"]["description"].clone());
            }
            let mut used = serde_json::Map::new();
            referenced_defs(r, &defs, &mut Vec::new(), &mut used);
            match used.is_empty() {
                true => drop(u.remove("$defs")),
                false => drop(u.insert("$defs".into(), used.into())),
            }
            let title = r["title"].as_str().map_or_else(|| format!("variant{}", i + 1), str::to_string);
            let mut v = t.clone();
            v["tool"]["name"] = format!("{}_{}", t["tool"]["name"].as_str().unwrap_or_default(), snake(&title)).into();
            v["tool"]["description"] =
                r.get("description").cloned().unwrap_or_else(|| t["tool"]["description"].clone());
            v["tool"]["inputSchema"] = u.into();
            v
        })
        .collect()
}

/// Desktop's `Pc`: the `$defs` that a schema refers to, with theirs.
fn referenced_defs(
    v: &Value, defs: &serde_json::Map<String, Value>, seen: &mut Vec<String>, out: &mut serde_json::Map<String, Value>,
) {
    match v {
        Value::Object(o) => {
            if let Some(r) = o.get("$ref").and_then(Value::as_str) {
                let name = r.strip_prefix("#/$defs/").filter(|n| !n.contains('/'));
                if let Some((name, def)) = name.and_then(|n| defs.get(n).map(|d| (n, d))) {
                    if !seen.iter().any(|s| s == r) {
                        out.insert(name.into(), def.clone());
                        seen.push(r.into());
                        referenced_defs(def, defs, seen, out);
                        seen.pop();
                    }
                }
                return;
            }
            o.iter().filter(|(k, _)| *k != "$defs").for_each(|(_, v)| referenced_defs(v, defs, seen, out));
        }
        Value::Array(a) => a.iter().for_each(|v| referenced_defs(v, defs, seen, out)),
        _ => {}
    }
}

/// Desktop's `ix`: the names longer than `len`, cut to it, or to `len - 1` and a number when two cuts would clash.
fn short_names(names: &[String], len: usize) -> std::collections::HashMap<String, String> {
    let long: Vec<&String> = names.iter().filter(|n| n.chars().count() > len).collect();
    let cut = |n: &str, k: usize| n.chars().take(k).collect::<String>();
    let distinct: std::collections::HashSet<String> = long.iter().map(|n| cut(n, len - 1)).collect();
    if distinct.len() == long.len() {
        return long.into_iter().map(|n| (n.clone(), cut(n, len))).collect();
    }
    let mut used = std::collections::HashSet::new();
    long.into_iter()
        .map(|n| {
            let base = cut(n, len - 1);
            let m = (1..).find(|m| !used.contains(&format!("{base}{m}"))).expect("a free number");
            used.insert(format!("{base}{m}"));
            (n.clone(), format!("{base}{m}"))
        })
        .collect()
}

/// Desktop's `cx`: the schema with its `$defs` references written out, and recursive ones dropped.
fn inline_refs(schema: &Value) -> Value {
    let defs = schema["$defs"].as_object().cloned().unwrap_or_default();
    let mut s = schema.clone();
    if let Some(o) = s.as_object_mut() {
        o.remove("$defs");
    }
    fn walk(v: &Value, defs: &serde_json::Map<String, Value>, seen: &[String]) -> Option<Value> {
        match v {
            Value::Array(a) => Some(a.iter().map(|v| walk(v, defs, seen).unwrap_or_else(|| json!({}))).collect()),
            Value::Object(o) => {
                if let Some(r) = o.get("$ref").and_then(Value::as_str).filter(|r| r.starts_with("#/$defs/")) {
                    let name = r.rsplit('/').next().unwrap_or_default();
                    if seen.iter().any(|s| s == r) {
                        return None;
                    }
                    let Some(def) = defs.get(name) else { return Some(v.clone()) };
                    let seen: Vec<String> = seen.iter().cloned().chain([r.to_string()]).collect();
                    let Some(Value::Object(mut out)) = walk(def, defs, &seen) else { return Some(v.clone()) };
                    out.extend(o.iter().filter(|(k, _)| *k != "$ref").map(|(k, v)| (k.clone(), v.clone())));
                    return Some(out.into());
                }
                let mut out = serde_json::Map::new();
                for (k, v) in o {
                    match v.is_object() || v.is_array() {
                        true => drop(walk(v, defs, seen).map(|w| out.insert(k.clone(), w))),
                        false => drop(out.insert(k.clone(), v.clone())),
                    }
                }
                Some(out.into())
            }
            v => Some(v.clone()),
        }
    }
    walk(&s, &defs, &[]).unwrap_or_else(|| json!({}))
}

/// Desktop's `kc`: each `anyOf` becomes its first variant, whose properties join the object's.
fn first_union(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.iter().map(first_union).collect(),
        Value::Object(o) => {
            let mut e = o.clone();
            if let Some(first) = e.remove("anyOf").and_then(|a| a.as_array().and_then(|a| a.first().cloned())) {
                if let Some(f) = first.as_object() {
                    if let Some(props) = f.get("properties").and_then(Value::as_object) {
                        let mut all = e.get("properties").and_then(Value::as_object).cloned().unwrap_or_default();
                        all.extend(props.clone());
                        e.insert("properties".into(), all.into());
                    }
                    e.extend(f.iter().filter(|(k, _)| *k != "properties").map(|(k, v)| (k.clone(), v.clone())));
                }
            }
            e.into_iter().map(|(k, v)| (k, first_union(&v))).collect::<serde_json::Map<_, _>>().into()
        }
        v => v.clone(),
    }
}

/// Desktop's `Rc`: each string `format` moves into its description as `(format: "…")`.
fn formats_in_descriptions(v: &Value) -> Value {
    match v {
        Value::Array(a) => a.iter().map(formats_in_descriptions).collect(),
        Value::Object(o) => {
            let mut t = o.clone();
            if let Some(Value::String(f)) = t.remove("format") {
                let note = format!("(format: \"{f}\")");
                let d = match t.get("description").and_then(Value::as_str) {
                    Some(d) => format!("{d} {note}"),
                    None => note,
                };
                t.insert("description".into(), d.into());
            }
            t.into_iter().map(|(k, v)| (k, formats_in_descriptions(&v))).collect::<serde_json::Map<_, _>>().into()
        }
        v => v.clone(),
    }
}

/// Desktop's `sx` for a client without `valid-json`: a string argument that is a JSON object becomes the object.
fn parse_json_args(args: &mut serde_json::Map<String, Value>) {
    for v in args.values_mut() {
        if let Some(o) = v.as_str().and_then(|s| serde_json::from_str::<Value>(s).ok()).filter(Value::is_object) {
            *v = o;
        }
    }
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
    store: &SharedStore, cloud: &Cloud, method: &str, msg: &HttpMessage, token: Option<&str>,
) -> Reply {
    match method {
        "OPTIONS" => return empty(StatusCode::NO_CONTENT, Vec::new()),
        "POST" => {}
        _ => return route_error(405, "Method not supported"),
    }
    // Desktop's Stainless route reads the MCP URL before its transport checks the request.
    let (tools, caps) = match tools(msg) {
        Ok(found) => found,
        Err(e) => return route_error(400, &format!("Invalid request: {e}")),
    };
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
    let Ok(body) = serde_json::from_slice::<Value>(&msg.body) else { return rpc_error(400, -32700, "Parse error") };
    let batch = match body {
        Value::Array(all) => all,
        one => vec![one],
    };
    if let Some(bad) = batch.iter().find(|m| !jsonrpc(m)) {
        let data = format!("Invalid JSON-RPC message: {bad}");
        let body = json!({ "jsonrpc": "2.0", "error": { "code": -32700, "message": "Parse error", "data": data }, "id": null });
        return transport_error(400, &body);
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
    let events: String = requests
        .map(|m| format!("event: message\ndata: {}\n\n", answer(store, cloud, &tools, caps, m, token)))
        .collect();
    if events.is_empty() {
        return empty(StatusCode(202), Vec::new());
    }
    let headers = vec![("content-type", "text/event-stream".into()), ("cache-control", "no-cache".into())];
    Ok(Response { status: StatusCode::OK, headers, body: events.into_bytes() })
}

fn answer(store: &SharedStore, cloud: &Cloud, tools: &[Value], caps: Caps, m: &Value, token: Option<&str>) -> Value {
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
        "tools/call" => call(store, cloud, tools, caps, &m["params"], token).map_err(|e| (-32603, e)),
        _ => Err((-32601, "Method not found".into())),
    };
    match result {
        Ok(result) => json!({ "jsonrpc": "2.0", "id": m["id"], "result": result }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": m["id"], "error": { "code": code, "message": message } })
        }
    }
}

/// The Stainless client's `path` (`VE`) of a tool's route: each parameter as JavaScript's `String`, encoded like its
/// `pv`, or its error for a missing value, `null`, an object, or a `.` or `..` segment.
fn tool_path(template: &str, args: &mut serde_json::Map<String, Value>) -> Result<String, String> {
    let pv = |s: &str| {
        let safe = |c: char| c.is_ascii_alphanumeric() || "-._~!$&'()*+,;=:@".contains(c);
        s.chars().map(|c| if safe(c) { c.to_string() } else { url_encode(&c.to_string()) }).collect::<String>()
    };
    let (mut path, mut errors, mut rest) = (String::new(), Vec::new(), template);
    while let Some(start) = rest.find('{') {
        let end = start + rest[start..].find('}').expect("a closed path parameter");
        path.push_str(&rest[..start]);
        let value = args.remove(&rest[start + 1..end]);
        let kind = match &value {
            None => Some("Undefined"),
            Some(Value::Null) => Some("Null"),
            Some(Value::Object(_)) => Some("Object"),
            _ => None,
        };
        let text = super::js_string(value.as_ref());
        let text = match kind {
            Some(kind) => {
                errors.push((path.len(), text.len(), format!("Value of type {kind} is not a valid path parameter")));
                text
            }
            None => pv(&text),
        };
        path.push_str(&text);
        rest = &rest[end + 1..];
    }
    path.push_str(rest);
    let mut at = 0;
    for segment in path.split('/') {
        if [".", ".."].contains(&segment) {
            errors.push((at, segment.len(), format!("Value \"{segment}\" can't be safely passed as a path parameter")));
        }
        at += segment.len() + 1;
    }
    if errors.is_empty() {
        return Ok(path);
    }
    errors.sort_by_key(|e| e.0);
    let mut marks = String::new();
    for (start, len, _) in &errors {
        marks.push_str(&" ".repeat(start - marks.len()));
        marks.push_str(&"^".repeat(*len));
    }
    let errors: Vec<&str> = errors.iter().map(|e| e.2.as_str()).collect();
    Err(format!("Path parameters result in path with invalid segments:\n{}\n{path}\n{marks}", errors.join("\n")))
}

/// The pairs of a query value, like `qs` with the `repeat` array format of the Stainless client: `k=a&k=b`,
/// `k[a]=b`, and `k=` for `null`.
fn query_pairs(key: &str, v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Array(items) => items.iter().for_each(|i| query_pairs(key, i, out)),
        Value::Object(o) => o.iter().for_each(|(k, v)| query_pairs(&format!("{key}[{k}]"), v, out)),
        Value::Null => out.push(format!("{}=", url_encode(key))),
        v => out.push(format!("{}={}", url_encode(key), url_encode(&super::js_string(Some(v))))),
    }
}

/// Calls the route with the token of the MCP client, so the route checks its scopes.
fn call(
    store: &SharedStore, cloud: &Cloud, tools: &[Value], caps: Caps, params: &Value, token: Option<&str>,
) -> Result<Value, String> {
    let name = params["name"].as_str().unwrap_or_default();
    let t = tools.iter().find(|t| t["tool"]["name"] == name).ok_or_else(|| format!("Unknown tool: {name}"))?;
    let mut args = params["arguments"].as_object().cloned().unwrap_or_default();
    if !caps.valid_json {
        parse_json_args(&mut args);
    }
    let path = tool_path(t["path"].as_str().expect("a tool path"), &mut args)?;
    let method = t["method"].as_str().expect("a tool method");
    let mut query = Vec::new();
    args.iter().for_each(|(k, v)| query_pairs(k, v, &mut query));
    let (target, body) = match method {
        "GET" | "DELETE" if query.is_empty() => (path, Vec::new()),
        "GET" | "DELETE" => (format!("{path}?{}", query.join("&")), Vec::new()),
        _ => (path, serde_json::to_vec(&args).expect("serialize tool arguments")),
    };
    let (status, bytes) = if target.starts_with("https://") {
        // The demo fetches no page.
        if cloud.offline {
            return Err(beeper_core::net::OFFLINE_ERROR.into());
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(query: &str) -> Result<(Vec<String>, Vec<Value>, Caps), String> {
        let start_line = format!("POST /v0/mcp?{query} HTTP/1.1");
        let msg = HttpMessage { start_line, headers: Default::default(), body: Vec::new() };
        let (tools, caps) = tools(&msg)?;
        let names = tools.iter().map(|t| t["tool"]["name"].as_str().unwrap().to_string()).collect();
        Ok((names, tools, caps))
    }

    fn names(query: &str) -> Vec<String> {
        at(query).unwrap().0
    }

    /// Desktop's `fx` and `pE`: the filters pick the API tools in their order, the last match wins, excludes come
    /// last, and `search_docs` follows `tools` and `no_tools` only.
    #[test]
    fn the_mcp_url_filters_tools_like_desktop() {
        assert_eq!(names("").len(), 12);
        assert_eq!(names("tool=search_chats&tool=get_chat"), ["get_chat", "search_chats", "search_docs"]);
        assert_eq!(names("no_tools=docs").len(), 11);
        let docs = at("tool=search_docs").unwrap_err();
        assert_eq!(docs, "The following filters did not match any endpoints: tool=search_docs", "no endpoint");
        let chats = ["get_chat", "archive_chat", "search_chats", "set_chat_reminder", "clear_chat_reminder"];
        assert_eq!(names("resource=chats*&no_tools=docs"), chats);
        assert_eq!(names("resource=$client"), ["focus_app", "search", "search_docs"], "`$` is dropped");
        let read = names("operation=read&no_tag=messages&no_tools=docs");
        assert_eq!(read, ["search", "get_accounts", "get_chat", "search_chats"]);
        let unknown = at("tool=nope&resource=x&tag=nope").unwrap_err();
        assert_eq!(unknown, "The following filters did not match any endpoints: resource=x, tool=nope");
    }

    /// Desktop's zod schema of the MCP URL, worded by `zod-validation-error`, and its capability parser `px`.
    #[test]
    fn the_mcp_url_options_are_checked_like_desktop() {
        let clients = "'openai-agents' | 'claude' | 'claude-code' | 'cursor' | 'infer'";
        assert_eq!(
            at("client=nope&operation=x").unwrap_err(),
            format!(
                "Validation error: Invalid enum value. Expected 'read' | 'write', received 'x' at \"operation[0]\"; \
                 Invalid enum value. Expected {clients}, received 'nope' at \"client\""
            )
        );
        assert_eq!(at("client=cursor").unwrap().2, CAPS, "a stateless request never infers the client");
        let unknown = at("capability=fast").unwrap_err();
        assert!(unknown.starts_with("Unknown capability: fast. Valid capabilities are: top-level-unions, valid-json"));
        assert_eq!(
            at("capability=tool-name-length=x").unwrap_err(),
            "Invalid tool-name-length value: x. Expected a number."
        );
        let caps = at("no_capability=valid-json&no_capability=refs").unwrap().2;
        assert!(!caps.valid_json && !caps.refs && caps.unions);
    }

    /// The Stainless client's `path` and `qs` query: a missing chat ID is an error, not the chat list.
    #[test]
    fn tool_arguments_make_the_path_and_query_of_stainless() {
        let path = |args: Value| tool_path("/v1/chats/{chatID}/messages", &mut args.as_object().unwrap().clone());
        assert_eq!(path(json!({ "chatID": "!a b:x.y" })).unwrap(), "/v1/chats/!a%20b:x.y/messages");
        assert_eq!(path(json!({ "chatID": 7 })).unwrap(), "/v1/chats/7/messages");
        let invalid = "Path parameters result in path with invalid segments:\n";
        assert_eq!(
            path(json!({})).unwrap_err(),
            format!("{invalid}Value of type Undefined is not a valid path parameter\n/v1/chats/undefined/messages\n          ^^^^^^^^^")
        );
        assert!(path(json!({ "chatID": null })).unwrap_err().contains("Value of type Null"));
        assert!(path(json!({ "chatID": {} })).unwrap_err().contains("/v1/chats/[object Object]/messages"));
        assert_eq!(
            path(json!({ "chatID": ".." })).unwrap_err(),
            format!(
                "{invalid}Value \"..\" can't be safely passed as a path parameter\n/v1/chats/../messages\n          ^^"
            )
        );
        let mut query = Vec::new();
        let args = json!({ "a": null, "ids": ["x", "y z"], "o": { "k": true }, "n": 1.0, "e": [] });
        args.as_object().unwrap().iter().for_each(|(k, v)| query_pairs(k, v, &mut query));
        assert_eq!(query, ["a=", "ids=x", "ids=y%20z", "o%5Bk%5D=true", "n=1"]);
    }

    /// Desktop's `ux`: short tool names, inline unions and formats in descriptions for a client without them.
    #[test]
    fn missing_capabilities_rewrite_the_tools_like_desktop() {
        let short = names("capability=tool-name-length=10");
        assert!(short.contains(&"search_cha".to_string()) && short.contains(&"focus_app".to_string()));
        let clash = names("capability=tool-name-length=7");
        assert_eq!(clash[1..4], ["search", "get_ac1", "get_ch1"], "a clash numbers every long name");
        assert_eq!(clash.iter().filter(|n| n.starts_with("search")).count(), 4);
        let (_, tools, _) = at("no_capability=formats&no_capability=unions").unwrap();
        let schemas =
            serde_json::to_string(&tools.iter().map(|t| &t["tool"]["inputSchema"]).collect::<Vec<_>>()).unwrap();
        assert!(!schemas.contains("\"format\"") && schemas.contains("(format: \\\"date-time\\\")"));
        assert!(!schemas.contains("anyOf"));
        let mut args = serde_json::Map::from_iter([("a".into(), json!("{\"b\":1}")), ("c".into(), json!("[1]"))]);
        parse_json_args(&mut args);
        assert_eq!(Value::from(args), json!({ "a": { "b": 1 }, "c": "[1]" }), "Desktop's `sx` parses objects only");
    }
}
