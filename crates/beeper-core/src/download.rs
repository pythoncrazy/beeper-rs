//! Desktop's media download: the media manager's `getAsset` over its SDK's `DownloadFileWithContext`. It asks the
//! authenticated `/_matrix/client/v1/media/download` endpoint whatever the homeserver's `/versions` says, with
//! `com.beeper.interactive_download_request`, follows redirects like Go's `http.Client`, fails a download that
//! stalls instead of one that takes long, and tries 3 more times before it fails.

use matrix_sdk::ruma::events::room::MediaSource;
use matrix_sdk::ruma::{MxcUri, OwnedMxcUri};
use reqwest::{StatusCode, Url};
use serde_json::Value;
use std::time::Duration;

/// `getAsset`'s `pRetry` with `retries: 3`: 4 tries, 1, 2 and 4 seconds apart.
pub(crate) const TRIES: u32 = 4;
/// The SDK's `StallTransport` with `UploadWriteTimeout`: a download fails when no byte comes for 60 seconds.
pub(crate) const STALL: Duration = Duration::from_secs(60);
/// Go's `http.Client` follows at most 10 redirects.
const REDIRECTS: usize = 10;
/// The error code of the SDK's `MEDIA_NOT_FOUND`, which `getAsset` does not try again, like a reload.
const NOT_FOUND: &str = "M_NOT_FOUND";

/// Why a try failed: what [`super::matrix::media`] logs and gives the view, and whether `getAsset` tries again.
#[derive(Debug)]
pub(crate) struct Failure {
    pub error: String,
    pub again: bool,
    /// The `retry_after_ms` of a 429.
    pub wait: Option<Duration>,
}

impl Failure {
    fn new(error: impl Into<String>) -> Self {
        Self { error: error.into(), again: true, wait: None }
    }
}

/// The HTTP client of media downloads: it follows no redirect itself, and gives up on a read that waits [`STALL`].
fn client() -> &'static reqwest::Client {
    static HTTP: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    HTTP.get_or_init(|| {
        let builder = crate::net::builder().redirect(reqwest::redirect::Policy::none());
        crate::net::client(
            builder.connect_timeout(Duration::from_secs(20)).read_timeout(STALL).user_agent("matrix-rust-sdk"),
        )
    })
}

/// The media of a download and how it decrypts.
#[derive(Clone, Debug)]
pub(crate) enum Source {
    /// Plain media, or media with the `v2` AES-CTR encryption of the spec.
    Matrix(MediaSource),
    /// Media with the `com.beeper.floe.v1` encryption of MSC4571, which matrix-sdk does not know.
    Floe(OwnedMxcUri, crate::floe::File),
}

impl Source {
    /// The `mxc://` URI of the media, without the keys of an encrypted one, for the log.
    pub(crate) fn uri(&self) -> &MxcUri {
        match self {
            Self::Matrix(MediaSource::Plain(uri)) | Self::Floe(uri, _) => uri,
            Self::Matrix(MediaSource::Encrypted(file)) => &file.url,
        }
    }

    pub(crate) fn encrypted(&self) -> bool {
        !matches!(self, Self::Matrix(MediaSource::Plain(_)))
    }

    /// The key of the media cache, which matrix-sdk makes of the URI alone, and where the bytes are decrypted.
    pub(crate) fn cache_key(&self) -> MediaSource {
        match self {
            Self::Matrix(source) => source.clone(),
            Self::Floe(uri, _) => MediaSource::Plain(uri.clone()),
        }
    }
}

/// The bytes of a body as they download: kept as they come, or decrypted a FLOE segment at a time.
enum Sink {
    Whole(Vec<u8>),
    Floe(Box<crate::floe::Stream>, Vec<u8>),
}

impl Sink {
    /// A FLOE stream checks its key and header before the download, and a wrong one is not tried again.
    fn new(source: &Source) -> Result<Self, Failure> {
        match source {
            Source::Matrix(_) => Ok(Self::Whole(Vec::new())),
            Source::Floe(_, file) => crate::floe::Stream::new(file)
                .map(|s| Self::Floe(Box::new(s), Vec::new()))
                .map_err(|e| Failure { again: false, ..Failure::new(format!("decrypt: {e}")) }),
        }
    }

    fn push(&mut self, chunk: &[u8]) -> Result<(), String> {
        match self {
            Self::Whole(bytes) => {
                bytes.extend_from_slice(chunk);
                Ok(())
            }
            Self::Floe(stream, plain) => stream.push(chunk, plain),
        }
    }

