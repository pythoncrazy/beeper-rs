use crate::codec::{base64_decode, base64_encode_standard, sha1};
use crate::http::{
    http_date, parse_query, read_message, read_ws_frame, weak_etag, write_ws_frame, HttpMessage, StatusCode,
};
use beeper_core::bridge_login;
use beeper_core::chat_index as search;
use beeper_core::cloud::{Cloud, CloudError, CodeResult};
use beeper_core::config::{
    cloud_bridge, millis_from_rfc3339, now_rfc3339_millis, CentralConfig, APP_BUNDLE_ID, APP_NAME, APP_VERSION,
    NETWORK_SPECS,
};
use beeper_core::connect::{Token, READ};
use beeper_core::matrix;
use beeper_core::models::*;
use beeper_core::platform::{extension_of, is_mime, mime_type_of, path_to_file_url, OsKind};
use beeper_core::store::{activity_ms, BeeperStore, Draft, OutFile, Query, SharedStore};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fmt::Write;
use std::ops::Bound::{self, Excluded, Included, Unbounded};
use std::sync::{Arc, LazyLock, RwLock};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

mod auth;
mod json_error;
mod mcp;
mod new_chat;
mod proxy;
mod routes;
mod view;

struct Response {
    status: StatusCode,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
}

type Reply = Result<Response, Response>;

/// The `Content-Type` of `res.json` in Express.
const JSON: &str = "application/json; charset=utf-8";

fn json_with<T: Serialize + ?Sized>(status: StatusCode, value: &T) -> Response {
    let body = serde_json::to_vec(value).expect("serialize JSON response");
    Response { status, headers: vec![("content-type", JSON.into())], body }
}

fn ok<T: Serialize + ?Sized>(value: &T) -> Reply {
    Ok(json_with(StatusCode::OK, value))
}

fn empty(status: StatusCode, headers: Vec<(&'static str, String)>) -> Reply {
    Ok(Response { status, headers, body: Vec::new() })
}

/// Desktop's `BeeperConnectError` body, in its key order.
#[derive(Serialize)]
struct ApiError<'a> {
    message: &'a str,
    code: &'a str,
    #[serde(skip_serializing_if = "Value::is_null")]
    details: Value,
}

fn error(status: StatusCode, message: &str, code: &str) -> Response {
    error_with(status, message, code, Value::Null)
}

fn error_with(status: StatusCode, message: &str, code: &str, details: Value) -> Response {
    json_with(status, &ApiError { message, code, details })
}

#[derive(Serialize)]
struct Issues {
    issues: Vec<Issue>,
}

/// A zod issue, with its keys in the order of zod.
struct Issue(Vec<(&'static str, Value)>);

impl Serialize for Issue {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_map(self.0.iter().map(|(k, v)| (k, v)))
    }
}

impl Issue {
    fn get(&self, k: &str) -> Option<&Value> {
        self.0.iter().find(|(key, _)| *key == k).map(|(_, v)| v)
    }

    /// Whether zod aborts the parse at the issue, so that it skips the refinements of the object: a wrong type or
    /// enum value. The failed checks of a value, like a bound or the `int` of a number, only make it dirty.
    fn aborts(&self) -> bool {
        match self.get("code").and_then(Value::as_str) {
            Some("too_small" | "too_big" | "invalid_string") => false,
            Some("invalid_type") => self.get("expected") != Some(&json!("integer")),
            _ => true,
        }
    }
}

/// The type name of zod for a JSON value, or `undefined` for a missing value.
fn zod_type(v: Option<&Value>) -> &'static str {
    match v {
        None => "undefined",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}

/// The zod `invalid_type` issue, with `keys` in the order of the zod type that makes it.
fn type_issue(keys: [&'static str; 3], expected: &str, received: &str, path: Value) -> Issue {
    let message = match received {
        "undefined" => "Required".to_string(),
        r => format!("Expected {expected}, received {r}"),
    };
    let value = |k| match k {
        "code" => json!("invalid_type"),
        "expected" => json!(expected),
        _ => json!(received),
    };
    Issue(keys.map(|k| (k, value(k))).into_iter().chain([("path", path), ("message", message.into())]).collect())
}

/// zod's `custom` issue of a refinement.
fn custom_issue(message: &str, path: Value) -> Issue {
    Issue(vec![("code", json!("custom")), ("message", json!(message)), ("path", path)])
}

/// zod's issue of a failed bound of a `kind`: `too_small` below its `minimum`, or `too_big` above its `maximum`.
fn bound_issue(code: &str, bound: i64, kind: &str, inclusive: bool, message: String, path: Value) -> Issue {
    Issue(vec![
        ("code", json!(code)),
        (if code == "too_small" { "minimum" } else { "maximum" }, json!(bound)),
        ("type", json!(kind)),
        ("inclusive", json!(inclusive)),
        ("exact", json!(false)),
        ("message", message.into()),
        ("path", path),
    ])
}

/// zod's issue of the `int()` check of a number with a fraction.
fn float_issue(path: Value) -> Issue {
    Issue(vec![
        ("code", json!("invalid_type")),
        ("expected", json!("integer")),
        ("received", json!("float")),
        ("message", json!("Expected integer, received float")),
        ("path", path),
    ])
}

/// zod's issue of a string that is not a `datetime()`.
fn datetime_issue(path: Value) -> Issue {
    Issue(vec![
        ("code", json!("invalid_string")),
        ("validation", json!("datetime")),
        ("message", json!("Invalid datetime")),
        ("path", path),
    ])
}

/// The zod issue of a string field, if the value is not a string.
fn string_issue(v: Option<&Value>, optional: bool, path: Value) -> Option<Issue> {
    let ok = matches!(v, Some(Value::String(_))) || (optional && v.is_none());
    (!ok).then(|| type_issue(["code", "expected", "received"], "string", zod_type(v), path))
}

/// The zod issue of an enum field, if the value is not one of the options.
fn enum_issue(v: Option<&Value>, options: &[&str], path: Value) -> Option<Issue> {
    let expected = options.iter().map(|o| format!("'{o}'")).collect::<Vec<_>>().join(" | ");
    match v {
        Some(Value::String(t)) if options.contains(&t.as_str()) => None,
        Some(Value::String(t)) => Some(Issue(vec![
            ("received", json!(t)),
            ("code", json!("invalid_enum_value")),
            ("options", json!(options)),
            ("path", path),
            ("message", format!("Invalid enum value. Expected {expected}, received '{t}'").into()),
        ])),
        v => Some(type_issue(["expected", "received", "code"], &expected, zod_type(v), path)),
    }
}

/// JavaScript's `Number` of a string: a decimal or `Infinity` with blanks trimmed, 0 for a blank, and NaN for the rest.
fn js_number(s: &str) -> f64 {
    match s.trim() {
        "" => 0.0,
        "Infinity" | "+Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        t if t.bytes().all(|b| b.is_ascii_digit() || b"+-.eE".contains(&b)) => t.parse().unwrap_or(f64::NAN),
        _ => f64::NAN,
    }
}

/// The Unix milliseconds of a string that zod's `datetime()` accepts: a valid date, a time with optional seconds and
/// fraction, and `Z`, or with `offset` also a `±hh[:]mm` offset. `Date` keeps the milliseconds of the fraction.
fn zod_datetime(s: &str, offset: bool) -> Option<i64> {
    let b = s.as_bytes();
    let num = |at: usize, len: usize| {
        let digits = b.get(at..at + len).filter(|d| d.iter().all(u8::is_ascii_digit))?;
        Some(digits.iter().fold(0, |n, d| n * 10 + i64::from(d - b'0')))
    };
    let is = |at: usize, c: u8| b.get(at) == Some(&c);
    let (y, m, d) = (num(0, 4)?, num(5, 2)?, num(8, 2)?);
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let date_ok =
        is(4, b'-') && is(7, b'-') && is(10, b'T') && (1..=12).contains(&m) && (1..=days[m as usize - 1]).contains(&d);
    let (h, min) = (num(11, 2)?, num(14, 2)?);
    if !date_ok || !is(13, b':') || h > 23 || min > 59 {
        return None;
    }
    let (mut sec, mut ms, mut at) = (0, 0, 16);
    if is(at, b':') {
        sec = num(at + 1, 2).filter(|&s| s <= 59)?;
        at += 3;
        if is(at, b'.') {
            let len = b[at + 1..].iter().take_while(|d| d.is_ascii_digit()).count();
            ms = num(at + 1, len.min(3)).filter(|_| len > 0)? * 10_i64.pow(3 - len.min(3) as u32);
            at += 1 + len;
        }
    }
    let zone = match &b[at..] {
        b"Z" => 0,
        [sign @ (b'+' | b'-'), ..] if offset => {
            let colon = is(at + 3, b':') as usize;
            let (oh, om) = (num(at + 1, 2)?, num(at + 3 + colon, 2)?);
            if b.len() != at + 5 + colon {
                return None;
            }
            (oh * 60 + om) * 60_000 * if *sign == b'-' { -1 } else { 1 }
        }
        _ => return None,
    };
    let utc = format!("{y:04}-{m:02}-{d:02}T{h:02}:{min:02}:{sec:02}.{ms:03}Z");
    Some(millis_from_rfc3339(&utc).expect("a valid time") - zone)
}

/// The `(minimum, inclusive)` of zod's number checks `min(1)` and `positive()`.
const MIN_1: (i64, bool) = (1, true);
const POSITIVE: (i64, bool) = (0, false);

/// The value of zod's `z.coerce.number().int()` with the minimum `(value, inclusive)` and the inclusive `max` for a
/// query value, a string or the strings of a repeated key that `String` joins with commas, or zod's issues.
fn coerce_int(v: &Value, (min, inclusive): (i64, bool), max: i64, path: Value) -> Result<i64, Vec<Issue>> {
    let n = match v {
        Value::Array(values) => js_number(&values.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(",")),
        v => js_number(v.as_str().expect("a string query value")),
    };
    if n.is_nan() {
        return Err(vec![type_issue(["code", "expected", "received"], "number", "nan", path)]);
    }
    let mut issues = Vec::new();
    // `fract` of an infinity is NaN, which is not an integer either.
    if n.fract() != 0.0 {
        issues.push(float_issue(path.clone()));
    }
    if n < min as f64 || (!inclusive && n == min as f64) {
        let message = format!("Number must be greater than {}{min}", if inclusive { "or equal to " } else { "" });
        issues.push(bound_issue("too_small", min, "number", inclusive, message, path.clone()));
    }
    if n > max as f64 {
        let message = format!("Number must be less than or equal to {max}");
        issues.push(bound_issue("too_big", max, "number", true, message, path));
    }
    if issues.is_empty() {
        Ok(n as i64)
    } else {
        Err(issues)
    }
}

/// Desktop's answer to a body that fails its zod schema.
fn invalid_input(issues: Vec<Issue>) -> Response {
    let details = json!(Issues { issues });
    json_with(StatusCode::BAD_REQUEST, &ApiError { message: "Invalid input", code: "VALIDATION_ERROR", details })
}

/// The zod issue of an empty string for `string().min(1)`.
fn too_short(k: &str) -> Issue {
    too_short_at(json!([k]))
}

fn too_short_at(path: Value) -> Issue {
    bound_issue("too_small", 1, "string", true, "String must contain at least 1 character(s)".into(), path)
}

/// The zod issue of a `string().min(1)` value at `path`.
fn filled_issue(v: Option<&Value>, path: Value) -> Option<Issue> {
    match v {
        Some(Value::String(s)) if s.is_empty() => Some(too_short_at(path)),
        v => string_issue(v, false, path),
    }
}

/// The zod issue of a value that is not of a JSON type, unless `optional` and missing: `boolean` or `number`.
fn kind_issue(v: Option<&Value>, kind: &str, optional: bool, path: Value) -> Option<Issue> {
    let fits = match v {
        None => optional,
        Some(v) => zod_type(Some(v)) == kind,
    };
    (!fits).then(|| type_issue(["code", "expected", "received"], kind, zod_type(v), path))
}

/// zod's checks of `number().int().nonnegative()`, after the type.
fn whole_issues(n: f64, path: Value) -> Vec<Issue> {
    let mut issues = Vec::new();
    if n.fract() != 0.0 {
        issues.push(float_issue(path.clone()));
    }
    if n < 0.0 {
        issues.push(bound_issue(
            "too_small",
            0,
            "number",
            true,
            "Number must be greater than or equal to 0".into(),
            path,
        ));
    }
    issues
}

/// Desktop's answer to a tool input with zod issues.
fn checked(issues: Vec<Issue>) -> Result<(), Response> {
    if issues.is_empty() {
        Ok(())
    } else {
        Err(invalid_input(issues))
    }
}

/// zod's `string().email()`: the regex of zod 3, `^(?!\.)(?!.*\.\.)([A-Z0-9_'+\-\.]*)[A-Z0-9_+-]@([A-Z0-9][A-Z0-9\-]*\.)+[A-Z]{2,}$`
/// without case.
fn zod_email(s: &str) -> bool {
    let Some((local, domain)) = s.split_once('@') else { return false };
    let label = |l: &str| {
        l.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric())
            && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    };
    let Some((labels, top)) = domain.rsplit_once('.') else { return false };
    !s.starts_with('.')
        && !s.contains("..")
        && local.bytes().all(|b| b.is_ascii_alphanumeric() || b"_'+-.".contains(&b))
        && local.bytes().last().is_some_and(|b| b.is_ascii_alphanumeric() || b"_+-".contains(&b))
        && labels.split('.').all(label)
        && top.len() >= 2
        && top.bytes().all(|b| b.is_ascii_alphabetic())
}

/// A field of the zod `.strict()` objects of Desktop's setup bodies.
#[derive(Clone, Copy)]
enum Field {
    /// `string()`.
    Text,
    /// `string().min(1)`.
    Filled,
    /// `string().email()`.
    Email,
    /// `string().optional()`.
    Optional,
    /// `literal(true)`.
    True,
    /// An enum with a default.
    Choice(&'static [&'static str]),
}

