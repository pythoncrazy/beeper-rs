//! Desktop's Matrix client-server proxy (`setupMatrixEndpoints`): account data, room state, events and profiles, and
//! joining, leaving and creating rooms, through the Matrix client of the signed-in account, with Matrix errors.

use super::{json_with, ok, Reply, Response, JSON};
use crate::codec::url_decode;
use crate::http::{HttpMessage, StatusCode};
use beeper_core::matrix;
use beeper_core::store::SharedStore;
use serde_json::{json, Map, Value};

/// Desktop's `MatrixError` codes: the status and the default message of each `errcode`.
const CODES: [(&str, u16, &str); 8] = [
    ("M_BAD_JSON", 400, "Content could not be parsed as JSON"),
    ("M_FORBIDDEN", 403, "Forbidden"),
    ("M_INVALID_PARAM", 400, "Invalid parameter"),
    ("M_NOT_FOUND", 404, "Not found"),
    ("M_NOT_JSON", 400, "Request body is not valid JSON"),
    ("M_UNKNOWN", 500, "Internal server error"),
    ("M_UNKNOWN_TOKEN", 401, "Unrecognized access token"),
    ("M_UNRECOGNIZED", 404, "Unrecognized request"),
];

/// The Matrix error body, `{errcode, error}`.
fn error(status: u16, errcode: &str, message: &str) -> Response {
    json_with(StatusCode(status), &json!({ "errcode": errcode, "error": message }))
}

/// Desktop's `createMatrixError` (`Ie`): the error of a code, with its default message for an empty `message`.
fn matrix_error(errcode: &str, message: &str) -> Response {
    let (_, status, default) = CODES.iter().find(|c| c.0 == errcode).expect("a Matrix error code");
    error(*status, errcode, if message.is_empty() { default } else { message })
}