    /// The bytes of the media, decrypted when it is encrypted.
    fn finish(self, source: &Source) -> Result<Vec<u8>, String> {
        match (self, source) {
            (Self::Floe(stream, mut plain), _) => (*stream).finish(&mut plain).map(|()| plain),
            (Self::Whole(bytes), Source::Matrix(MediaSource::Encrypted(file))) => {
                let mut cursor = std::io::Cursor::new(bytes);
                let mut plain = Vec::new();
                matrix_sdk_crypto::AttachmentDecryptor::new(&mut cursor, file.as_ref().clone().into())
                    .map_err(|e| e.to_string())
                    .and_then(|mut r| std::io::Read::read_to_end(&mut r, &mut plain).map_err(|e| e.to_string()))?;
                Ok(plain)
            }
            (Self::Whole(bytes), _) => Ok(bytes),
        }
    }
}

/// The download URL of the SDK's `DownloadFileWithContext`: `/_matrix/client/v1/media/download/{server}/{id}`, with
/// `com.beeper.interactive_download_request` set to `true` or `false` like the media manager's `downloadMedia`.
pub(crate) fn url(homeserver: &Url, uri: &MxcUri, interactive: bool) -> Result<Url, Failure> {
    let (server, id) =
        uri.parts().map_err(|e| Failure { again: false, ..Failure::new(format!("bad MXC URI: {e}")) })?;
    let mut url = homeserver.clone();
    let path = ["_matrix", "client", "v1", "media", "download", server.as_str(), id];
    url.path_segments_mut().expect("a base URL").pop_if_empty().extend(path);
    url.query_pairs_mut().append_pair("com.beeper.interactive_download_request", &interactive.to_string());
    Ok(url)
}

/// Go's `shouldCopyHeaderOnRedirect`: a redirect keeps the token on the host of the first request and its subdomains.
fn keeps_token(first: &Url, next: &Url) -> bool {
    match (first.host_str(), next.host_str()) {
        (Some(a), Some(b)) => a == b || b.strip_suffix(a).is_some_and(|sub| sub.ends_with('.')),
        _ => false,
    }
}

/// The kind of a failed request, without its URL, whose query can hold a signature.
fn network(e: reqwest::Error) -> Failure {
    let kind = match () {
        _ if e.is_timeout() => "stalled",
        _ if e.is_connect() => "connect",
        _ if e.is_body() || e.is_decode() => "body",
        _ => "request",
    };
    let detail = e.without_url().to_string();
    match kind {
        "stalled" => Failure::new(format!("stalled: no data for {} s ({detail})", STALL.as_secs())),
        _ => Failure::new(format!("network ({kind}): {detail}")),
    }
}

/// One try of the download of `source`: its bytes, decrypted when it is encrypted. A FLOE body decrypts a segment at
/// a time as it downloads, like the SDK's `FLOEStream`.
pub(crate) async fn once(
    homeserver: &Url, token: &str, source: &Source, interactive: bool,
) -> Result<Vec<u8>, Failure> {
    let mut sink = Sink::new(source)?;
    let first = url(homeserver, source.uri(), interactive)?;
    let (mut at, mut trail) = (first.clone(), Vec::new());
    let resp = loop {
        let mut req = client().get(at.clone());
        if keeps_token(&first, &at) {
            req = req.bearer_auth(token);
        }
        let resp = req.send().await.map_err(network)?;
        let location = resp.headers().get(reqwest::header::LOCATION).and_then(|l| l.to_str().ok());
        let Some(next) = location.filter(|_| resp.status().is_redirection()) else { break resp };
        if trail.len() == REDIRECTS {
            return Err(Failure::new(format!("stopped after {REDIRECTS} redirects")));
        }
        let next = at.join(next).map_err(|e| Failure::new(format!("bad redirect: {e}")))?;
        trail.push(format!("{} to {}", resp.status().as_u16(), next.host_str().unwrap_or("?")));
        at = next;
    };
    let via = match trail.is_empty() {
        true => String::new(),
        false => format!(" (after {})", trail.join(", ")),
    };
    let status = resp.status();
    let broken = |e: reqwest::Error| {
        let f = network(e);
        Failure { error: format!("{}{via}", f.error), ..f }
    };
    if !status.is_success() {
        return Err(refused(status, &resp.bytes().await.map_err(broken)?, &via));
    }
    let decrypt = |e: String| Failure::new(format!("decrypt: {e}{via}"));
    let mut resp = resp;
    while let Some(chunk) = resp.chunk().await.map_err(broken)? {
        sink.push(&chunk).map_err(decrypt)?;
    }
    sink.finish(source).map_err(decrypt)
}

