use crate::config::account_network;
use crate::models::{BridgeAccountInfo, BridgeMode, MatrixSession, Profile};
use serde_json::{json, Value};
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};

const GUEST_TOKEN: &str = "BEEPER-PRIVATE-API-PLEASE-DONT-USE";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloudError {
    pub status: u16,
    pub message: String,
}

fn fail(status: u16, message: impl Into<String>) -> CloudError {
    CloudError { status, message: message.into() }
}

pub enum CodeResult {
    Token(String),
    Registration { lead_token: String, username_suggestions: Vec<String> },
}

pub struct Cloud {
    agent: ureq::Agent,
    pub api_url: String,
    pub homeserver: String,
}

fn field(v: &Value, k: &str) -> Option<String> {
    v[k].as_str().map(str::to_string)
}

pub fn cloud_accounts(bridges: &Value) -> Result<Vec<BridgeAccountInfo>, CloudError> {
    let bad = |what: &str| fail(502, format!("Bad whoami response: {what}"));
    let bridges = bridges.as_object().ok_or_else(|| bad("user.bridges is not an object"))?;
    let cloud = bridges.iter().filter(|(id, b)| !id.starts_with("local-") && b["bridgeState"]["isSelfHosted"] != true);
    let mut out = Vec::new();
    for (id, b) in cloud {
        let state = &b["bridgeState"];
        let remotes = b["remoteState"].as_object().into_iter().flatten();
        let remotes: Vec<_> = remotes.filter(|(_, r)| r["state_event"] != "UNCONFIGURED").collect();
        if remotes.len() > 1 && id != "slackgo" {
            return Err(bad(&format!("{id} has {} cloud logins", remotes.len())));
        }
        for (remote, r) in remotes {
            out.push(BridgeAccountInfo {
                account_id: if id == "slackgo" { format!("{id}.{remote}") } else { id.clone() },
                bridge_id: id.clone(),
                bridge_type: field(state, "bridgeType").ok_or_else(|| bad(&format!("{id} has no bridgeType")))?,
                is_running: state["stateEvent"] == "RUNNING",
                login_id: remote.clone(),
                mode: BridgeMode::Cloud,
                network_name: account_network(id).display_name.into(),
                state: field(r, "state_event").ok_or_else(|| bad(&format!("{id} login has no state_event")))?,
            });
        }
    }
    Ok(out)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

impl Cloud {
    pub fn new(api_url: &str, homeserver: &str) -> Self {
        let tls = TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build();
        let agent = ureq::Agent::config_builder()
            .tls_config(tls)
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(20)))
            .build()
            .new_agent();
        Self { agent, api_url: api_url.into(), homeserver: homeserver.into() }
    }

    fn call(&self, url: &str, token: Option<&str>, body: Option<&Value>) -> Result<Value, CloudError> {
        let net = |e: ureq::Error| fail(0, format!("Could not reach {url}: {e}"));
        let auth = token.map(|t| format!("Bearer {t}"));
        let mut resp = match body {
            Some(b) => {
                let req = self.agent.post(url);
                auth.iter().fold(req, |r, a| r.header("authorization", a)).send_json(b)
            }
            None => {
                let req = self.agent.get(url);
                auth.iter().fold(req, |r, a| r.header("authorization", a)).call()
            }
        }
        .map_err(net)?;
        let status = resp.status().as_u16();
        let text = resp.body_mut().read_to_string().map_err(net)?;
        let json = serde_json::from_str::<Value>(&text);
        if status >= 300 {
            let msg = json.ok().and_then(|v| ["error", "message", "errcode"].iter().find_map(|k| field(&v, k)));
            return Err(fail(status, msg.unwrap_or(format!("HTTP {status}: {text}"))));
        }
        match text.is_empty() {
            true => Ok(Value::Null),
            false => json.map_err(|e| fail(502, format!("Bad JSON from {url}: {e}"))),
        }
    }

    fn guest(&self, path: &str, body: Value) -> Result<Value, CloudError> {
        self.call(&format!("{}{path}", self.api_url), Some(GUEST_TOKEN), Some(&body))
    }

    pub fn start_login(&self) -> Result<(String, Vec<String>), CloudError> {
        let v = self.guest("/user/login", json!({}))?;
        Ok((field(&v, "request").ok_or_else(|| fail(502, "Could not start sign-in"))?, strings(&v["type"])))
    }

    pub fn send_login_email(&self, request: &str, email: &str) -> Result<(), CloudError> {
        self.guest("/user/login/email", json!({ "request": request, "email": email, "supportsOTP": true })).map(drop)
    }

    pub fn send_login_code(&self, request: &str, code: &str) -> Result<CodeResult, CloudError> {
        let body = json!({ "request": request, "response": code, "reactivationSupported": true, "appType": "desktop" });
        let v = self.guest("/user/login/response", body)?;
        match (field(&v, "token"), field(&v, "leadToken")) {
            (Some(token), _) => Ok(CodeResult::Token(token)),
            (None, Some(lead_token)) => {
                Ok(CodeResult::Registration { lead_token, username_suggestions: strings(&v["usernameSuggestions"]) })
            }
            (None, None) => Err(fail(502, "Could not finish sign-in")),
        }
    }

    pub fn register(&self, request: &str, lead_token: &str, username: &str) -> Result<String, CloudError> {
        let body = json!({
            "userLoginRequestId": request,
            "appType": "desktop",
            "leadToken": lead_token,
            "username": username,
            "acceptTerms": true
        });
        field(&self.guest("/user/register", body)?, "token").ok_or_else(|| fail(502, "Could not create the account"))
    }

    pub fn matrix_login(&self, jwt: &str) -> Result<MatrixSession, CloudError> {
        let name = format!("beeper-rs ({})", std::env::consts::OS);
        let body = json!({ "type": "org.matrix.login.jwt", "token": jwt, "initial_device_display_name": name });
        let v = self.call(&format!("{}/_matrix/client/v3/login", self.homeserver), None, Some(&body))?;
        serde_json::from_value(v).map_err(|e| fail(502, format!("Bad Matrix login response: {e}")))
    }

    pub fn whoami(&self, access_token: &str) -> Result<(Profile, Vec<BridgeAccountInfo>), CloudError> {
        let v = self.call(&format!("{}/whoami", self.api_url), Some(access_token), None)?;
        let info = &v["userInfo"];
        let profile = Profile {
            full_name: field(info, "fullName"),
            username: field(info, "username"),
            email: field(info, "email"),
        };
        Ok((profile, cloud_accounts(&v["user"]["bridges"])?))
    }

    pub fn finish(&self, jwt: &str) -> Result<(MatrixSession, Profile, Vec<BridgeAccountInfo>), CloudError> {
        let session = self.matrix_login(jwt)?;
        let (profile, accounts) = self.whoami(&session.access_token)?;
        Ok((session, profile, accounts))
    }
}
