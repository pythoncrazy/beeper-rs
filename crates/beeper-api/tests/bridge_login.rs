//! Desktop's bridge login routes against the stub Beeper Cloud bridges of `beeper_core::bridge_stub`. No test
//! contacts a real bridge.

use beeper_api::server::serve_http;
use beeper_core::bridge_stub::{homeserver, Bridges, WRONG_CODE};
use beeper_core::config::CentralConfig;
use beeper_core::models::{BridgeAccountInfo, MatrixSession, Profile};
use beeper_core::store::{BeeperStore, StoreLock};
use serde_json::{json, Value};

/// A signed-in server whose homeserver and Beeper API are the stub, and its port.
async fn server(name: &str) -> u16 {
    server_with(name, Vec::new()).await.0
}

/// A signed-in server with cloud accounts, its port, and the stub bridges.
async fn server_with(name: &str, accounts: Vec<BridgeAccountInfo>) -> (u16, Bridges) {
    let (url, bridges) = homeserver().await;
    let dir = std::env::temp_dir().join(format!("beeper-rs-bridge-login-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = CentralConfig::new(dir, "127.0.0.1".into(), port, None);
    (cfg.api_url, cfg.homeserver) = (url.clone(), url.clone());
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let session = MatrixSession { user_id: "@me:beeper.com".into(), access_token: "tok".into(), device_id: "D".into() };
    store.write().sign_in(session, url, &Profile::default(), accounts);
    tokio::spawn(serve_http(listener, store));
    (port, bridges)
}

/// One HTTP/1.1 request with the API token, and the status and JSON body of the reply.
async fn call(port: u16, method: &str, path: String, body: Value) -> (u16, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body = if body.is_null() { String::new() } else { body.to_string() };
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: x\r\nauthorization: Bearer tok\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    stream.write_all(req.as_bytes()).await.unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).await.unwrap();
    let status = out[9..12].parse().unwrap();
    let json = out.split_once("\r\n\r\n").map_or(Value::Null, |(_, b)| serde_json::from_str(b).unwrap_or(Value::Null));
    (status, json)
}

/// Desktop's login session routes go through the Telegram steps of the stub: the flows, a session, a wrong step,
/// a wrong code, and the login.
#[tokio::test(flavor = "multi_thread")]
async fn login_sessions_drive_the_bridge_steps() {
    let port = server("telegram").await;
    let call = |m: &'static str, p: String, b: Value| call(port, m, p, b);
    let base = "/v1/bridges/telegram";
    let (status, flows) = call("GET", format!("{base}/login-flows"), Value::Null).await;
    let first =
        json!({ "id": "phone", "name": "Phone Number", "description": "Login using your Telegram phone number" });
    assert_eq!((status, &flows["items"][0]), (200, &first));
    let (status, s) = call("POST", format!("{base}/login-sessions"), json!({})).await;
    assert_eq!(status, 200, "{s}");
    let sid = s["loginSessionID"].as_str().unwrap().to_string();
    let field = json!({ "id": "fi.mau.telegram.login.phone_number", "type": "phone_number" });
    let step = json!({ "type": "user_input", "stepID": "fi.mau.telegram.login.phone_number",
                       "instructions": "Please enter your phone number", "fields": [field] });
    assert_eq!(
        s,
        json!({ "loginSessionID": sid, "bridgeID": "telegram", "status": "waiting_for_input", "currentStep": step })
    );
    assert_eq!(call("GET", format!("{base}/login-sessions/{sid}"), Value::Null).await.1, s);
    let steps = format!("{base}/login-sessions/{sid}/steps");
    let wrong = call("POST", format!("{steps}/other"), json!({ "type": "user_input" })).await;
    assert_eq!((wrong.0, &wrong.1["code"]), (409, &json!("INVALID_LOGIN_STEP")));
    let phone = json!({ "type": "user_input", "fields": { "fi.mau.telegram.login.phone_number": "+15550100" } });
    let (_, code) = call("POST", format!("{steps}/fi.mau.telegram.login.phone_number"), phone).await;
    assert_eq!(code["currentStep"]["stepID"], "fi.mau.telegram.login.code");
    let bad = json!({ "type": "user_input", "fields": { "fi.mau.telegram.login.code": WRONG_CODE } });
    let (status, err) = call("POST", format!("{steps}/fi.mau.telegram.login.code"), bad).await;
    assert_eq!((status, err), (500, json!({ "message": "Invalid phone code", "code": "UNKNOWN_ERROR" })));
    let good = json!({ "type": "user_input", "fields": { "fi.mau.telegram.login.code": "12345" } });
    let (_, pw) = call("POST", format!("{steps}/fi.mau.telegram.login.code"), good).await;
    let pw_field = json!({ "type": "user_input", "fields": { "fi.mau.telegram.login.password": "pw" } });
    let (_, done) = call("POST", format!("{steps}/fi.mau.telegram.login.password"), pw_field).await;
    assert_eq!(pw["currentStep"]["fields"][0]["type"], "password");
    assert_eq!((&done["status"], &done["currentStep"]["type"]), (&json!("complete"), &json!("complete")));
}

/// A QR session, its cancel, the cookie step shape, and the refusals of Desktop.
#[tokio::test(flavor = "multi_thread")]
async fn sessions_show_qr_and_cookie_steps_and_cancel() {
    let port = server("qr").await;
    let call = |m: &'static str, p: String, b: Value| call(port, m, p, b);
    let (_, s) = call("POST", "/v1/bridges/whatsapp/login-sessions".into(), json!({ "flowID": "qr" })).await;
    let display = json!({ "type": "qr", "data": "2@first" });
    assert_eq!((&s["status"], &s["currentStep"]["display"]), (&json!("waiting_for_display"), &display));
    let sid = s["loginSessionID"].as_str().unwrap();
    let path = format!("/v1/bridges/whatsapp/login-sessions/{sid}");
    let wait = json!({ "type": "display_and_wait" });
    let (_, next) = call("POST", format!("{path}/steps/fi.mau.whatsapp.login.qr"), wait).await;
    assert_eq!(next["currentStep"]["display"]["data"], "2@second");
    let cancelled = json!({ "bridgeID": "whatsapp", "loginSessionID": sid, "status": "cancelled" });
    assert_eq!(call("DELETE", path.clone(), Value::Null).await, (200, cancelled));
    assert_eq!(call("GET", path, Value::Null).await.0, 404);
    let (_, x) = call("POST", "/v1/bridges/twitter/login-sessions".into(), json!({})).await;
    let fields = json!([{ "id": "ct0" }, { "id": "auth_token" }]);
    assert_eq!(
        (&x["status"], &x["currentStep"]["url"]),
        (&json!("waiting_for_cookies"), &json!("https://x.com/i/flow/login"))
    );
    assert_eq!((&x["currentStep"]["fields"], &x["currentStep"]["userAgent"]), (&fields, &json!("Mozilla/5.0")));
    assert_eq!(call("GET", "/v1/bridges/nope/login-flows".into(), Value::Null).await.0, 400);
    let other = call("POST", "/v1/bridges/signal/login-sessions".into(), json!({ "accountID": "missing" })).await;
    assert_eq!(other.0, 404);
}

/// The cloud bridges have Desktop's IDs, such as `slackgo`, and `removeBridgeLogin` logs a login out of the bridge,
/// or deletes the bridge for its last login.
#[tokio::test(flavor = "multi_thread")]
async fn bridges_use_cloud_ids_and_remove_logins() {
    let remotes = json!({ "T1-U1": { "state_event": "CONNECTED" }, "T2-U2": { "state_event": "CONNECTED" } });
    let state = json!({ "bridgeType": "slackgo", "stateEvent": "RUNNING" });
    let whoami = json!({ "slackgo": { "bridgeState": state, "remoteState": remotes } });
    let accounts = beeper_core::cloud::cloud_accounts(&whoami).unwrap();
    let (port, bridges) = server_with("slack", accounts).await;
    let call = |m: &'static str, p: &str, b: Value| call(port, m, p.into(), b);
    let (_, list) = call("GET", "/v1/bridges", Value::Null).await;
    let slack = list["items"].as_array().unwrap().iter().find(|b| b["displayName"] == "Slack").unwrap().clone();
    let shown = (&slack["id"], &slack["type"], &slack["activeAccountCount"]);
    assert_eq!(shown, (&json!("slackgo"), &json!("slackgo"), &json!(2)));
    assert_eq!(call("GET", "/v1/bridges/slack", Value::Null).await.0, 404);
    let (_, logins) = call("GET", "/v1/bridges/slackgo/logins", Value::Null).await;
    let ids: Vec<&Value> = logins["items"].as_array().unwrap().iter().map(|l| &l["loginID"]).collect();
    assert_eq!((ids, &logins["items"][0]["bridgeID"]), (vec![&json!("T1-U1"), &json!("T2-U2")], &json!("slackgo")));

    let remove = "/v1/bridges/slackgo/logins/T1-U1/remove";
    let (status, missing) = call("POST", remove, json!({})).await;
    let issue = json!({ "expected": "'current-device' | 'all-devices'", "received": "undefined",
                        "code": "invalid_type", "path": ["scope"], "message": "Required" });
    assert_eq!((status, &missing["details"]["issues"][0]), (400, &issue));
    let (status, local) = call("POST", remove, json!({ "scope": "current-device" })).await;
    assert_eq!((status, &local["code"]), (400, &json!("INVALID_SCOPE")));
    let removed = json!({ "bridgeID": "slackgo", "loginID": "T1-U1", "affectedAccountIDs": ["slackgo.T1-U1"],
                          "status": "removed", "scope": "all-devices" });
    assert_eq!(call("POST", remove, json!({ "scope": "all-devices" })).await, (200, removed));
    assert_eq!(call("POST", remove, json!({ "scope": "all-devices" })).await.0, 404);
    let last = call("POST", "/v1/bridges/slackgo/logins/T2-U2/remove", json!({ "scope": "all-devices" })).await;
    assert_eq!(last.0, 200);
    let sent: Vec<String> =
        bridges.requests().into_iter().filter(|r| r.contains("logout") || r.starts_with("DELETE")).collect();
    let logout = "POST /_matrix/client/unstable/com.beeper.bridge/slackgo/_matrix/provision/v3/logout/T1-U1";
    assert_eq!(sent, [logout, "DELETE /bridge/slackgo"]);
    assert_eq!(call("GET", "/v1/accounts", Value::Null).await.1, json!([]));
}
