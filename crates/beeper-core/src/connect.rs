//! Desktop's `BeeperConnectAuth`: the access tokens of the Desktop API, its OAuth clients and codes, and the token
//! requests that wait for the user in the "Approve connection" dialog.
use crate::codec::base64_encode_url_nopad;
use ring::digest::{digest, SHA256};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use tokio::sync::oneshot;

/// How long a token request waits for the user.
pub const APPROVAL: std::time::Duration = std::time::Duration::from_secs(120);
const CODE_MS: i64 = 300_000;
/// Tokens that expired more than 30 days ago are dropped.
const KEEP_EXPIRED_MS: i64 = 30 * 86_400_000;
pub const KEY_TOKENS: &str = "conn:tokens";
pub const KEY_CLIENTS: &str = "conn:clients";
pub const READ: &str = "read";
pub const WRITE: &str = "write";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClientInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "clientID", default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
    #[serde(rename = "clientURI", default, skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_address: Option<String>,
}

/// A stored access token; only the SHA-256 of the token itself is kept.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Token {
    pub id: String,
    pub token_hash: String,
    pub created_at: i64,
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_info: Option<ClientInfo>,
}

/// An OAuth client from dynamic client registration.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    #[serde(rename = "clientID")]
    pub client_id: String,
    pub client_name: String,
    #[serde(rename = "clientURI", default, skip_serializing_if = "Option::is_none")]
    pub client_uri: Option<String>,
    #[serde(rename = "redirectURIs")]
    pub redirect_uris: Vec<String>,
    pub scope: String,
    pub created_at: i64,
}

/// A token request that waits for the user.
pub struct Request {
    pub id: String,
    pub client: ClientInfo,
    pub scopes: Vec<String>,
    redirect_uri: String,
    waiters: Vec<oneshot::Sender<Option<String>>>,
}

/// The end of a token request, for the "Approve connection" dialog.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Outcome {
    Approved,
    Rejected,
    Timeout,
}

struct Code {
    token: String,
    client_id: String,
    challenge: String,
    resource: Option<String>,
    created_at: i64,
}

#[derive(Default)]
pub struct Connect {
    pub tokens: BTreeMap<String, Token>,
    pub clients: HashMap<String, Client>,
    pub requests: Vec<Request>,
    codes: HashMap<String, Code>,
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("clock").as_millis() as i64
}

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    SystemRandom::new().fill(&mut bytes).expect("system random bytes");
    bytes
}

/// Desktop's 21-character `nanoid`.
pub fn random_id() -> String {
    const ALPHABET: &[u8; 64] = b"useandom-26T198340PX75pxJACKVERYMINDBUSHWOLF_GQZbfghjklqvwyzrict";
    random::<21>().iter().map(|b| ALPHABET[usize::from(b & 63)] as char).collect()
}

pub fn hash(token: &str) -> String {
    digest(&SHA256, token.as_bytes()).as_ref().iter().map(|b| format!("{b:02x}")).collect()
}

/// The PKCE S256 challenge of a code verifier.
pub fn s256(verifier: &str) -> String {
    base64_encode_url_nopad(digest(&SHA256, verifier.as_bytes()).as_ref())
}

impl Connect {
    pub fn load(tokens: Option<String>, clients: Option<String>) -> Self {
        fn parse<T: Default + serde::de::DeserializeOwned>(json: Option<String>) -> T {
            json.map_or_else(T::default, |j| serde_json::from_str(&j).expect("parse stored Desktop API data"))
        }
        Self { tokens: parse(tokens), clients: parse(clients), ..Self::default() }
    }

    pub fn tokens_json(&mut self) -> String {
        let cutoff = now_ms() - KEEP_EXPIRED_MS;
        self.tokens.retain(|_, t| t.expires_at.is_none_or(|e| e >= cutoff));
        serde_json::to_string(&self.tokens).unwrap()
    }

    pub fn clients_json(&self) -> String {
        serde_json::to_string(&self.clients).unwrap()
    }

    /// Stores a new token, which expires after `expires_secs` unless it is 0, and returns the token.
    pub fn create(&mut self, scopes: Vec<String>, expires_secs: i64, client: Option<ClientInfo>) -> String {
        let token = format!("bdapi_{}", base64_encode_url_nopad(&random::<32>()));
        let now = now_ms();
        let id = random_id();
        let expires_at = (expires_secs > 0).then(|| now + expires_secs * 1000);
        let entry = Token {
            id: id.clone(),
            token_hash: hash(&token),
            created_at: now,
            scopes,
            expires_at,
            client_info: client,
        };
        self.tokens.insert(id, entry);
        token
    }