/// The failure of an answer that is not a success: its Matrix error, which `getAsset` does not try again when the
/// bridge needs the user to ask for the media or the media is gone.
fn refused(status: StatusCode, body: &[u8], via: &str) -> Failure {
    let json: Value = serde_json::from_slice(body).unwrap_or_default();
    let code = json["errcode"].as_str();
    let error = match code {
        Some(code) => format!("{code} (HTTP {}): {}{via}", status.as_u16(), json["error"].as_str().unwrap_or_default()),
        None => format!("HTTP {status}{via}"),
    };
    let again = !(code == Some(crate::matrix::MEDIA_RELOAD_NEEDED) || status == StatusCode::NOT_FOUND)
        && code != Some(NOT_FOUND);
    let wait = json["retry_after_ms"].as_u64().map(Duration::from_millis);
    Failure { error, again, wait }
}

/// `getAsset`: the download of `source`, tried again 1, 2 and 4 seconds after a failure that can pass. `tried`
/// hears of each failure that is tried again.
pub(crate) async fn download(
    homeserver: &Url, token: &str, source: &Source, interactive: bool, mut tried: impl FnMut(u32, &str),
) -> Result<Vec<u8>, String> {
    let mut n = 1;
    loop {
        match once(homeserver, token, source, interactive).await {
            Ok(bytes) => return Ok(bytes),
            Err(f) if !f.again || n == TRIES => return Err(f.error),
            Err(f) => {
                tried(n, &f.error);
                let backoff = Duration::from_secs(1 << (n - 1));
                // A rate limit waits as long as it asks, up to a minute.
                let wait = f.wait.map_or(backoff, |w| w.clamp(backoff, Duration::from_secs(60)));
                tokio::time::sleep(wait).await;
                n += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_redirect_keeps_the_token_on_the_host_and_its_subdomains_like_go() {
        let u = |s: &str| Url::parse(s).expect("a URL");
        let first = u("https://matrix.beeper.com/_matrix/client/v1/media/download/x/y");
        assert!(keeps_token(&first, &u("https://matrix.beeper.com/other")));
        assert!(keeps_token(&first, &u("https://media.matrix.beeper.com/blob")));
        assert!(!keeps_token(&first, &u("https://cdn.example.com/blob")), "another host");
        assert!(!keeps_token(&first, &u("https://evilmatrix.beeper.com/blob")), "not a subdomain");
        assert!(!keeps_token(&first, &u("https://beeper.com/blob")), "the parent domain");
    }

    #[test]
    fn the_download_url_is_the_authenticated_one_of_desktop() {
        let hs = Url::parse("https://matrix.beeper.com").expect("a URL");
        let mxc: &MxcUri = "mxc://local.beeper.com/me_instagramgo_abc".into();
        assert_eq!(
            url(&hs, mxc, false).expect("a URL").as_str(),
            "https://matrix.beeper.com/_matrix/client/v1/media/download/local.beeper.com/me_instagramgo_abc\
             ?com.beeper.interactive_download_request=false"
        );
        assert!(url(&hs, mxc, true).expect("a URL").as_str().ends_with("interactive_download_request=true"));
    }

    #[test]
    fn only_a_reload_or_missing_media_is_not_tried_again() {
        let f = |status: u16, body: &str| refused(StatusCode::from_u16(status).expect("a status"), body.as_bytes(), "");
        let reload = format!(r#"{{"errcode":"{}","error":"on the phone"}}"#, crate::matrix::MEDIA_RELOAD_NEEDED);
        assert!(!f(400, &reload).again, "a reload waits for the user");
        assert!(!f(404, r#"{"errcode":"M_NOT_FOUND","error":"gone"}"#).again, "missing media");
        assert!(!f(404, "").again, "a 404 without a Matrix error");
        assert!(f(502, "<html>bad gateway</html>").again);
        assert!(f(504, r#"{"errcode":"M_NOT_YET_UPLOADED","error":"later"}"#).again, "an async upload");
        let limited = f(429, r#"{"errcode":"M_LIMIT_EXCEEDED","retry_after_ms":2500}"#);
        assert!(limited.again && limited.wait == Some(Duration::from_millis(2500)));
        assert_eq!(f(502, "").error, "HTTP 502 Bad Gateway");
    }
}
