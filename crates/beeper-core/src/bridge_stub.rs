//! Test stubs: an HTTP server that answers each request with a function, and a homeserver with Beeper cloud
//! bridges behind it. The bridges replay the login steps of the mautrix bridgev2 bridges (WhatsApp, Telegram,
//! Signal, Discord, X, Google Messages and LinkedIn), with their errors, so that no test contacts a real bridge.
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

/// An HTTP server on a free port that answers each request with `reply(method, path, body)`. A body that is not
/// JSON is a JSON string, and an empty body is null.
pub async fn serve(reply: impl Fn(&str, &str, Value) -> (u16, Value) + Send + Sync + 'static) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind the stub server");
    let url = format!("http://{}", listener.local_addr().expect("the stub address"));
    let reply = Arc::new(reply);
    tokio::spawn(async move {
        loop {
            let (stream, _) = listener.accept().await.expect("accept a stub connection");
            let reply = reply.clone();
            tokio::spawn(async move {
                let (mut stream, mut line, mut len) = (BufReader::new(stream), String::new(), 0);
                stream.read_line(&mut line).await.expect("read the request line");
                let mut header = String::new();
                while stream.read_line(&mut header).await.expect("read a header") > 2 {
                    if let Some(v) = header.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().expect("a content length");
                    }
                    header.clear();
                }
                let mut body = vec![0; len];
                stream.read_exact(&mut body).await.expect("read the request body");
                let mut parts = line.split(' ');
                let (method, path) = (parts.next().expect("a method"), parts.next().expect("a path"));
                let body = match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(_) if body.is_empty() => Value::Null,
                    Err(_) => Value::String(String::from_utf8_lossy(&body).into()),
                };
                let (code, json) = reply(method, path, body);
                let json = json.to_string();
                let head = format!(
                    "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    json.len()
                );
                stream.get_mut().write_all(format!("{head}{json}").as_bytes()).await.expect("write the reply");
            });
        }
    });
    url
}

/// The value that the stub bridges reject as a wrong code.
pub const WRONG_CODE: &str = "00000";
/// The bridge that the homeserver does not know until the Beeper API starts it.
pub const STOPPED_BRIDGE: &str = "linkedin";

fn qr(step: &str, instructions: &str, data: &str) -> Value {
    json!({ "type": "display_and_wait", "step_id": step, "instructions": instructions,
            "display_and_wait": { "type": "qr", "data": data } })
}

fn form(step: &str, instructions: &str, fields: Value, can_cancel: bool) -> Value {
    json!({ "type": "user_input", "step_id": step, "instructions": instructions,
            "user_input": { "fields": fields, "can_cancel": can_cancel } })
}

fn cookies(step: &str, url: &str, names: &[&str]) -> Value {
    let fields: Vec<_> = names
        .iter()
        .map(|n| json!({ "id": n, "required": true, "sources": [{ "type": "cookie", "name": n }] }))
        .collect();
    json!({ "type": "cookies", "step_id": step,
            "cookies": { "url": url, "user_agent": "Mozilla/5.0", "fields": fields } })
}

fn done(step: &str, login: &str) -> Value {
    json!({ "type": "complete", "step_id": step, "instructions": format!("Successfully logged in as {login}"),
            "complete": { "user_login_id": login } })
}

