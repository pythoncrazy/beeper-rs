use crate::config::account_network;
use crate::models::{BridgeAccountInfo, BridgeMode, MatrixSession, Plan, Profile, User};
use serde_json::{json, Value};
use std::future::Future;
use std::sync::{Once, OnceLock};
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
        // Desktop's account ID (`Uo`) is `bridge.login` for each cloud login. A bridge with one login other than Slack
        // keeps the bridge name.
        let several = remotes.len() > 1 || id == "slackgo";
        for (remote, r) in remotes {
            out.push(BridgeAccountInfo {
                account_id: if several { format!("{id}.{remote}") } else { id.clone() },
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
                remote_error: field(r, "message").filter(|_| r["has_error"] == true),
            });
        }
    }
    Ok(out)
}

/// The profile of the `userInfo` of `whoami`.
fn profile(info: &Value) -> Profile {
    let sub = &info["subscription"];
    let active = sub["isActive"] == true;
    let plan = match sub["planId"].as_str() {
        Some("PLUS") if active => Plan::Plus,
        Some("PLUS_PLUS") if active => Plan::PlusPlus,
        _ => Plan::Free,
    };
    // Desktop's `subscriptionAction`: app store and Stripe subscribers manage their plan elsewhere.
    let managed = ["GOOGLEPLAY", "APPSTORE", "REVENUECAT", "STRIPE", "STRIPE_LEGACY"];
    let provider = sub["provider"].as_str().filter(|_| plan != Plan::Free);
    Profile {
        full_name: field(info, "fullName"),
        username: field(info, "username"),
        email: field(info, "email"),
        plan,
        can_subscribe: provider.is_none_or(|p| !managed.contains(&p))
            && info["canSubscribe"]["STRIPE"].as_bool().unwrap_or(!active),
        internal: info["channel"] == "INTERNAL",
        analytics_disabled: info["analyticsDisabled"] != false,
    }
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
        Self { api_url: api_url.into(), homeserver: homeserver.into() }
    }

    fn call(&self, url: &str, token: Option<&str>, body: Option<&Value>) -> Result<Value, CloudError> {
        let method = if body.is_some() { reqwest::Method::POST } else { reqwest::Method::GET };
        self.send(method, url, token, body)
    }

    fn send(
        &self,
        method: reqwest::Method,
        url: &str,
        token: Option<&str>,
        body: Option<&Value>,
    ) -> Result<Value, CloudError> {
        let req = http_client().request(method, url);
        let req = match body {
            Some(b) => req.header("content-type", "application/json").body(b.to_string()),
            None => req,
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

    /// Desktop's `loginWithJWT`, whose SDK names the new device with `DEFAULT_DEVICE_NAME`.
    pub fn matrix_login(&self, jwt: &str) -> Result<MatrixSession, CloudError> {
        let name = crate::config::DEVICE_NAME;
        let body = json!({ "type": "org.matrix.login.jwt", "token": jwt, "initial_device_display_name": name });
        let v = self.call(&format!("{}/_matrix/client/v3/login", self.homeserver), None, Some(&body))?;
        let session: MatrixSession =
            serde_json::from_value(v).map_err(|e| fail(502, format!("Bad Matrix login response: {e}")))?;
        Ok(MatrixSession { jwt: Some(jwt.into()), ..session })
    }

    pub fn whoami(&self, access_token: &str) -> Result<(Profile, Vec<BridgeAccountInfo>), CloudError> {
        let v = self.call(&format!("{}/whoami", self.api_url), Some(access_token), None)?;
        Ok((profile(&v["userInfo"]), cloud_accounts(&v["user"]["bridges"])?))
    }

    /// Desktop's `bapi.setAnalyticsDisabled`.
    pub fn set_analytics_disabled(&self, access_token: &str, disabled: bool) -> Result<(), CloudError> {
        let url = format!("{}/user", self.api_url);
        let body = json!({ "analyticsDisabled": disabled });
        self.send(reqwest::Method::PATCH, &url, Some(access_token), Some(&body)).map(drop)
    }

    /// Desktop's `bapi.setEmail`: the server sends a confirmation to the new email.
    pub fn set_email(&self, access_token: &str, email: &str) -> Result<(), CloudError> {
        let url = format!("{}/user", self.api_url);
        self.send(reqwest::Method::PATCH, &url, Some(access_token), Some(&json!({ "email": email }))).map(drop)
    }

    /// Desktop's `bapi.deleteAllBridges`: the reset of the recovery key disconnects every account.
    pub fn delete_all_bridges(&self, access_token: &str) -> Result<(), CloudError> {
        let url = format!("{}/bridge/delete-all-bridges", self.api_url);
        self.send(reqwest::Method::POST, &url, Some(access_token), None).map(drop)
    }

    /// Desktop's `bapi.deleteAccount`, which deletes the Beeper account for good.
    pub fn delete_account(&self, access_token: &str) -> Result<(), CloudError> {
        self.send(reqwest::Method::DELETE, &format!("{}/user", self.api_url), Some(access_token), None).map(drop)
    }

    pub fn finish(&self, jwt: &str) -> Result<(MatrixSession, Profile, Vec<BridgeAccountInfo>), CloudError> {
        let session = self.matrix_login(jwt)?;
        let (profile, accounts) = self.whoami(&session.access_token)?;
        Ok((session, profile, accounts))
    }

    pub fn contacts(&self, access_token: &str, user_id: &str, accounts: &[BridgeAccountInfo]) -> Vec<User> {
        let uid = crate::codec::url_encode(user_id);
        let mut out = Vec::new();
        // Desktop's `fetchContacts` asks every account; a bridge without a contact list gives none.
        for a in accounts.iter().filter(|a| a.state == "CONNECTED") {
            let lid = crate::codec::url_encode(&a.login_id);
            let url = format!(
                "{}/_matrix/client/unstable/com.beeper.bridge/{}/_matrix/provision/v3/contacts?user_id={uid}&login_id={lid}",
                self.homeserver, a.bridge_id
            );
            // A bridge that fails, or answers without a list, gives no contacts.
            let Ok(v) = self.call(&url, Some(access_token), None) else { continue };
            let Some(list) = v["contacts"].as_array() else { continue };
            for c in list {
                let ids = strings(&c["identifiers"]);
                let key = ids.first().cloned().or_else(|| field(c, "id")).or_else(|| field(c, "mxid"));
                let Some(name) = field(c, "name").filter(|n| !n.is_empty()) else { continue };
                let phone = ids.iter().find_map(|s| s.strip_prefix("tel:")).map(|p| match p.starts_with('+') {
                    true => p.to_string(),
                    false => format!("+{p}"),
                });
                // Desktop's `NewChat` lists a contact with a name and a phone number, or with a Matrix user.
                let mxid = field(c, "mxid");
                if phone.is_none() && mxid.is_none() {
                    continue;
                }
                let id = format!("contact:{}:{}", a.account_id, key.as_deref().unwrap_or(&name));
                if out.iter().any(|u: &User| u.id == id) {
                    continue;
                }
                out.push(User {
                    id,
                    full_name: Some(name),
                    phone_number: phone,
                    email: ids.iter().find_map(|s| s.strip_prefix("mailto:")).map(String::from),
                    mxid,
                    img_url: field(c, "avatar_url"),
                    username: None,
                    is_self: Some(false),
                    cannot_message: None,
                    custom_nickname: None,
                    is_network_bot: None,
                    ..Default::default()
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

/// The account of a contact of [`Cloud::contacts`], from its `contact:{account}:{key}` ID.
pub fn contact_account(id: &str) -> Option<&str> {
    Some(id.strip_prefix("contact:")?.split_once(':')?.0)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KlipyGif {
    pub id: String,
    pub preview_url: String,
    pub preview_size: (u32, u32),
    pub gif_url: String,
    pub gif_size: (u32, u32),
    /// The byte size of the GIF, when KLIPY gives it.
    pub gif_bytes: Option<u64>,
    pub mp4_url: String,
    pub mp4_size: (u32, u32),
}

fn shared(cell: &'static OnceLock<reqwest::Client>, timeout: Option<Duration>) -> &'static reqwest::Client {
    cell.get_or_init(|| {
        install_tls();
        let builder = reqwest::Client::builder().connect_timeout(Duration::from_secs(20));
        timeout.into_iter().fold(builder, |b, t| b.timeout(t)).build().expect("build the HTTP client")
    })
}

/// The shared HTTP client: one connection pool, and a 20-second limit for each request.
pub(crate) fn http_client() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    shared(&HTTP, Some(Duration::from_secs(20)))
}

/// The shared HTTP client with no request limit, for bridge steps that answer only when the user acts.
pub(crate) fn wait_client() -> &'static reqwest::Client {
    static HTTP: OnceLock<reqwest::Client> = OnceLock::new();
    shared(&HTTP, None)
}

/// Desktop's OpenAI `baseURL` (`uke`): Beeper's proxy beside the homeserver, with "matrix." changed to "ai-services.".
pub fn ai_services(homeserver: &str) -> String {
    let url = reqwest::Url::parse(homeserver).expect("a homeserver URL");
    let host = url.host_str().expect("a homeserver host");
    let host = host.strip_prefix("matrix.").map_or(host.to_string(), |h| format!("ai-services.{h}"));
    format!("https://{host}/proxy/_/v1")
}

/// The file name and type of a voice note (Desktop's `toFile` defaults), and of a Talk to Type recording.
pub const VOICE_NOTE_FILE: (&str, &str) = ("speech.mp3", "audio/mpeg");
pub const TALK_FILE: (&str, &str) = ("a.wav", "audio/wav");

/// Desktop's Whisper transcription (`xre`): `audio.transcriptions.create` of the OpenAI SDK with `whisper-1` at
/// `base`, and the Matrix access token as the API key. The file has a name and a type: Desktop sends each voice note
/// as [`VOICE_NOTE_FILE`] and Talk to Type as [`TALK_FILE`].
pub async fn transcribe(
    base: &str,
    token: &str,
    (audio, (name, mime)): (Vec<u8>, (&str, &str)),
    language: Option<&str>,
) -> Result<String, String> {
    let b = matrix_sdk::ruma::TransactionId::new();
    let part =
        |name: &str, extra: &str| format!("--{b}\r\nContent-Disposition: form-data; name=\"{name}\"{extra}\r\n\r\n");
    let fields = [("model", "whisper-1")].into_iter().chain(language.map(|l| ("language", l)));
    let mut body: Vec<u8> = fields.flat_map(|(k, v)| format!("{}{v}\r\n", part(k, "")).into_bytes()).collect();
    body.extend(part("file", &format!("; filename=\"{name}\"\r\nContent-Type: {mime}")).bytes());
    body.extend(audio);
    body.extend(format!("\r\n--{b}--\r\n").bytes());
    let res = http_client()
        .post(format!("{base}/audio/transcriptions"))
        .bearer_auth(token)
        .header("content-type", format!("multipart/form-data; boundary={b}"))
        // The OpenAI SDK waits 10 minutes.
        .timeout(Duration::from_secs(600))
        .body(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body = res.bytes().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&body).map_err(|e| format!("{status}: {e}"))?;
    match v["text"].as_str() {
        Some(t) if status.is_success() => Ok(t.into()),
        _ => Err(v["error"]["message"].as_str().map_or_else(|| status.to_string(), str::to_string)),
    }
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

/// One page of KLIPY's featured GIFs, or of a search, from the position `pos`, and the position of the next page,
/// which is empty after the last page.
pub async fn klipy_gifs(query: String, pos: String) -> Result<(Vec<KlipyGif>, String), String> {
    let key = klipy_key();
    let q = query.trim();
    let url = match q.is_empty() {
        true => format!(
            "https://api.klipy.com/v2/featured?key={key}&limit=10&contentfilter=high&media_filter=gif,tinygif,mp4,tinymp4&pos={}",
            crate::codec::url_encode(&pos)
        ),
        false => format!(
            "https://api.klipy.com/v2/search?key={key}&limit=10&contentfilter=high&media_filter=gif,tinygif,mp4,tinymp4&q={}&pos={}",
            crate::codec::url_encode(q),
            crate::codec::url_encode(&pos)
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
    let gifs = v["results"]
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
                gif_bytes: m["gif"]["size"].as_u64(),
                mp4_url,
                mp4_size,
            })
        })
        .collect();
    Ok((gifs, v["next"].as_str().unwrap_or_default().to_string()))
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
    // Desktop keeps a preview whose image does not load, without the image.
    let bytes = match image {
        Some(i) => http_bytes(i.as_str()).await.ok(),
        None => None,
    };
    Ok(Some((p, bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_accounts_read_remote_errors() {
        let bridges = json!({ "discordgo": { "bridgeState": { "bridgeType": "discordgo" }, "remoteState": {
            "d1": { "state_event": "BAD_CREDENTIALS", "has_error": true, "message": "Token expired" } } },
            "signal": { "bridgeState": { "bridgeType": "signal" }, "remoteState": {
            "s1": { "state_event": "CONNECTED", "has_error": false, "message": "ok" } } } });
        let mut errors: Vec<_> = cloud_accounts(&bridges).unwrap().into_iter().map(|a| a.remote_error).collect();
        errors.sort();
        assert_eq!(errors, [None, Some("Token expired".to_string())]);
    }

    #[test]
    fn profile_follows_desktop_subscription_action() {
        let free = profile(&json!({ "username": "alex" }));
        assert_eq!((free.plan, free.can_subscribe, free.analytics_disabled), (Plan::Free, true, true));
        let stripe = json!({ "subscription": { "isActive": true, "planId": "PLUS", "provider": "STRIPE" }, "analyticsDisabled": false });
        let stripe = profile(&stripe);
        assert_eq!((stripe.plan, stripe.can_subscribe, stripe.analytics_disabled), (Plan::Plus, false, false));
        let ended = json!({ "subscription": { "isActive": false, "planId": "PLUS_PLUS", "provider": "APPSTORE" } });
        assert_eq!((profile(&ended).plan, profile(&ended).can_subscribe), (Plan::Free, true));
        let blocked = json!({ "canSubscribe": { "STRIPE": false } });
        assert!(!profile(&blocked).can_subscribe, "the server can forbid Stripe");
    }

    /// The Matrix login names the device like Desktop on Linux, to a local stub of the homeserver.
    #[cfg(target_os = "linux")]
    #[tokio::test(flavor = "multi_thread")]
    async fn matrix_login_names_the_device_like_desktop() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |method, path, body| {
            log.lock().unwrap().push((method.to_string(), path.to_string(), body));
            (200, json!({ "user_id": "@me:x", "access_token": "token", "device_id": "DEV" }))
        })
        .await;
        let cloud = Cloud::new(&url, &url);
        let session = tokio::task::spawn_blocking(move || cloud.matrix_login("jwt")).await.unwrap().unwrap();
        assert_eq!((session.device_id.as_str(), session.jwt.as_deref()), ("DEV", Some("jwt")));
        let body = json!({ "type": "org.matrix.login.jwt", "token": "jwt",
            "initial_device_display_name": "Beeper Desktop v4 (Linux)" });
        assert_eq!(*sent.lock().unwrap(), [("POST".into(), "/_matrix/client/v3/login".into(), body)]);
    }

    /// The reset of the recovery key deletes every bridge, like Desktop's `deleteAllBridges`.
    #[tokio::test(flavor = "multi_thread")]
    async fn reset_deletes_all_bridges_like_desktop() {
        let sent = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = sent.clone();
        let url = crate::bridge_stub::serve(move |method, path, _| {
            log.lock().unwrap().push((method.to_string(), path.to_string()));
            (200, json!({}))
        })
        .await;
        let cloud = Cloud::new(&url, &url);
        tokio::task::spawn_blocking(move || cloud.delete_all_bridges("token")).await.unwrap().unwrap();
        assert_eq!(*sent.lock().unwrap(), [("POST".to_string(), "/bridge/delete-all-bridges".to_string())]);
    }

    /// Desktop's Whisper request, to a local stub of Beeper's OpenAI proxy.
    #[tokio::test]
    async fn transcriptions_go_to_whisper_like_desktop() {
        assert_eq!(ai_services("https://matrix.beeper.com"), "https://ai-services.beeper.com/proxy/_/v1");
        assert_eq!(ai_services("https://matrix.beeper-dev.com/"), "https://ai-services.beeper-dev.com/proxy/_/v1");
        let url = crate::bridge_stub::serve(|method, path, body| {
            let body = body.as_str().expect("a multipart body");
            assert_eq!((method, path), ("POST", "/proxy/_/v1/audio/transcriptions"));
            assert!(body.contains("name=\"model\"\r\n\r\nwhisper-1\r\n"), "{body}");
            let de = body.contains("name=\"language\"\r\n\r\nde\r\n");
            let file =
                if de { "speech.mp3\"\r\nContent-Type: audio/mpeg" } else { "a.wav\"\r\nContent-Type: audio/wav" };
            assert!(body.contains(&format!("filename=\"{file}\r\n\r\nOggS\r\n--")), "{body}");
            match de {
                true => (200, json!({ "text": "Hallo" })),
                false => (401, json!({ "error": { "message": "Invalid token" } })),
            }
        })
        .await;
        let base = format!("{url}/proxy/_/v1");
        assert_eq!(transcribe(&base, "t", (b"OggS".to_vec(), VOICE_NOTE_FILE), Some("de")).await, Ok("Hallo".into()));
        assert_eq!(transcribe(&base, "t", (b"OggS".to_vec(), TALK_FILE), None).await, Err("Invalid token".into()));
    }

    #[tokio::test]
    async fn klipy_key_and_live_endpoints() {
        let key = klipy_key();
        assert!(key.len() == 64 && key.bytes().all(|b| b.is_ascii_alphanumeric()));
        if std::env::var_os("KLIPY_LIVE").is_some() {
            let (featured, next) = klipy_gifs(String::new(), String::new()).await.expect("KLIPY featured");
            assert!(!featured.is_empty() && !next.is_empty(), "a first page has a next page");
            let (more, _) = klipy_gifs(String::new(), next).await.expect("KLIPY featured page 2");
            assert!(!more.is_empty(), "the next page has GIFs");
            let (search, _) = klipy_gifs("cat".into(), String::new()).await.expect("KLIPY search");
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