/// The zod issues of a body for a `.strict()` object of the fields: the issues of the fields in their order, then the
/// unknown keys. `None` is a request without a body, which Express leaves `undefined`.
fn strict_issues(body: Option<&Value>, fields: &[(&'static str, Field)]) -> Vec<Issue> {
    let Some(Value::Object(o)) = body else {
        return vec![type_issue(["code", "expected", "received"], "object", zod_type(body), json!([]))];
    };
    let mut issues = Vec::new();
    for &(k, field) in fields {
        let (v, path) = (o.get(k), json!([k]));
        match (field, v) {
            (Field::Filled, Some(Value::String(s))) if s.is_empty() => issues.push(too_short(k)),
            (Field::Email, Some(Value::String(s))) if !zod_email(s) => issues.push(Issue(vec![
                ("validation", json!("email")),
                ("code", json!("invalid_string")),
                ("message", json!("Invalid email")),
                ("path", path),
            ])),
            (Field::Text | Field::Filled | Field::Email, v) => issues.extend(string_issue(v, false, path)),
            (Field::Optional, v) => issues.extend(string_issue(v, true, path)),
            (Field::True, Some(Value::Bool(true))) | (Field::Choice(_), None) => {}
            (Field::True, v) => {
                // zod's `received` is `undefined` for a missing key, which JSON leaves out.
                let received = v.map(|v| ("received", v.clone()));
                issues.push(Issue(
                    received
                        .into_iter()
                        .chain([
                            ("code", json!("invalid_literal")),
                            ("expected", json!(true)),
                            ("path", path),
                            ("message", json!("Invalid literal value, expected true")),
                        ])
                        .collect(),
                ));
            }
            (Field::Choice(options), v) => issues.extend(enum_issue(v, options, path)),
        }
    }
    let extra: Vec<&String> = o.keys().filter(|k| !fields.iter().any(|(f, _)| f == k)).collect();
    if !extra.is_empty() {
        let names = extra.iter().map(|k| format!("'{k}'")).collect::<Vec<_>>().join(", ");
        issues.push(Issue(vec![
            ("code", json!("unrecognized_keys")),
            ("keys", json!(extra)),
            ("path", json!([])),
            ("message", format!("Unrecognized key(s) in object: {names}").into()),
        ]));
    }
    issues
}

/// Desktop's `Re` of its setup routes: the body passes the zod `.strict()` object of the fields, or the answer names
/// its first issue.
fn validate(body: Option<&Value>, fields: &[(&'static str, Field)]) -> Result<(), Response> {
    let issues = strict_issues(body, fields);
    let Some(first) = issues.first() else { return Ok(()) };
    let message = first.get("message").and_then(Value::as_str).unwrap_or("Invalid request body").to_string();
    Err(error_with(StatusCode::BAD_REQUEST, &message, "VALIDATION_ERROR", json!(Issues { issues })))
}

/// The value of a query parameter, or `None` once its zod issue joins `issues`.
fn check<T>(issues: &mut Vec<Issue>, v: Result<T, Issue>) -> Option<T> {
    v.map_err(|issue| issues.push(issue)).ok()
}

/// The issues of Desktop's zod schema `CreateChatInput` for a request body.
fn create_chat_issues(b: &Value) -> Vec<Issue> {
    let field = |k: &'static str, optional| string_issue(b.get(k), optional, json!([k]));
    let mut issues: Vec<Issue> =
        [field("accountID", false), field("messageText", true)].into_iter().flatten().collect();
    issues.extend(enum_issue(b.get("type"), &["single", "group"], json!(["type"])));
    let count = b.get("participantIDs").and_then(Value::as_array).map(Vec::len);
    match b.get("participantIDs") {
        Some(Value::Array(ids)) if ids.is_empty() => issues.push(bound_issue(
            "too_small",
            1,
            "array",
            true,
            "Array must contain at least 1 element(s)".into(),
            json!(["participantIDs"]),
        )),
        Some(Value::Array(ids)) => {
            let items = ids.iter().enumerate();
            issues.extend(items.filter_map(|(i, v)| string_issue(Some(v), false, json!(["participantIDs", i]))));
        }
        v => issues.push(type_issue(["code", "expected", "received"], "array", zod_type(v), json!(["participantIDs"]))),
    }
    issues.extend(field("title", true));
    // zod runs the refinement when only the `too_small` issue, which does not abort, is present.
    if issues.len() == (count == Some(0)) as usize && b["type"] == "single" && count != Some(1) {
        issues.push(custom_issue("'single' chat must have exactly one participantID.", json!(["participantIDs"])));
    }
    issues
}

fn bad(message: &str) -> Response {
    error(StatusCode::BAD_REQUEST, message, "VALIDATION_ERROR")
}

fn missing(message: &str) -> Response {
    error(StatusCode::NOT_FOUND, message, "NOT_FOUND")
}

/// Desktop's `getChatMeta` error, which names the chat ID of the request.
fn no_chat(id: &str) -> Response {
    missing(&format!("Chat not found: {id}"))
}

/// The required-input check of Desktop's tools.
fn required(k: &str) -> Response {
    error(StatusCode::BAD_REQUEST, &format!("{k} is required"), "INVALID_INPUT")
}

/// Desktop's body for an error that is not a `BeeperConnectError`.
fn failed(message: &str) -> Response {
    error(StatusCode(500), message, "UNKNOWN_ERROR")
}

fn no_route() -> Response {
    error(StatusCode::NOT_FOUND, "Not found", "not_found")
}

fn store_error(e: beeper_core::Error) -> Response {
    use beeper_core::Error::{ChatNotFound, DraftNotEmpty, MessageNotFound, ReactionNotFound};
    match e {
        DraftNotEmpty => error(StatusCode::CONFLICT, &e.to_string(), "DRAFT_NOT_EMPTY"),
        ChatNotFound | MessageNotFound | ReactionNotFound => missing(&e.to_string()),
    }
}

/// The `limit` items of a page from an iterator, newest first, and whether the iterator has more items.
fn take_page<T>(items: impl Iterator<Item = T>, limit: usize) -> (Vec<T>, bool) {
    let mut items: Vec<T> = items.take(limit + 1).collect();
    let more = items.len() > limit;
    items.truncate(limit);
    (items, more)
}

/// The first `limit` stored messages of `q` that `keep` keeps, newest first, and whether more messages follow.
fn message_page(st: &BeeperStore, q: &Query, limit: usize, keep: impl Fn(&Message) -> bool) -> (Vec<Message>, bool) {
    let mut items = Vec::new();
    st.scan(q, |m| {
        if keep(&m) {
            items.push(m);
        }
        items.len() <= limit
    });
    take_page(items.into_iter(), limit)
}

/// All the values of an array parameter of a parsed query, which is a repeated key or a `key[]` key.
fn query_values<'a>(query: &'a serde_json::Map<String, Value>, k: &str) -> Vec<&'a str> {
    match query.get(k) {
        Some(Value::Array(values)) => values.iter().map(|v| v.as_str().expect("a string query value")).collect(),
        v => v.and_then(Value::as_str).into_iter().collect(),
    }
}

struct Req {
    query: serde_json::Map<String, Value>,
    msg: HttpMessage,
    body: Value,
}

impl Req {
    fn q(&self, k: &str) -> Option<&str> {
        self.query.get(k).and_then(Value::as_str)
    }

    fn qs(&self, k: &str) -> Vec<&str> {
        query_values(&self.query, k)
    }

    /// Desktop's boolean query parameter: `true` or `false`.
    fn flag(&self, k: &str, default: bool) -> Result<bool, Response> {
        match self.q(k) {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(_) => Err(bad(&format!("{k} must be true or false"))),
        }
    }

    /// An optional zod `string()` query parameter, or its zod issue.
    fn string(&self, k: &'static str) -> Result<Option<&str>, Issue> {
        string_issue(self.query.get(k), true, json!([k])).map_or(Ok(self.q(k)), Err)
    }

    /// An optional zod `string().trim().min(1)` query parameter, trimmed, or its zod issue.
    fn filled(&self, k: &'static str) -> Result<Option<&str>, Issue> {
        match self.string(k)?.map(search::js_trim) {
            Some("") => Err(too_short(k)),
            v => Ok(v),
        }
    }

    /// An optional zod enum query parameter, or its zod issue.
    fn choice(&self, k: &'static str, options: &[&str]) -> Result<Option<&str>, Issue> {
        let v = self.query.get(k);
        v.and_then(|v| enum_issue(Some(v), options, json!([k]))).map_or(Ok(v.and_then(Value::as_str)), Err)
    }

    /// An optional array of zod enum values, from a repeated key, a `key[]` key or a single value, or its zod issues.
    fn choices(&self, k: &'static str, options: &[&str]) -> Result<Vec<&str>, Vec<Issue>> {
        let values = self.qs(k);
        let issue = |(i, v): (usize, &&str)| enum_issue(Some(&json!(v)), options, json!([k, i]));
        let issues: Vec<Issue> = values.iter().enumerate().filter_map(issue).collect();
        if issues.is_empty() {
            Ok(values)
        } else {
            Err(issues)
        }
    }

    /// Desktop's optional boolean query parameter `Xt`, `true` or `false`, which is `default` when missing, or its zod
    /// issue.
    fn boolean(&self, k: &'static str, default: bool) -> Result<bool, Issue> {
        match self.query.get(k) {
            None => Ok(default),
            Some(v) if v == "true" || v == "false" => Ok(v == "true"),
            v => Err(type_issue(["code", "expected", "received"], "boolean", zod_type(v), json!([k]))),
        }
    }

    /// An optional zod `datetime()` query parameter, with an offset when `offset`, in Unix milliseconds, or its zod
    /// issue.
    fn datetime(&self, k: &'static str, offset: bool) -> Result<Option<i64>, Issue> {
        self.string(k)?.map(|s| zod_datetime(s, offset).ok_or_else(|| datetime_issue(json!([k])))).transpose()
    }

    /// Desktop's optional `direction` enum: `before` (the default) or `after`, or its zod issue.
    fn direction(&self) -> Result<bool, Issue> {
        self.choice("direction", &["after", "before"]).map(|d| d == Some("after"))
    }

    /// [`Self::direction`], or Desktop's answer with its zod issue.
    fn after(&self) -> Result<bool, Response> {
        self.direction().map_err(|issue| invalid_input(vec![issue]))
    }

    fn str(&self, k: &str) -> Option<&str> {
        self.body[k].as_str()
    }

    /// A key of the input of Desktop's tool routes, `{...query, ...params, ...body}`: the body's, else the query's.
    fn input(&self, k: &str) -> Option<&Value> {
        self.body.get(k).or_else(|| self.query.get(k))
    }

    /// The body as Express's JSON parser leaves it: `None` (`undefined`) without a JSON body, and `{}` for an empty
    /// one with a length.
    fn sent(&self) -> Option<&Value> {
        static EMPTY: LazyLock<Value> = LazyLock::new(|| json!({}));
        let json = self.msg.header("content-type").is_some_and(|t| t.starts_with("application/json"));
        match self.msg.body.is_empty() {
            true => (json && self.msg.header("content-length").is_some()).then(|| &*EMPTY),
            false => json.then_some(&self.body),
        }
    }

    fn owned(&self, k: &str) -> Option<String> {
        self.str(k).map(str::to_string)
    }

    fn need(&self, k: &str) -> Result<&str, Response> {
        self.str(k).filter(|v| !v.trim().is_empty()).ok_or_else(|| required(k))
    }

    fn parse<T: DeserializeOwned>(&self, k: &str) -> Result<T, Response> {
        serde_json::from_value(self.body[k].clone()).map_err(|e| bad(&format!("{k}: {e}")))
    }

    /// Desktop's `limit`, `z.coerce.number().int()` with the minimum `(value, inclusive)` and the maximum `max`, which
    /// is `default` when missing, or its zod issues.
    fn page_size(&self, default: usize, min: (i64, bool), max: i64) -> Result<usize, Vec<Issue>> {
        let Some(v) = self.query.get("limit") else { return Ok(default) };
        coerce_int(v, min, max, json!(["limit"])).map(|n| n as usize)
    }
}

/// The body of a request, or the answer of Desktop's body parser to a malformed JSON body.
fn parse_body(msg: &HttpMessage) -> Result<Value, Response> {
    let malformed = || bad("malformed request body");
    Ok(match msg.header("content-type") {
        _ if msg.body.is_empty() => Value::Null,
        Some(ct) if ct.starts_with("application/json") => {
            // body-parser decodes the bytes like iconv-lite: without a byte order mark, and bad UTF-8 as U+FFFD.
            let text = String::from_utf8_lossy(&msg.body);
            let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
            if let Some(message) = json_error::message(text) {
                return Err(error(StatusCode::BAD_REQUEST, &message, "UNKNOWN_ERROR"));
            }
            serde_json::from_str(text).map_err(|_| malformed())?
        }
        Some(ct) if ct.starts_with("application/x-www-form-urlencoded") => {
            Value::Object(parse_query(std::str::from_utf8(&msg.body).map_err(|_| malformed())?))
        }
        _ => Value::Null,
    })
}

/// Serves the Desktop API. It reads the config at once, so that the info and the spec answer while the store is
/// locked to load its chats.
pub fn serve_http(listener: TcpListener, store: SharedStore) -> impl std::future::Future<Output = ()> {
    let config = Arc::new(store.read().config.clone());
    let cloud = Arc::new(Cloud::of(&config));
    async move {
        // The connections end with the server: when the API turns off, Desktop closes its clients too.
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let accepted = tokio::select! {
                accepted = listener.accept() => accepted,
                Some(_) = connections.join_next() => continue,
            };
            // A failed accept, such as one past the limit of open files, refuses that connection only, like Node.
            let stream = match accepted {
                Ok((stream, _)) => stream,
                Err(e) => {
                    eprintln!("accept a Desktop API connection: {e}");
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    continue;
                }
            };
            connections.spawn(handle_connection(stream, store.clone(), cloud.clone(), config.clone()));
        }
    }
}

/// The limits of Desktop's body parsers: 64 KB for OAuth and setup, 100 MB for JSON uploads, 500 MB for multipart
/// uploads, and 10 MB for the other routes.
fn body_limit(head: &HttpMessage) -> usize {
    match head.path().split('/').filter(|s| !s.is_empty()).collect::<Vec<_>>()[..] {
        ["oauth", ..] | ["register"] | ["v1", "app", "setup", ..] => 64 << 10,
        ["v1", "assets", "upload"] if head.header("content-type").is_some_and(|t| t.contains("application/json")) => {
            100 << 20
        }
        ["v1", "assets", "upload"] => 500 << 20,
        _ => 10 << 20,
    }
}

/// The origin of a request that Desktop's CORS allows: an `http:` or `https:` origin, on all routes except
/// `/focus` and `/deeplink`.
fn cors_origin(head: &HttpMessage) -> Option<String> {
    let origin = head.header("origin")?;
    let blocked = ["/focus", "/deeplink"]
        .iter()
        .any(|p| head.path().strip_prefix(p).is_some_and(|rest| rest.is_empty() || rest.starts_with('/')));
    let web = url::Url::parse(origin).is_ok_and(|u| matches!(u.scheme(), "http" | "https"));
    (web && !blocked).then(|| origin.into())
}

const EXPOSE_HEADERS: (&str, &str) = ("Access-Control-Expose-Headers", "WWW-Authenticate,X-Beeper-Desktop-Version");

/// The answer of the `cors` package to a preflight request.
fn preflight(origin: String) -> Response {
    let headers = [
        ("Access-Control-Allow-Methods", "GET,POST,PUT,PATCH,DELETE,OPTIONS"),
        (
            "Access-Control-Allow-Headers",
            "Authorization,Content-Type,MCP-Protocol-Version,X-Beeper-Access-Token,X-Stainless-MCP",
        ),
        ("Access-Control-Max-Age", "600"),
        EXPOSE_HEADERS,
        ("Content-Length", "0"),
    ];
    let mut cors =
        vec![("X-Powered-By", "Express".into()), ("Access-Control-Allow-Origin", origin), ("Vary", "Origin".into())];
    cors.extend(headers.map(|(k, v)| (k, v.into())));
    Response { status: StatusCode::NO_CONTENT, headers: cors, body: Vec::new() }
}

/// Adds the headers of the CORS and version middleware of Desktop, and the `ETag` of Express, which answers a
/// conditional GET of a fresh body with 304.
fn express(resp: Response, origin: Option<String>, if_none_match: Option<&str>) -> Response {
    let Response { mut status, headers: route, mut body } = resp;
    let mut headers = vec![("X-Powered-By", "Express".into())];
    if let Some(origin) = origin {
        headers.extend([("Access-Control-Allow-Origin", origin), ("Vary", "Origin".into())]);
        headers.push((EXPOSE_HEADERS.0, EXPOSE_HEADERS.1.into()));
    }
    headers.push(("X-Beeper-Desktop-Version", APP_VERSION.into()));
    // `res.redirect` ends the response without the ETag of `res.send`.
    let sent = status != StatusCode::FOUND
        && route
            .iter()
            .any(|(k, v)| *k == "content-type" && ["application/json", "text/html"].iter().any(|t| v.starts_with(t)));
    headers.extend(route);
    let tag = sent.then(|| weak_etag(&body));
    let fresh = tag.as_ref().is_some_and(|tag| if_none_match.is_some_and(|m| m.split(',').any(|t| t.trim() == tag)));
    if fresh && status.0 / 100 == 2 {
        status = StatusCode::NOT_MODIFIED;
        headers.retain(|(k, _)| *k != "content-type");
        body.clear();
    }
    if ![StatusCode::NO_CONTENT, StatusCode::NOT_MODIFIED].contains(&status) {
        headers.push(("Content-Length", body.len().to_string()));
    }
    headers.extend(tag.map(|tag| ("ETag", tag)));
    Response { status, headers, body }
}

/// The `keepAliveTimeout` of Desktop's server.
const KEEP_ALIVE: std::time::Duration = std::time::Duration::from_secs(360);

/// Node keeps an HTTP/1.1 connection open unless the client asks to close it, and an HTTP/1.0 connection only when
/// the client asks to keep it open.
fn keep_alive(msg: &HttpMessage) -> bool {
    let asks =
        |t: &str| msg.header("connection").is_some_and(|c| c.split(',').any(|v| v.trim().eq_ignore_ascii_case(t)));
    if msg.start_line.ends_with(" HTTP/1.0") {
        asks("keep-alive")
    } else {
        !asks("close")
    }
}

/// The checks of Node's HTTP parser on a request line, and the `Host` header that HTTP/1.1 needs.
fn valid_request(msg: &HttpMessage) -> bool {
    let [method, target, version] = msg.start_line.split(' ').collect::<Vec<_>>()[..] else { return false };
    let digit = |s: &str| s.len() == 1 && s.bytes().all(|b| b.is_ascii_digit());
    let version_ok =
        version.strip_prefix("HTTP/").and_then(|v| v.split_once('.')).is_some_and(|(a, b)| digit(a) && digit(b));
    let method_ok = !method.is_empty() && method.bytes().all(|b| b.is_ascii_uppercase() || b == b'-');
    method_ok && !target.is_empty() && version_ok && (version != "HTTP/1.1" || msg.header("host").is_some())
}

/// Desktop's PAS gives these paths to the API server, and ends all other requests with an empty 200.
fn api_path(msg: &HttpMessage) -> bool {
    const ROOTS: [&str; 8] = [".well-known", "oauth", "register", "v0", "v1", "_matrix", "deeplink", "focus"];
    let target = msg.start_line.split(' ').nth(1).expect("a valid request line");
    let Some(rest) = target.strip_prefix('/') else { return false };
    let end = rest.find(['/', '?']).unwrap_or(rest.len());
    match &rest[..end] {
        "" => !rest.starts_with('/'),
        root => ROOTS.contains(&root),
    }
}

async fn write_response(stream: &mut TcpStream, resp: Response, keep_alive: bool, with_body: bool) -> bool {
    let mut head = format!("HTTP/1.1 {} {}\r\n", resp.status.0, resp.status.reason());
    resp.headers.iter().for_each(|(k, v)| write!(head, "{k}: {v}\r\n").unwrap());
    let connection = if keep_alive { "keep-alive\r\nKeep-Alive: timeout=360" } else { "close" };
    write!(head, "Date: {}\r\nConnection: {connection}\r\n\r\n", http_date(beeper_core::config::now_unix_secs()))
        .unwrap();
    let body = if with_body { resp.body } else { Vec::new() };
    stream.write_all(&[head.into_bytes(), body].concat()).await.is_ok()
}

/// Closes a connection after the server refuses a request: the server discards the rest of the request until the
/// client closes, so that the client gets the response and not a reset.
async fn linger<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(writer: &mut W, reader: &mut R) {
    writer.shutdown().await.ok();
    tokio::time::timeout(LINGER, tokio::io::copy(reader, &mut tokio::io::sink())).await.ok();
}

/// The `closeTimeout` of the `ws` package.
const LINGER: std::time::Duration = std::time::Duration::from_secs(30);

async fn handle_connection(mut stream: TcpStream, store: SharedStore, cloud: Arc<Cloud>, config: Arc<CentralConfig>) {
    let mut buf = Vec::new();
    loop {
        let (mut origin, mut api_head) = (None, None);
        let limit = |head: &HttpMessage| {
            origin = cors_origin(head);
            let (start_line, headers) = (head.start_line.clone(), head.headers.clone());
            api_head = (valid_request(head) && api_path(head)).then_some(HttpMessage {
                start_line,
                headers,
                body: Vec::new(),
            });
            body_limit(head)
        };
        let Ok(read) = tokio::time::timeout(KEEP_ALIVE, read_message(&mut stream, &mut buf, limit)).await else {
            return;
        };
        let read = read.and_then(|msg| if valid_request(&msg) { Ok(msg) } else { Err(Some(StatusCode::BAD_REQUEST)) });
        let mut msg = match read {
            Ok(msg) => msg,
            Err(None) => return,
            Err(Some(StatusCode::PAYLOAD_TOO_LARGE)) => {
                let too_large = || error(StatusCode::PAYLOAD_TOO_LARGE, "request entity too large", "UNKNOWN_ERROR");
                // The store lock waits on a blocking thread, like the handlers.
                let st = store.clone();
                let refused = tokio::task::spawn_blocking(move || api_head.and_then(|h| refusal(&st.read(), &h)));
                let resp = refused.await.expect("the refusal check panicked").unwrap_or_else(too_large);
                write_response(&mut stream, express(resp, origin, None), false, true).await;
                let (mut reader, mut writer) = stream.split();
                linger(&mut writer, &mut reader).await;
                return;
            }
            Err(Some(status)) => {
                let head = format!("HTTP/1.1 {} {}\r\nConnection: close\r\n\r\n", status.0, status.reason());
                stream.write_all(head.as_bytes()).await.ok();
                let (mut reader, mut writer) = stream.split();
                linger(&mut writer, &mut reader).await;
                return;
            }
        };
        let keep_alive = keep_alive(&msg);
        if !api_path(&msg) {
            let empty =
                Response { status: StatusCode::OK, headers: vec![("Content-Length", "0".into())], body: Vec::new() };
            if !write_response(&mut stream, empty, keep_alive, true).await || !keep_alive {
                return;
            }
            continue;
        }
        if msg.start_line.starts_with("GET ") && msg.path() == "/v1/ws" && msg.header_is("upgrade", "websocket") {
            // The token check waits for the store lock on a blocking thread, like the handlers.
            let (st, token) = (store.clone(), auth::access_token(&msg).map(str::to_string));
            let checked = tokio::task::spawn_blocking(move || {
                let st = st.read();
                auth::authorize(&st, token.as_deref(), &[beeper_core::connect::READ]).map(|_| st.ws_tx.subscribe())
            });
            return match checked.await.expect("the token check panicked") {
                Ok(events) => serve_ws(stream, buf, msg, store, events).await,
                Err((status, message)) => {
                    let head = format!("HTTP/1.1 {status} {message}\r\nConnection: close\r\n\r\n");
                    drop(stream.write_all(head.as_bytes()).await);
                }
            };
        }
        // Express answers HEAD with the response of GET, without the body.
        let head_only = msg.method() == "HEAD";
        if head_only {
            msg.start_line.replace_range(..4, "GET");
        }
        let resp = match origin {
            Some(origin) if msg.method() == "OPTIONS" => preflight(origin),
            origin => {
                let if_none_match = (msg.method() == "GET").then(|| msg.header("if-none-match")).flatten();
                let if_none_match = if_none_match.map(String::from);
                let resp = if let Some(resp) = config_route(&config, &msg) {
                    resp
                } else {
                    let (store, cloud) = (store.clone(), cloud.clone());
                    tokio::task::spawn_blocking(move || respond(&store, &cloud, msg).unwrap_or_else(|e| e))
                        .await
                        .expect("request handler panicked")
                };
                express(resp, origin, if_none_match.as_deref())
            }
        };
        if !write_response(&mut stream, resp, keep_alive, !head_only).await || !keep_alive {
            return;
        }
    }
}

/// Desktop's `requireAuth` of a route: the token of the request, or the refusal.
fn require_auth(st: &BeeperStore, route: &routes::Route, msg: &HttpMessage) -> Result<Option<Token>, Response> {
    match route.auth {
        routes::Auth::Token(scopes) => {
            Ok(Some(auth::authorize(st, auth::access_token(msg), scopes).map_err(auth::refusal)?))
        }
        routes::Auth::Matrix => {
            let scopes: &[&str] = if route.method == "GET" { &[READ] } else { &[beeper_core::connect::WRITE] };
            Ok(Some(auth::authorize(st, auth::access_token(msg), scopes).map_err(proxy::refusal)?))
        }
        _ => Ok(None),
    }
}

/// The answer to a request that does not depend on its body: no route, a refused token, or a Matrix client that has
/// not started. The server sends it in place of 413 for a body that is too large, because Desktop checks these
/// before the body.
fn refusal(st: &BeeperStore, head: &HttpMessage) -> Option<Response> {
    let (route, _) = match routes::find(head.method(), head.path()) {
        Ok(found) => found,
        Err(resp) => return Some(resp),
    };
    let refused = require_auth(st, route, head).err();
    match route.auth {
        routes::Auth::Matrix => refused.or_else(|| st.matrix.is_none().then(proxy::not_ready)),
        _ => refused,
    }
}

/// The info and the spec, which read only the config, so that they answer while the store loads its chats.
fn config_route(config: &CentralConfig, msg: &HttpMessage) -> Option<Response> {
    let path = routes::find(msg.method(), msg.path()).ok()?.0.path;
    parse_body(msg).ok()?;
    match path {
        "/v1/info" => Some(json_with(StatusCode::OK, &info(config, &base_url(config, msg)))),
        "/v1/spec" => Some(Response {
            status: StatusCode::OK,
            headers: vec![("content-type", JSON.into()), ("cache-control", "public, max-age=3600".into())],
            body: routes::spec(&base_url(config, msg)),
        }),
        _ => None,
    }
}

/// Desktop's order: the route, then the token, then the body.
fn respond(store: &SharedStore, cloud: &Cloud, msg: HttpMessage) -> Reply {
    let mut parts = msg.start_line.split(' ');
    let (method, target) = (parts.next().expect("a method"), parts.next().expect("a request target"));
    let method = method.to_string();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    let (route, segs) = routes::find(&method, path)?;
    let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
    if route.path == "/v0/mcp" {
        let token = auth::access_token(&msg);
        if method != "OPTIONS" && auth::authorize(&store.read(), token, &[]).is_err() {
            return Err(auth::mcp_refusal(&store.read(), &msg));
        }
        // Desktop's JSON parser reads a POST body after the token, before the MCP transport.
        if method == "POST" && msg.header("content-type").is_some_and(|t| t.starts_with("application/json")) {
            parse_body(&msg)?;
        }
        return mcp::handle(store, cloud, &method, &msg, token);
    }
    let granted = require_auth(&store.read(), route, &msg)?;
    let query = parse_query(query);
    if let routes::Auth::Matrix = route.auth {
        return proxy::handle(store, &method, &segs, &query, &msg);
    }
    let body = parse_body(&msg)?;
    let req = Req { query, msg, body };
    if let Some(reply) = auth::route(store, &method, &segs, &req, granted.as_ref()) {
        return reply;
    }
    // The bridge routes' schemas take a bridge ID of `^[A-Za-z0-9_-]+$`.
    if let ["v1", "bridges", id, ..] = &segs[..] {
        if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            let message = "Bridge ID may only contain letters, numbers, underscores, and hyphens";
            return Err(invalid_input(vec![Issue(vec![
                ("validation", json!("regex")),
                ("code", json!("invalid_string")),
                ("message", json!(message)),
                ("path", json!(["bridgeID"])),
            ])]));
        }
    }
    if let ("POST", ["v1", "app", "setup", step @ ("start" | "email" | "response" | "register")]) =
        (method.as_str(), &segs[..])
    {
        auth::setup_origin(&req.msg)?;
        return sign_in_step(store, cloud, step, &req);
    }
    if let ("GET" | "POST", ["v1", "app", "setup", rest @ ..]) = (method.as_str(), &segs[..]) {
        return e2ee_setup(store, &method, rest, &req);
    }
    if let ("GET", ["focus", rest @ ..]) = (method.as_str(), &segs[..]) {
        return focus_page(&mut store.write(), rest, &req);
    }
    if let ("GET", ["deeplink", rest @ ..]) = (method.as_str(), &segs[..]) {
        return deeplink_page(&store.read(), rest, &req);
    }
    if let Some(reply) = bridge_login(store, cloud, &method, &segs, &req) {
        return reply;
    }
    if let Some(reply) = new_chat::route(store, &method, &segs, &req) {
        return reply;
    }
    if method == "GET" {
        return get(store, &segs, &req);
    }
    dispatch(&mut store.write(), &method, &segs, &req)
}

