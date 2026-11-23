//! Desktop's `BridgeAuthAPI` for cloud bridges: the bridgev2 provisioning login API, which the homeserver passes
//! to the bridge (the SDK's `CloudBridgeLogin*`). The bridge describes each step; nothing here knows a network.
use crate::matrix::Provision;
use serde::Deserialize;
use serde_json::Value;
use std::time::Duration;

/// How often and how long the SDK's `ForceStartCloudBridge` waits for a started bridge.
const START_TRIES: u32 = 15;
const START_WAIT: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Flow {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
}

/// A login step of the bridge. `raw` keeps the step as the bridge sent it, for the Desktop API.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Step {
    #[serde(rename = "type")]
    pub kind: String,
    pub step_id: String,
    pub instructions: String,
    pub user_input: Option<UserInput>,
    pub cookies: Option<Box<Cookies>>,
    pub display_and_wait: Option<Display>,
    #[serde(skip)]
    pub raw: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct UserInput {
    pub fields: Vec<Field>,
    pub can_cancel: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Field {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub name: String,
    pub description: String,
    pub default_value: String,
    pub pattern: String,
    pub options: Vec<String>,
    pub min_length: usize,
    pub max_length: usize,
}

/// The browser login of a `cookies` step: the page, and the fields that the browser window reads from it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Cookies {
    pub url: String,
    /// `None` is Desktop's own user agent, and an empty one is the browser's.
    pub user_agent: Option<String>,
    /// The window closes only on a URL that matches this pattern, after it has all the required fields.
    pub wait_for_url_pattern: String,
    pub fields: Vec<CookieField>,
    /// The window does not show, and closes after a time.
    pub hidden: bool,
    /// A script whose result has the values of the `special` fields.
    pub extract_js: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CookieField {
    pub id: String,
    pub required: bool,
    pub pattern: String,
    /// Desktop reads the first source of each type.
    pub sources: Vec<CookieSource>,
}

/// Where the browser window reads a field: a `cookie`, a `local_storage` item, a `request_header`, or a `special`
/// value of the `extract_js` result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct CookieSource {
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    pub cookie_domain: String,
    pub request_url_regex: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Display {
    #[serde(rename = "type")]
    pub kind: String,
    pub data: String,
    pub image_url: String,
    pub can_cancel: bool,
}

impl Step {
    pub fn parse(raw: Value) -> Result<Self, String> {
        let step: Step = serde_json::from_value(raw.clone()).map_err(|e| format!("Bad login step: {e}"))?;
        match step.kind.as_str() {
            // Desktop's `getBrowserLoginFromStep`.
            "cookies" if step.cookies.is_none() => Err("No cookies step".into()),
            "cookies" if step.cookies.as_ref().is_some_and(|c| c.url.is_empty()) => Err("No cookies url".into()),
            "user_input" | "cookies" | "display_and_wait" | "complete" => Ok(Step { raw, ..step }),
            kind => Err(format!("Unknown login step type: {kind}")),
        }
    }

    /// Desktop's `canCancelStep`: the bridge allows "Back" from this step.
    pub fn can_cancel(&self) -> bool {
        self.user_input.as_ref().is_some_and(|u| u.can_cancel) || self.display_and_wait.as_ref().is_some_and(|d| d.can_cancel)
    }
}

fn json(body: &str) -> Result<Value, String> {
    serde_json::from_str(body).map_err(|e| format!("Bad reply from the bridge: {e}"))
}

fn flow_list(v: Value) -> Result<Vec<Flow>, String> {
    serde_json::from_value(v["flows"].clone()).map_err(|e| format!("Bad login flows: {e}"))
}

/// The SDK's `startCloudBridge`: the Beeper API starts the bridge of the user.
async fn start_bridge(p: &Provision, api_url: &str) -> Result<(), String> {
    let url = format!("{}/bridge/{}/start", api_url.trim_end_matches('/'), crate::codec::url_encode(&p.bridge_id));
    let resp = crate::cloud::http_client().post(url).bearer_auth(&p.access_token).send().await;
    let resp = resp.map_err(|e| e.without_url().to_string())?;
    resp.error_for_status().map(drop).map_err(|e| e.without_url().to_string())
}

/// The login flows of the bridge. The SDK starts a bridge that the homeserver does not know yet (`M_NOT_FOUND`),
/// and asks again until it runs.
pub async fn flows(p: Provision, api_url: String) -> Result<Vec<Flow>, String> {
    if let Some(v) = p.get("login/flows").await? {
        return flow_list(v);
    }
    start_bridge(&p, &api_url).await?;
    for _ in 0..START_TRIES {
        tokio::time::sleep(START_WAIT).await;
        if let Some(v) = p.get("login/flows").await? {
            return flow_list(v);
        }
    }
    Err(format!("The {} bridge did not start", p.bridge_id))
}

/// Starts a login with the flow. A `login_id` in `p` logs that login in again (Desktop's reconnect). It returns the
/// login process ID and the first step.
pub async fn start(p: Provision, flow: String) -> Result<(String, Step), String> {
    let mut v = json(&p.post(&format!("login/start/{}", crate::codec::url_encode(&flow)), "{}".into()).await?)?;
    let process = v.as_object_mut().and_then(|o| o.remove("login_id"));
    let process = process.and_then(|id| id.as_str().map(str::to_string)).ok_or("The bridge sent no login ID")?;
    Ok((process, Step::parse(v)?))
}

fn step_path(process: &str, step: &str, action: &str) -> String {
    let enc = crate::codec::url_encode;
    format!("login/step/{}/{}/{action}", enc(process), enc(step))
}

/// Sends the values of a `user_input` or `cookies` step, or waits for a `display_and_wait` step to end. It returns
/// the next step.
pub async fn submit(p: Provision, process: String, step: Step, fields: Value) -> Result<Step, String> {
    assert!(matches!(step.kind.as_str(), "user_input" | "cookies" | "display_and_wait"), "{} has no submit", step.kind);
    let reply = p.post(&step_path(&process, &step.step_id, &step.kind), fields.to_string()).await?;
    Step::parse(json(&reply)?)
}

/// Desktop's `cancelLoginStep` ("Back"): the bridge returns the step before.
pub async fn cancel_step(p: Provision, process: String, step: String) -> Result<Step, String> {
    Step::parse(json(&p.post(&step_path(&process, &step, "cancel"), "{}".into()).await?)?)
}

/// Desktop's `cancelLoginProcess`.
pub async fn cancel(p: Provision, process: String) -> Result<(), String> {
    p.post(&format!("login/cancel/{}", crate::codec::url_encode(&process)), "{}".into()).await.map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn steps_follow_the_bridgev2_shape() {
        let raw = json!({
            "type": "display_and_wait", "step_id": "fi.mau.whatsapp.login.qr", "instructions": "Scan",
            "display_and_wait": { "type": "qr", "data": "2@abc", "can_cancel": true },
        });
        let step = Step::parse(raw.clone()).expect("a QR step");
        assert_eq!(step.display_and_wait.as_ref().map(|d| (d.kind.as_str(), d.data.as_str())), Some(("qr", "2@abc")));
        assert!(step.can_cancel());
        assert_eq!(step.raw, raw);
        let form = Step::parse(json!({ "type": "user_input", "step_id": "s", "user_input": { "fields": [
            { "type": "phone_number", "id": "phone", "name": "Phone number", "max_length": 15 },
        ] } }))
        .expect("a form step");
        let field = &form.user_input.as_ref().expect("fields").fields[0];
        assert_eq!((field.kind.as_str(), field.name.as_str(), field.max_length), ("phone_number", "Phone number", 15));
        assert!(!form.can_cancel());
        assert_eq!(Step::parse(json!({ "type": "webauthn" })).unwrap_err(), "Unknown login step type: webauthn");
    }

    fn provision(url: &str, bridge: &str, login: &str) -> Provision {
        Provision {
            homeserver: url.into(),
            access_token: "token".into(),
            user_id: "@me:beeper.com".into(),
            bridge_id: bridge.into(),
            login_id: login.into(),
        }
    }

    /// The Telegram phone flow of the stub: a wrong code, "Back", the code again, the password, and the login.
    #[tokio::test]
    async fn a_form_login_goes_through_the_bridge_steps() {
        let (url, bridges) = crate::bridge_stub::homeserver().await;
        let p = provision(&url, "telegram", "");
        let flows = flows(p.clone(), url.clone()).await.expect("the Telegram flows");
        assert_eq!(flows.iter().map(|f| f.id.as_str()).collect::<Vec<_>>(), ["phone", "qr"]);
        let (process, phone) = start(p.clone(), "phone".into()).await.expect("start the phone flow");
        assert_eq!(phone.step_id, "fi.mau.telegram.login.phone_number");
        let field = |s: &Step| s.user_input.as_ref().expect("a form").fields[0].id.clone();
        let bad = submit(p.clone(), process.clone(), phone.clone(), json!({ field(&phone): "5550100" })).await;
        assert_eq!(bad.unwrap_err(), "Phone number must be in international format");
        let code = submit(p.clone(), process.clone(), phone.clone(), json!({ field(&phone): "+15550100" })).await;
        let code = code.expect("the code step");
        assert!(code.can_cancel());
        let wrong = submit(p.clone(), process.clone(), code.clone(), json!({ field(&code): crate::bridge_stub::WRONG_CODE }));
        assert_eq!(wrong.await.unwrap_err(), "Invalid phone code");
        let back = cancel_step(p.clone(), process.clone(), code.step_id.clone()).await.expect("go back");
        assert_eq!(back.step_id, phone.step_id);
        let code = submit(p.clone(), process.clone(), back, json!({ field(&phone): "+15550100" })).await.unwrap();
        let password = submit(p.clone(), process.clone(), code.clone(), json!({ field(&code): "12345" })).await;
        let password = password.expect("the password step");
        assert_eq!(password.user_input.as_ref().unwrap().fields[0].kind, "password");
        let done = submit(p.clone(), process.clone(), password.clone(), json!({ field(&password): "pw" })).await;
        assert_eq!(done.expect("the last step").kind, "complete");
        let requests = bridges.requests();
        let prefix = "POST /_matrix/client/unstable/com.beeper.bridge/telegram/_matrix/provision/v3/login";
        assert!(requests.contains(&format!("{prefix}/start/phone")), "{requests:?}");
        assert!(requests.contains(&format!("{prefix}/step/{process}/fi.mau.telegram.login.code/cancel")));
    }

    /// The WhatsApp QR flow of the stub: each wait gives the next QR code, then the login; a cancel ends a process.
    #[tokio::test]
    async fn a_qr_login_waits_for_each_code() {
        let (url, _) = crate::bridge_stub::homeserver().await;
        let p = provision(&url, "whatsapp", "15550100");
        let (process, first) = start(p.clone(), "qr".into()).await.expect("start the QR flow");
        let data = |s: &Step| s.display_and_wait.as_ref().expect("a display").data.clone();
        assert_eq!(data(&first), "2@first");
        let second = submit(p.clone(), process.clone(), first, Value::Null).await.expect("the next QR code");
        assert_eq!(data(&second), "2@second");
        let done = submit(p.clone(), process.clone(), second, Value::Null).await.expect("the login");
        assert_eq!(done.raw["complete"]["user_login_id"], "15550100");
        let (other, _) = start(p.clone(), "qr".into()).await.expect("start again");
        cancel(p.clone(), other.clone()).await.expect("cancel the process");
        assert_eq!(cancel(p, other).await.unwrap_err(), "Login not found");
    }

    /// The SDK's `ForceStartCloudBridge`: the API starts a bridge that the homeserver does not know, then the flows
    /// load.
    #[tokio::test]
    async fn the_flows_of_a_stopped_bridge_start_it() {
        let (url, bridges) = crate::bridge_stub::homeserver().await;
        let p = provision(&url, crate::bridge_stub::STOPPED_BRIDGE, "");
        let flows = flows(p.clone(), url.clone()).await.expect("the flows after the start");
        assert_eq!(flows[0].name, "Cookies");
        assert!(bridges.requests().contains(&"POST /bridge/linkedin/start".to_string()));
        let (process, step) = start(p.clone(), "cookies".into()).await.expect("start the cookie flow");
        assert_eq!(step.cookies.as_ref().map(|c| c.url.as_str()), Some("https://www.linkedin.com/login"));
        let missing = submit(p.clone(), process.clone(), step.clone(), json!({ "li_at": "a" })).await;
        assert_eq!(missing.unwrap_err(), "Missing cookies");
        let done = submit(p, process, step, json!({ "li_at": "a", "JSESSIONID": "b" })).await;
        assert_eq!(done.expect("the login").kind, "complete");
    }
}