/// The flows of a bridge and the steps of each flow, as the bridgev2 bridges send them.
fn bridge(id: &str) -> Option<Vec<(Value, Vec<Value>)>> {
    let flow = |id: &str, name: &str, description: &str| json!({ "id": id, "name": name, "description": description });
    let phone = |id: &str, description: &str| json!([{ "type": "phone_number", "id": id, "name": "Phone number", "description": description }]);
    Some(match id {
        "whatsapp" => vec![
            (
                flow("qr", "QR", "Scan a QR code to pair the bridge to your WhatsApp account"),
                vec![
                    qr(
                        "fi.mau.whatsapp.login.qr",
                        "Scan the QR code with the WhatsApp mobile app to log in",
                        "2@first",
                    ),
                    qr(
                        "fi.mau.whatsapp.login.qr",
                        "Scan the QR code with the WhatsApp mobile app to log in",
                        "2@second",
                    ),
                    done("fi.mau.whatsapp.login.complete", "15550100"),
                ],
            ),
            (
                flow("phone", "Pairing code", "Input your phone number to get a pairing code"),
                vec![
                    form(
                        "fi.mau.whatsapp.login.phone",
                        "Please enter your phone number",
                        phone("phone_number", "Your WhatsApp phone number in international format"),
                        false,
                    ),
                    json!({ "type": "display_and_wait", "step_id": "fi.mau.whatsapp.login.code",
                        "instructions": "Input the pairing code in the WhatsApp mobile app to log in",
                        "display_and_wait": { "type": "code", "data": "ABCD-EFGH", "can_cancel": true } }),
                    done("fi.mau.whatsapp.login.complete", "15550100"),
                ],
            ),
        ],
        "telegram" => vec![
            (
                flow("phone", "Phone Number", "Login using your Telegram phone number"),
                vec![
                    form(
                        "fi.mau.telegram.login.phone_number",
                        "Please enter your phone number",
                        phone("fi.mau.telegram.login.phone_number", "Include the country code with +"),
                        false,
                    ),
                    form(
                        "fi.mau.telegram.login.code",
                        "Please enter the code sent to your phone",
                        json!([
                    { "type": "2fa_code", "id": "fi.mau.telegram.login.code", "name": "Code",
                      "description": "The code was sent to the Telegram app on your phone" }]),
                        true,
                    ),
                    form(
                        "fi.mau.telegram.login.password",
                        "You have two-factor authentication enabled.",
                        json!([
                    { "type": "password", "id": "fi.mau.telegram.login.password", "name": "Password" }]),
                        true,
                    ),
                    form(
                        "fi.mau.telegram.login.password.incorrect",
                        "Incorrect password, please try again.",
                        json!([
                    { "type": "password", "id": "fi.mau.telegram.login.password", "name": "Password" }]),
                        true,
                    ),
                    done("fi.mau.telegram.login.complete", "12345"),
                ],
            ),
            (
                flow("qr", "QR Code", "Login by scanning a QR code from your phone"),
                vec![
                    qr(
                        "fi.mau.telegram.login.show_qr",
                        "Scan the QR code on your phone to log in",
                        "tg://login?token=a",
                    ),
                    done("fi.mau.telegram.login.complete", "12345"),
                ],
            ),
        ],
        "signal" => vec![(
            flow("qr", "QR", "Scan a QR code to pair the bridge to your Signal app"),
            vec![
                qr(
                    "fi.mau.signal.login.qr",
                    "Scan the QR code on your Signal app to log in",
                    "sgnl://linkdevice?uuid=a",
                ),
                done("fi.mau.signal.login.complete", "5d2c0b9e"),
            ],
        )],
        "discordgo" => vec![
            (
                flow("token", "Browser", "Log in to your Discord account in a web browser."),
                vec![
                    cookies("fi.mau.discord.cookies", "https://discord.com/login", &["token"]),
                    done("fi.mau.discord.complete", "1234567890"),
                ],
            ),
            (
                flow("qr", "QR Code", "Scan a QR code with the Discord mobile app to log in."),
                vec![
                    qr(
                        "fi.mau.discord.qr",
                        "On your phone, find “Scan QR Code” in Discord’s settings.",
                        "https://discord.com/ra/a",
                    ),
                    done("fi.mau.discord.complete", "1234567890"),
                ],
            ),
        ],
        "twitter" => vec![(
            flow("cookies", "Cookies", "Log in with your X account using your cookies"),
            vec![
                cookies("fi.mau.twitter.login.enter_cookies", "https://x.com/i/flow/login", &["ct0", "auth_token"]),
                done("fi.mau.twitter.login.complete", "44196397"),
            ],
        )],
        "gmessages" => vec![(
            flow(
                "google",
                "Google Account",
                "Log in with your Google account and pair by tapping an emoji on your phone",
            ),
            vec![
                json!({ "type": "display_and_wait", "step_id": "fi.mau.gmessages.emoji",
                    "instructions": "Tap the emoji on the Google Messages app on your phone",
                    "display_and_wait": { "type": "emoji", "data": "🐈",
                                          "image_url": "https://fonts.gstatic.com/s/e/notoemoji/latest/1f408/512.png" } }),
                done("fi.mau.gmessages.complete", "gm-a"),
            ],
        )],
        STOPPED_BRIDGE => vec![(
            flow("cookies", "Cookies", "Log in with your LinkedIn account using your cookies"),
            vec![
                cookies(
                    "fi.mau.linkedin.login.enter_cookies",
                    "https://www.linkedin.com/login",
                    &["li_at", "JSESSIONID"],
                ),
                done("fi.mau.linkedin.complete", "ACoAA"),
            ],
        )],
        _ => return None,
    })
}

