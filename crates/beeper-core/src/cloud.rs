use crate::config::account_network;
use crate::models::{BridgeAccountInfo, BridgeMode, MatrixSession, Profile, User};
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
            subscribed: info["subscription"]["isActive"] == true
                && matches!(info["subscription"]["planId"].as_str(), Some("PLUS" | "PLUS_PLUS")),
        };
        Ok((profile, cloud_accounts(&v["user"]["bridges"])?))
    }

    pub fn finish(&self, jwt: &str) -> Result<(MatrixSession, Profile, Vec<BridgeAccountInfo>), CloudError> {
        let session = self.matrix_login(jwt)?;
        let (profile, accounts) = self.whoami(&session.access_token)?;
        Ok((session, profile, accounts))
    }

    pub fn contacts(&self, access_token: &str, user_id: &str, accounts: &[BridgeAccountInfo]) -> Vec<User> {
        let uid = crate::codec::url_encode(user_id);
        let mut out = Vec::new();
        for a in accounts.iter().filter(|a| {
            a.state == "CONNECTED"
                && matches!(a.bridge_type.as_str(), "signal" | "whatsapp" | "telegram" | "gmessages" | "imessagecloud")
        }) {
            let lid = crate::codec::url_encode(&a.login_id);
            let url = format!(
                "{}/_matrix/client/unstable/com.beeper.bridge/{}/_matrix/provision/v3/contacts?user_id={uid}&login_id={lid}",
                self.homeserver, a.bridge_id
            );
            let Ok(v) = self.call(&url, Some(access_token), None) else { continue };
            for c in v["contacts"].as_array().expect("contacts is a list") {
                let ids = strings(&c["identifiers"]);
                let key = ids.first().cloned().or_else(|| field(c, "id")).or_else(|| field(c, "mxid"));
                let Some(name) = field(c, "name").filter(|n| !n.is_empty()) else { continue };
                let phone = ids.iter().find_map(|s| s.strip_prefix("tel:")).map(|p| match p.starts_with('+') {
                    true => p.to_string(),
                    false => format!("+{p}"),
                });
                let Some(phone) = phone else { continue };
                let id = format!("contact:{}:{}", a.account_id, key.as_deref().unwrap_or(&name));
                if out.iter().any(|u: &User| u.id == id) {
                    continue;
                }
                out.push(User {
                    id,
                    full_name: Some(name),
                    phone_number: Some(phone),
                    img_url: field(c, "avatar_url"),
                    username: None,
                    email: None,
                    is_self: Some(false),
                    cannot_message: None,
                    custom_nickname: None,
                    is_network_bot: None,
                    is_admin: None,
                });
            }
        }
        out.sort_by_key(|u| {
            let ch = u.full_name.as_deref().and_then(|s| s.chars().next()).unwrap_or(' ').to_ascii_uppercase();
            (!ch.is_ascii_alphabetic(), ch)
        });
        out
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

pub(crate) fn http_client() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        install_tls();
        reqwest::Client::builder().timeout(Duration::from_secs(20)).build().expect("build the HTTP client")
    })
}

/// Beeper Desktop's KLIPY API key, masked so that it is not plain text in the repository (see NOTICE).
const KLIPY_KEY: [u8; 64] = [
    5, 93, 45, 61, 22, 55, 85, 26, 60, 48, 2, 36, 6, 21, 1, 64, 71, 6, 35, 6, 22, 21, 85, 70, 116, 74, 27, 54, 17, 38,
    69, 38, 51, 94, 4, 42, 26, 7, 35, 36, 48, 64, 97, 5, 67, 81, 93, 9, 36, 50, 2, 64, 21, 20, 83, 55, 80, 70, 92, 40,
    104, 20, 71, 9,
];

fn klipy_key() -> String {
    KLIPY_KEY.iter().zip(b"beeper-rs".iter().cycle()).map(|(k, m)| char::from(k ^ m)).collect()
}

pub async fn klipy_gifs(query: String) -> Result<Vec<KlipyGif>, String> {
    let key = klipy_key();
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
    let resp = http_client().get(&url).send().await.map_err(|e| e.without_url().to_string())?;
    let status = resp.status();
    let text = resp.text().await.map_err(|e| e.without_url().to_string())?;
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

/// A `com.beeper.linkpreviews` entry without its image, and the bytes of the image.
pub type PagePreview = (Value, Option<Vec<u8>>);

/// The Beeper SDK's link preview of a web page, with the rules of Desktop's `getLinkPreview`.
pub async fn link_preview(url: String) -> Result<Option<PagePreview>, String> {
    let agent = (reqwest::header::USER_AGENT, "BeeperLinkPreviewBot/1.0");
    let resp = http_client().get(&url).header(agent.0, agent.1).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("HTTP {} for {url}", resp.status().as_u16()));
    }
    let page = resp.url().clone();
    let tags = crate::codec::page_meta(&resp.text().await.map_err(|e| e.to_string())?);
    let get = |k: &str| tags.iter().find(|(n, c)| n == k && !c.is_empty()).map(|(_, c)| c.clone());
    let image = get("og:image:secure_url").or_else(|| get("og:image")).and_then(|i| page.join(&i).ok());
    let (title, summary) =
        (get("og:title").or_else(|| get("title")), get("og:description").or_else(|| get("description")));
    let Some(kind) = get("og:type").filter(|_| title.is_some() || image.is_some() || summary.is_some()) else {
        return Ok(None);
    };
    let title = title.unwrap_or("Untitled".into());
    let mut p = json!({ "matched_url": url, "og:url": url, "og:title": title, "og:type": kind });
    if let Some(s) = summary {
        p["og:description"] = s.into();
    }
    let bytes = match image {
        Some(i) => Some(http_bytes(i.as_str()).await?),
        None => None,
    };
    Ok(Some((p, bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn klipy_key_and_live_endpoints() {
        let key = klipy_key();
        assert!(key.len() == 64 && key.bytes().all(|b| b.is_ascii_alphanumeric()));
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