/// Desktop's `{verification, session}` body of the verification routes.
#[derive(Serialize)]
struct Progress<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<&'a AppVerification>,
    session: AppState,
}

/// The progress body. A route that `requires` a verification fails without one.
fn progress(store: &SharedStore, requires: bool) -> Reply {
    let st = store.read();
    let verification = st.app_verification();
    if requires && verification.is_none() {
        return Err(verification_not_found());
    }
    ok(&Progress { verification, session: st.app_state() })
}

/// Desktop's app setup state and its encrypted messaging setup routes, which act on the Matrix client of a
/// signed-in account. The state needs a token only when the app is signed in.
fn e2ee_setup(store: &SharedStore, method: &str, path: &[&str], r: &Req) -> Reply {
    let signed_in = || match store.read().signed_in() {
        true => Ok(()),
        false => Err(error(StatusCode::CONFLICT, "Sign in to Beeper Desktop first", "needs_login")),
    };
    let not_ready = || {
        signed_in()?;
        match store.read().beeper_state.app_state() {
            "ready" => Err(error(StatusCode::CONFLICT, "Beeper Desktop is already set up", "already_ready")),
            _ => Ok(()),
        }
    };
    let rt = tokio::runtime::Handle::current();
    match (method, path) {
        ("GET", []) => {
            if store.read().signed_in() {
                auth::authorize(&store.read(), auth::access_token(&r.msg), &[READ]).map_err(auth::refusal)?;
            }
            ok(&store.read().app_state())
        }
        ("POST", ["verification", "recovery-key"]) => {
            not_ready()?;
            validate(r.sent(), &[("recoveryKey", Field::Filled)])?;
            let key = r.str("recoveryKey").expect("the body passed its schema").to_string();
            rt.block_on(matrix::recover(store.clone(), key)).map_err(|e| {
                match e.to_lowercase().contains("invalid recovery key") {
                    true => error(StatusCode::BAD_REQUEST, "The recovery key is invalid", "invalid_recovery_code"),
                    false => failed(&e),
                }
            })?;
            ok(&json!({ "session": store.read().app_state() }))
        }
        ("POST", ["verification", "recovery-key", "reset"]) => {
            not_ready()?;
            validate(r.sent(), &[("existingRecoveryKey", Field::Optional)])?;
            let key = rt.block_on(matrix::new_recovery_key(store)).map_err(|e| failed(&e))?;
            ok(&json!({ "recoveryKey": key, "session": store.read().app_state() }))
        }
        ("POST", ["verification", "recovery-key", "reset", "confirm"]) => {
            // Desktop uploads the new keys at the confirmation. beeper-rs uploads them at the reset, so the account
            // can be ready while its new key waits for the confirmation.
            if store.read().pending_recovery_key.is_none() {
                not_ready()?;
            }
            validate(r.sent(), &[("recoveryKey", Field::Filled)])?;
            let key = r.str("recoveryKey").expect("the body passed its schema");
            matrix::confirm_recovery_key(store, key).map_err(|e| failed(&e))?;
            ok(&json!({ "session": store.read().app_state() }))
        }
        ("GET", ["verifications"]) => {
            signed_in()?;
            ok(&json!({ "items": store.read().app_verification().into_iter().collect::<Vec<_>>() }))
        }
        ("POST", ["verifications"]) => {
            signed_in()?;
            let empty = json!({});
            let fields = [("purpose", Field::Choice(&["login", "device"])), ("userID", Field::Optional)];
            validate(Some(r.sent().filter(|b| !b.is_null()).unwrap_or(&empty)), &fields)?;
            let purpose = if r.str("purpose") == Some("login") { "login" } else { "device" };
            if r.str("userID").is_some_and(|u| u != store.read().beeper_state.user_id) {
                return Err(failed("Verification of other users is not supported"));
            }
            if let Some(v) = store.read().app_verification().filter(|v| !matches!(v.state, "done" | "cancelled")) {
                let details = json!({ "verificationID": v.id, "state": v.state });
                return Err(error_with(
                    StatusCode::CONFLICT,
                    "A verification is already in progress",
                    "verification_in_progress",
                    details,
                ));
            }
            rt.block_on(matrix::start_verification(store, purpose)).map_err(|e| failed(&e))?;
            progress(store, true)
        }
        ("GET", ["verifications", id]) => {
            signed_in()?;
            let found = store.read().app_verification().is_some_and(|v| v.id == *id);
            match found {
                true => progress(store, true),
                false => Err(verification_not_found()),
            }
        }
        ("POST", ["verifications", "qr", "scan"]) => {
            signed_in()?;
            validate(r.sent(), &[("data", Field::Filled)])?;
            let data = r.str("data").expect("the body passed its schema").to_string();
            rt.block_on(matrix::scan_qr(store, &data)).map_err(|e| failed(&e))?;
            progress(store, false)
        }
        ("POST", ["verifications", id, action @ ..]) => {
            let action = match action {
                ["accept"] => "accept",
                ["cancel"] => "cancel",
                ["sas", "start"] => "sas.start",
                ["sas", "confirm"] => "sas.confirm",
                ["qr", "confirm-scanned"] => "qr.confirmScanned",
                _ => return Err(no_route()),
            };
            signed_in()?;
            available(&store.read(), id, action)?;
            if action == "cancel" {
                let empty = json!({});
                let body = r.sent().filter(|b| !b.is_null()).unwrap_or(&empty);
                validate(Some(body), &[("code", Field::Optional), ("reason", Field::Optional)])?;
            }
            match action {
                "accept" => rt.block_on(matrix::accept_verification(store, id)).map_err(|e| failed(&e))?,
                "cancel" => rt.block_on(matrix::cancel_verification(store, id)).map_err(|e| failed(&e))?,
                "sas.confirm" => {
                    let tx = store.write().sas_confirm.take().ok_or_else(verification_not_found)?;
                    tx.send(true).ok();
                }
                "sas.start" => rt.block_on(matrix::start_sas(store, id)).map_err(|e| failed(&e))?,
                "qr.confirmScanned" => rt.block_on(matrix::confirm_qr(store, id)).map_err(|e| failed(&e))?,
                _ => unreachable!("the action table has no {action}"),
            }
            progress(store, true)
        }
        _ => Err(no_route()),
    }
}

/// Desktop's check that the current verification has the `id` and allows the `action`.
fn available(st: &BeeperStore, id: &str, action: &str) -> Result<(), Response> {
    let v = st.app_verification();
    if v.is_some_and(|v| v.id == id && v.available_actions.contains(&action)) {
        return Ok(());
    }
    let actions = v.map_or(&Vec::new(), |v| &v.available_actions).clone();
    let mut details = json!({ "action": action, "verificationID": id, "availableActions": actions });
    if let Some(v) = v {
        details["state"] = v.state.into();
    }
    let message = "Verification action is not available in the current app state";
    Err(error_with(StatusCode::CONFLICT, message, "verification_action_not_available", details))
}

fn verification_not_found() -> Response {
    error(StatusCode::NOT_FOUND, "Verification not found", "verification_not_found")
}

/// The GET routes only read the store, so they share its lock with each other and with the UI.
fn get(store: &SharedStore, path: &[&str], r: &Req) -> Reply {
    if path == ["v1", "assets", "serve"] {
        let file = {
            let st = store.read();
            // Desktop's `serveAsset` also takes the token from the query, for media elements.
            let token = auth::access_token(&r.msg).or(r.q("dangerouslyUseTokenInQuery")).filter(|t| !t.is_empty());
            let token =
                token.ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Unauthorized: Missing token", "unauthorized"))?;
            auth::authorize(&st, Some(token), &[READ]).map_err(auth::refusal)?;
            asset_file(&st, r)?
        };
        return serve_asset(file, r);
    }
    if let ["v1", "chats", id, "messages"] = path {
        backfill(store, id, r)?;
    }
    let st = &*store.read();
    match path {
        [] => Ok(redirect("/v1/info", r.msg.header("accept"))),
        ["v1", "accounts"] => ok(&st.accounts),
        ["v1", "accounts", id] => account(st, id),
        ["v1", "labels"] => ok(&st.labels),
        ["v1", "bridges", rest @ ..] => bridge_get(st, rest),
        ["v1", "chats"] => list_chats(st, r),
        ["v1", "chats", "search"] => search_chats(st, r),
        ["v1", "chats", id, rest @ ..] => {
            // Desktop's `getChat` names no chat, unlike its `getChatMeta` of the other chat routes.
            let unknown = || if rest.is_empty() { missing("Chat not found") } else { no_chat(id) };
            // Desktop checks `GetChatInput` before it looks for the chat.
            let max = match (rest, r.query.get("maxParticipantCount")) {
                ([], Some(v)) => {
                    coerce_int(v, (-1, true), 500, json!(["maxParticipantCount"])).map_err(invalid_input)?
                }
                _ => 100,
            };
            let idx = st.resolve_chat_idx(id).ok_or_else(unknown)?;
            match rest {
                [] => ok(&view::chat(st, &st.chats[idx], usize::try_from(max).ok(), false)),
                ["messages"] => list_messages(st, idx, r),
                ["messages", mid] => {
                    let m = st.stored_message(mid).filter(|m| st.in_chat(idx, &m.chat_id));
                    ok(&view::message(&m.ok_or_else(|| missing("Message not found"))?))
                }
                _ => Err(no_route()),
            }
        }
        ["v1", "messages", "search"] => search_messages(st, r),
        ["v1", "search"] => unified_search(st, r),
        _ => Err(no_route()),
    }
}

fn dispatch(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        (_, ["v1", "chats", rest @ ..]) => chats(st, method, rest, r),
        ("POST", ["v1", "assets", "upload"]) => match r.msg.header("content-type").unwrap_or_default() {
            t if t.contains("multipart/form-data") => upload_multipart(st, r),
            t if t.contains("application/json") => upload_json(st, r),
            _ => Err(error(
                StatusCode::BAD_REQUEST,
                "Use application/json or multipart/form-data",
                "INVALID_CONTENT_TYPE",
            )),
        },
        ("POST", ["v1", "assets", "upload", "base64"]) => {
            let field = |k: &'static str, optional| string_issue(r.body.get(k), optional, json!([k]));
            let issues: Vec<Issue> = [field("content", false), field("fileName", true), field("mimeType", true)]
                .into_iter()
                .flatten()
                .collect();
            if !issues.is_empty() {
                return Err(invalid_input(issues));
            }
            let bytes = base64_decode(r.str("content").expect("a string content")).filter(|b| !b.is_empty());
            upload(
                st,
                r.str("fileName"),
                r.str("mimeType"),
                &bytes.ok_or_else(|| bad("content must be non-empty base64"))?,
            )
        }
        ("POST", ["v1", "assets", "download"]) => {
            // Desktop's `DownloadAssetInput`: a URL of at least one character, which its refinement checks for a
            // media scheme, unless the type of the URL aborts the parse.
            let url = r.input("url");
            let mut issues: Vec<Issue> = filled_issue(url, json!(["url"])).into_iter().collect();
            let url = url.and_then(Value::as_str);
            if url.is_some_and(|u| !u.starts_with("mxc://") && !u.starts_with("localmxc://")) {
                let message = "URL must start with mxc:// or localmxc://";
                issues.push(custom_issue(message, json!(["url"])));
            }
            checked(issues)?;
            let url = url.expect("the input passed its schema");
            let failed = || json_with(StatusCode::OK, &json!({ "error": asset_failure(url).1 }));
            // Desktop's `resolveMatrixAsset` downloads only media URLs.
            let media = url.starts_with("mxc://") || url.starts_with("localmxc://");
            let (path, _) = st.resolve_asset_path(url).filter(|_| media).ok_or_else(failed)?;
            ok(&json!({ "srcURL": path_to_file_url(&path) }))
        }
        ("POST", ["v1", "focus"]) => focus(st, r),
        _ => Err(no_route()),
    }
}

/// Desktop's `buildBaseURL`: the bind address `0.0.0.0` and IPv6 addresses give the loopback address, and the port
/// of the scheme drops.
fn build_base_url(scheme: &str, host: &str, port: &str) -> String {
    let host = match host.trim() {
        h if h.is_empty() || h == "0.0.0.0" || h.contains(':') => "127.0.0.1",
        h => h,
    };
    let port = port.trim();
    match port.is_empty() || port == if scheme == "http" { "80" } else { "443" } {
        true => format!("{scheme}://{host}"),
        false => format!("{scheme}://{host}:{port}"),
    }
}

/// Desktop's `getBaseURL`: the bind address, or the loopback host name of the request. With remote access, the host,
/// the scheme and the port of the request after the proxy headers.
fn base_url(cfg: &CentralConfig, msg: &HttpMessage) -> String {
    let port = cfg.port.to_string();
    let bound = build_base_url("http", &cfg.host, &port);
    let first = |k| msg.header(k).map(|v| v.split(',').next().expect("a first item").trim());
    // Express's `req.hostname`.
    let host = cfg.remote.then(|| first("x-forwarded-host")).flatten().or(msg.header("host")).filter(|h| !h.is_empty());
    let Some(host) = host else { return bound };
    let end = host.find(']').map_or(0, |i| i + 1);
    let name = host[end..].find(':').map_or(host, |i| &host[..end + i]);
    if name.contains(':') {
        return bound;
    }
    if !cfg.remote {
        return match auth::loopback_name(name) {
            true => build_base_url("http", name, &port),
            false => bound,
        };
    }
    let scheme = if first("x-forwarded-proto") == Some("https") { "https" } else { "http" };
    let port = first("x-forwarded-port").or(msg.header("host").and_then(|h| h.split(':').nth(1)));
    build_base_url(scheme, name, port.unwrap_or_default())
}

