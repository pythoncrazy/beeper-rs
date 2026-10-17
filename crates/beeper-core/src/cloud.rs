use crate::config::account_network;
use crate::models::{BridgeAccountInfo, BridgeMode, MatrixSession, Profile};
use serde_json::{json, Value};
use std::future::Future;
use std::sync::Once;
use std::time::Duration;

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
    http: reqwest::Client,
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
                remote_name: field(r, "remote_name"),
                remote_profile: r
                    .get("remote_profile")
                    .map(|p| serde_json::from_value(p.clone()))
                    .transpose()
                    .map_err(|e| bad(&format!("{id} remote_profile: {e}")))?,
            });
        }
    }
    Ok(out)
}

fn strings(v: &Value) -> Vec<String> {
    v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(str::to_string)).collect()
}

pub fn install_tls() {
    static TLS: Once = Once::new();
    TLS.call_once(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .expect("no other rustls crypto provider is installed")
    });
}

fn block_on<F: Future>(f: F) -> F::Output {
    tokio::runtime::Handle::current().block_on(f)
}

impl Cloud {
    pub fn new(api_url: &str, homeserver: &str) -> Self {
        install_tls();
        let http = reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("build the HTTP client");
        Self { http, api_url: api_url.into(), homeserver: homeserver.into() }
    }

    fn call(&self, url: &str, token: Option<&str>, body: Option<&Value>) -> Result<Value, CloudError> {
        let req = match body {
            Some(b) => self.http.post(url).header("content-type", "application/json").body(b.to_string()),
            None => self.http.get(url),
        };
        let req = token.into_iter().fold(req, |r, t| r.bearer_auth(t));
        let (status, text) = block_on(async {
            let resp = req.send().await?;
            Ok((resp.status().as_u16(), resp.text().await?))
        })
        .map_err(|e: reqwest::Error| fail(0, format!("Could not reach {url}: {e}")))?;
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlipyGif {
    pub id: String,
    pub preview_url: String,
    pub preview_size: (u32, u32),
    pub gif_url: String,
    pub gif_size: (u32, u32),
    pub mp4_url: String,
    pub mp4_size: (u32, u32),
}

fn http_client() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        install_tls();
        reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("build the HTTP client")
    })
}

pub fn klipy_key() -> Result<&'static str, String> {
    static KEY: std::sync::OnceLock<Result<String, String>> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        let rel = "target/beeper/squashfs-root/resources/app.asar";
        let exe_paths = std::env::current_exe()
            .ok()
            .into_iter()
            .flat_map(|p| p.ancestors().map(|a| a.join(rel)).collect::<Vec<_>>());
        let candidates = std::env::var_os("BEEPER_ASAR")
            .map(std::path::PathBuf::from)
            .into_iter()
            .chain([std::path::PathBuf::from(rel)])
            .chain(exe_paths)
            .chain(
                [
                    "/opt/BeeperTexts/resources/app.asar",
                    "/usr/lib/beepertexts/resources/app.asar",
                    "/Applications/Beeper Desktop.app/Contents/Resources/app.asar",
                ]
                .map(std::path::PathBuf::from),
            );
        let path = candidates.into_iter().find(|p| p.is_file()).ok_or("Beeper Desktop app.asar not found")?;
        let bytes = std::fs::read(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
        let needle = b"https://api.klipy.com/v2";
        let pos = bytes.windows(needle.len()).position(|w| w == needle).ok_or("KLIPY URL not found in app.asar")?;
        let window = &bytes[pos.saturating_sub(600)..pos];
        let key = window
            .windows(66)
            .find(|w| w[0] == b'"' && w[65] == b'"' && w[1..65].iter().all(u8::is_ascii_alphanumeric))
            .map(|w| std::str::from_utf8(&w[1..65]).expect("ASCII key").to_string())
            .ok_or("KLIPY key not found in app.asar")?;
        Ok(key)
    })
    .as_deref()
    .map_err(Clone::clone)
}

pub async fn klipy_gifs(query: String) -> Result<Vec<KlipyGif>, String> {
    let key = klipy_key()?;
    let q = query.trim();
    let url = match q.is_empty() {
        true => format!(
            "https://api.klipy.com/v2/featured?key={key}&limit=10&contentfilter=high&media_filter=gif,tinygif,mp4,tinymp4&pos="
        ),
        false => format!(
            "https://api.klipy.com/v2/search?key={key}&limit=10&contentfilter=high&media_filter=gif,tinygif,mp4,tinymp4&q={}&pos=",
            crate::codec::url_encode(q)
        ),
    };
    let resp = http_client().get(&url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {}: {text}", status.as_u16()));
    }
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let fmt = |f: &Value| {
        let u = f["url"].as_str()?.to_string();
        let w = u32::try_from(f["dims"][0].as_u64()?).ok()?;
        let h = u32::try_from(f["dims"][1].as_u64()?).ok()?;
        Some((u, (w.max(1), h.max(1))))
    };
    Ok(v["results"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            let m = &r["media_formats"];
            let (preview_url, preview_size) = fmt(&m["tinygif"])?;
            let (gif_url, gif_size) = fmt(&m["gif"])?;
            let (mp4_url, mp4_size) = fmt(&m["mp4"])?;
            Some(KlipyGif {
                id: r["id"].as_str()?.to_string(),
                preview_url,
                preview_size,
                gif_url,
                gif_size,
                mp4_url,
                mp4_size,
            })
        })
        .collect())
}

pub async fn http_bytes(url: &str) -> Result<Vec<u8>, String> {
    let resp = http_client().get(url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} for {url}", resp.status().as_u16()));
    }
    resp.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn klipy_key_and_live_endpoints() {
        if std::path::Path::new("target/beeper/squashfs-root/resources/app.asar").is_file() {
            let key = klipy_key().expect("extract KLIPY key from app.asar");
            assert_eq!(key.len(), 64);
        }
        if std::env::var_os("KLIPY_LIVE").is_some() {
            let featured = klipy_gifs(String::new()).await.expect("KLIPY featured");
            assert!(!featured.is_empty());
            let search = klipy_gifs("cat".into()).await.expect("KLIPY search");
            assert!(!search.is_empty());
            let g = &search[0];
            let tiny = http_bytes(&g.preview_url).await.expect("download tinygif");
            let gif = http_bytes(&g.gif_url).await.expect("download gif");
            let mp4 = http_bytes(&g.mp4_url).await.expect("download mp4");
            assert!(tiny.starts_with(b"GIF8") && gif.starts_with(b"GIF8") && mp4.len() > 16);
            println!(
                "KLIPY live OK: featured={} search={} tinygif={}B gif={}B mp4={}B",
                featured.len(),
                search.len(),
                tiny.len(),
                gif.len(),
                mp4.len()
            );
        }
    }
}