/// Whether `hay` has `word` between the word boundaries (`\b`) of JavaScript.
fn has_word(hay: &str, word: &str) -> bool {
    let at = |i: usize| hay.as_bytes().get(i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_');
    hay.match_indices(word).any(|(i, _)| (i == 0 || !at(i - 1)) && !at(i + word.len()))
}

/// Desktop's patterns of the codes of SDK errors (`yT`), in its order: the code, whether the pattern ignores case,
/// its whole words, and its other parts.
const PATTERNS: [(&str, bool, &[&str], &[&str]); 8] = [
    ("M_BAD_JSON", true, &["M_BAD_JSON"], &["invalid JSON", "unexpected end of JSON input"]),
    ("M_FORBIDDEN", true, &["M_FORBIDDEN", "forbidden"], &[]),
    ("M_INVALID_PARAM", true, &["M_INVALID_PARAM"], &["invalid_parameter", "missing_parameter"]),
    ("M_NOT_FOUND", true, &["M_NOT_FOUND", "HTTP 404", "not found"], &[]),
    ("M_NOT_JSON", false, &["M_NOT_JSON"], &[]),
    ("M_UNKNOWN_TOKEN", true, &["M_UNKNOWN_TOKEN", "unauthorized"], &[]),
    ("M_UNRECOGNIZED", true, &["M_UNRECOGNIZED"], &["method_not_allowed"]),
    ("M_UNKNOWN", true, &["M_UNKNOWN"], &[]),
];

/// Desktop's `toMatrixError` (`oa`) of an SDK error: the code of the first pattern that its message matches, or
/// `M_UNKNOWN`, with the message.
fn sdk_error(message: String) -> Response {
    let matches = |(_, ignore_case, words, parts): &&(&str, bool, &[&str], &[&str])| {
        let fold = |s: &str| if *ignore_case { s.to_ascii_lowercase() } else { s.to_string() };
        let hay = fold(&message);
        words.iter().any(|w| has_word(&hay, &fold(w))) || parts.iter().any(|p| hay.contains(&fold(p)))
    };
    matrix_error(PATTERNS.iter().find(matches).map_or("M_UNKNOWN", |p| p.0), &message)
}

/// Desktop's `toMatrixError` of a refusal of `requireAuth`: 401 is `M_UNKNOWN_TOKEN` and 403 is `M_FORBIDDEN`.
pub(super) fn refusal((status, message): (u16, String)) -> Response {
    matrix_error(if status == 401 { "M_UNKNOWN_TOKEN" } else { "M_FORBIDDEN" }, &message)
}

/// Desktop's answer to a request before `sdkStarted`, when the Matrix client has not started.
pub(super) fn not_ready() -> Response {
    error(503, "M_UNKNOWN", "Matrix SDK is not ready")
}

/// Runs a call of the Matrix client, with Desktop's Matrix error of its failure.
fn run<T>(call: impl std::future::Future<Output = Result<T, String>>) -> Result<T, Response> {
    tokio::runtime::Handle::current().block_on(call).map_err(sdk_error)
}

/// Desktop's `zl`: the text of the body, `{}` for none, which must be JSON. Like the `raw` parser of Express, it
/// reads the body only of a request with a `Content-Type`.
fn body(msg: &HttpMessage) -> Result<(String, Value), Response> {
    let text = match msg.header("content-type") {
        Some(_) if !msg.body.is_empty() => String::from_utf8_lossy(&msg.body).into_owned(),
        _ => return Ok(("{}".into(), json!({}))),
    };
    let parsed = serde_json::from_str(&text).map_err(|_| matrix_error("M_NOT_JSON", ""))?;
    Ok((text, parsed))
}

/// Desktop's `Fe` of the JSON text of an SDK result, which it sends as it is, or `M_NOT_FOUND` for none.
fn found_text(text: String) -> Reply {
    if text.is_empty() {
        return Err(matrix_error("M_NOT_FOUND", ""));
    }
    Ok(Response { status: StatusCode::OK, headers: vec![("content-type", JSON.into())], body: text.into_bytes() })
}

/// Desktop's `Fe` of a JSON value: a string goes as it is, like an SDK result.
fn found(v: Value) -> Reply {
    match v {
        Value::Null => Err(matrix_error("M_NOT_FOUND", "")),
        Value::String(s) => found_text(s),
        v => ok(&v),
    }
}

/// Desktop's `Ts`: a string, or the first item of an array of strings.
fn first_str(v: Option<&Value>) -> &str {
    match v {
        Some(Value::String(s)) => s,
        Some(Value::Array(a)) => a.first().and_then(Value::as_str).unwrap_or_default(),
        _ => "",
    }
}

/// Desktop's `Of`: the event type and the state key of the segments after `/state`, which it decodes again.
fn state_path(rest: &[&str]) -> Result<Option<(String, String)>, Response> {
    let Some((kind, key)) = rest.split_first() else { return Ok(None) };
    let decode = |s: &&str| url_decode(&s.replace('+', "%2B")).ok_or_else(|| sdk_error("URI malformed".into()));
    let key: Vec<String> = key.iter().map(decode).collect::<Result<_, _>>()?;
    Ok(Some((decode(kind)?, key.join("/"))))
}

/// The routes of the proxy, after `requireAuth`. The router decoded the parameters, which are never empty, so the
/// `M_UNRECOGNIZED` of Desktop's `ge` for a missing parameter does not happen.
pub(super) fn handle(store: &SharedStore, method: &str, segs: &[&str], query: &Map<String, Value>, msg: &HttpMessage) -> Reply {
    let (c, user) = {
        let st = store.read();
        (st.matrix.clone().ok_or_else(not_ready)?, st.beeper_state.user_id.clone())
    };
    // Desktop's `li`: the user of a path is the signed-in user.
    let mine = |id: &str| match id == "me" || id == user {
        true => Ok(()),
        false => Err(error(403, "M_FORBIDDEN", "You can't access data for other users")),
    };
    let s = |v: &str| v.to_string();
    let ["_matrix", "client", "v3", path @ ..] = segs else { unreachable!("a Matrix proxy route") };
    match (method, path) {
        ("GET", ["user", id, "account_data", kind]) => {
            mine(id)?;
            found_text(run(matrix::get_account_data(c, s(kind)))?)
        }
        ("PUT", ["user", id, "account_data", kind]) => {
            mine(id)?;
            run(matrix::set_account_data(c, s(kind), body(msg)?.0))?;
            ok(&json!({}))
        }
        ("GET", ["user", id, "rooms", room, "account_data", kind]) => {
            mine(id)?;
            found_text(run(matrix::get_room_account_data(c, s(room), s(kind)))?)
        }
        ("PUT", ["user", id, "rooms", room, "account_data", kind]) => {
            mine(id)?;
            run(matrix::set_room_account_data(c, s(room), s(kind), body(msg)?.0))?;
            ok(&json!({}))
        }
        ("GET", ["rooms", room, "account_data", kind]) => found_text(run(matrix::get_room_account_data(c, s(room), s(kind)))?),
        ("PUT", ["rooms", room, "account_data", kind]) => {
            run(matrix::set_room_account_data(c, s(room), s(kind), body(msg)?.0))?;
            ok(&json!({}))
        }
        ("GET", ["rooms", room, "state", rest @ ..]) => {
            let Some((kind, key)) = state_path(rest)? else {
                return found_text(run(matrix::fetch_room_state(c, s(room)))?);
            };
            let event = run(matrix::fetch_room_state_event(c, s(room), kind, key))?;
            if query.get("format").and_then(Value::as_str) == Some("event") {
                return found_text(event);
            }
            let event: Value =
                serde_json::from_str(&event).map_err(|_| matrix_error("M_BAD_JSON", "Room state event response is not valid JSON"))?;
            match event {
                Value::Object(mut e) => found(e.remove("content").unwrap_or_default()),
                _ => Err(matrix_error("M_BAD_JSON", "Room state event response is not a JSON object")),
            }
        }
        ("GET", ["rooms", room, "event", event]) => found_text(run(matrix::fetch_event(c, s(room), s(event)))?),
        ("GET", ["profile", id]) => found_text(run(matrix::get_profile(c, s(id)))?),
        ("POST", ["join", room]) => {
            let Value::Object(b) = body(msg)?.1 else { return Err(matrix_error("M_BAD_JSON", "")) };
            let via = Some(first_str(query.get("server_name"))).filter(|v| !v.is_empty());
            let via = via.unwrap_or_else(|| first_str(b.get("server_name")));
            ok(&json!({ "room_id": run(matrix::join_room(c, s(room), s(via)))? }))
        }
        ("POST", ["rooms", room, "leave"]) => {
            run(matrix::leave_room(c, s(room)))?;
            ok(&json!({}))
        }
        ("POST", ["createRoom"]) => ok(&json!({ "room_id": run(matrix::create_room(c, body(msg)?.0))? })),
        _ => unreachable!("the route table has no {method} {}", segs.join("/")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body_of(r: Response) -> (u16, Value) {
        (r.status.0, serde_json::from_slice(&r.body).unwrap())
    }

    #[test]
    fn sdk_errors_get_the_codes_of_desktop() {
        for (message, status, code) in [
            ("M_NOT_FOUND (HTTP 404): Event not found", 404, "M_NOT_FOUND"),
            ("HTTP 404 Not Found", 404, "M_NOT_FOUND"),
            ("M_FORBIDDEN (HTTP 403): You are not in the room", 403, "M_FORBIDDEN"),
            ("M_UNKNOWN_TOKEN (HTTP 401): Invalid token", 401, "M_UNKNOWN_TOKEN"),
            ("M_LIMIT_EXCEEDED (HTTP 429): Too many requests", 500, "M_UNKNOWN"),
            ("M_UNKNOWN (HTTP 500): x", 500, "M_UNKNOWN"),
            ("Unexpected end of JSON input", 400, "M_BAD_JSON"),
            ("m_not_json", 500, "M_UNKNOWN"),
            ("Room !r:x not found", 404, "M_NOT_FOUND"),
            ("notfound", 500, "M_UNKNOWN"),
        ] {
            let error = json!({ "errcode": code, "error": message });
            assert_eq!(body_of(sdk_error(message.into())), (status, error), "{message}");
        }
        assert_eq!(body_of(sdk_error(String::new())).1["error"], "Internal server error", "the default message");
        let refused = refusal((403, "Insufficient permissions".into()));
        assert_eq!(body_of(refused), (403, json!({ "errcode": "M_FORBIDDEN", "error": "Insufficient permissions" })));
    }

    #[test]
    fn state_paths_decode_like_desktop() {
        assert_eq!(state_path(&[]).ok(), Some(None));
        assert_eq!(state_path(&["m.room.create"]).ok(), Some(Some(("m.room.create".into(), String::new()))));
        let member = Some(Some(("m.room.member".into(), "@me:x/a+b".into())));
        assert_eq!(state_path(&["m.room.member", "%40me:x", "a+b"]).ok(), member, "decoded again and joined");
        assert_eq!(body_of(state_path(&["m.room.member", "%"]).unwrap_err()).0, 500, "URI malformed");
    }
}