    /// Desktop's `validateToken`: the token, or the message of the 401. The Matrix access token of the session is
    /// valid with all scopes.
    pub fn validate(&self, token: &str, session: &str, user_id: &str) -> Result<Token, &'static str> {
        if !session.is_empty() && hash(token) == hash(session) {
            let client = ClientInfo {
                name: Some("Beeper Matrix session".into()),
                client_id: Some("beeper-matrix-session".into()),
                ..ClientInfo::default()
            };
            let scopes = vec![READ.into(), WRITE.into()];
            let id = if user_id.is_empty() { "matrix" } else { user_id }.into();
            return Ok(Token {
                id,
                token_hash: hash(token),
                created_at: now_ms(),
                scopes,
                expires_at: None,
                client_info: Some(client),
            });
        }
        let hashed = hash(token);
        let found = self.tokens.values().find(|t| t.token_hash == hashed).ok_or("Invalid token")?;
        match found.expires_at {
            Some(e) if e < now_ms() => Err("Token expired"),
            _ => Ok(found.clone()),
        }
    }

    pub fn revoke_access(&mut self, token: &str) {
        let hashed = hash(token);
        self.tokens.retain(|_, t| t.token_hash != hashed);
    }

    /// Adds a token request, or joins the request of the same client, redirect and scopes. The receiver gets the
    /// token after an approval, `None` after a rejection, and an error when the request ends without an answer.
    pub fn request(
        &mut self,
        client: ClientInfo,
        scopes: Vec<String>,
        redirect_uri: &str,
    ) -> (String, bool, oneshot::Receiver<Option<String>>) {
        let (tx, rx) = oneshot::channel();
        let same = |r: &&mut Request| {
            r.client.client_id.is_some()
                && r.client.client_id == client.client_id
                && r.redirect_uri == redirect_uri
                && r.scopes == scopes
        };
        if let Some(r) = self.requests.iter_mut().find(same) {
            r.waiters.push(tx);
            return (r.id.clone(), false, rx);
        }
        let id = random_id();
        self.requests.push(Request {
            id: id.clone(),
            client,
            scopes,
            redirect_uri: redirect_uri.into(),
            waiters: vec![tx],
        });
        (id, true, rx)
    }

    /// Ends a token request: with the scopes and expiry of an approval, or a rejection. Returns `false` when the
    /// request already ended.
    pub fn answer(&mut self, id: &str, grant: Option<(Vec<String>, i64)>) -> bool {
        let Some(i) = self.requests.iter().position(|r| r.id == id) else { return false };
        let r = self.requests.remove(i);
        let token = grant.map(|(scopes, secs)| self.create(scopes, secs, Some(r.client)));
        for w in r.waiters {
            w.send(token.clone()).ok();
        }
        true
    }

    /// Drops a token request without an answer. Returns `false` when the request already ended.
    pub fn expire(&mut self, id: &str) -> bool {
        let before = self.requests.len();
        self.requests.retain(|r| r.id != id);
        self.requests.len() < before
    }

    pub fn store_code(
        &mut self,
        token: String,
        client_id: String,
        challenge: String,
        resource: Option<String>,
    ) -> String {
        let code = random_id();
        self.codes.insert(code.clone(), Code { token, client_id, challenge, resource, created_at: now_ms() });
        code
    }

    /// Desktop's `handleOAuth2TokenRequest` for a code: the token and its stored entry, or the OAuth error code and
    /// description.
    pub fn exchange(
        &mut self,
        code: &str,
        verifier: &str,
        client_id: Option<&str>,
        resource: Option<&str>,
    ) -> Result<(String, Token), (&'static str, &'static str)> {
        let now = now_ms();
        self.codes.retain(|_, c| c.created_at + CODE_MS >= now);
        let invalid = ("invalid_grant", "Invalid or expired authorization code");
        let c = self.codes.get(code).ok_or(invalid)?;
        if s256(verifier) != c.challenge {
            return Err(invalid);
        }
        let c = self.codes.remove(code).expect("checked code");
        if client_id != Some(c.client_id.as_str()) {
            return Err(("invalid_grant", "Client mismatch"));
        }
        if resource.zip(c.resource.as_deref()).is_some_and(|(a, b)| a != b) {
            return Err(("invalid_grant", "Resource parameter mismatch"));
        }
        let hashed = hash(&c.token);
        let entry = self
            .tokens
            .values()
            .find(|t| t.token_hash == hashed)
            .ok_or(("server_error", "Failed to retrieve token details"))?;
        Ok((c.token, entry.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_like_desktop() {
        let mut c = Connect::default();
        let token = c.create(vec![READ.into()], 0, None);
        assert!(token.starts_with("bdapi_") && token.len() == 49, "bdapi_ and 32 bytes in base64url");
        assert_eq!(c.validate(&token, "session", "").unwrap().scopes, [READ]);
        assert_eq!(c.validate("session", "session", "@me:beeper.com").unwrap().scopes, [READ, WRITE]);
        assert_eq!(c.validate("nope", "", "").unwrap_err(), "Invalid token");
        let old = c.create(vec![READ.into()], 1, None);
        c.tokens.values_mut().for_each(|t| t.expires_at = t.expires_at.map(|_| now_ms() - 1));
        assert_eq!(c.validate(&old, "", "").unwrap_err(), "Token expired");
        assert!(!c.tokens_json().contains("bdapi_"), "only hashes are stored");
        assert_eq!(random_id().len(), 21);
        assert_eq!(s256("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[test]
    fn requests_merge_and_end() {
        let mut c = Connect::default();
        let client = ClientInfo { client_id: Some("c".into()), ..ClientInfo::default() };
        let (id, new, mut a) = c.request(client.clone(), vec![READ.into()], "http://127.0.0.1/cb");
        let (same, again, mut b) = c.request(client, vec![READ.into()], "http://127.0.0.1/cb");
        assert!(new && !again && id == same && c.requests.len() == 1);
        assert!(c.answer(&id, Some((vec![READ.into()], 3600))));
        let token = a.try_recv().unwrap().expect("approved");
        assert_eq!(b.try_recv().unwrap(), Some(token.clone()));
        let code = c.store_code(token.clone(), "c".into(), s256("verifier"), None);
        assert_eq!(c.exchange(&code, "wrong", Some("c"), None).unwrap_err().0, "invalid_grant");
        assert_eq!(c.exchange(&code, "verifier", Some("c"), None).unwrap().0, token);
        assert!(c.exchange(&code, "verifier", Some("c"), None).is_err(), "a code works once");
        let (id, _, mut rx) = c.request(ClientInfo::default(), vec![READ.into()], "x:/");
        assert!(c.expire(&id) && rx.try_recv().is_err() && !c.answer(&id, None));
    }
}