#[derive(Default)]
struct State {
    started: HashSet<String>,
    /// The login processes: the bridge, the flow and the index of the current step.
    processes: HashMap<String, (String, usize, usize)>,
    /// The finished logins of each bridge, for `whoami`.
    logins: Vec<(String, String)>,
    /// Each request as `METHOD path`, for the tests to check.
    log: Vec<String>,
    /// The body of each request that has one.
    bodies: Vec<Value>,
}

/// The cloud bridges that the stub homeserver has, and the requests that it got.
#[derive(Clone)]
pub struct Bridges(Arc<Mutex<State>>);

impl Bridges {
    pub fn requests(&self) -> Vec<String> {
        self.0.lock().unwrap().log.clone()
    }

    /// The bodies of the requests, in order, without the requests that had none.
    pub fn bodies(&self) -> Vec<Value> {
        self.0.lock().unwrap().bodies.clone()
    }
}

fn err(code: u16, errcode: &str, error: &str) -> (u16, Value) {
    (code, json!({ "errcode": errcode, "error": error }))
}

fn reply(st: &mut State, method: &str, path: &str, body: &Value) -> (u16, Value) {
    let path = path.split_once('?').map_or(path, |p| p.0);
    if path == "/whoami" {
        let mut bridges = json!({});
        for (b, login) in &st.logins {
            bridges[b]["bridgeState"] = json!({ "bridgeType": b, "stateEvent": "RUNNING" });
            bridges[b]["remoteState"][login] = json!({ "state_event": "CONNECTED", "remote_name": login });
        }
        return (200, json!({ "userInfo": { "username": "me" }, "user": { "bridges": bridges } }));
    }
    // Desktop's app API: Google Messages logins are off, and Signal logins ask to continue.
    if path == "/platform-status" {
        let gmessages = json!({ "message": "Google Messages logins are paused.", "disabled": true });
        let signal = json!({ "message": "Signal logins may be slow.", "confirm": true });
        return (200, json!({ "bridge-gmessages": gmessages, "bridge-signal": signal }));
    }
    if let Some(b) = path.strip_prefix("/bridge/").and_then(|p| p.strip_suffix("/start")) {
        st.started.insert(b.into());
        return (200, json!({}));
    }
    if let Some(b) = path.strip_prefix("/bridge/").filter(|_| method == "DELETE") {
        st.logins.retain(|(bridge, _)| bridge != b);
        return (200, json!({}));
    }
    let Some(rest) = path.strip_prefix("/_matrix/client/unstable/com.beeper.bridge/") else {
        return err(404, "M_UNRECOGNIZED", "Unrecognized request");
    };
    let (b, rest) = rest.split_once("/_matrix/provision/v3/").expect("a provisioning request");
    if let Some(login) = rest.strip_prefix("logout/").filter(|_| method == "POST") {
        st.logins.retain(|l| l != &(b.to_string(), login.to_string()));
        return (200, json!({}));
    }
    let Some(flows) = bridge(b).filter(|_| b != STOPPED_BRIDGE || st.started.contains(b)) else {
        return err(404, "M_NOT_FOUND", "Unknown bridge");
    };
    let parts: Vec<&str> = rest.split('/').collect();
    let step = |st: &State, id: &str| {
        let (_, f, i) = &st.processes[id];
        flows[*f].1[*i].clone()
    };
    match (method, parts.as_slice()) {
        ("GET", ["login", "flows"]) => (200, json!({ "flows": flows.iter().map(|f| &f.0).collect::<Vec<_>>() })),
        ("POST", ["login", "start", flow]) => {
            let Some(f) = flows.iter().position(|f| f.0["id"] == *flow) else {
                return err(404, "M_NOT_FOUND", "Invalid login flow ID");
            };
            let id = format!("process{}", st.log.len());
            st.processes.insert(id.clone(), (b.into(), f, 0));
            let mut first = step(st, &id);
            first["login_id"] = id.into();
            (200, first)
        }
        ("POST", ["login", "step", id, step_id, action]) => {
            let Some(current) = st.processes.get(*id).map(|_| step(st, id)) else {
                return err(404, "M_NOT_FOUND", "Login not found");
            };
            if current["step_id"] != *step_id {
                return err(400, "M_BAD_STATE", "Step ID does not match");
            }
            let values: Vec<&str> = body.as_object().into_iter().flatten().filter_map(|(_, v)| v.as_str()).collect();
            match *action {
                "cancel" => {
                    let p = st.processes.get_mut(*id).expect("a known process");
                    p.2 -= 1;
                    return (200, step(st, id));
                }
                kind if kind != current["type"] => return err(400, "M_BAD_STATE", "Step type does not match"),
                // A wrong value goes to the step's `.incorrect` retry when the flow has one, else it is an error.
                "user_input" if values.contains(&WRONG_CODE) => {
                    let retry = format!("{}.incorrect", current["step_id"].as_str().unwrap_or_default());
                    let retry = retry.replace(".incorrect.incorrect", ".incorrect");
                    let p = st.processes.get_mut(*id).expect("a known process");
                    let Some(i) = flows[p.1].1.iter().position(|s| s["step_id"] == retry.as_str()) else {
                        return err(400, "FI.MAU.INVALID_CODE", "Invalid phone code");
                    };
                    p.2 = i;
                    return (200, step(st, id));
                }
                "user_input"
                    if current["user_input"]["fields"][0]["type"] == "phone_number"
                        && values.first().is_some_and(|v| !v.starts_with('+')) =>
                {
                    return err(400, "FI.MAU.BAD_PHONE", "Phone number must be in international format");
                }
                "cookies" if values.len() != current["cookies"]["fields"].as_array().expect("cookie fields").len() => {
                    return err(400, "FI.MAU.MISSING_COOKIES", "Missing cookies");
                }
                _ => {}
            }
            let p = st.processes.get_mut(*id).expect("a known process");
            p.2 += 1;
            // The retry steps only follow a wrong value.
            while flows[p.1].1[p.2]["step_id"].as_str().is_some_and(|s| s.ends_with(".incorrect")) {
                p.2 += 1;
            }
            let next = step(st, id);
            if next["type"] == "complete" {
                st.logins.push((b.into(), next["complete"]["user_login_id"].as_str().expect("a login ID").into()));
            }
            (200, next)
        }
        ("POST", ["login", "cancel", id]) => match st.processes.remove(*id) {
            Some(_) => (200, json!({})),
            None => err(404, "M_NOT_FOUND", "Login not found"),
        },
        _ => err(404, "M_UNRECOGNIZED", "Unrecognized request"),
    }
}

/// A homeserver, Beeper API and Desktop app API on one free port, with the stub bridges behind the homeserver.
pub async fn homeserver() -> (String, Bridges) {
    crate::cloud::install_tls();
    let bridges = Bridges(Arc::default());
    let state = bridges.clone();
    let url = serve(move |method, path, body| {
        let mut st = state.0.lock().unwrap();
        st.log.push(format!("{method} {}", path.split_once('?').map_or(path, |p| p.0)));
        if !body.is_null() {
            st.bodies.push(body.clone());
        }
        reply(&mut st, method, path, &body)
    })
    .await;
    (url, bridges)
}