/// The offer that an `Accept` header prefers, like the `negotiator` package of Express, without media type
/// parameters.
fn preferred<'a>(accept: Option<&str>, offers: &[&'a str]) -> Option<&'a str> {
    let accepted: Vec<(&str, &str, f64)> = (accept.unwrap_or("*/*").split(','))
        .filter_map(|a| {
            let mut params = a.split(';');
            let (t, s) = params.next().expect("a first item").trim().split_once('/')?;
            let q = params.find_map(|p| p.trim().strip_prefix("q=")).map_or(Some(1.0), |q| q.parse().ok())?;
            Some((t.trim(), s.trim(), q))
        })
        .collect();
    let part = |a: &str, o: &str, bit| match a.eq_ignore_ascii_case(o) {
        true => Some(bit),
        false => (a == "*").then_some(0),
    };
    // The most specific entry of the header for the offer, then the highest quality, then the last one.
    let priority = |offer: &str| {
        let (t, s) = offer.split_once('/').expect("a media type");
        let specs = accepted.iter().enumerate();
        let specs = specs.filter_map(|(o, &(at, sub, q))| Some((part(at, t, 4)? | part(sub, s, 2)?, q, o)));
        specs.max_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)))
    };
    let ranked = offers.iter().enumerate().filter_map(|(i, &offer)| {
        let (s, q, o) = priority(offer)?;
        (q > 0.0).then_some((q, s, o, i, offer))
    });
    let best = ranked.min_by(|a, b| b.0.total_cmp(&a.0).then(b.1.cmp(&a.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
    best.map(|r| r.4)
}

/// Express's `res.redirect`: a 302 with a body of the type that the client prefers.
fn redirect(to: &str, accept: Option<&str>) -> Response {
    let text = format!("Found. Redirecting to {to}");
    let mut headers = vec![("location", to.to_string()), ("vary", "Accept".into())];
    let body = match preferred(accept, &["text/plain", "text/html"]) {
        Some(t) => {
            headers.push(("content-type", format!("{t}; charset=utf-8")));
            if t == "text/html" {
                format!("<p>{text}</p>")
            } else {
                text
            }
        }
        None => String::new(),
    };
    Response { status: StatusCode::FOUND, headers, body: body.into_bytes() }
}

fn info(cfg: &CentralConfig, base_url: &str) -> InfoResponse {
    let base_url = base_url.to_string();
    let oauth = |p: &str| format!("{base_url}/oauth/{p}");
    InfoResponse {
        app: InfoApp { bundle_id: APP_BUNDLE_ID.into(), name: APP_NAME.into(), version: APP_VERSION.into() },
        endpoints: InfoEndpoints {
            mcp: format!("{base_url}/v0/mcp"),
            oauth: InfoOAuthEndpoints {
                authorization_endpoint: oauth("authorize"),
                introspection_endpoint: oauth("introspect"),
                registration_endpoint: oauth("register"),
                revocation_endpoint: oauth("revoke"),
                token_endpoint: oauth("token"),
                userinfo_endpoint: oauth("userinfo"),
            },
            spec: format!("{base_url}/v1/spec"),
            ws_events: format!("{base_url}/v1/ws"),
        },
        platform: InfoPlatform {
            // Node's `process.arch`.
            arch: match std::env::consts::ARCH {
                "x86_64" => "x64",
                "aarch64" => "arm64",
                "x86" => "ia32",
                other => other,
            }
            .into(),
            os: OsKind::current().node_platform_str().into(),
            release: "v24.20.0".into(),
        },
        server: InfoServer {
            hostname: cfg.host.clone(),
            mcp_enabled: true,
            port: cfg.port,
            remote_access: cfg.remote,
            status: "running".into(),
            base_url,
        },
    }
}

fn sign_in_step(store: &SharedStore, cloud: &Cloud, step: &str, r: &Req) -> Reply {
    let already = || error(StatusCode::CONFLICT, "Beeper Desktop is already signed in", "already_logged_in");
    if store.read().signed_in() {
        return Err(already());
    }
    // Desktop's own errors for a cloud answer without a request or a token. The other cloud errors keep their message,
    // and only the code step names a refused code.
    let upstream = |e: CloudError| match (e.status, e.message.as_str()) {
        (502, "Could not start sign-in" | "Could not finish sign-in") => {
            error(StatusCode::BAD_GATEWAY, &e.message, "invalid_sign_in_response")
        }
        (502, "Could not create the account") => {
            error(StatusCode::BAD_GATEWAY, &e.message, "invalid_registration_response")
        }
        _ => failed(&e.message),
    };
    let text = |k: &str| r.str(k).expect("the body passed its schema");
    let jwt = match step {
        "start" => {
            let (request, methods) = cloud.start_login().map_err(upstream)?;
            if !methods.iter().any(|m| m == "email") {
                return Err(error(
                    StatusCode::CONFLICT,
                    "Email sign-in is not available right now",
                    "email_sign_in_not_supported",
                ));
            }
            return ok(&json!({ "setupRequestID": request, "signInMethods": methods }));
        }
        "email" => {
            validate(r.sent(), &[("setupRequestID", Field::Text), ("email", Field::Email)])?;
            cloud.send_login_email(text("setupRequestID"), text("email")).map_err(upstream)?;
            return empty(StatusCode::NO_CONTENT, Vec::new());
        }
        "response" => {
            validate(r.sent(), &[("setupRequestID", Field::Text), ("response", Field::Text)])?;
            let refused = |e: CloudError| match e.status {
                401 => error(StatusCode::UNAUTHORIZED, &e.message, "invalid_sign_in_code"),
                _ => upstream(e),
            };
            match cloud.send_login_code(text("setupRequestID"), text("response")).map_err(refused)? {
                CodeResult::Token(jwt) => jwt,
                CodeResult::Registration { lead_token, username_suggestions } => {
                    return ok(&json!({
                        "registrationRequired": true,
                        "setupRequestID": text("setupRequestID"),
                        "leadToken": lead_token,
                        "usernameSuggestions": username_suggestions,
                        "copy": {
                            "title": "Choose your username",
                            "usernamePlaceholder": "Username",
                            "submit": "Continue",
                            "terms": "By continuing, you agree to the Terms of Use and acknowledge the Privacy Policy."
                        }
                    }));
                }
            }
        }
        "register" => {
            let fields = [
                ("setupRequestID", Field::Text),
                ("leadToken", Field::Text),
                ("username", Field::Filled),
                ("acceptTerms", Field::True),
            ];
            validate(r.sent(), &fields)?;
            cloud.register(text("setupRequestID"), text("leadToken"), text("username")).map_err(upstream)?
        }
        other => unreachable!("sign-in step {other}"),
    };
    let (session, profile, accounts) = cloud.finish(&jwt).map_err(upstream)?;
    let mut st = store.write();
    if st.signed_in() {
        return Err(already());
    }
    st.sign_in(session, cloud.homeserver.clone(), &profile, accounts);
    let s = &st.beeper_state;
    let matrix = json!({ "userID": s.user_id, "deviceID": s.device_id, "accessToken": s.access_token, "homeserver": s.homeserver });
    ok(&json!({ "matrix": matrix, "session": st.app_state() }))
}

fn account(st: &BeeperStore, id: &str) -> Reply {
    ok(st.accounts.iter().find(|a| a.account_id == id).ok_or_else(|| missing("Account not found"))?)
}

/// Desktop's routes that call a Beeper Cloud bridge through the homeserver: the provisioning capabilities, the
/// login flows, and the login sessions (`createLoginSession`, `submitLoginStep` and `cancelLoginSession`). The
/// bridge calls run without the store lock.
fn bridge_login(store: &SharedStore, cloud: &Cloud, method: &str, segs: &[&str], r: &Req) -> Option<Reply> {
    let ["v1", "bridges", id, rest @ ..] = segs else { return None };
    match (method, rest) {
        ("GET", ["capabilities" | "login-flows"])
        | ("POST", ["login-sessions"] | ["login-sessions", _, "steps", _])
        | ("DELETE", ["login-sessions", _]) => Some(bridge_call(store, cloud, method, id, rest, r)),
        ("POST", ["logins", login_id, "remove"]) => Some(remove_login(store, cloud, id, login_id, r)),
        _ => None,
    }
}

/// Desktop's `removeBridgeLogin`. Beeper Cloud logs the login out, or deletes the bridge when the login is its only
/// one. The demo has no cloud, so it only forgets the login.
fn remove_login(store: &SharedStore, cloud: &Cloud, id: &str, login_id: &str, r: &Req) -> Reply {
    if let Some(issue) = enum_issue(r.body.get("scope"), &["current-device", "all-devices"], json!(["scope"])) {
        return Err(invalid_input(vec![issue]));
    }
    let scope = r.str("scope").expect("the scope is an option");
    let (bridge, p, sole, affected) = {
        let st = store.read();
        let spec = view::bridge_network(id);
        let login = spec.and_then(|s| view::logins(&st, s).into_iter().find(|l| l.login_id == login_id));
        let login = login.ok_or_else(|| missing("Bridge login not found"))?;
        if !login.remove_scopes.contains(&scope) {
            let message = format!("Remove scope {scope} is not available for this bridge login");
            return Err(error(StatusCode::BAD_REQUEST, &message, "INVALID_SCOPE"));
        }
        let affected: Vec<String> = login.account_ids.iter().map(|a| a.to_string()).collect();
        let found = st.bridge_accounts.iter().find(|b| b.account_id == affected[0]).expect("an account of the login");
        let bridge = found.bridge_id.clone();
        let sole = st.bridge_accounts.iter().filter(|b| b.bridge_id == bridge).all(|b| b.login_id == login_id);
        let s = &st.beeper_state;
        let p = matrix::Provision {
            homeserver: s.homeserver.clone(),
            access_token: s.access_token.clone(),
            user_id: s.user_id.clone(),
            bridge_id: bridge.clone(),
            login_id: login_id.into(),
        };
        (bridge, (!st.config.demo).then_some(p), sole, affected)
    };
    if let Some(p) = p {
        let removing = matrix::remove_account(p, cloud.api_url.clone(), sole);
        tokio::runtime::Handle::current().block_on(removing).map_err(|e| failed(&e))?;
    }
    let mut st = store.write();
    st.bridge_accounts.retain(|b| b.bridge_id != bridge || b.login_id != login_id);
    st.rebuild_accounts_from_bridges();
    st.persist_state_to_sqlite();
    let body = json!({ "bridgeID": id, "loginID": login_id, "affectedAccountIDs": affected, "status": "removed", "scope": scope });
    ok(&body)
}

fn bridge_call(store: &SharedStore, cloud: &Cloud, method: &str, id: &str, rest: &[&str], r: &Req) -> Reply {
    // Desktop's `assertSupportedBridgeID`, with the network IDs of `/v1/bridges` and the cloud bridge IDs.
    let cloud_id = |n: &NetworkSpec| cloud_bridge(n).filter(|b| n.bridge_id == id || *b == id);
    let bridge = NETWORK_SPECS.iter().find_map(cloud_id);
    let bridge = bridge.ok_or_else(|| error(StatusCode::BAD_REQUEST, "Invalid bridgeID", "invalid_parameter"))?;
    let p = {
        let st = store.read();
        if st.config.demo || !st.signed_in() {
            return Err(error(StatusCode::CONFLICT, "Sign in to Beeper Desktop first", "needs_login"));
        }
        let s = &st.beeper_state;
        matrix::Provision {
            homeserver: s.homeserver.clone(),
            access_token: s.access_token.clone(),
            user_id: s.user_id.clone(),
            bridge_id: bridge.into(),
            login_id: String::new(),
        }
    };
    let rt = tokio::runtime::Handle::current();
    let flows = |p: matrix::Provision| -> Result<Vec<bridge_login::Flow>, Response> {
        let flows = rt.block_on(bridge_login::flows(p, cloud.api_url.clone())).map_err(|e| failed(&e))?;
        Ok(store.read().features.login_flows(bridge, flows))
    };
    let session = |sid: &str| {
        let st = store.read();
        let found = st.bridge_sessions.get(sid).filter(|s| s.view["bridgeID"] == id).cloned();
        found.ok_or_else(|| missing("Bridge login session not found"))
    };
    // Desktop's `buildLoginSession`: the session with the mapped step, kept for the next calls.
    let keep = |sid: &str, login: Value, account: Value, step: bridge_login::Step| {
        let view = view::login_session(sid, id, login, account, &step.raw);
        store.write().bridge_sessions.insert(sid.into(), BridgeLoginSession { view: view.clone(), step });
        ok(&view)
    };
    match (method, rest) {
        ("GET", ["capabilities"]) => ok(&rt.block_on(matrix::capabilities(p)).map_err(|e| failed(&e))?),
        ("GET", ["login-flows"]) => {
            let items: Vec<Value> = flows(p)?
                .into_iter()
                .map(|f| {
                    let mut v = json!({ "id": f.id });
                    for (k, x) in [("name", f.name), ("description", f.description)] {
                        if !x.is_empty() {
                            v[k] = x.into();
                        }
                    }
                    v
                })
                .collect();
            ok(&json!({ "items": items }))
        }
        ("POST", ["login-sessions"]) => {
            let (account_id, login_id) = (r.str("accountID"), r.str("loginID"));
            let (login, configured) = {
                let st = store.read();
                let found = account_id.map(|a| st.bridge_accounts.iter().find(|b| b.account_id == a));
                if found == Some(None) {
                    return Err(missing("Account not found or has no bridge login ID"));
                }
                let found = found.flatten();
                if found.is_some_and(|b| b.bridge_id != bridge) {
                    return Err(error(
                        StatusCode::BAD_REQUEST,
                        "Account does not belong to this bridge",
                        "INVALID_ACCOUNT",
                    ));
                }
                let login = login_id.map(str::to_string).or(found.map(|b| b.login_id.clone()));
                let mine = st.bridge_accounts.iter().filter(|b| b.bridge_id == bridge);
                (login.clone(), mine.clone().count() > 0 && !mine.clone().any(|b| Some(&b.login_id) == login.as_ref()))
            };
            if configured && !store.read().features.multi_account(bridge) {
                let message = "Adding another account to this bridge is disabled";
                return Err(error(StatusCode::CONFLICT, message, "MULTI_ACCOUNT_DISABLED"));
            }
            let flow = match r.str("flowID") {
                Some(f) => f.to_string(),
                None => flows(p.clone())?.into_iter().next().map(|f| f.id).ok_or_else(|| {
                    error(StatusCode::CONFLICT, "No login flow is available for this bridge", "NO_LOGIN_FLOW")
                })?,
            };
            let p = matrix::Provision { login_id: login.clone().unwrap_or_default(), ..p };
            let (sid, step) = rt.block_on(bridge_login::start(p, flow)).map_err(|e| failed(&e))?;
            keep(&sid, login.map_or(Value::Null, Value::from), account_id.map_or(Value::Null, Value::from), step)
        }
        ("POST", ["login-sessions", sid, "steps", step_id]) => {
            let s = session(sid)?;
            let kind = r.need("type")?;
            if s.view["currentStep"]["stepID"] != *step_id || s.view["currentStep"]["type"] != kind {
                let message = "Login step does not match the current session state";
                return Err(error(StatusCode::CONFLICT, message, "INVALID_LOGIN_STEP"));
            }
            let fields = match kind {
                "display_and_wait" => Value::Null,
                _ => r.body.get("fields").cloned().unwrap_or(json!({})),
            };
            let p = matrix::Provision { login_id: s.view["loginID"].as_str().unwrap_or_default().into(), ..p };
            let step = rt.block_on(bridge_login::submit(p, sid.to_string(), s.step, fields)).map_err(|e| failed(&e))?;
            keep(sid, s.view["loginID"].clone(), s.view["accountID"].clone(), step)
        }
        ("DELETE", ["login-sessions", sid]) => {
            session(sid)?;
            rt.block_on(bridge_login::cancel(p, sid.to_string())).map_err(|e| failed(&e))?;
            store.write().bridge_sessions.remove(*sid);
            ok(&json!({ "bridgeID": id, "loginSessionID": sid, "status": "cancelled" }))
        }
        _ => unreachable!("bridge_login picks the routes"),
    }
}

fn bridge_get(st: &BeeperStore, path: &[&str]) -> Reply {
    let [id, rest @ ..] = path else {
        let items: Vec<_> = NETWORK_SPECS.iter().filter(|s| view::listed(s)).map(|s| view::bridge(st, s)).collect();
        return ok(&json!({ "items": items }));
    };
    let spec = view::bridge_network(id);
    // Desktop's login routes read the bridge accounts, so a bridge without accounts has no logins.
    let logins = || spec.map_or_else(Vec::new, |spec| view::logins(st, spec));
    match rest {
        [] => ok(&view::bridge(st, spec.filter(|s| view::listed(s)).ok_or_else(|| missing("Bridge not found"))?)),
        ["logins"] => ok(&json!({ "items": logins() })),
        ["logins", login_id] => {
            let login = logins().into_iter().find(|l| l.login_id == *login_id);
            ok(&login.ok_or_else(|| missing("Bridge login not found"))?)
        }
        ["login-sessions", sid] => {
            let session = st.bridge_sessions.get(*sid).filter(|s| s.view["bridgeID"] == *id);
            ok(&session.ok_or_else(|| missing("Bridge login session not found"))?.view)
        }
        _ => Err(no_route()),
    }
}

/// A range of `chat_order` keys.
type ChatRange = (Bound<(i64, String)>, Bound<(i64, String)>);

/// The `chat_order` keys of `range` newest first. Both directions take the newest chats past a cursor, like
/// Desktop's `ORDER BY timestamp DESC LIMIT`.
fn chat_keys(st: &BeeperStore, range: ChatRange) -> impl Iterator<Item = (i64, &str)> {
    st.chat_order().range(range).rev().map(|(ms, id)| (*ms, id.as_str()))
}

/// The `chat_order` keys past the key of a cursor: the older ones `before` it, or the newer ones `after` it.
fn past(key: (i64, String), after: bool) -> ChatRange {
    match after {
        true => (Excluded(key), Unbounded),
        false => (Unbounded, Excluded(key)),
    }
}

/// The `chat_order` keys past a cursor of Desktop's `chatPreviewRepo.getRooms`, which reads `<timestamp>:<row ID>`
/// with `j8`, here with the chat ID as the row ID, or a bare timestamp with `Nh`, which skips every chat of that
/// millisecond. `None` for a cursor that it cannot read.
fn rooms_range(cursor: &str, after: bool) -> Option<ChatRange> {
    match cursor.split_once(':') {
        Some((_, "")) => None,
        Some((ms, id)) => Some(past((ms.parse().ok()?, id.into()), after)),
        None => {
            let ms = Some(js_number(cursor)).filter(|ms| ms.is_finite())?;
            Some(match after {
                true => (Included(((ms.floor() as i64).saturating_add(1), String::new())), Unbounded),
                false => (Unbounded, Excluded((ms.ceil() as i64, String::new()))),
            })
        }
    }
}

/// Desktop's `listChats` over `chatPreviewRepo.getRooms`: the chats newest first, from the `limit` rows past the
/// cursor. `hasMore` means a full page of rows, and the rows of the member chats of merged chats do not show, like
/// Desktop.
fn list_chats(st: &BeeperStore, r: &Req) -> Reply {
    let mut issues = Vec::new();
    check(&mut issues, r.string("cursor"));
    let after = check(&mut issues, r.direction());
    let limit = r.page_size(25, MIN_1, 200).map_err(|found| issues.extend(found)).ok();
    let (Some(after), Some(limit), true) = (after, limit, issues.is_empty()) else { return Err(invalid_input(issues)) };
    let accounts: Vec<&str> = r.qs("accountIDs").into_iter().filter(|a| !a.trim().is_empty()).collect();
    // The part of a cursor before `|` pages the Beeper chats, and the part after it the iMessage chats of macOS.
    let cursor = r.q("cursor").and_then(|c| c.split('|').next()).filter(|c| !c.is_empty());
    let range = cursor.map_or(Some((Unbounded, Unbounded)), |c| rooms_range(c, after));
    let in_accounts = |k: &(i64, &str)| accounts.is_empty() || accounts.contains(&st.chat(k.1).account_id.as_str());
    let rows: Vec<(i64, &str)> =
        range.map_or_else(Vec::new, |range| chat_keys(st, range).filter(in_accounts).take(limit).collect());
    let shown: Vec<(i64, &str)> = rows.iter().copied().filter(|k| st.chat(k.1).listed()).collect();
    let (newest, oldest) = match shown.is_empty() {
        // With only member chats, Desktop moves the cursor of the direction to a row of `getRooms`, which returns its
        // rows oldest first: to the last one, the newest, `before`, and to the first one, the oldest, `after`.
        true if after => (rows.last(), None),
        true => (None, rows.first()),
        false => (shown.first(), shown.last()),
    };
    let cursor_of = |k: Option<&(i64, &str)>| k.map(|(ms, id)| format!("{ms}:{id}")).or(cursor.map(str::to_string));
    let items = shown.iter().map(|k| view::chat(st, st.chat(k.1), Some(20), true)).collect();
    let (oldest_cursor, newest_cursor) = (cursor_of(oldest), cursor_of(newest));
    ok(&CursorPage { items, has_more: rows.len() == limit, oldest_cursor, newest_cursor })
}

fn chats(st: &mut BeeperStore, method: &str, path: &[&str], r: &Req) -> Reply {
    match (method, path) {
        ("POST", []) => merge_chats(st, r),
        (_, [id, rest @ ..]) => {
            chat_input(method, rest, r)?;
            let idx = st.resolve_chat_idx(id).ok_or_else(|| no_chat(id))?;
            chat(st, method, idx, id, rest, r)
        }
        _ => Err(no_route()),
    }
}

/// beeper-rs's `mode: "merge"` of `POST /v1/chats`, which Desktop does not have: a merged chat of the chats.
fn merge_chats(st: &mut BeeperStore, r: &Req) -> Reply {
    let merge = &r.body["merge"];
    let chat_ids: Vec<String> =
        serde_json::from_value(merge["chatIDs"].clone()).map_err(|_| bad("merge.chatIDs is required"))?;
    let default_chat_id = merge["defaultChatID"].as_str().map(str::to_string);
    check_merge(st, &chat_ids, default_chat_id.as_deref())?;
    let lead = default_chat_id.as_deref().unwrap_or(&chat_ids[0]);
    let account = st.chats[st.resolve_chat_idx(lead).expect("check_merge found the chat")].account_id.clone();
    let title = r.need("title")?.into();
    let (chat, status) =
        st.create_or_start_chat(&account, "merged", Some(title), Vec::new(), None, Some(chat_ids), default_chat_id);
    ok(&json!({ "chatID": chat.id, "status": status }))
}

/// The member chats of a merged chat exist.
fn check_merge(st: &BeeperStore, chat_ids: &[String], default_chat_id: Option<&str>) -> Result<(), Response> {
    if chat_ids.is_empty() {
        return Err(bad("merge.chatIDs is required"));
    }
    let mut ids = chat_ids.iter().map(String::as_str).chain(default_chat_id);
    ids.all(|id| st.resolve_chat_idx(id).is_some()).then_some(()).ok_or_else(|| missing("Chat not found"))
}

/// The input of Desktop's `searchChats` (`SearchChatsInput`).
#[derive(Default)]
struct ChatSearch<'a> {
    cursor: Option<&'a str>,
    after: bool,
    inbox: Option<&'a str>,
    label: Option<&'a str>,
    unread_only: bool,
    limit: usize,
    chat_type: Option<&'a str>,
    query: &'a str,
    people: bool,
    active_before: Option<i64>,
    active_after: Option<i64>,
    accounts: Vec<&'a str>,
    include_muted: bool,
}

impl<'a> ChatSearch<'a> {
    /// The chat sections of Desktop's `search`, which call `searchChats` with a query, a scope and a limit.
    fn of(query: &'a str, people: bool, limit: usize) -> Self {
        ChatSearch { query: search::js_trim(query), people, limit, include_muted: true, ..ChatSearch::default() }
    }

    /// The `SearchChatsInput` of a request, or Desktop's answer with its zod issues in the order of the schema.
    fn parse(r: &'a Req) -> Result<Self, Response> {
        let mut issues = Vec::new();
        let cursor = check(&mut issues, r.string("cursor")).flatten();
        let after = check(&mut issues, r.direction()).unwrap_or_default();
        let inbox = check(&mut issues, r.choice("inbox", &["primary", "low-priority", "archive"])).flatten();
        let label = check(&mut issues, r.string("labelID")).flatten();
        let unread_only = check(&mut issues, r.boolean("unreadOnly", false)).unwrap_or_default();
        let limit = r.page_size(50, MIN_1, 200).unwrap_or_else(|found| {
            issues.extend(found);
            0
        });
        let chat_type = check(&mut issues, r.choice("type", &["single", "group", "any"])).flatten();
        let query = check(&mut issues, r.filled("query")).flatten();
        let scope = check(&mut issues, r.choice("scope", &["titles", "participants"])).flatten();
        let active_before = check(&mut issues, r.datetime("lastActivityBefore", false)).flatten();
        let active_after = check(&mut issues, r.datetime("lastActivityAfter", false)).flatten();
        let include_muted = check(&mut issues, r.boolean("includeMuted", true)).unwrap_or_default();
        if !issues.is_empty() {
            return Err(invalid_input(issues));
        }
        Ok(ChatSearch {
            cursor,
            after,
            inbox,
            label,
            unread_only,
            limit,
            chat_type: chat_type.filter(|&t| t != "any"),
            query: query.unwrap_or_default(),
            people: scope == Some("participants"),
            active_before,
            active_after,
            accounts: r.qs("accountIDs"),
            include_muted,
        })
    }
}

/// Desktop's `timestamp` of a chat in Unix milliseconds: its last activity, or its newer draft.
fn chat_time(c: &Chat) -> Option<i64> {
    (c.last_activity.is_some() || c.draft.as_ref().is_some_and(|d| d.ts.is_some())).then(|| activity_ms(c))
}

/// A key of Desktop's chat search cursors: the title hits for a query in the scope `titles`, the time and the chat.
struct ChatKey<'a> {
    score: Option<usize>,
    ts: i64,
    id: &'a str,
}

