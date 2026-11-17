//! The browser sign-in of the upstream CLI: `lib/desktop-auth.js` finds a running Desktop API, and `lib/oauth.js`
//! gets a token from it with OAuth and PKCE, through a callback on a local port.

use super::{iso_in, obj, update_config, CliError, Ctx, Res};
use beeper_api::client::BeeperClient;
use beeper_api::codec::{base64_encode_url_nopad, sha256};
use beeper_core::codec::{url_decode, url_encode};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

const DEFAULT_PORT: u16 = 23373;
const CLIENT_NAME: &str = "Beeper CLI";
const SCOPE: &str = "read write";
/// `createCallbackServer`'s wait for the browser.
const CALLBACK_WAIT: Duration = Duration::from_secs(120);

/// The status and the body of a request that gives up after `wait`.
async fn fetch(base: &str, method: &str, path: &str, wait: Duration) -> Option<(u16, Vec<u8>)> {
    let client = BeeperClient::try_new(base, "").ok()?;
    tokio::time::timeout(wait, client.fetch(method, path, None)).await.ok()?.ok()
}

/// `probeDesktop`: the base URL and the app state when `/v1/info` answers. A token-only app state is `Null`.
async fn probe(base: String, wait: Duration) -> Option<(String, Value)> {
    match fetch(&base, "GET", "/v1/info", wait).await? {
        (200..=299, _) => {}
        _ => return None,
    }
    // `getDesktopAppStatus`.
    let state = match fetch(&base, "GET", "/v1/app/setup", Duration::from_secs(2)).await? {
        (401 | 403 | 404, _) => Value::Null,
        (200..=299, body) => serde_json::from_slice(&body).ok()?,
        _ => return None,
    };
    Some((base, state))
}

/// `findLocalDesktop`: `preferred`, else with `scan` any of ports 23373 to 23392 on 127.0.0.1 and localhost that
/// answers first, with its app state.
pub fn find_local_desktop(c: &Ctx, preferred: &str, scan: bool, wait: Duration) -> Res<(String, Value)> {
    let mut candidates = vec![preferred.to_string()];
    let ports = (DEFAULT_PORT..DEFAULT_PORT + 20).filter(|_| scan);
    for url in ports.flat_map(|p| ["127.0.0.1", "localhost"].map(|h| format!("http://{h}:{p}"))) {
        if !candidates.contains(&url) {
            candidates.push(url);
        }
    }
    let rest = candidates[1..].to_vec();
    let found = c.block(async move {
        if let Some(found) = probe(preferred.to_string(), wait).await {
            return Some(found);
        }
        let mut probes = tokio::task::JoinSet::new();
        rest.into_iter().for_each(|url| drop(probes.spawn(probe(url, wait))));
        while let Some(done) = probes.join_next().await {
            if let Ok(Some(found)) = done {
                return Some(found);
            }
        }
        None
    });
    let msg = || format!("Could not find a running Beeper Desktop API on {}.", candidates.join(", "));
    found.ok_or_else(|| CliError::abort(msg(), 4))
}

/// `ensureDesktopToken`: a signed-out Desktop stops with exit 3, else the browser sign-in gives a token, which `save`
/// keeps in the config.
pub fn ensure_desktop_token(c: &Ctx, base: &str, scan: bool, save: bool) -> Res<String> {
    let (base, state) = find_local_desktop(c, base, scan, Duration::from_millis(500))?;
    if state["state"] == "needs-login" {
        let msg = "Beeper Desktop is not signed in. Open Beeper Desktop and sign in, then rerun this command.";
        return Err(CliError::abort(msg, 3));
    }
    let token = login_with_pkce(c, &base, save, "desktop-oauth")?;
    Ok(token["access_token"].as_str().unwrap_or_default().to_string())
}

/// `URLSearchParams.toString`: the pairs, form-encoded with `+` for a space.
fn form_encode(pairs: &[(&str, &str)]) -> String {
    let pairs = pairs.iter().map(|(k, v)| format!("{k}={}", url_encode(v).replace("%20", "+")));
    pairs.collect::<Vec<_>>().join("&")
}

/// The base and the path of an absolute URL, for [`BeeperClient`].
fn split_url(url: &str) -> (String, String) {
    let scheme = url.find("://").map_or(0, |i| i + 3);
    let at = url[scheme..].find('/').map_or(url.len(), |i| scheme + i);
    let path = if at == url.len() { "/" } else { &url[at..] };
    (url[..at].to_string(), path.to_string())
}