impl ChatKey<'_> {
    /// The cursor of the key: its JSON in base64.
    fn cursor(&self) -> String {
        let key = match self.score {
            Some(score) => json!({ "score": score, "ts": self.ts, "id": self.id }),
            None => json!({ "ts": self.ts, "id": self.id }),
        };
        base64_encode_standard(key.to_string().as_bytes())
    }

    /// Desktop's comparators `B` (with a score) and `me` of the key and a decoded cursor, with JavaScript's `!==` and
    /// `-` on the values of the cursor: positive when the key comes after the cursor, and NaN for a bad cursor.
    fn past(&self, cursor: &Value) -> f64 {
        let diff = |k: &str, own: i64| match cursor.get(k) {
            Some(Value::Number(n)) if n.as_f64() == Some(own as f64) => None,
            v => Some(js_to_number(v) - own as f64),
        };
        let id = || f64::from(locale_cmp(self.id, &js_string(cursor.get("id"))) as i8);
        self.score.and_then(|s| diff("score", s as i64)).or_else(|| diff("ts", self.ts)).unwrap_or_else(id)
    }
}

/// JavaScript's `Number` of a JSON value, or of `undefined` for none.
fn js_to_number(v: Option<&Value>) -> f64 {
    match v {
        None => f64::NAN,
        Some(Value::Null) => 0.0,
        Some(Value::Bool(b)) => f64::from(u8::from(*b)),
        Some(Value::Number(n)) => n.as_f64().unwrap_or(f64::NAN),
        Some(v) => js_number(&js_string(Some(v))),
    }
}

/// JavaScript's `String` of a JSON value, or of `undefined` for none.
fn js_string(v: Option<&Value>) -> String {
    match v {
        None => "undefined".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => {
            let item = |v: &Value| if v.is_null() { String::new() } else { js_string(Some(v)) };
            a.iter().map(item).collect::<Vec<_>>().join(",")
        }
        Some(Value::Object(_)) => "[object Object]".into(),
        // JavaScript writes a whole float without its `.0`.
        Some(Value::Number(n)) if n.as_f64().is_some_and(|f| n.is_f64() && f.fract() == 0.0 && f.abs() < 1e21) => {
            format!("{:.0}", n.as_f64().expect("a float") + 0.0)
        }
        Some(v) => v.to_string(),
    }
}

/// Desktop's decoding of a chat search cursor: a JSON object (or array) in base64, read like Node's lenient
/// `Buffer.from(cursor, "base64")`, which takes the URL-safe alphabet too, skips other characters and stops at `=`.
fn decode_chat_cursor(cursor: &str) -> Option<Value> {
    let b64: String = cursor
        .chars()
        .take_while(|&c| c != '=')
        .filter_map(|c| match c {
            '-' => Some('+'),
            '_' => Some('/'),
            c if c.is_ascii_alphanumeric() || c == '+' || c == '/' => Some(c),
            _ => None,
        })
        .collect();
    let json = String::from_utf8_lossy(&base64_decode(&b64).expect("base64 characters")).into_owned();
    serde_json::from_str(&json).ok().filter(|v: &Value| v.is_object() || v.is_array())
}

/// Desktop's `searchChats`: the chats that match the query in the scope `titles` (the title and the network), by the
/// title hits of the query, or in the scope `participants`, or all chats without a query, then newest first. Member
/// chats show as their merged chat, with the most hits of the members. Both directions take the first chats past
/// the cursor, whose keys are base64 JSON.
fn chat_search<'a>(st: &'a BeeperStore, s: &ChatSearch) -> CursorPage<view::ApiChat<'a>> {
    let accounts = |c: &&Chat| s.accounts.is_empty() || s.accounts.contains(&c.account_id.as_str());
    // Desktop hides a merged chat past the plan's limit.
    let pool = st.chats.iter().filter(|c| c.flags & CHAT_INACTIVE_UNION == 0).filter(accounts);
    let found: Vec<(&Chat, usize)> = match s.query {
        "" => pool.map(|c| (c, 0)).collect(),
        q => {
            let keep = |c: &Chat| c.flags & CHAT_INACTIVE_UNION == 0 && accounts(&c);
            search::search_in(st, keep, q, s.people, true)
        }
    };
    let (mut chats, mut merged) = (Vec::new(), Vec::<(&Chat, usize)>::new());
    for (c, hits) in found {
        let parent = match &c.merged_into_chat_id {
            _ if c.merge.is_some() => Some(c),
            Some(id) => st.resolve_chat_idx(id).map(|i| &st.chats[i]),
            None => None,
        };
        match parent {
            None => chats.push((c, hits)),
            Some(p) => match merged.iter_mut().find(|(m, _)| m.id == p.id) {
                Some(m) => m.1 = m.1.max(hits),
                None => merged.push((p, hits)),
            },
        }
    }
    chats.extend(merged);
    let titles = !s.query.is_empty() && !s.people;
    let ts = |c: &Chat| chat_time(c).unwrap_or(0);
    chats.sort_by(|a, b| {
        let hits = if titles { b.1.cmp(&a.1) } else { std::cmp::Ordering::Equal };
        hits.then(ts(b.0).cmp(&ts(a.0))).then_with(|| locale_cmp(&a.0.id, &b.0.id))
    });
    let keep = |c: &Chat| {
        let ms = chat_time(c);
        s.chat_type.is_none_or(|t| view::chat_type(c) == t)
            && (!s.unread_only || c.unread_count > 0)
            && s.label.is_none_or(|l| c.label_ids.iter().any(|id| id == l))
            && (s.include_muted || c.flags & CHAT_MUTED == 0)
            && s.active_before.is_none_or(|b| ms.is_none_or(|ms| ms < b))
            && s.active_after.is_none_or(|a| ms.is_none_or(|ms| ms > a))
            && match s.inbox {
                Some("primary") => c.flags & (CHAT_ARCHIVED | CHAT_LOW_PRIORITY | CHAT_HIDDEN) == 0,
                Some("low-priority") => c.flags & CHAT_LOW_PRIORITY != 0,
                Some("archive") => c.flags & CHAT_ARCHIVED != 0,
                _ => true,
            }
    };
    let cursor = s.cursor.and_then(decode_chat_cursor);
    let past = |k: &ChatKey| cursor.as_ref().is_none_or(|c| if s.after { k.past(c) < 0.0 } else { k.past(c) > 0.0 });
    let rows: Vec<(&Chat, ChatKey)> = chats
        .iter()
        .filter(|(c, _)| keep(c))
        .map(|&(c, hits)| (c, ChatKey { score: titles.then_some(hits), ts: ts(c), id: &c.id }))
        .filter(|(_, k)| past(k))
        .collect();
    let page = &rows[..rows.len().min(s.limit)];
    CursorPage {
        items: page.iter().map(|(c, _)| view::chat(st, c, Some(20), false)).collect(),
        has_more: rows.len() > s.limit,
        oldest_cursor: page.last().map(|(_, k)| k.cursor()),
        newest_cursor: page.first().map(|(_, k)| k.cursor()),
    }
}

fn search_chats(st: &BeeperStore, r: &Req) -> Reply {
    ok(&chat_search(st, &ChatSearch::parse(r)?))
}

/// Desktop's `Sp` and `SetChatReminderInput`: the time of `reminder.remindAt`, or of the older `remindAtMs`, and
/// `dismissOnIncomingMessage`.
fn reminder_input(r: &Req) -> Result<(i64, Option<bool>), Response> {
    let reminder = r.input("reminder");
    let Some(Value::Object(o)) = reminder else {
        let issue = type_issue(["code", "expected", "received"], "object", zod_type(reminder), json!(["reminder"]));
        return Err(invalid_input(vec![issue]));
    };
    let path = |k: &str| json!(["reminder", k]);
    let legacy = match (o.get("remindAt"), o.get("remindAtMs")) {
        (None, Some(ms)) => Some(match ms {
            Value::String(s) => js_number(s),
            v => v.as_f64().unwrap_or(f64::NAN),
        })
        .filter(|ms| ms.is_finite()),
        _ => None,
    };
    // `new Date(ms).toISOString()` throws for a time past the range of `Date`.
    if legacy.is_some_and(|ms| ms.abs() > 8.64e15) {
        return Err(failed("Invalid time value"));
    }
    let mut issues = Vec::new();
    let at = match (legacy, o.get("remindAt")) {
        (Some(ms), _) => Some(ms.trunc() as i64),
        (None, Some(Value::String(s))) => zod_datetime(s, false).or_else(|| {
            issues.push(datetime_issue(path("remindAt")));
            None
        }),
        (None, v) => {
            issues.extend(string_issue(v, false, path("remindAt")));
            None
        }
    };
    let dismiss = o.get("dismissOnIncomingMessage");
    issues.extend(kind_issue(dismiss, "boolean", true, path("dismissOnIncomingMessage")));
    checked(issues)?;
    Ok((at.expect("a checked time"), dismiss.and_then(Value::as_bool)))
}

/// The zod issues of `SendMessageInput`: the text, the reply, the attachment, then the refinement that needs one of
/// the text and the attachment.
fn send_issues(r: &Req) -> Vec<Issue> {
    let at = |k: &str| json!(["attachment", k]);
    let mut issues: Vec<Issue> =
        ["text", "replyToMessageID"].into_iter().filter_map(|k| string_issue(r.input(k), true, json!([k]))).collect();
    match r.input("attachment") {
        None => {}
        Some(Value::Object(a)) => {
            issues.extend(string_issue(a.get("uploadID"), false, at("uploadID")));
            issues.extend(["mimeType", "fileName"].into_iter().filter_map(|k| string_issue(a.get(k), true, at(k))));
            match a.get("size") {
                None => {}
                Some(Value::Object(size)) => issues.extend(
                    ["width", "height"]
                        .into_iter()
                        .filter_map(|k| kind_issue(size.get(k), "number", false, json!(["attachment", "size", k]))),
                ),
                v => issues.extend(kind_issue(v, "object", false, at("size"))),
            }
            issues.extend(kind_issue(a.get("duration"), "number", true, at("duration")));
            let kinds = ["image", "video", "audio", "file", "gif", "voice-note", "sticker"];
            issues.extend(a.get("type").and_then(|t| enum_issue(Some(t), &kinds, at("type"))));
        }
        v => issues.extend(kind_issue(v, "object", false, json!(["attachment"]))),
    }
    // Every issue above aborts the parse, which skips the refinement.
    let text = r.input("text").and_then(Value::as_str).unwrap_or_default();
    if issues.is_empty() && r.input("attachment").is_none() && text.is_empty() {
        issues.push(custom_issue("Either text or attachment is required.", json!([])));
    }
    issues
}

/// Desktop's zod schemas of the chat routes, which it checks before it looks for the chat.
fn chat_input(method: &str, rest: &[&str], r: &Req) -> Result<(), Response> {
    let at = |k: &str| json!([k]);
    let optional = |k: &'static str| string_issue(r.input(k), true, at(k));
    checked(match (method, rest) {
        ("PATCH", []) => {
            let nullable =
                |k: &'static str| r.input(k).filter(|v| !v.is_null()).and_then(|v| string_issue(Some(v), false, at(k)));
            let mut issues: Vec<Issue> = ["title", "description", "imgURL"].into_iter().filter_map(nullable).collect();
            let flags = ["isArchived", "isLowPriority", "isPinned", "isMuted"];
            issues.extend(flags.into_iter().filter_map(|k| kind_issue(r.input(k), "boolean", true, at(k))));
            match r.input("messageExpirySeconds") {
                None | Some(Value::Null) => {}
                Some(v) => match v.as_f64() {
                    Some(n) => issues.extend(whole_issues(n, at("messageExpirySeconds"))),
                    None => issues.extend(kind_issue(Some(v), "number", false, at("messageExpirySeconds"))),
                },
            }
            issues
        }
        ("POST", ["archive"]) => kind_issue(r.input("archived"), "boolean", true, at("archived")).into_iter().collect(),
        ("POST", ["read" | "unread"]) => optional("messageID").into_iter().collect(),
        ("POST", ["reminders"]) => return reminder_input(r).map(drop),
        ("POST", ["messages"]) => send_issues(r),
        ("PUT", ["messages", _]) => filled_issue(r.input("text"), at("text")).into_iter().collect(),
        // Desktop's `Xt` reads the strings `true` and `false` of a query as booleans.
        ("DELETE", ["messages", _]) => match r.input("forEveryone") {
            Some(Value::String(s)) if s == "true" || s == "false" => Vec::new(),
            v => kind_issue(v, "boolean", true, at("forEveryone")).into_iter().collect(),
        },
        ("POST", ["messages", _, "reactions"]) => {
            [filled_issue(r.input("reactionKey"), at("reactionKey")), optional("transactionID")]
                .into_iter()
                .flatten()
                .collect()
        }
        _ => Vec::new(),
    })
}

fn chat(st: &mut BeeperStore, method: &str, idx: usize, id: &str, rest: &[&str], r: &Req) -> Reply {
    match (method, rest) {
        ("PATCH", []) => {
            let draft = r.body.get("draft");
            let new_draft = draft.filter(|d| !d.is_null()).map(|d| draft_input(st, d)).transpose()?;
            let new_merge: Option<Option<ChatMergeInfo>> = r.body.get("merge").map(|_| r.parse("merge")).transpose()?;
            if let Some(Some(m)) = &new_merge {
                check_merge(st, &m.chat_ids, m.default_chat_id.as_deref())?;
            }
            let fields = ChatFields::parse(&r.body)?;
            if fields.expiry.is_some() && st.chats[idx].timers.is_none() {
                return Err(bad("messageExpirySeconds: the chat has no disappearing messages"));
            }
            st.patch_chat(id, draft.is_some_and(Value::is_null), new_draft, new_merge).map_err(store_error)?;
            fields.apply(st, idx);
            chat_reply(st, idx)
        }
        ("POST", ["archive"]) => {
            let archived = r.input("archived").and_then(Value::as_bool).unwrap_or(true);
            st.set_chat_flag(idx, ChatFlag::Archived, Some(archived));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["read"]) => {
            st.mark_read(idx, true);
            chat_reply(st, idx)
        }
        ("POST", ["unread"]) => {
            st.set_chat_flag(idx, ChatFlag::MarkedUnread, Some(true));
            chat_reply(st, idx)
        }
        ("POST", ["notify-anyway"]) => {
            st.update_chat(idx, |c| c.flags |= CHAT_NOTIFY_ANYWAY);
            chat_reply(st, idx)
        }
        ("POST", ["reminders"]) => {
            let (at, dismiss) = reminder_input(r)?;
            // Desktop's `setReminder` dismisses the reminder on an incoming message when the request does not say.
            let reminder = ChatReminder::new(at, dismiss.unwrap_or(true));
            st.set_reminder(idx, Some(reminder));
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("DELETE", ["reminders"]) => {
            st.set_reminder(idx, None);
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["messages"]) => send_message(st, id, r),
        ("PUT", ["messages", mid]) => {
            let text = r.input("text").and_then(Value::as_str).expect("the input passed its schema");
            let (msg, rules) = message_rules(st, id, mid)?;
            let refuse = |message| Err(error(StatusCode::BAD_REQUEST, message, "EDIT_NOT_ALLOWED"));
            if !MessageRules::young(rules.edit_max_age, &msg.timestamp) {
                return refuse("This message is too old to edit on this network");
            }
            // Desktop edits the caption of one media file, and no other attachments.
            let media =
                ["IMAGE", "VIDEO", "AUDIO", "VOICE", "FILE"].contains(&msg.message_type.as_deref().unwrap_or(""));
            if msg.attachments.len() > 1 || (msg.attachments.len() == 1 && !media) {
                return refuse("Cannot edit messages with attachments");
            }
            let (_, msg) = st.edit_message(id, mid, text.into(), Vec::new()).map_err(store_error)?;
            let mut edited = serde_json::to_value(view::message(&msg)).expect("serialize a message");
            (edited["success"], edited["messageID"]) = (true.into(), msg.id.clone().into());
            ok(&edited)
        }
        ("DELETE", ["messages", mid]) => {
            let everyone = match r.body.get("forEveryone") {
                Some(v) => v.as_bool().ok_or_else(|| bad("forEveryone must be a boolean"))?,
                None => r.flag("forEveryone", true)?,
            };
            // Desktop's `deleteMessage` of the Matrix platform does nothing when `forEveryone` is false.
            if !everyone {
                return empty(StatusCode::NO_CONTENT, Vec::new());
            }
            let (msg, rules) = message_rules(st, id, mid)?;
            if msg.is_deleted != Some(true) && !MessageRules::young(rules.delete_max_age, &msg.timestamp) {
                let message = "This message is too old to delete for everyone on this network";
                return Err(error(StatusCode::BAD_REQUEST, message, "DELETE_NOT_ALLOWED"));
            }
            st.redact_message(id, mid).map_err(store_error)?;
            empty(StatusCode::NO_CONTENT, Vec::new())
        }
        ("POST", ["messages", mid, "reactions"]) => {
            let key = r.input("reactionKey").and_then(Value::as_str).expect("the input passed its schema");
            let tx = r.input("transactionID").and_then(Value::as_str).map(str::to_string);
            let (cid, tx) = st.add_reaction(id, mid, key, None, tx).map_err(store_error)?;
            ok(&json!({ "success": true, "chatID": cid, "messageID": mid, "reactionKey": key, "transactionID": tx }))
        }
        // Desktop also takes the key from the query, for keys that a path cannot carry. That route skips the schema,
        // so its handler checks the key.
        ("DELETE", ["messages", mid, "reactions", rest @ ..]) if rest.len() <= 1 => {
            let key = rest.first().copied().unwrap_or_else(|| r.q("reactionKey").unwrap_or_default());
            if key.is_empty() {
                return Err(required("reactionKey"));
            }
            let cid = st.remove_reaction(id, mid, key).map_err(store_error)?;
            ok(&json!({ "success": true, "chatID": cid, "messageID": mid, "reactionKey": key }))
        }
        _ => Err(no_route()),
    }
}

/// A message of a chat and the message features of its room.
fn message_rules(st: &mut BeeperStore, chat: &str, id: &str) -> Result<(Message, MessageRules), Response> {
    let (cid, m) = st.message_in_chat(chat, id).map_err(store_error)?;
    Ok((st.messages[m].clone(), st.chat(&cid).messages))
}

/// The canonical IDs of the chat `idx` and of the member chats of a merged chat.
fn chat_and_members(st: &BeeperStore, idx: usize) -> Vec<&str> {
    let members = st.members(idx).into_iter().map(|i| st.chats[i].id.as_str());
    std::iter::once(st.chats[idx].id.as_str()).chain(members).collect()
}

/// Desktop's page size of `getRoomMessages`.
const MESSAGE_PAGE: usize = 20;

/// The chat `idx` and its merged member chats whose rooms have older messages on the homeserver than in the store.
fn unpaginated(st: &BeeperStore, idx: usize) -> Vec<&str> {
    let all = chat_and_members(st, idx).into_iter();
    all.filter(|c| st.matrix.is_some() && st.chat(c).older != Older::Start).collect()
}

/// Desktop's `fetchMessagesPage`: a page back in time that the store cannot fill waits for the homeserver to give the
/// chats an older page, like Desktop's `waitForBackfill`.
fn backfill(store: &SharedStore, id: &str, r: &Req) -> Result<(), Response> {
    let older: Vec<String> = {
        let st = store.read();
        let Some(idx) = st.resolve_chat_idx(id).filter(|_| matches!(r.after(), Ok(false))) else { return Ok(()) };
        let q = Query::default().chats(&chat_and_members(&st, idx)).past(r.q("cursor"), false);
        if message_page(&st, &q, MESSAGE_PAGE, |_| true).0.len() == MESSAGE_PAGE {
            return Ok(());
        }
        unpaginated(&st, idx).into_iter().map(String::from).collect()
    };
    let rt = tokio::runtime::Handle::current();
    for chat in older {
        if let Err(e) = rt.block_on(matrix::load_older(store.clone(), chat.clone())) {
            eprintln!("Could not load older messages of {chat}: {e}");
        }
    }
    Ok(())
}

/// Desktop's `listMessages`: one page of the chat and its merged member chats, newest first. The cursor is a sort
/// key, and both directions take the newest messages past it, like Desktop's `ORDER BY hsOrder DESC LIMIT 20`. A page
/// back in time has more while the homeserver has older messages, like Desktop's `is_fully_paginated`.
fn list_messages(st: &BeeperStore, idx: usize, r: &Req) -> Reply {
    let after = r.after()?;
    // Desktop's `getRoomMessages` reads the cursor as a number of the message order, and its message IDs are numbers
    // of the rows, so the CLI's `messages context` passes a message ID as the cursor. Here an ID of a message of the
    // chat stands for its sort key.
    let cursor = r.q("cursor").map(|c| {
        let message = st.stored_message(c).filter(|m| st.in_chat(idx, &m.chat_id));
        message.map_or_else(|| c.to_string(), |m| m.sort_key)
    });
    let q = Query::default().chats(&chat_and_members(st, idx)).past(cursor.as_deref(), after);
    let (items, has_more) = message_page(st, &q, MESSAGE_PAGE, |_| true);
    let has_more = has_more || (!after && !unpaginated(st, idx).is_empty());
    let (oldest, newest) = (items.last().map(|m| m.sort_key.clone()), items.first().map(|m| m.sort_key.clone()));
    let items = items.iter().map(view::message).collect();
    ok(&CursorPage { items, has_more, oldest_cursor: oldest, newest_cursor: newest })
}

fn upload_not_found(upload_id: &str) -> Response {
    let message = format!("Upload not found: {upload_id}. File may have expired.");
    error(StatusCode::NOT_FOUND, &message, "UPLOAD_NOT_FOUND")
}

/// Desktop's reply of the chat routes that change a chat: the chat with at most 100 participants.
fn chat_reply(st: &BeeperStore, idx: usize) -> Reply {
    ok(&view::chat(st, &st.chats[idx], Some(100), false))
}

/// Desktop's `ChatDraftInput`: the text, and the attachments keyed by upload ID.
fn draft_input(st: &BeeperStore, v: &Value) -> Result<ChatDraft, Response> {
    let text = match &v["text"] {
        Value::Null => None,
        Value::String(t) => Some(t.clone()),
        _ => return Err(bad("draft.text must be a string")),
    };
    let ids = match &v["attachments"] {
        Value::Null => Vec::new(),
        Value::Object(m) => m.keys().collect(),
        _ => return Err(bad("draft.attachments must be an object keyed by upload ID")),
    };
    let attachment = |id: &String| {
        let a = st.assets.get(id).ok_or_else(|| upload_not_found(id))?;
        let (file_name, mime_type) = (Some(a.file_name.clone()), Some(a.mime_type.clone()));
        Ok(ChatDraftAttachment { upload_id: id.clone(), file_name, mime_type })
    };
    let attachments = ids.into_iter().map(attachment).collect::<Result<_, Response>>()?;
    Ok(ChatDraft { text, attachments, ..ChatDraft::default() })
}

/// The fields of Desktop's `patchChat` besides `draft`, checked before the chat changes.
struct ChatFields {
    title: Option<String>,
    description: Option<String>,
    avatar: Option<Option<(String, Vec<u8>, String)>>,
    flags: Vec<(ChatFlag, bool)>,
    expiry: Option<u64>,
}

impl ChatFields {
    fn parse(b: &Value) -> Result<Self, Response> {
        let text = |k: &str| match b.get(k) {
            None => Ok(None),
            Some(Value::String(t)) => Ok(Some(t.clone())),
            Some(Value::Null) if k == "description" => Ok(Some(String::new())),
            Some(_) => Err(bad(&format!("{k} must be a string"))),
        };
        let avatar = match b.get("imgURL") {
            None => None,
            Some(Value::Null) => Some(None),
            Some(Value::String(url)) => Some(Some(avatar_file(url)?)),
            Some(_) => return Err(bad("imgURL must be a string")),
        };
        let mut flags = Vec::new();
        // The pin goes first, because the store does not archive a pinned chat.
        for flag in [ChatFlag::Pinned, ChatFlag::Archived, ChatFlag::Muted, ChatFlag::LowPriority] {
            let k = flag.api_key();
            match b.get(k) {
                None => {}
                Some(Value::Bool(on)) => flags.push((flag, *on)),
                Some(_) => return Err(bad(&format!("{k} must be a boolean"))),
            }
        }
        let expiry = match b.get("messageExpirySeconds") {
            None => None,
            Some(Value::Null) => Some(0),
            Some(v) => Some(v.as_u64().ok_or_else(|| bad("messageExpirySeconds must be a whole number or null"))?),
        };
        Ok(Self { title: text("title")?, description: text("description")?, avatar, flags, expiry })
    }

    fn apply(self, st: &mut BeeperStore, idx: usize) {
        let id = st.chats[idx].id.clone();
        if self.title.is_some() || self.description.is_some() || self.avatar.is_some() {
            st.update_group(&id, self.title, self.description, self.avatar);
        }
        for (flag, on) in self.flags {
            st.set_chat_flag(idx, flag, Some(on));
        }
        if let Some(seconds) = self.expiry {
            st.set_disappearing(&id, seconds * 1000);
        }
    }
}

/// The file URL, the bytes, and the MIME type of a new chat avatar from an absolute path or a `file://` URL.
fn avatar_file(url: &str) -> Result<(String, Vec<u8>, String), Response> {
    let path = match url.starts_with("file://") {
        true => url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok()),
        false => Some(std::path::PathBuf::from(url)).filter(|p| p.is_absolute()),
    };
    let path = path.ok_or_else(|| bad("imgURL must be an absolute path or a file URL"))?;
    let mime = mime_type_of(&path.to_string_lossy())
        .filter(|m| ["image/png", "image/jpeg", "image/gif", "image/webp"].contains(m))
        .ok_or_else(|| bad("imgURL must be a PNG, JPEG, GIF, or WebP image"))?;
    let bytes = std::fs::read(&path).map_err(|e| bad(&format!("imgURL: {e}")))?;
    Ok((path_to_file_url(&path), bytes, mime.into()))
}

/// Desktop's `resolveUploadedMessageAttachment`: the upload, with the name, the type, the size and the length of the
/// request in place of the stored ones. The `type` of the request marks GIFs, voice notes and stickers.
fn upload_file(st: &BeeperStore, a: &Value) -> Result<OutFile, Response> {
    let missing = || error(StatusCode::BAD_REQUEST, "uploadID is required", "MISSING_UPLOAD_ID");
    let upload_id = a["uploadID"].as_str().ok_or_else(missing)?;
    let stored = st.assets.get(upload_id).ok_or_else(|| upload_not_found(upload_id))?;
    let mime = a["mimeType"].as_str().unwrap_or(&stored.mime_type).to_string();
    if !is_mime(&mime) {
        return Err(bad("mimeType must be a MIME type"));
    }
    let expired = || error(StatusCode(410), &format!("Upload expired: {upload_id}"), "UPLOAD_EXPIRED");
    let bytes = std::fs::read(st.config.assets_dir.join(&stored.disk_name)).map_err(|_| expired())?;
    let kind = a["type"].as_str();
    let dim = |k: &str| a["size"][k].as_f64().map(|n| n as u32);
    Ok(OutFile {
        bytes,
        name: a["fileName"].as_str().unwrap_or(&stored.file_name).to_string(),
        size: dim("width").zip(dim("height")).or(stored.width.zip(stored.height)),
        duration: a["duration"].as_f64().or(stored.duration),
        gif: kind == Some("gif") || mime == "image/gif",
        sticker: kind == Some("sticker"),
        voice: (kind == Some("voice-note")).then(Vec::new),
        echo_url: Some(stored.mxc_url.clone()),
        echo_poster: None,
        mime,
        pack: None,
    })
}

/// Desktop's `sendMessage`: the text, or the file with the text as its caption, like the composer.
fn send_message(st: &mut BeeperStore, id: &str, r: &Req) -> Reply {
    let file = r.input("attachment").map(|a| upload_file(st, a)).transpose()?;
    let text = |k: &str| r.input(k).and_then(Value::as_str).map(str::to_string);
    let (text, reply_to) = (text("text").filter(|t| !t.is_empty() || file.is_none()), text("replyToMessageID"));
    let draft = Draft { text, reply_to, files: file.into_iter().collect(), ..Draft::default() };
    let (chat_id, txns) = st.send(id, draft).map_err(store_error)?;
    ok(&json!({ "chatID": chat_id, "pendingMessageID": txns[0] }))
}

/// The input of Desktop's `searchMessages` (`SearchMessagesInput`).
#[derive(Default)]
struct MessageSearch<'a> {
    query: &'a str,
    cursor: Option<&'a str>,
    after: bool,
    chat_ids: Vec<&'a str>,
    accounts: Vec<&'a str>,
    chat_type: Option<&'a str>,
    media: Vec<&'a str>,
    sender: Option<&'a str>,
    date_after: Option<i64>,
    date_before: Option<i64>,
    limit: usize,
    exclude_low_priority: bool,
    include_muted: bool,
}

impl<'a> MessageSearch<'a> {
    /// The message section of Desktop's `search`, which calls `searchMessages` with a query and a limit of 20.
    fn of(query: &'a str) -> Self {
        MessageSearch { query, limit: 20, exclude_low_priority: true, include_muted: true, ..MessageSearch::default() }
    }

    /// The `SearchMessagesInput` of a request, or Desktop's answer with its zod issues in the order of the schema.
    /// The refinement on the dates runs unless an issue aborts the parse.
    fn parse(r: &'a Req) -> Result<Self, Response> {
        let mut issues = Vec::new();
        let query = check(&mut issues, r.string("query")).flatten();
        let cursor = check(&mut issues, r.string("cursor")).flatten();
        let after = check(&mut issues, r.direction()).unwrap_or_default();
        let chat_type = check(&mut issues, r.choice("chatType", &["group", "single"])).flatten();
        let media = r.choices("mediaTypes", &["any", "video", "image", "link", "file"]).unwrap_or_else(|found| {
            issues.extend(found);
            Vec::new()
        });
        let sender = check(&mut issues, r.string("sender")).flatten();
        let date_after = check(&mut issues, r.datetime("dateAfter", true)).flatten();
        let date_before = check(&mut issues, r.datetime("dateBefore", true)).flatten();
        let limit = r.page_size(20, POSITIVE, 20).unwrap_or_else(|found| {
            issues.extend(found);
            0
        });
        let exclude_low_priority = check(&mut issues, r.boolean("excludeLowPriority", true)).unwrap_or_default();
        let include_muted = check(&mut issues, r.boolean("includeMuted", true)).unwrap_or_default();
        if !issues.iter().any(Issue::aborts) && date_after.zip(date_before).is_some_and(|(a, b)| a >= b) {
            issues.push(custom_issue(
                "'dateAfter' must be earlier than 'dateBefore' when both are provided.",
                json!(["dateAfter"]),
            ));
        }
        if !issues.is_empty() {
            return Err(invalid_input(issues));
        }
        Ok(MessageSearch {
            query: query.unwrap_or_default(),
            cursor,
            after,
            chat_ids: r.qs("chatIDs"),
            accounts: r.qs("accountIDs"),
            chat_type,
            media,
            sender,
            date_after,
            date_before,
            limit,
            exclude_low_priority,
            include_muted,
        })
    }
}

/// Desktop's `dr`: a cursor of its Beeper part and its iMessage part, `<beeper>|<imessage>`.
fn joined_cursor(beeper: Option<String>, imessage: Option<&str>) -> Option<String> {
    match imessage {
        Some(i) => Some(format!("{}|{i}", beeper.unwrap_or_default())),
        None => beeper,
    }
}

/// A page of Desktop's `searchMessages`, with the chats of its messages.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MessagePage<'a> {
    items: Vec<view::ApiMessage<'a>>,
    chats: serde_json::Map<String, Value>,
    has_more: bool,
    oldest_cursor: Option<String>,
    newest_cursor: Option<String>,
}