/// `loginWithPKCE`: registers the CLI as a public client, opens the authorization page in the browser, waits for
/// its redirect to a local callback, and trades the code for a token. With `save` the config keeps the token as the
/// auth of `base`. The token has the `clientID` too.
pub fn login_with_pkce(c: &Ctx, base: &str, save: bool, source: &str) -> Res<Value> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").map_err(|e| CliError::bug(e.to_string()))?;
    let redirect = format!("http://127.0.0.1:{}/callback", listener.local_addr().expect("a bound address").port());
    let callback = callback_server(listener);
    let client = BeeperClient::try_new(base, "").map_err(CliError::bug)?;
    let body = json!({
        "client_name": CLIENT_NAME,
        "grant_types": ["authorization_code"],
        "response_types": ["code"],
        "redirect_uris": [redirect],
        "scope": SCOPE,
        "token_endpoint_auth_method": "none",
    });
    let (status, reply) = c.block(client.fetch("POST", "/oauth/register", Some(&body)))?;
    if status >= 300 {
        let text = String::from_utf8_lossy(&reply);
        return Err(CliError::bug(format!("OAuth client registration failed: {status} {text}")));
    }
    let registered: Value = serde_json::from_slice(&reply).map_err(|e| CliError::bug(e.to_string()))?;
    let client_id = registered["client_id"].as_str().unwrap_or_default().to_string();
    let verifier = base64_encode_url_nopad(&beeper_core::connect::random::<64>());
    let challenge = base64_encode_url_nopad(&sha256(verifier.as_bytes()));
    let state = base64_encode_url_nopad(&beeper_core::connect::random::<24>());
    // `new URL(endpoint ?? path, base)`.
    let endpoint = |key: &str, path: &str| match registered[key].as_str().unwrap_or(path) {
        url if url.starts_with('/') => format!("{base}{url}"),
        url => url.to_string(),
    };
    let params = [
        ("client_id", client_id.as_str()),
        ("redirect_uri", &redirect),
        ("response_type", "code"),
        ("scope", SCOPE),
        ("state", &state),
        ("code_challenge", &challenge),
        ("code_challenge_method", "S256"),
    ];
    let query = form_encode(&params);
    open_external(&format!("{}?{}", endpoint("authorization_endpoint", "/oauth/authorize"), query));
    let reply =
        callback.recv_timeout(CALLBACK_WAIT).map_err(|_| CliError::bug("Timed out waiting for OAuth callback."))?;
    let param = |k: &str| reply.iter().find(|(name, _)| name == k).map(|(_, v)| v.as_str());
    if param("state") != Some(state.as_str()) {
        return Err(CliError::bug("OAuth state mismatch."));
    }
    if let Some(error) = param("error") {
        return Err(CliError::bug(format!("OAuth authorization failed: {error}")));
    }
    let code = param("code").ok_or_else(|| CliError::bug("OAuth callback did not include an authorization code."))?;
    let form = [
        ("grant_type", "authorization_code"),
        ("client_id", client_id.as_str()),
        ("code", code),
        ("code_verifier", &verifier),
        ("redirect_uri", &redirect),
    ];
    let form = form_encode(&form);
    let (token_base, token_path) = split_url(&endpoint("token_endpoint", "/oauth/token"));
    let token_client = BeeperClient::try_new(&token_base, "").map_err(CliError::bug)?;
    let body = Some(("application/x-www-form-urlencoded".to_string(), form.into_bytes()));
    let (status, reply) = c.block(token_client.request_raw("POST", &token_path, &[], body))?;
    if status >= 300 {
        let text = String::from_utf8_lossy(&reply);
        return Err(CliError::bug(format!("OAuth token exchange failed: {status} {text}")));
    }
    let mut token: Value = serde_json::from_slice(&reply).map_err(|e| CliError::bug(e.to_string()))?;
    if save {
        let auth = obj([
            ("accessToken", token["access_token"].clone()),
            ("clientID", json!(client_id)),
            (
                "expiresAt",
                token["expires_in"].as_i64().filter(|s| *s != 0).map_or(Value::Null, |s| json!(iso_in(s * 1000))),
            ),
            ("scope", token["scope"].clone()),
            ("source", json!(source)),
            ("tokenType", token["token_type"].clone()),
        ]);
        update_config(|config| {
            config.insert("baseURL".into(), json!(base));
            config.insert("auth".into(), Value::Object(auth));
        })?;
    }
    token["clientID"] = json!(client_id);
    Ok(token)
}

/// `openExternal`: the browser of the system opens `url`. Nothing waits for the opener.
fn open_external(url: &str) {
    let (program, args): (&str, Vec<&str>) = match std::env::consts::OS {
        "macos" => ("open", vec![url]),
        "windows" => ("cmd", vec!["/c", "start", "", url]),
        _ => ("xdg-open", vec![url]),
    };
    let quiet = std::process::Stdio::null;
    drop(std::process::Command::new(program).args(args).stdin(quiet()).stdout(quiet()).stderr(quiet()).spawn());
}

/// `createCallbackServer`: answers the redirect to `/callback` with a page to close, and passes on its query.
fn callback_server(listener: std::net::TcpListener) -> std::sync::mpsc::Receiver<Vec<(String, String)>> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let mut line = String::new();
            if std::io::BufReader::new(&stream).read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split(' ').nth(1).unwrap_or("/");
            let (path, query) = target.split_once('?').unwrap_or((target, ""));
            if path != "/callback" {
                drop(
                    stream.write_all(
                        b"HTTP/1.1 404 Not Found\r\ncontent-length: 9\r\nconnection: close\r\n\r\nNot found",
                    ),
                );
                continue;
            }
            let page =
                "<!doctype html><title>Beeper CLI</title><p>You can close this tab and return to the terminal.</p>";
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                page.len()
            );
            drop(stream.write_all(format!("{head}{page}").as_bytes()));
            let pairs = query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').unwrap_or((p, "")));
            let pairs = pairs.map(|(k, v)| (url_decode(k).unwrap_or_default(), url_decode(v).unwrap_or_default()));
            if tx.send(pairs.collect()).is_err() {
                return;
            }
        }
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_gives_the_base_and_the_path() {
        let pair = |a: &str, b: &str| (a.to_string(), b.to_string());
        assert_eq!(split_url("http://localhost:9/oauth/token"), pair("http://localhost:9", "/oauth/token"));
        assert_eq!(split_url("http://localhost:9"), pair("http://localhost:9", "/"));
    }
}