/// Desktop's `searchMessages` over `searchRoomMessages`: the newest `limit` messages past the cursor in both
/// directions, by time and row, that match the query as a phrase and the filters, then in sort order, with the chats
/// of the messages. A cursor is `<time>:<row>`, and `|` and an iMessage part, which Desktop keeps.
fn message_search(st: &BeeperStore, s: &MessageSearch) -> Result<Value, Response> {
    let mut rooms = Vec::new();
    for id in &s.chat_ids {
        let chat = &st.chats[st.resolve_chat_idx(id).ok_or_else(|| no_chat(id))?];
        match &chat.merge {
            Some(m) => {
                rooms.extend(m.chat_ids.iter().map(|id| st.resolve_canonical_chat_id(id).expect("member exists")))
            }
            None => rooms.push(chat.id.clone()),
        }
    }
    let mut q = Query::default();
    if !rooms.is_empty() {
        q = q.chats(&rooms.iter().map(String::as_str).collect::<Vec<_>>());
    }
    match s.query {
        "" => {}
        // Desktop asks SQLite for the empty full-text query of a blank one, which fails.
        q if search::js_trim(q).is_empty() => {
            let message = "Failed to execute tool: searchMessages";
            return Err(error_with(StatusCode(500), message, "TOOL_EXECUTION_ERROR", json!({})));
        }
        text => q = q.text(text),
    }
    let (beeper, imessage) = match s.cursor.map(|c| c.split_once('|').unwrap_or((c, ""))) {
        Some((b, i)) => (b, Some(i).filter(|i| !i.is_empty())),
        None => ("", None),
    };
    let integer = |n: &str| Some(js_number(n)).filter(|n| n.fract() == 0.0).map(|n| n as i64);
    let row = beeper.split_once(':').filter(|(ms, n)| !ms.is_empty() && !n.is_empty());
    if let Some(at) = row.and_then(|(ms, n)| integer(ms).zip(integer(n))) {
        q = q.past_row(at, s.after);
    }
    let kind = |m: &Message, k: &str| m.attachments.iter().any(|a| a.attachment_type == k);
    let media = |m: &Message| match &s.media[..] {
        [] => true,
        media if media.contains(&"any") => !m.attachments.is_empty(),
        media => {
            (media.contains(&"image") && kind(m, "img"))
                || (media.contains(&"video") && kind(m, "video"))
                || (media.contains(&"link") && (kind(m, "link") || !m.links.is_empty()))
                || (media.contains(&"file") && kind(m, "unknown"))
        }
    };
    let keep = |m: &Message| {
        let (ms, c) = (millis_from_rfc3339(&m.timestamp).expect("RFC 3339 message time"), st.chat(&m.chat_id));
        m.flags & MSG_DELETED == 0
            && !matches!(m.message_type.as_deref(), Some("HIDDEN" | "REACTION"))
            && match s.sender {
                None => true,
                Some("me") => m.is_sender == Some(true),
                Some("others") => m.is_sender != Some(true),
                Some(id) => m.sender_id == id,
            }
            && s.date_before.is_none_or(|b| ms < b)
            && s.date_after.is_none_or(|a| ms > a)
            && media(m)
            && (s.accounts.is_empty() || s.accounts.contains(&c.account_id.as_str()))
            && s.chat_type.is_none_or(|t| view::chat_type(c) == t)
            && !(s.exclude_low_priority && c.flags & CHAT_LOW_PRIORITY != 0)
            && (s.include_muted || c.flags & CHAT_MUTED == 0)
    };
    let mut rows = Vec::new();
    st.scan_by_time(&q, false, |n, m| {
        if keep(&m) {
            rows.push((n, m));
        }
        rows.len() <= s.limit
    });
    let has_more = rows.len() > s.limit;
    rows.truncate(s.limit);
    let cursor = |(n, m): &(i64, Message)| format!("{}:{n}", millis_from_rfc3339(&m.timestamp).expect("message time"));
    let (oldest, newest) = (rows.last().map(cursor), rows.first().map(cursor));
    rows.sort_by(|a, b| b.1.sort_key.cmp(&a.1.sort_key));
    let mut chats = serde_json::Map::new();
    for (_, m) in &rows {
        chats.entry(&m.chat_id).or_insert_with(|| json!(view::chat(st, st.chat(&m.chat_id), None, false)));
    }
    Ok(json!(MessagePage {
        items: rows.iter().map(|(_, m)| view::message(m)).collect(),
        chats,
        has_more,
        oldest_cursor: joined_cursor(oldest, imessage),
        newest_cursor: joined_cursor(newest, imessage),
    }))
}

fn search_messages(st: &BeeperStore, r: &Req) -> Reply {
    ok(&message_search(st, &MessageSearch::parse(r)?)?)
}

/// Desktop's `search`: `searchChats` with the scope `titles` and with the scope `participants`, 30 chats each, and the
/// first page of `searchMessages`.
fn unified_search(st: &BeeperStore, r: &Req) -> Reply {
    // Desktop's `UnifiedSearchInput`: `query`, a string of at least one character.
    checked(filled_issue(r.query.get("query"), json!(["query"])).into_iter().collect())?;
    let q = r.q("query").expect("the input passed its schema");
    let chats = |people| chat_search(st, &ChatSearch::of(q, people, 30)).items;
    let messages = message_search(st, &MessageSearch::of(q))?;
    ok(&json!({ "results": { "chats": chats(false), "in_groups": chats(true), "messages": messages } }))
}

/// The MIME type and the extension of the `file-type` package for the first bytes of a file.
fn sniff(b: &[u8]) -> Option<(&'static str, &'static str)> {
    let at = |i: usize, magic: &[u8]| b.get(i..i + magic.len()) == Some(magic);
    Some(match () {
        _ if at(0, b"\x89PNG\r\n\x1a\n") => ("image/png", "png"),
        _ if at(0, b"\xFF\xD8\xFF") => ("image/jpeg", "jpg"),
        _ if at(0, b"GIF8") => ("image/gif", "gif"),
        _ if at(0, b"RIFF") && at(8, b"WEBP") => ("image/webp", "webp"),
        _ if at(0, b"RIFF") && at(8, b"WAVE") => ("audio/wav", "wav"),
        _ if at(0, b"%PDF") => ("application/pdf", "pdf"),
        _ if at(0, b"OggS") && at(28, b"OpusHead") => ("audio/ogg; codecs=opus", "opus"),
        _ if at(0, b"OggS") && at(28, b"\x01vorbis") => ("audio/ogg", "ogg"),
        _ if at(0, b"OggS") => ("application/ogg", "ogx"),
        _ if at(0, b"ID3") => ("audio/mpeg", "mp3"),
        _ if at(0, b"PK\x03\x04") => ("application/zip", "zip"),
        _ if at(4, b"ftypM4A ") => ("audio/x-m4a", "m4a"),
        _ if at(4, b"ftypheic") || at(4, b"ftypheix") => ("image/heic", "heic"),
        _ if at(4, b"ftypqt  ") => ("video/quicktime", "mov"),
        _ if at(4, b"ftyp") => ("video/mp4", "mp4"),
        _ if at(0, b"\x1A\x45\xDF\xA3") && find(&b[..b.len().min(64)], b"webm").is_some() => ("video/webm", "webm"),
        _ if at(0, b"\x1A\x45\xDF\xA3") => ("video/x-matroska", "mkv"),
        _ => return None,
    })
}

/// Desktop's `uploadAsset`: the base name of the file with an extension, the MIME type of the request or else of
/// the bytes, and the file URL of the stored file.
fn upload(st: &mut BeeperStore, name: Option<&str>, mime: Option<&str>, bytes: &[u8]) -> Reply {
    let name = name.filter(|n| !n.is_empty()).unwrap_or("file").rsplit('/').next().expect("a last item");
    let name = if matches!(name, "" | "." | "..") { "file" } else { name };
    let lower = name.to_ascii_lowercase();
    let device = |p: &str| lower.strip_prefix(p).is_some_and(|d| d.len() == 1 && d.as_bytes()[0].is_ascii_digit());
    if name.starts_with('.')
        || name.contains("::")
        || name.contains('~')
        || ["con", "prn", "aux", "nul"].contains(&lower.as_str())
        || device("com")
        || device("lpt")
    {
        let message = "Invalid filename: hidden files or reserved names not allowed";
        return Err(error(StatusCode::BAD_REQUEST, message, "INVALID_FILENAME"));
    }
    if name.contains('\0') || name.encode_utf16().count() > 255 {
        return Err(error(StatusCode::BAD_REQUEST, "Invalid filename", "INVALID_FILENAME"));
    }
    let sniffed = sniff(bytes);
    let mime = mime.filter(|m| !m.is_empty()).or(sniffed.map(|s| s.0)).unwrap_or("application/octet-stream");
    let ext = std::path::Path::new(name).extension().is_none().then(|| sniffed.map(|s| s.1).or(extension_of(mime)));
    let name = match ext.flatten() {
        Some(ext) => format!("{name}.{ext}"),
        None => name.into(),
    };
    let asset = st.store_uploaded_asset(name, mime.into(), bytes);
    let mut out = json!({
        "uploadID": asset.upload_id,
        "srcURL": path_to_file_url(&st.config.assets_dir.join(&asset.disk_name)),
        "fileName": asset.file_name,
        "mimeType": asset.mime_type,
        "fileSize": asset.file_size,
    });
    if let Some((w, h)) = asset.width.zip(asset.height) {
        (out["width"], out["height"]) = (w.into(), h.into());
    }
    ok(&out)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Desktop's JSON form of `POST /v1/assets/upload`: `{content, fileName, mimeType}` with base64 content.
fn upload_json(st: &mut BeeperStore, r: &Req) -> Reply {
    let field = |k: &str, message: &str, code: &str| {
        r.str(k).filter(|v| !v.is_empty()).ok_or_else(|| error(StatusCode::BAD_REQUEST, message, code))
    };
    let content = field("content", "Missing required field: content (base64 string)", "MISSING_CONTENT")?;
    let file_name = field("fileName", "Missing required field: fileName", "MISSING_FILENAME")?;
    let mime = field("mimeType", "Missing required field: mimeType", "MISSING_MIMETYPE")?;
    let bytes = base64_decode(content).filter(|b| !b.is_empty());
    let bytes = bytes.ok_or_else(|| error(StatusCode::BAD_REQUEST, "Invalid base64 content", "INVALID_BASE64"))?;
    upload(st, Some(file_name), Some(mime), &bytes)
}

/// Desktop's `GET /focus{/:chatID{/:messageID}}?draft=`: `POST /v1/focus` for other programs, with an HTML answer.
fn focus_page(st: &mut BeeperStore, path: &[&str], r: &Req) -> Reply {
    auth::reject_browser(&r.msg)?;
    let (chat, draft) = match path {
        [] => (None, r.q("draft")),
        [chat] | [chat, _] => (Some(*chat), r.q("draft")),
        _ => return Err(no_route()),
    };
    if draft.is_some() && chat.is_none() {
        return auth::html(400, "Bad Request", "<h1>Draft text requires a chat ID</h1>", Vec::new());
    }
    let idx = chat.map(|c| st.resolve_chat_idx(c));
    if idx == Some(None) {
        return auth::html(404, "Not Found", "<h1>Chat not found</h1>", Vec::new());
    }
    let idx = idx.flatten();
    if let (Some(i), Some(text)) = (idx, draft) {
        let ts = Some(beeper_core::config::now_unix_millis());
        let draft = ChatDraft { text: Some(text.into()), ts, ..ChatDraft::default() };
        st.put_draft(i, Some(draft));
    }
    st.focus_tx.send(idx.map(|i| st.chats[i].id.clone())).ok();
    let body = r#"<div class="message">You can close this window</div><div class="hint">Having issues? Contact help@beeper.com.</div>"#;
    auth::html(200, "Beeper Client API", body, Vec::new())
}

/// Desktop's `GET /deeplink{/*path}` (`handleDeeplink`): opens `beeper://<path>` in the app, like a click on such a
/// link, with the HTML answer of `/focus`.
fn deeplink_page(st: &BeeperStore, path: &[&str], r: &Req) -> Reply {
    auth::reject_browser(&r.msg)?;
    let path = path.join("/");
    if path.is_empty() {
        return Err(error(StatusCode::BAD_REQUEST, "Missing deeplink parameter", "missing_parameter"));
    }
    // `decodeURIComponent`, which keeps a "+".
    let link = format!("beeper://{path}");
    let link = beeper_core::codec::url_decode(&link.replace('+', "%2B")).unwrap_or(link);
    st.launch_tx.send(vec![link]).ok();
    let body = r#"<div class="message">You can close this window</div><div class="hint">Having issues? Contact help@beeper.com.</div>"#;
    auth::html(200, "Beeper Client API", body, Vec::new())
}

fn upload_multipart(st: &mut BeeperStore, r: &Req) -> Reply {
    let boundary = r.msg.header("content-type").and_then(|ct| ct.split_once("boundary="));
    let delim = format!("\r\n--{}", boundary.ok_or_else(|| bad("multipart boundary is required"))?.1.trim_matches('"'));
    let body = [b"\r\n", r.msg.body.as_slice()].concat();
    let mut rest = body.as_slice();
    let mut file = None;
    while let Some(start) = find(rest, delim.as_bytes()) {
        rest = &rest[start + delim.len()..];
        let part = &rest[..find(rest, delim.as_bytes()).unwrap_or(rest.len())];
        let Some(head_end) = find(part, b"\r\n\r\n") else { continue };
        let head = std::str::from_utf8(&part[..head_end]).map_err(|_| bad("multipart headers must be UTF-8"))?;
        let content = &part[head_end + 4..];
        let attr = |key: &str| head.split(&format!("{key}=\"")).nth(1).and_then(|s| s.split('"').next());
        let content_type = head.lines().find_map(|l| {
            l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-type")).map(|(_, v)| v.trim())
        });
        // multer takes the part with a file name as the file. The other parts are fields that Desktop ignores, and
        // busboy gives a part without a type the RFC 7578 default type.
        if let (Some("file"), Some(name)) = (attr("name"), attr("filename")) {
            file = Some((name, content_type.unwrap_or("text/plain"), content));
        }
    }
    let missing = || error(StatusCode::BAD_REQUEST, "No file uploaded. Use field name \"file\"", "MISSING_FILE");
    let (name, mime, content) = file.ok_or_else(missing)?;
    if content.is_empty() {
        return Err(bad("file part is empty"));
    }
    upload(st, Some(name), Some(mime), content)
}

/// The first satisfiable range of a `Range` header, like the `range-parser` package.
fn parse_range(header: &str, total: usize) -> Option<(usize, usize)> {
    let last = total - 1;
    let one = |spec: &str| {
        let (s, e) = spec.trim().split_once('-')?;
        let (start, end) = match (s, e) {
            ("", n) => (total.saturating_sub(n.parse().ok()?), last),
            (s, "") => (s.parse().ok()?, last),
            (s, e) => (s.parse().ok()?, e.parse::<usize>().ok()?.min(last)),
        };
        (start <= end).then_some((start, end))
    };
    header.strip_prefix("bytes=")?.split(',').find_map(one)
}

/// Desktop's failures of `resolveMatrixAsset`.
fn asset_failure(url: &str) -> (StatusCode, &'static str) {
    match url {
        u if u.starts_with("mxc://") || u.starts_with("localmxc://") => {
            (StatusCode::BAD_GATEWAY, "Failed to download asset: Not found")
        }
        _ => (StatusCode::BAD_REQUEST, "URL must be mxc:// or localmxc://"),
    }
}

/// The folders of the data folder from which Desktop serves `file://` URLs.
const ATTACHMENT_DIRS: &[&str] = &[
    "media",
    "api-uploads",
    "report_a_problem_attachments",
    "draft_attachments",
    "app_icons",
    "user_avatar",
    "quick_reply_attachments",
    "temp_avatar_uploads",
    "transcoded_cache",
    "transcribed_notes",
    "linkpreviews",
];

/// Desktop's `serveAsset` check of a `file://` URL: the real path is in an attachment folder of the data folder.
fn attachment_file(st: &BeeperStore, url: &str) -> Result<std::path::PathBuf, Response> {
    let fail = |status, message| error(status, message, "SERVE_ASSET_ERROR");
    let path = url::Url::parse(url).ok().and_then(|u| u.to_file_path().ok());
    let path = path.ok_or_else(|| fail(StatusCode::BAD_REQUEST, "Malformed file URL"))?;
    let (Ok(real), Ok(data)) = (path.canonicalize(), st.config.data_dir.canonicalize()) else {
        return Err(fail(StatusCode::NOT_FOUND, "File not found"));
    };
    match real.strip_prefix(&data).ok().and_then(|rel| rel.components().next()) {
        Some(std::path::Component::Normal(dir)) if ATTACHMENT_DIRS.iter().any(|d| dir == *d) => Ok(real),
        _ => Err(fail(StatusCode(403), "Access denied: Path must be within allowed attachment directories")),
    }
}

fn asset_file(st: &BeeperStore, r: &Req) -> Result<std::path::PathBuf, Response> {
    let missing_url = || error(StatusCode::BAD_REQUEST, "Missing required query parameter: url", "MISSING_URL");
    let url = r.q("url").filter(|u| !u.is_empty()).ok_or_else(missing_url)?;
    match url.starts_with("file://") {
        true => attachment_file(st, url),
        false => st.resolve_asset_path(url).map(|(path, _)| path).ok_or_else(|| {
            let (status, message) = asset_failure(url);
            error(status, message, "SERVE_ASSET_ERROR")
        }),
    }
}

/// Desktop's `serveAsset` answer: the cache headers, 304 for the `ETag` of the file, the type of the file name, and
/// the first range of a `Range` header. It reads the file after the caller releases the store lock.
fn serve_asset(path: std::path::PathBuf, r: &Req) -> Reply {
    let read = |e: std::io::Error| {
        // Windows reports "Access is denied" for a directory.
        let message = match path.is_dir() {
            true => "EISDIR: illegal operation on a directory, read".into(),
            false => e.to_string(),
        };
        error(StatusCode(500), &message, "UNKNOWN_ERROR")
    };
    let bytes = std::fs::read(&path).map_err(read)?;
    let modified = std::fs::metadata(&path).and_then(|m| m.modified()).map_err(read)?;
    let since = modified.duration_since(std::time::UNIX_EPOCH).expect("a file time after 1970");
    // Node's `mtimeMs`.
    let etag = format!("\"{}-{}\"", bytes.len(), since.as_nanos() as f64 / 1e6);
    let fresh = r.msg.header("if-none-match") == Some(etag.as_str());
    let mut headers = vec![
        ("ETag", etag),
        ("Last-Modified", http_date(since.as_secs() as i64)),
        ("Cache-Control", "private, max-age=3600".into()),
    ];
    if fresh {
        return Ok(Response { status: StatusCode::NOT_MODIFIED, headers, body: Vec::new() });
    }
    let name = path.file_name().expect("a file name").to_string_lossy();
    let text = |m: &str| m.starts_with("text/") || m == "application/json";
    let kind = mime_type_of(&name).map(|m| if text(m) { format!("{m}; charset=utf-8") } else { m.into() });
    headers.extend(kind.map(|k| ("Content-Type", k)));
    headers.push(("Accept-Ranges", "bytes".into()));
    let total = bytes.len();
    let Some(range) = r.msg.header("range").filter(|_| total > 0) else {
        return Ok(Response { status: StatusCode::OK, headers, body: bytes });
    };
    if let Some((start, end)) = parse_range(range, total) {
        headers.push(("Content-Range", format!("bytes {start}-{end}/{total}")));
        Ok(Response { status: StatusCode::PARTIAL_CONTENT, headers, body: bytes[start..=end].to_vec() })
    } else {
        headers.push(("Content-Range", format!("bytes */{total}")));
        Ok(Response { status: StatusCode::RANGE_NOT_SATISFIABLE, headers, body: Vec::new() })
    }
}

fn focus(st: &mut BeeperStore, r: &Req) -> Reply {
    // Desktop's `FocusAppInput`: optional strings.
    let fields = ["chatID", "messageID", "draftText", "draftAttachmentPath"];
    checked(fields.into_iter().filter_map(|k| string_issue(r.input(k), true, json!([k]))).collect())?;
    let text = |k: &str| r.input(k).and_then(Value::as_str);
    // Desktop's `focusApp` uses the message and the draft only with a chat, and fails on a draft text without one.
    let idx = text("chatID").map(|id| st.resolve_chat_idx(id).ok_or_else(|| no_chat(id))).transpose()?;
    let (text, path) = (text("draftText").map(str::to_string), text("draftAttachmentPath").map(str::to_string));
    if idx.is_none() && text.is_some() {
        let message = "Failed to execute tool: focusApp";
        return Err(error_with(StatusCode(500), message, "TOOL_EXECUTION_ERROR", json!({})));
    }
    if let Some(idx) = idx.filter(|_| text.is_some() || path.is_some()) {
        let attachments =
            path.into_iter().map(|p| ChatDraftAttachment { upload_id: p.clone(), file_name: Some(p), mime_type: None });
        let ts = Some(beeper_core::config::now_unix_millis());
        let draft = ChatDraft { text, attachments: attachments.collect(), ts, ..ChatDraft::default() };
        st.put_draft(idx, Some(draft));
    }
    st.focus_tx.send(idx.map(|i| st.chats[i].id.clone())).ok();
    ok(&json!({ "success": true }))
}

/// Desktop's event frame `{type, seq, ts, chatID, ids, entries?}`. A message event carries the message in `entries`.
#[derive(Serialize)]
struct WsFrame<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
    seq: u64,
    ts: String,
    #[serde(rename = "chatID")]
    chat_id: &'a str,
    ids: Vec<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    entries: Option<Vec<view::ApiMessage<'a>>>,
}

impl<'a> WsFrame<'a> {
    fn new(ev: &'a WsEvent, seq: u64) -> Self {
        let chat_id = ev.chat_id.as_deref().expect("store events have a chat");
        WsFrame {
            kind: &ev.event_type,
            seq,
            ts: now_rfc3339_millis(),
            chat_id,
            ids: vec![ev.message_id.as_deref().unwrap_or(chat_id)],
            entries: ev.message.as_ref().map(|m| vec![view::message(m)]),
        }
    }
}

/// JavaScript's truthiness of a JSON value.
fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

/// The message of the first issue of Desktop's zod schema of `subscriptions.set` (`ql`), a `.strict()` object of
/// `type`, an optional string `requestID`, an optional array of strings `chatIDs` and an optional `.strict()` object
/// `app` with a boolean `state`, and then its refinement on `*`.
fn subscription_issue(o: &serde_json::Map<String, Value>) -> Option<String> {
    let expected = |what: &str, v: &Value| format!("Expected {what}, received {}", zod_type(Some(v)));
    let unknown = |o: &serde_json::Map<String, Value>, known: &[&str]| {
        let extra: Vec<String> = o.keys().filter(|k| !known.contains(&k.as_str())).map(|k| format!("'{k}'")).collect();
        (!extra.is_empty()).then(|| format!("Unrecognized key(s) in object: {}", extra.join(", ")))
    };
    if let Some(r) = o.get("requestID").filter(|r| !r.is_string()) {
        return Some(expected("string", r));
    }
    match o.get("chatIDs") {
        None => {}
        Some(Value::Array(ids)) => {
            if let Some(id) = ids.iter().find(|id| !id.is_string()) {
                return Some(expected("string", id));
            }
        }
        Some(ids) => return Some(expected("array", ids)),
    }
    match o.get("app") {
        None => {}
        Some(Value::Object(app)) => match app.get("state") {
            None => return Some("Required".into()),
            Some(state) if !state.is_boolean() => return Some(expected("boolean", state)),
            _ => {
                if let Some(message) = unknown(app, &["state"]) {
                    return Some(message);
                }
            }
        },
        Some(app) => return Some(expected("object", app)),
    }
    if let Some(message) = unknown(o, &["type", "requestID", "chatIDs", "app"]) {
        return Some(message);
    }
    let ids = o.get("chatIDs").and_then(Value::as_array).map_or(&[][..], Vec::as_slice);
    (ids.len() > 1 && ids.iter().any(|id| id == "*")).then(|| "chatIDs cannot combine '*' with specific IDs".into())
}

/// Desktop's `handleClientMessage`: applies a `subscriptions.set` command to the chats of `subs` and the `app.state`
/// subscription of `app_on`, and returns its `subscriptions.updated` acknowledgement or an `error` frame. The chat IDs
/// are sorted and unique, and `*` stands for all chats.
fn subscribe(subs: &RwLock<Vec<String>>, app_on: &std::sync::atomic::AtomicBool, payload: &[u8]) -> Value {
    // Desktop's `sendError`: `{type, requestID?, code, message}`, with a request ID that JavaScript finds truthy.
    let error = |code: &str, message: &str, request: Option<&Value>| {
        let mut e = json!({ "type": "error" });
        if let Some(r) = request.filter(|r| truthy(r)) {
            e["requestID"] = r.clone();
        }
        (e["code"], e["message"]) = (code.into(), message.into());
        e
    };
    let Ok(v) = serde_json::from_str::<Value>(&String::from_utf8_lossy(payload)) else {
        return error("INVALID_PAYLOAD", "Invalid JSON payload", None);
    };
    let Some(command) = v.as_object().filter(|o| o.contains_key("type")) else {
        return error("INVALID_PAYLOAD", "Payload must be an object with a type field", None);
    };
    if v["type"] != "subscriptions.set" {
        let kind = js_string(Some(&v["type"]));
        return error("INVALID_COMMAND", &format!("Unsupported command type: {kind}"), None);
    }
    if let Some(message) = subscription_issue(command) {
        return error("INVALID_PAYLOAD", &message, v.get("requestID"));
    }
    let ids: Vec<String> = serde_json::from_value(v.get("chatIDs").cloned().unwrap_or(json!([])))
        .expect("the chat IDs passed their schema");
    let app = v["app"]["state"] == true;
    // Desktop's `Sf`: the trimmed IDs that are not empty, in locale order, or `*` alone.
    let mut ids: Vec<String> = ids.iter().map(|id| id.trim()).filter(|id| !id.is_empty()).map(Into::into).collect();
    if ids.iter().any(|id| id == "*") {
        ids = vec!["*".into()]
    } else {
        ids.sort_by(|a, b| beeper_core::models::locale_cmp(a, b));
        ids.dedup();
    }
    let mut ack = json!({ "type": "subscriptions.updated" });
    if let Some(r) = v.get("requestID").filter(|r| truthy(r)) {
        ack["requestID"] = r.clone();
    }
    (ack["chatIDs"], ack["app"]) = (json!(ids), json!({ "state": app }));
    *subs.write().unwrap() = ids;
    app_on.store(app, std::sync::atomic::Ordering::Relaxed);
    ack
}

/// The payload of a close frame.
fn close_payload(code: u16, reason: &str) -> Vec<u8> {
    [&code.to_be_bytes()[..], reason.as_bytes()].concat()
}

/// Desktop's `maxPayload` of the WebSocket server.
const WS_MAX_PAYLOAD: usize = 64 * 1024;

async fn serve_ws(
    mut stream: TcpStream, leftover: Vec<u8>, msg: HttpMessage, store: SharedStore,
    mut events: tokio::sync::broadcast::Receiver<WsEvent>,
) {
    let Some(key) = msg.header("sec-websocket-key") else { return };
    let accept = base64_encode_standard(&sha1(format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11").as_bytes()));
    let handshake = format!(
        "HTTP/1.1 101 Switching Protocols\r\nupgrade: websocket\r\nconnection: Upgrade\r\nsec-websocket-accept: {accept}\r\n\r\n"
    );
    if stream.write_all(handshake.as_bytes()).await.is_err() {
        return;
    }
    let (reader, mut writer) = stream.into_split();
    let subs = Arc::new(RwLock::new(Vec::<String>::new()));
    let app_on = Arc::new(std::sync::atomic::AtomicBool::new(false));
    // The frames to send as `(opcode, payload)`, and `None` for Desktop's `sendAppStateSnapshot` when the
    // `app.state` subscription starts.
    let (out_tx, mut out_rx) = tokio::sync::mpsc::unbounded_channel::<Option<(u8, Vec<u8>)>>();
    let frame = |v: &Value| Some((0x1, serde_json::to_vec(v).expect("serialize a WebSocket frame")));
    out_tx.send(frame(&json!({ "type": "ready", "version": 1, "chatIDs": [] }))).expect("the writer runs");
    let (reader_subs, reader_app) = (subs.clone(), app_on.clone());
    // The reader ends with the payload of the close frame to send, if any.
    let mut reader_task = tokio::spawn(async move {
        let mut reader = std::io::Cursor::new(leftover).chain(reader);
        // Like the `ws` package, a message can come in fragments, and its whole size counts for `maxPayload`.
        let mut message = Vec::new();
        loop {
            let (fin, opcode, payload) = match read_ws_frame(&mut reader, WS_MAX_PAYLOAD - message.len()).await {
                Ok(frame) => frame,
                Err(close) => return (close.map(|code| close_payload(code, "")), reader),
            };
            match opcode {
                // Desktop reads a binary message as UTF-8 text too.
                0x0..=0x2 => {
                    message.extend(payload);
                    if !fin {
                        continue;
                    }
                    let before = reader_app.load(std::sync::atomic::Ordering::Relaxed);
                    let reply = subscribe(&reader_subs, &reader_app, &std::mem::take(&mut message));
                    // Desktop's `applySubscriptionUpdate` sends the app state snapshot before its acknowledgement.
                    if !before && reader_app.load(std::sync::atomic::Ordering::Relaxed) {
                        out_tx.send(None).ok();
                    }
                    out_tx.send(frame(&reply)).ok();
                }
                // The `ws` package answers a ping with its payload, and a close with its code and reason.
                0x9 => drop(out_tx.send(Some((0xA, payload)))),
                0xA => {}
                0x8 => return (Some(payload), reader),
                _ => return (Some(close_payload(1002, "")), reader),
            }
        }
    });
    let token = auth::access_token(&msg).expect("the upgrade has a token").to_string();
    // The payload of the close frame, a code and a reason, when the server ends the connection.
    let send_events = async {
        let (mut seq, mut recent) = (0, HashMap::<String, tokio::time::Instant>::new());
        // Desktop's `lastAppStateFingerprint`, and the verification of the last snapshot.
        let mut last: Option<(String, Option<Value>)> = None;
        let mut keepalive = tokio::time::interval_at(tokio::time::Instant::now() + WS_KEEPALIVE, WS_KEEPALIVE);
        loop {
            let (opcode, bytes) = tokio::select! {
                ev = events.recv() => {
                    // A client that falls behind the events loses them, so Desktop closes it like a client whose
                    // buffer passes 5 MB.
                    let ev = match ev {
                        Ok(ev) => ev,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            return Some(close_payload(1008, "Client is too slow"));
                        }
                        Err(_) => return None,
                    };
                    // Desktop's `processAppStateEvent`: a changed setup state goes to the `app.state` subscribers, with
                    // its verification when the verification changed.
                    if ev.event_type == "setup.updated" {
                        if !app_on.load(std::sync::atomic::Ordering::Relaxed) {
                            continue;
                        }
                        let frames = app_state_frames(&store, &mut last, &mut seq, false);
                        if frames.is_empty() {
                            continue;
                        }
                        for f in &frames[..frames.len() - 1] {
                            if write_ws_frame(&mut writer, 0x1, f, false).await.is_err() {
                                return None;
                            }
                        }
                        (0x1, frames.last().expect("a frame").clone())
                    } else {
                    let chat = ev.chat_id.as_deref().expect("store events have a chat");
                    let wanted = {
                        let s = subs.read().unwrap();
                        s.first().is_some_and(|f| f == "*") || s.iter().any(|c| c == chat)
                    };
                    if !wanted {
                        continue;
                    }
                    // Desktop drops an event that repeats one of the last 250 ms, without the times.
                    let event = serde_json::to_value(WsFrame::new(&ev, 0)).expect("serialize a WebSocket event");
                    let now = tokio::time::Instant::now();
                    recent.retain(|_, at| now - *at < WS_DUPLICATE);
                    if recent.insert(without_times(&event).to_string(), now).is_some() {
                        continue;
                    }
                    seq += 1;
                    (0x1, serde_json::to_vec(&WsFrame::new(&ev, seq)).expect("serialize a WebSocket event"))
                    }
                }
                Some(out) = out_rx.recv() => if let Some(frame) = out { frame } else {
                    let frames = app_state_frames(&store, &mut last, &mut seq, true);
                    (0x1, frames.into_iter().next().expect("a snapshot"))
                },
                // Desktop pings each client every 30 s, and closes the clients whose token it revoked.
                _ = keepalive.tick() => {
                    let (st, token) = (store.clone(), token.clone());
                    let check = tokio::task::spawn_blocking(move || auth::authorize(&st.read(), Some(&token), &[]).err());
                    if let Some((401, reason)) = check.await.expect("the token check panicked") {
                        return Some(close_payload(1008, &reason));
                    }
                    (0x9, Vec::new())
                }
            };
            if write_ws_frame(&mut writer, opcode, &bytes, false).await.is_err() {
                return None;
            }
        }
    };
    let close = tokio::select! {
        r = &mut reader_task => {
            let (close, reader) = r.expect("WS reader task panicked");
            close.map(|close| (close, Some(reader)))
        }
        closed = send_events => closed.map(|close| (close, None)),
    };
    reader_task.abort();
    if let Some((close, reader)) = close {
        write_ws_frame(&mut writer, 0x8, &close, false).await.ok();
        match reader {
            Some(mut reader) => linger(&mut writer, &mut reader).await,
            None => drop(writer.shutdown().await),
        }
    }
}

/// Desktop's `app.state.updated` frame of the setup state when it changed since `last` (always with `snapshot`, its
/// `sendAppStateSnapshot`), then its `verification.updated` or `verification.removed` frame when the verification
/// changed.
fn app_state_frames(
    store: &SharedStore, last: &mut Option<(String, Option<Value>)>, seq: &mut u64, snapshot: bool,
) -> Vec<Vec<u8>> {
    let state = store.read().app_state();
    let state = serde_json::to_value(&state).expect("serialize the app state");
    let fingerprint = state.to_string();
    let verification = state.get("verification").cloned();
    let before = last.replace((fingerprint.clone(), verification.clone()));
    if !snapshot && before.as_ref().is_some_and(|b| b.0 == fingerprint) {
        return Vec::new();
    }
    let ts = now_rfc3339_millis();
    // Desktop's frames start with their type, `seq` and `ts`.
    let mut frame = |kind: &str, body: Option<(&str, Value)>| {
        *seq += 1;
        let mut v = json!({ "type": kind, "seq": *seq, "ts": ts });
        if let Some((key, value)) = body {
            v[key] = value;
        }
        serde_json::to_vec(&v).expect("serialize a WebSocket frame")
    };
    let mut frames = vec![frame("app.state.updated", Some(("appState", state)))];
    let had = before.and_then(|b| b.1);
    match (verification, snapshot) {
        (Some(v), false) if had.as_ref() != Some(&v) => {
            frames.push(frame("verification.updated", Some(("verification", v))));
        }
        (None, false) if had.is_some() => frames.push(frame("verification.removed", None)),
        _ => {}
    }
    frames
}

/// The ping interval of Desktop's WebSocket server.
const WS_KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(30);

/// Desktop's window of repeated WebSocket events.
const WS_DUPLICATE: std::time::Duration = std::time::Duration::from_millis(250);

/// An event without its `seq`, `ts` and `timestamp` keys, for Desktop's duplicate check.
fn without_times(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let kept = o.iter().filter(|(k, _)| !["seq", "ts", "timestamp"].contains(&k.as_str()));
            Value::Object(kept.map(|(k, v)| (k.clone(), without_times(v))).collect())
        }
        Value::Array(a) => Value::Array(a.iter().map(without_times).collect()),
        v => v.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Desktop's `handleClientMessage` answers the first zod issue of `subscriptions.set`, keeps the request ID that
    /// JavaScript finds truthy, and writes its frames in its key order.
    #[test]
    fn subscriptions_are_checked_like_desktop() {
        let (subs, app) = (RwLock::new(Vec::new()), std::sync::atomic::AtomicBool::new(false));
        let reply = |payload: &str| subscribe(&subs, &app, payload.as_bytes()).to_string();
        let error = |request: &str, message: &str| {
            format!(r#"{{"type":"error",{request}"code":"INVALID_PAYLOAD","message":"{message}"}}"#)
        };
        let set = |rest: &str| format!(r#"{{"type":"subscriptions.set",{rest}}}"#);
        let cases = [
            (set(r#""requestID":5"#), error(r#""requestID":5,"#, "Expected string, received number")),
            (set(r#""requestID":"","chatIDs":"x""#), error("", "Expected array, received string")),
            (set(r#""requestID":"r","chatIDs":[1]"#), error(r#""requestID":"r","#, "Expected string, received number")),
            (set(r#""chatIDs":null"#), error("", "Expected array, received null")),
            (set(r#""app":true"#), error("", "Expected object, received boolean")),
            (set(r#""app":{}"#), error("", "Required")),
            (set(r#""app":{"state":"yes"}"#), error("", "Expected boolean, received string")),
            (set(r#""app":{"state":true,"x":1}"#), error("", "Unrecognized key(s) in object: 'x'")),
            (set(r#""a":1,"b":2,"chatIDs":["*","x"]"#), error("", "Unrecognized key(s) in object: 'a', 'b'")),
            (set(r#""chatIDs":["*","x"]"#), error("", "chatIDs cannot combine '*' with specific IDs")),
        ];
        for (payload, expected) in cases {
            assert_eq!(reply(&payload), expected, "{payload}");
        }
        let unsupported = r#"{"type":"error","code":"INVALID_COMMAND","message":"Unsupported command type: 1,2"}"#;
        assert_eq!(reply(r#"{"type":[1,2]}"#), unsupported, "JavaScript's String of the type");
        let ack = reply(&set(r#""requestID":"r","chatIDs":["b","a","b"],"app":{"state":true}"#));
        assert_eq!(ack, r#"{"type":"subscriptions.updated","requestID":"r","chatIDs":["a","b"],"app":{"state":true}}"#);
        assert_eq!(
            (subs.read().unwrap().clone(), app.load(std::sync::atomic::Ordering::Relaxed)),
            (vec!["a".into(), "b".into()], true)
        );
    }
}
