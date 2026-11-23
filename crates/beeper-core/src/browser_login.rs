//! Desktop's `BrowserAuth` window (`Am` of main-entry): the login page of a bridgev2 `cookies` step in a browser
//! with a fresh profile, which reads the cookies, local storage items, request headers and script values of the
//! step's fields. beeper-rs drives an installed Chromium-family browser over the Chrome DevTools Protocol on a pipe.
use crate::bridge_login::{CookieField, Cookies};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot, watch};

mod scripts;

/// Desktop's `Fi`: the global that the `extract_js` wrapper sets.
const RESULTS: &str = "__BEEP_BEEP_AUTH_RESULTS__";
/// Desktop's `Eb`: how long a hidden login runs.
const HIDDEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long the browser has to close before it is killed.
const CLOSE_WAIT: Duration = Duration::from_secs(5);
/// The browsers that `find_browser` looks for on the `PATH`, in order.
const BROWSERS: [&str; 10] = [
    "chromium",
    "chromium-browser",
    "google-chrome-stable",
    "google-chrome",
    "brave-browser",
    "brave",
    "microsoft-edge-stable",
    "microsoft-edge",
    "vivaldi-stable",
    "vivaldi",
];

/// The user agent of the login window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Agent {
    /// Desktop's `USER_AGENT` (`Qg`), for a step without one: Chrome's user agent on this OS.
    Desktop,
    /// Desktop's `GOOGLE_SAFE_USER_AGENT` (`vr`): the Chrome user agent with `Beeper/3.110.1`.
    Google,
    /// The browser's own user agent, for a step with an empty one.
    Browser,
    Custom(String),
}

impl Agent {
    /// The user agent for the browser's Chrome version, or `None` for the browser's own.
    fn string(&self, chrome: &str) -> Option<String> {
        let os = match () {
            _ if cfg!(target_os = "macos") => "Macintosh; Intel Mac OS X 10_15_7",
            _ if cfg!(windows) => "Windows NT 10.0; Win64; x64",
            _ => "X11; Linux x86_64",
        };
        let beeper = if *self == Agent::Google { " Beeper/3.110.1" } else { "" };
        match self {
            Agent::Desktop | Agent::Google => Some(format!(
                "Mozilla/5.0 ({os}) AppleWebKit/537.36 (KHTML, like Gecko){beeper} Chrome/{chrome}.0.0.0 Safari/537.36"
            )),
            Agent::Browser => None,
            Agent::Custom(ua) => Some(ua.clone()),
        }
    }
}

/// Desktop's `BrowserLoginOptions` of a step: what the window loads, runs and reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BrowserLogin {
    pub url: String,
    pub agent: Agent,
    /// Desktop's `expectedFinalURLRegex`.
    pub final_url: String,
    pub fields: Vec<CookieField>,
    pub hidden: bool,
    pub launch_js: Option<String>,
    pub navigate_js: Option<String>,
    /// The script whose result has the values of the `special` fields.
    pub close_js: Option<String>,
    /// Desktop's `blockedRequestURLs`: the window blocks the requests whose URL has one of these.
    pub blocked: Vec<String>,
}

/// Desktop's `IR`: the `extract_js` of a step, which keeps its result in a global.
fn extract_js(js: &str) -> String {
    let js = js.replace("\\n", "\n").replace("\\t", "\t");
    format!(
        r#";(async () => {{
  try {{
    console.log('{RESULTS}: Extracting credentials')
    const result = await ({})
    if (!result) {{
      console.error('{RESULTS}: No result from extract_js')
      return
    }}
    console.log('{RESULTS}: Extracted credentials')
    globalThis.{RESULTS} = result
  }} catch (e) {{
    console.error('{RESULTS}: Error extracting credentials', e)
  }}
}})()"#,
        js.trim()
    )
}

impl BrowserLogin {
    /// Desktop's `getBrowserLoginFromStep` with the `mapBrowserLogin` of the platform of `network`, for the login
    /// flow `flow`.
    pub fn new(c: &Cookies, network: &str, flow: &str) -> Self {
        let extract = (!c.extract_js.is_empty()).then(|| extract_js(&c.extract_js));
        let mut l = BrowserLogin {
            url: c.url.clone(),
            agent: match c.user_agent.as_deref() {
                None => Agent::Desktop,
                Some("") => Agent::Browser,
                Some(ua) => Agent::Custom(ua.into()),
            },
            final_url: c.wait_for_url_pattern.clone(),
            fields: c.fields.clone(),
            hidden: c.hidden,
            launch_js: extract.clone(),
            navigate_js: extract.clone(),
            close_js: extract.map(|_| format!("globalThis.{RESULTS}")),
            blocked: Vec::new(),
        };
        // Desktop's `??` keeps the values of the step.
        let or = |js: &mut Option<String>, default: &str| {
            js.get_or_insert_with(|| default.into());
        };
        let agent_or = |agent: &mut Agent, default: Agent| {
            if *agent == Agent::Desktop {
                *agent = default;
            }
        };
        match network {
            "twitter" => {
                or(&mut l.launch_js, scripts::TWITTER);
                or(&mut l.navigate_js, scripts::TWITTER);
            }
            "slack" => {
                l.url = "https://slack.com/signin".into();
                let navigate = l.navigate_js.unwrap_or_default();
                l.navigate_js = Some(format!("\n    {}\n    {navigate}\n    ", scripts::SLACK));
                l.launch_js = Some(scripts::SLACK.into());
            }
            "facebook" => {
                let js = if flow == "messenger" { scripts::MESSENGER } else { scripts::FACEBOOK };
                let google = reqwest::Url::parse(&l.url).is_ok_and(|u| u.host_str() == Some("accounts.google.com"));
                if google {
                    agent_or(&mut l.agent, Agent::Google);
                }
                or(&mut l.launch_js, js);
                or(&mut l.navigate_js, js);
            }
            "instagram" => {
                or(&mut l.launch_js, scripts::INSTAGRAM);
                or(&mut l.navigate_js, scripts::INSTAGRAM);
                or(&mut l.close_js, "successfulLogin");
            }
            "linkedin" => {
                l.url = "https://www.linkedin.com/login".into();
                let with = |js: Option<String>| Some(format!("{}\n{}", js.unwrap_or_default(), scripts::LINKEDIN));
                (l.navigate_js, l.launch_js) = (with(l.navigate_js), with(l.launch_js));
                l.blocked = vec!["checkpoint/pk/initiateLogin".into()];
            }
            "gmessages" => {
                agent_or(&mut l.agent, Agent::Google);
                l.url = "https://accounts.google.com/AccountChooser?continue=https://messages.google.com/web/".into();
            }
            "gvoice" | "chatgpt-dots" => agent_or(&mut l.agent, Agent::Google),
            "grokbot" => agent_or(&mut l.agent, Agent::Browser),
            _ => {}
        }
        l
    }
}

/// What the window read when it closed: Desktop's `{didFinish, lastURL, fields}`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Outcome {
    /// The window had all the required fields.
    pub finished: bool,
    pub last_url: String,
    pub fields: Map<String, Value>,
}

/// The browser that logs in: `BEEPER_BROWSER` (a path or a command), else the first Chromium-family browser on the
/// `PATH`.
#[cfg(unix)]
pub fn find_browser() -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let find = |name: &str| -> Option<PathBuf> {
        let runs = |p: &Path| {
            use std::os::unix::fs::PermissionsExt;
            p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        };
        if name.contains('/') {
            return Some(PathBuf::from(name)).filter(|p| runs(p));
        }
        std::env::split_paths(&path).map(|d| d.join(name)).find(|p| runs(p))
    };
    match std::env::var("BEEPER_BROWSER") {
        Ok(b) if !b.is_empty() => find(&b),
        _ => BROWSERS.iter().find_map(|b| find(b)),
    }
}

/// beeper-rs drives a browser over a pipe only on Unix for now.
#[cfg(not(unix))]
pub fn find_browser() -> Option<PathBuf> {
    None
}

/// A fresh browser profile, Desktop's new session partition. It is removed on drop.
#[cfg(unix)]
struct Profile(PathBuf);

#[cfg(unix)]
impl Profile {
    fn new() -> std::io::Result<Self> {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("beeper-browser-login-{}-{n}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(Profile(path))
    }
}

#[cfg(unix)]
impl Drop for Profile {
    fn drop(&mut self) {
        if let Err(e) = std::fs::remove_dir_all(&self.0) {
            eprintln!("could not remove the browser profile {}: {e}", self.0.display());
        }
    }
}

/// The browser's arguments: a fresh profile, a pipe for the protocol, and an 800×800 app window without a URL bar
/// (or no window for a hidden login) that `drive` sends to the login page.
#[cfg(unix)]
fn args(login: &BrowserLogin, profile: &Path) -> Vec<String> {
    let blank = "data:text/html,";
    let mut args = vec![
        "--remote-debugging-pipe".to_string(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--password-store=basic".into(),
        "--window-size=800,800".into(),
    ];
    // Desktop's `BEEPER_LOGIN_DEVTOOLS` opens the developer tools of the window.
    if std::env::var_os("BEEPER_LOGIN_DEVTOOLS").is_some() {
        args.push("--auto-open-devtools-for-tabs".into());
    }
    args.extend(match login.hidden {
        true => vec!["--headless=new".into(), blank.into()],
        false => vec![format!("--app={blank}")],
    });
    args
}

/// Desktop's `launchBrowserWindow`: logs in with `browser` until the window closes, then removes its profile.
#[cfg(unix)]
pub async fn log_in(browser: PathBuf, login: BrowserLogin) -> Result<Outcome, String> {
    use std::os::fd::{AsRawFd, OwnedFd};
    use tokio::net::unix::pipe;
    let fail = |e: std::io::Error| format!("Could not start {}: {e}", browser.display());
    let profile = Profile::new().map_err(fail)?;
    // The browser reads the commands on its fd 3, and writes its replies on its fd 4. The pipes close on exec.
    let ((commands, to_browser), (from_browser, replies)) =
        (std::io::pipe().map_err(fail)?, std::io::pipe().map_err(fail)?);
    let (to_browser, from_browser) = (OwnedFd::from(to_browser), OwnedFd::from(from_browser));
    let mut cmd = tokio::process::Command::new(&browser);
    cmd.args(args(&login, &profile.0)).stdin(std::process::Stdio::null()).kill_on_drop(true);
    cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    let (read_fd, write_fd) = (commands.as_raw_fd(), replies.as_raw_fd());
    // SAFETY: the closure calls only `fcntl` and `dup2`, which are async-signal-safe.
    unsafe {
        cmd.pre_exec(move || {
            // Copies above 4 first, so that a descriptor that is 3 or 4 already is not overwritten.
            let (r, w) =
                (libc::fcntl(read_fd, libc::F_DUPFD_CLOEXEC, 10), libc::fcntl(write_fd, libc::F_DUPFD_CLOEXEC, 10));
            if r < 0 || w < 0 || libc::dup2(r, 3) < 0 || libc::dup2(w, 4) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut child = cmd.spawn().map_err(fail)?;
    drop((commands, replies));
    let to_browser = pipe::Sender::from_owned_fd(to_browser).map_err(fail)?;
    let from_browser = pipe::Receiver::from_owned_fd(from_browser).map_err(fail)?;
    let outcome = drive(from_browser, to_browser, &login).await;
    if tokio::time::timeout(CLOSE_WAIT, child.wait()).await.is_err() {
        child.kill().await.map_err(fail)?;
    }
    drop(profile);
    outcome
}

#[cfg(not(unix))]
pub async fn log_in(_: PathBuf, _: BrowserLogin) -> Result<Outcome, String> {
    unreachable!("find_browser finds no browser on this OS")
}

type Reply = oneshot::Sender<Result<Value, String>>;

/// A Chrome DevTools Protocol connection: each call waits for its reply.
struct Cdp {
    out: mpsc::UnboundedSender<String>,
    /// The calls that wait for a reply, or `None` once the browser has closed.
    pending: Mutex<Option<HashMap<u64, Reply>>>,
    next: AtomicU64,
}

impl Cdp {
    async fn call(&self, session: Option<&str>, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let closed = || "The browser closed".to_string();
        self.pending.lock().expect("the pending calls").as_mut().ok_or_else(closed)?.insert(id, tx);
        let mut msg = json!({ "id": id, "method": method, "params": params });
        if let Some(s) = session {
            msg["sessionId"] = s.into();
        }
        self.out.send(msg.to_string()).map_err(|_| closed())?;
        rx.await.map_err(|_| closed())?
    }

    /// `Runtime.evaluate` in the page: the value of the script, awaited when `value` is set.
    async fn eval(&self, session: &str, js: &str, value: bool) -> Option<Value> {
        let params = json!({ "expression": js, "returnByValue": value, "awaitPromise": value });
        let reply = self.call(Some(session), "Runtime.evaluate", params).await.ok()?;
        reply.get("exceptionDetails").is_none().then(|| reply["result"].get("value").cloned()).flatten()
    }
}

/// Reads the NUL-terminated messages of the browser: a reply goes to its call, and an event to `events`.
async fn read(cdp: Arc<Cdp>, input: impl AsyncRead + Unpin, events: mpsc::UnboundedSender<Value>) {
    let (mut input, mut buf) = (BufReader::new(input), Vec::new());
    while input.read_until(0, &mut buf).await.is_ok_and(|n| n > 0) {
        let msg: Value = match serde_json::from_slice(buf.strip_suffix(&[0]).unwrap_or(&buf)) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("bad message from the browser: {e}");
                break;
            }
        };
        buf.clear();
        match msg["id"].as_u64() {
            Some(id) => {
                let reply = match msg.get("error") {
                    Some(e) => Err(e["message"].as_str().unwrap_or("CDP error").to_string()),
                    None => Ok(msg["result"].clone()),
                };
                let tx = cdp.pending.lock().expect("the pending calls").as_mut().and_then(|p| p.remove(&id));
                tx.map(|tx| tx.send(reply));
            }
            None => drop(events.send(msg)),
        }
    }
    cdp.pending.lock().expect("the pending calls").take();
}

async fn write(mut output: impl AsyncWrite + Unpin, mut out: mpsc::UnboundedReceiver<String>) {
    while let Some(msg) = out.recv().await {
        if output.write_all(msg.as_bytes()).await.and(output.write_all(&[0]).await).is_err() {
            break;
        }
    }
}

/// The login window as the events of the browser show it.
#[derive(Default)]
struct Window {
    /// The session of the login page, and its target, which is also its main frame.
    main: Option<(String, String)>,
    /// The page is set up and can load the login page.
    ready: bool,
    loads: u32,
    /// The launch script ran, so each navigation runs the navigation script.
    launched: bool,
    /// Desktop's `getURL()`: the URL of the page.
    url: String,
    /// Desktop's `B`: the fields that the window has read.
    fields: Map<String, Value>,
    /// The URLs of the requests by session and request ID, for the headers that come later.
    requests: HashMap<(String, String), String>,
    closed: bool,
}

type Shared = Arc<watch::Sender<Window>>;

/// Desktop's `fe.pattern` and `request_url_regex` tests: an empty pattern matches, and a bad one does not.
fn matches(pattern: &str, s: &str) -> bool {
    pattern.is_empty() || regex::Regex::new(pattern).is_ok_and(|r| r.is_match(s))
}

/// JavaScript's `decodeURIComponent`, or `None` where it throws.
fn decode_uri_component(s: &str) -> Option<String> {
    let (b, mut out, mut i) = (s.as_bytes(), Vec::with_capacity(s.len()), 0);
    while i < b.len() {
        match b[i] {
            b'%' => {
                let hex = b.get(i + 1..i + 3).filter(|h| h.iter().all(u8::is_ascii_hexdigit))?;
                out.push(u8::from_str_radix(std::str::from_utf8(hex).expect("hex digits are ASCII"), 16).ok()?);
                i += 3;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// Desktop's `bb`: a cookie of `domain` is for `wanted`.
fn domain_matches(domain: &str, wanted: &str) -> bool {
    match domain.starts_with('.') {
        true => domain.ends_with(&format!(".{}", wanted.trim_start_matches('.'))),
        false => domain == wanted,
    }
}

/// The first source of the type of each field that has one.
fn sourced<'a>(
    fields: &'a [CookieField],
    kind: &'a str,
) -> impl Iterator<Item = (&'a CookieField, &'a crate::bridge_login::CookieSource)> {
    fields.iter().filter_map(move |f| f.sources.iter().find(|s| s.kind == kind).map(|s| (f, s)))
}

/// Desktop's `onBeforeSendHeaders`: the `request_header` fields in the headers of a request.
fn read_headers(fields: &[CookieField], url: &str, headers: &Map<String, Value>, found: &mut Map<String, Value>) {
    for (f, s) in sourced(fields, "request_header") {
        let value = headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(&s.name)).and_then(|(_, v)| v.as_str());
        if let Some(v) = value.filter(|v| matches(&f.pattern, v) && matches(&s.request_url_regex, url)) {
            found.insert(f.id.clone(), v.into());
        }
    }
}

/// Desktop's `Ee`: the `cookie` fields in the cookies of the profile.
fn read_cookies(fields: &[CookieField], cookies: &[Value], found: &mut Map<String, Value>) {
    for (f, s) in sourced(fields, "cookie") {
        let domain = |c: &Value| {
            let d = c["domain"].as_str().unwrap_or_default();
            s.cookie_domain.is_empty() || d.is_empty() || domain_matches(d, &s.cookie_domain)
        };
        let cookie = cookies.iter().find(|c| c["name"] == s.name.as_str() && domain(c));
        let value = cookie.and_then(|c| c["value"].as_str()).and_then(decode_uri_component);
        if let Some(v) = value.filter(|v| matches(&f.pattern, v)) {
            found.insert(f.id.clone(), v.into());
        }
    }
}

/// Desktop's `forEach` of the `special` fields: the values in the result of the close script.
fn read_special(fields: &[CookieField], result: &Value, found: &mut Map<String, Value>) {
    for f in fields.iter().filter(|f| f.sources.iter().any(|s| s.kind == "special")) {
        let value = result
            .get(&f.id)
            .filter(|v| matches(&f.pattern, &v.as_str().map_or_else(|| v.to_string(), str::to_string)));
        if let Some(v) = value {
            found.insert(f.id.clone(), v.clone());
        }
    }
}

/// Sets up a page, frame or worker that the browser attached: its user agent, blocked URLs and requests, the
/// navigations of the login page, and its own frames and workers. Then it runs.
async fn set_up(cdp: Arc<Cdp>, win: Shared, session: String, main: bool, agent: Option<String>, blocked: Vec<String>) {
    let s = Some(session.as_str());
    let mut calls = vec![("Network.enable", json!({}))];
    calls.extend(agent.map(|ua| ("Network.setUserAgentOverride", json!({ "userAgent": ua }))));
    if !blocked.is_empty() {
        let urls: Vec<String> = blocked.iter().map(|b| format!("*{b}*")).collect();
        calls.push(("Network.setBlockedURLs", json!({ "urls": urls })));
    }
    if main {
        calls.push(("Page.enable", json!({})));
    }
    calls
        .push(("Target.setAutoAttach", json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true })));
    // A target without a domain, such as a worker without `Page`, rejects its call, and the others still run.
    for (method, params) in calls {
        let _ = cdp.call(s, method, params).await;
    }
    if main {
        win.send_modify(|w| w.ready = true);
    }
    let _ = cdp.call(s, "Runtime.runIfWaitingForDebugger", json!({})).await;
}

/// Follows the events of the browser until it closes.
async fn follow(
    cdp: Arc<Cdp>,
    win: Shared,
    login: BrowserLogin,
    agent: Option<String>,
    mut events: mpsc::UnboundedReceiver<Value>,
) {
    let mut extra: HashMap<(String, String), Map<String, Value>> = HashMap::new();
    while let Some(e) = events.recv().await {
        let (p, session) = (&e["params"], e["sessionId"].as_str().unwrap_or_default().to_string());
        let main = win.borrow().main.clone();
        let on_main = main.as_ref().is_some_and(|m| m.0 == session);
        let navigated = |url: Option<&str>| {
            let mut launched = false;
            win.send_modify(|w| {
                launched = w.launched;
                if let Some(u) = url {
                    w.url = u.into();
                }
            });
            // Desktop's `did-navigate`, `did-navigate-in-page` and `did-frame-navigate`.
            if let Some(js) = login.navigate_js.clone().filter(|_| launched) {
                let (cdp, session) = (cdp.clone(), session.clone());
                tokio::spawn(async move { cdp.eval(&session, &js, false).await });
            }
        };
        match e["method"].as_str().unwrap_or_default() {
            "Target.attachedToTarget" => {
                let child = p["sessionId"].as_str().expect("an attached target has a session").to_string();
                let page = p["targetInfo"]["type"] == "page" && main.is_none();
                if page {
                    let target = p["targetInfo"]["targetId"].as_str().expect("a target ID").to_string();
                    win.send_modify(|w| w.main = Some((child.clone(), target)));
                }
                let (cdp, win, agent) = (cdp.clone(), win.clone(), agent.clone());
                tokio::spawn(set_up(cdp, win, child, page, agent, login.blocked.clone()));
            }
            "Target.detachedFromTarget" if main.as_ref().is_some_and(|m| p["sessionId"] == m.0.as_str()) => break,
            "Network.requestWillBeSent" => {
                let id = (session, p["requestId"].as_str().unwrap_or_default().to_string());
                let url = p["request"]["url"].as_str().unwrap_or_default().to_string();
                win.send_modify(|w| {
                    let empty = Map::new();
                    read_headers(
                        &login.fields,
                        &url,
                        p["request"]["headers"].as_object().unwrap_or(&empty),
                        &mut w.fields,
                    );
                    if let Some(h) = extra.remove(&id) {
                        read_headers(&login.fields, &url, &h, &mut w.fields);
                    }
                    w.requests.insert(id, url.clone());
                });
            }
            // The headers as the browser sent them, with its cookies: they can come before the request.
            "Network.requestWillBeSentExtraInfo" => {
                let id = (session, p["requestId"].as_str().unwrap_or_default().to_string());
                let headers = p["headers"].as_object().cloned().unwrap_or_default();
                let url = win.borrow().requests.get(&id).cloned();
                match url {
                    Some(url) => win.send_modify(|w| read_headers(&login.fields, &url, &headers, &mut w.fields)),
                    None => drop(extra.insert(id, headers)),
                }
            }
            "Page.frameNavigated" if on_main => {
                navigated(p["frame"].get("parentId").is_none().then(|| p["frame"]["url"].as_str().unwrap_or_default()))
            }
            "Page.navigatedWithinDocument" if on_main => {
                navigated(main.as_ref().filter(|m| p["frameId"] == m.1.as_str()).and(p["url"].as_str()))
            }
            "Page.loadEventFired" if on_main => win.send_modify(|w| w.loads += 1),
            _ => {}
        }
    }
    win.send_modify(|w| w.closed = true);
}

/// Waits `d`, or less when the window closes. It returns `false` when the window closed.
async fn wait(win: &Shared, d: Duration) -> bool {
    let mut rx = win.subscribe();
    tokio::select! {
        _ = tokio::time::sleep(d) => !win.borrow().closed,
        _ = rx.wait_for(|w| w.closed) => false,
    }
}

/// Desktop's `Am` on a protocol connection to a browser that has just started: loads the login page in the
/// browser's page, reads the fields of the login every 100 ms, and closes the browser when it has them all.
pub async fn drive(
    input: impl AsyncRead + Unpin + Send + 'static,
    output: impl AsyncWrite + Unpin + Send + 'static,
    login: &BrowserLogin,
) -> Result<Outcome, String> {
    let final_url = match login.final_url.as_str() {
        "" => None,
        p => Some(regex::Regex::new(p).map_err(|e| format!("Bad final URL pattern: {e}"))?),
    };
    let (out, out_rx) = mpsc::unbounded_channel();
    let (events, events_rx) = mpsc::unbounded_channel();
    let cdp = Arc::new(Cdp { out, pending: Mutex::new(Some(HashMap::new())), next: AtomicU64::new(1) });
    tokio::spawn(read(cdp.clone(), input, events));
    tokio::spawn(write(output, out_rx));
    let version = cdp.call(None, "Browser.getVersion", json!({})).await?;
    let product = version["product"].as_str().unwrap_or_default();
    let chrome = product.split_once('/').map_or("", |p| p.1).split('.').next().unwrap_or_default();
    let win: Shared = Arc::new(watch::Sender::new(Window::default()));
    tokio::spawn(follow(cdp.clone(), win.clone(), login.clone(), login.agent.string(chrome), events_rx));
    let attach = json!({ "autoAttach": true, "waitForDebuggerOnStart": true, "flatten": true });
    cdp.call(None, "Target.setAutoAttach", attach).await?;
    let outcome = |win: &Shared, finished| {
        let w = win.borrow();
        let last_url = if w.url.is_empty() { login.url.clone() } else { w.url.clone() };
        Ok(Outcome { finished, last_url, fields: w.fields.clone() })
    };
    if !win.subscribe().wait_for(|w| w.ready || w.closed).await.is_ok_and(|w| !w.closed) {
        return outcome(&win, false);
    }
    let session = win.borrow().main.clone().expect("a ready page").0;
    let loads = win.borrow().loads;
    // Desktop's `loadURL`, which logs a failure and goes on.
    let nav = cdp.call(Some(&session), "Page.navigate", json!({ "url": login.url })).await;
    match nav.as_ref().map(|n| n["errorText"].as_str()) {
        Ok(None) => drop(win.subscribe().wait_for(|w| w.loads > loads || w.closed).await),
        Ok(Some(e)) => eprintln!("the login page did not load: {e}"),
        Err(e) => eprintln!("the login page did not load: {e}"),
    }
    if let Some(js) = &login.launch_js {
        let (cdp, session, js) = (cdp.clone(), session.clone(), js.clone());
        tokio::spawn(async move { cdp.eval(&session, &js, false).await });
    }
    win.send_modify(|w| w.launched = true);
    let deadline = login.hidden.then(|| tokio::time::Instant::now() + HIDDEN_TIMEOUT);
    let mut finished = false;
    let mut pause = Duration::from_secs(1);
    while wait(&win, pause).await {
        pause = Duration::from_millis(100);
        if deadline.is_some_and(|d| tokio::time::Instant::now() >= d) {
            break;
        }
        let url = win.borrow().url.clone();
        // Desktop's `F`: the result of the close script, `{}` when it fails.
        let special = match &login.close_js {
            Some(js) => cdp.eval(&session, js, true).await,
            None => None,
        };
        let cookies = cdp.call(None, "Storage.getCookies", json!({})).await.ok();
        let cookies = cookies.as_ref().and_then(|c| c["cookies"].as_array()).map_or(&[][..], Vec::as_slice);
        let mut storage = Map::new();
        for (f, s) in sourced(&login.fields, "local_storage") {
            let js = format!("window.localStorage.getItem({})", Value::from(s.name.as_str()));
            if let Some(v) =
                cdp.eval(&session, &js, true).await.filter(|v| v.as_str().is_some_and(|v| matches(&f.pattern, v)))
            {
                storage.insert(f.id.clone(), v);
            }
        }
        let mut done = false;
        win.send_modify(|w| {
            read_cookies(&login.fields, cookies, &mut w.fields);
            w.fields.extend(storage);
            read_special(&login.fields, special.as_ref().unwrap_or(&Value::Null), &mut w.fields);
            done = login.fields.iter().filter(|f| f.required).all(|f| w.fields.contains_key(&f.id));
        });
        if !done {
            continue;
        }
        finished = true;
        match &final_url {
            Some(re) if re.is_match(&url) => break,
            Some(_) => {}
            None => {
                cdp.eval(&session, "document.title = 'Authenticating...'", false).await;
                wait(&win, Duration::from_secs(1)).await;
                break;
            }
        }
    }
    // The browser can still run, as when the user closed the login page but not a popup of it.
    let _ = tokio::time::timeout(CLOSE_WAIT, cdp.call(None, "Browser.close", json!({}))).await;
    outcome(&win, finished)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_login::CookieSource;

    /// What the page of the fake browser has, and when.
    #[derive(Clone, Default)]
    struct Site {
        cookies: Vec<Value>,
        /// The cookies show from this read of the cookies.
        cookies_at: usize,
        storage: HashMap<String, String>,
        /// The value of the close script.
        special: Value,
        /// The headers of the page's request to `/api`, which come before the request.
        headers: Value,
        /// The page goes to this URL in the page at the second read of the cookies.
        then: Option<String>,
        /// The user closes the window at this read of the cookies.
        close_at: Option<usize>,
    }

    type Log = Arc<Mutex<Vec<Value>>>;

    fn event(session: Option<&str>, method: &str, params: Value) -> Value {
        let mut e = json!({ "method": method, "params": params });
        if let Some(s) = session {
            e["sessionId"] = s.into();
        }
        e
    }

    /// A Chromium-family browser with one page `T` on session `S`, which answers the protocol like Chrome and
    /// logs the calls.
    async fn fake(input: impl AsyncRead + Unpin, mut output: impl AsyncWrite + Unpin, site: Site, log: Log) {
        let (mut input, mut buf, mut reads) = (BufReader::new(input), Vec::new(), 0);
        while input.read_until(0, &mut buf).await.expect("read a call") > 0 {
            let msg: Value = serde_json::from_slice(buf.strip_suffix(&[0]).expect("a NUL-terminated call")).unwrap();
            buf.clear();
            log.lock().unwrap().push(msg.clone());
            let (p, page) = (&msg["params"], Some("S"));
            let (mut before, mut after, mut end) = (Vec::new(), Vec::new(), false);
            let result = match (msg["method"].as_str().unwrap(), msg.get("sessionId").is_some()) {
                ("Browser.getVersion", false) => json!({ "product": "Chrome/131.0.6778.85" }),
                ("Target.setAutoAttach", false) => {
                    let info = json!({ "targetId": "T", "type": "page", "url": "data:text/html," });
                    let attached = json!({ "sessionId": "S", "targetInfo": info, "waitingForDebugger": false });
                    before.push(event(None, "Target.attachedToTarget", attached));
                    json!({})
                }
                ("Page.navigate", true) => {
                    let url = p["url"].as_str().unwrap();
                    let api = format!("{}/api", url.trim_end_matches("/login"));
                    after.push(event(page, "Page.frameNavigated", json!({ "frame": { "id": "T", "url": url } })));
                    let extra = json!({ "requestId": "1", "headers": site.headers });
                    after.push(event(page, "Network.requestWillBeSentExtraInfo", extra));
                    let request = json!({ "requestId": "1", "request": { "url": api, "headers": {} } });
                    after.push(event(page, "Network.requestWillBeSent", request));
                    after.push(event(page, "Page.loadEventFired", json!({})));
                    json!({ "frameId": "T", "loaderId": "L" })
                }
                ("Runtime.evaluate", true) => {
                    let js = p["expression"].as_str().unwrap();
                    let value = match js.strip_prefix("window.localStorage.getItem(") {
                        Some(name) => {
                            let name: String = serde_json::from_str(name.strip_suffix(')').unwrap()).unwrap();
                            site.storage.get(&name).map_or(Value::Null, |v| v.as_str().into())
                        }
                        None if js == format!("globalThis.{RESULTS}") => site.special.clone(),
                        None => Value::Null,
                    };
                    json!({ "result": { "type": "object", "value": value } })
                }
                ("Storage.getCookies", false) => {
                    reads += 1;
                    if let Some(url) = site.then.as_ref().filter(|_| reads == 2) {
                        after.push(event(page, "Page.navigatedWithinDocument", json!({ "frameId": "T", "url": url })));
                    }
                    if site.close_at == Some(reads) {
                        after.push(event(None, "Target.detachedFromTarget", json!({ "sessionId": "S" })));
                        end = true;
                    }
                    json!({ "cookies": if reads >= site.cookies_at { site.cookies.clone() } else { vec![] } })
                }
                ("Browser.close", false) => {
                    end = true;
                    json!({})
                }
                (_, true) => json!({}),
                (m, false) => panic!("{m} is not a browser call"),
            };
            let reply = json!({ "id": msg["id"], "result": result });
            for m in before.into_iter().chain([reply]).chain(after) {
                output.write_all(format!("{m}\0").as_bytes()).await.expect("write a message");
            }
            if end {
                return;
            }
        }
    }

    /// Drives the fake browser with `site`, and returns the outcome and the calls of the login.
    async fn run(login: &BrowserLogin, site: Site) -> (Outcome, Vec<Value>) {
        let ((to_browser, commands), (replies, from_browser)) =
            (tokio::io::duplex(1 << 16), tokio::io::duplex(1 << 16));
        let log = Log::default();
        tokio::spawn(fake(commands, replies, site, log.clone()));
        let outcome = drive(from_browser, to_browser, login).await.expect("the login runs");
        let calls = log.lock().unwrap().clone();
        (outcome, calls)
    }

    fn field(id: &str, kind: &str, name: &str, required: bool) -> CookieField {
        let source = CookieSource { kind: kind.into(), name: name.into(), ..CookieSource::default() };
        CookieField { id: id.into(), required, sources: vec![source], ..CookieField::default() }
    }

    fn cookie(name: &str, value: &str, domain: &str) -> Value {
        json!({ "name": name, "value": value, "domain": domain, "path": "/" })
    }

    fn calls_of<'a>(calls: &'a [Value], method: &str) -> Vec<&'a Value> {
        calls.iter().filter(|c| c["method"] == method).collect()
    }

    /// Desktop's `Am` with each source of a field: a cookie of a domain, URL-encoded and with a pattern, a local
    /// storage item, a request header of an API URL, and a value of `extract_js`. The window waits for the final
    /// URL, and the navigation in the page runs the script again.
    #[tokio::test(start_paused = true)]
    async fn a_login_reads_each_source_of_its_fields() {
        let mut ct0 = field("ct0", "cookie", "ct0", true);
        ct0.sources[0].cookie_domain = "x.test".into();
        let mut auth = field("auth", "cookie", "auth_token", true);
        auth.pattern = "^[0-9a-f]+$".into();
        let mut header = field("authorization", "request_header", "Authorization", true);
        header.sources[0].request_url_regex = "/api$".into();
        let cookies = Cookies {
            url: "https://x.test/login".into(),
            wait_for_url_pattern: "/home$".into(),
            extract_js: "Promise.resolve({ special: 's3cret' })".into(),
            fields: vec![
                ct0,
                auth,
                field("token", "local_storage", "tok", true),
                header,
                field("special", "special", "", true),
                field("optional", "cookie", "absent", false),
            ],
            ..Cookies::default()
        };
        let login = BrowserLogin::new(&cookies, "gvoice", "cookies");
        let site = Site {
            cookies: vec![
                cookie("ct0", "other", ".other.test"),
                cookie("ct0", "c%30", ".x.test"),
                cookie("auth_token", "abc%31", "x.test"),
            ],
            storage: HashMap::from([("tok".into(), "t0k".into())]),
            special: json!({ "special": "s3cret" }),
            headers: json!({ "authorization": "Bearer a" }),
            then: Some("https://x.test/home".into()),
            ..Site::default()
        };
        let (outcome, calls) = run(&login, site).await;
        let fields =
            json!({ "authorization": "Bearer a", "ct0": "c0", "auth": "abc1", "token": "t0k", "special": "s3cret" });
        assert_eq!(Value::Object(outcome.fields), fields);
        assert!(outcome.finished);
        assert_eq!(outcome.last_url, "https://x.test/home");
        let ua = calls_of(&calls, "Network.setUserAgentOverride");
        let google = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Beeper/3.110.1 Chrome/131.0.0.0 Safari/537.36";
        assert_eq!(ua[0]["params"]["userAgent"], google, "Google Voice has Desktop's Google user agent");
        let at = |m: &str| calls.iter().position(|c| c["method"] == m).unwrap();
        assert!(at("Network.setUserAgentOverride") < at("Page.navigate"), "the page has its user agent first");
        assert!(calls_of(&calls, "Storage.getCookies").len() >= 2, "the fields were there before the final URL");
        let script = login.launch_js.as_deref().expect("the extract_js wrapper");
        let runs = calls.iter().filter(|c| c["params"]["expression"] == script).count();
        assert_eq!(runs, 2, "the launch, and the navigation in the page");
        assert_eq!(calls.last().unwrap()["method"], "Browser.close");
    }

    /// A user who closes the window before the login ends it unfinished, with the URL of the page.
    #[tokio::test(start_paused = true)]
    async fn closing_the_window_ends_the_login_unfinished() {
        let cookies = Cookies {
            url: "https://x.test/login".into(),
            user_agent: Some(String::new()),
            fields: vec![field("ct0", "cookie", "ct0", true)],
            ..Cookies::default()
        };
        let login = BrowserLogin::new(&cookies, "discord", "token");
        let site =
            Site { cookies: vec![cookie("ct0", "a", "x.test")], cookies_at: 3, close_at: Some(2), ..Site::default() };
        let (outcome, calls) = run(&login, site).await;
        assert_eq!(outcome, Outcome { finished: false, last_url: "https://x.test/login".into(), fields: Map::new() });
        assert!(calls_of(&calls, "Network.setUserAgentOverride").is_empty(), "an empty user agent is the browser's");
    }

    /// Desktop's `Eb`: a hidden login closes after 30 seconds.
    #[tokio::test(start_paused = true)]
    async fn a_hidden_login_times_out() {
        let cookies = Cookies {
            url: "https://x.test/login".into(),
            hidden: true,
            fields: vec![field("ct0", "cookie", "ct0", true)],
            ..Cookies::default()
        };
        let start = tokio::time::Instant::now();
        let (outcome, calls) = run(&BrowserLogin::new(&cookies, "discord", "token"), Site::default()).await;
        assert!(!outcome.finished && start.elapsed() >= HIDDEN_TIMEOUT);
        assert_eq!(calls.last().unwrap()["method"], "Browser.close");
    }

    /// The X cookie flow of the stub bridge: the browser reads the cookies of the step, and the bridge logs in.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_cookies_of_the_browser_log_in_to_the_stub_bridge() {
        let (url, _) = crate::bridge_stub::homeserver().await;
        let p = crate::matrix::Provision {
            homeserver: url,
            access_token: "token".into(),
            user_id: "@me:beeper.com".into(),
            bridge_id: "twitter".into(),
            login_id: String::new(),
        };
        let (process, step) = crate::bridge_login::start(p.clone(), "cookies".into()).await.expect("the cookie step");
        let login = BrowserLogin::new(step.cookies.as_ref().expect("cookies"), "twitter", "cookies");
        assert_eq!(login.launch_js.as_deref(), Some(scripts::TWITTER), "Desktop hides the Google and Apple logins");
        let site = Site {
            cookies: vec![cookie("ct0", "a", ".x.com"), cookie("auth_token", "b", ".x.com")],
            ..Site::default()
        };
        let (outcome, _) = run(&login, site).await;
        assert!(outcome.finished);
        let done = crate::bridge_login::submit(p, process, step, Value::Object(outcome.fields)).await;
        assert_eq!(done.expect("the login").kind, "complete");
    }

    /// Desktop's `mapBrowserLogin` of each platform that has one.
    #[test]
    fn each_platform_maps_the_login_like_desktop() {
        let step = Cookies { url: "https://www.facebook.com/".into(), ..Cookies::default() };
        let map = |network: &str, flow: &str| BrowserLogin::new(&step, network, flow);
        assert_eq!(map("discord", "token").launch_js, None);
        assert_eq!(map("discord", "token").agent, Agent::Desktop);
        assert_eq!(map("facebook", "messenger").launch_js.as_deref(), Some(scripts::MESSENGER));
        let facebook = map("facebook", "messenger-lite");
        assert_eq!((facebook.navigate_js.as_deref(), facebook.agent), (Some(scripts::FACEBOOK), Agent::Desktop));
        let google = Cookies { url: "https://accounts.google.com/signin".into(), ..step.clone() };
        assert_eq!(BrowserLogin::new(&google, "facebook", "messenger").agent, Agent::Google);
        assert_eq!(map("instagram", "cookies").close_js.as_deref(), Some("successfulLogin"));
        let slack = map("slack", "token");
        assert_eq!(
            (slack.url.as_str(), slack.launch_js.as_deref()),
            ("https://slack.com/signin", Some(scripts::SLACK))
        );
        assert_eq!(slack.navigate_js, Some(format!("\n    {}\n    \n    ", scripts::SLACK)));
        let linkedin = map("linkedin", "cookies");
        assert_eq!(linkedin.url, "https://www.linkedin.com/login");
        assert_eq!(linkedin.launch_js, Some(format!("\n{}", scripts::LINKEDIN)));
        assert_eq!(linkedin.blocked, ["checkpoint/pk/initiateLogin"]);
        let gmessages = map("gmessages", "google");
        assert!(
            gmessages.url.starts_with("https://accounts.google.com/AccountChooser") && gmessages.agent == Agent::Google
        );
        assert_eq!(map("chatgpt-dots", "cookies").agent, Agent::Google);
        assert_eq!(map("grokbot", "cookies").agent, Agent::Browser);
        let own = Cookies { user_agent: Some("UA".into()), ..step.clone() };
        assert_eq!(BrowserLogin::new(&own, "gvoice", "cookies").agent, Agent::Custom("UA".into()));
        // Desktop's `IR` turns the escaped new lines of `extract_js` into new lines.
        let extract = Cookies { extract_js: r"  (() => {\n\treturn 1\n})()  ".into(), ..step };
        let x = BrowserLogin::new(&extract, "twitter", "cookies");
        assert!(x.launch_js.as_deref().unwrap().contains("const result = await ((() => {\n\treturn 1\n})())"));
        assert_eq!(x.navigate_js, x.launch_js);
        assert_eq!(x.close_js.as_deref(), Some("globalThis.__BEEP_BEEP_AUTH_RESULTS__"));
        let desktop =
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/152.0.0.0 Safari/537.36";
        assert_eq!(Agent::Desktop.string("152").as_deref(), Some(desktop), "Desktop's `Qg`");
    }

    #[test]
    fn values_decode_and_domains_match_like_desktop() {
        assert_eq!(decode_uri_component("a%20b%E2%9C%93+").as_deref(), Some("a b✓+"));
        assert_eq!(decode_uri_component("100%"), None);
        assert_eq!(decode_uri_component("%+1"), None);
        assert_eq!(decode_uri_component("%FF"), None, "not UTF-8");
        assert!(domain_matches(".x.com", "x.com") && domain_matches(".api.x.com", ".x.com"));
        assert!(domain_matches("x.com", "x.com") && !domain_matches("api.x.com", "x.com"));
        assert!(!domain_matches(".notx.com", "x.com"));
        assert!(matches("", "a") && matches("^a", "ab") && !matches("(", "a"), "a bad pattern does not match");
    }

    /// The browser of `the_browser_gets_a_pipe_and_a_fresh_profile`: the test binary runs this test as the fake
    /// browser, with the pipe of the login on its fds 3 and 4.
    #[cfg(unix)]
    #[test]
    fn fake_browser_process() {
        use std::os::fd::{FromRawFd, OwnedFd};
        if std::env::var_os("BEEPER_FAKE_BROWSER").is_none() {
            return;
        }
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        rt.block_on(async {
            // SAFETY: the login gives the browser the pipe on these descriptors, and nothing else owns them.
            let (commands, replies) = unsafe { (OwnedFd::from_raw_fd(3), OwnedFd::from_raw_fd(4)) };
            let commands = tokio::net::unix::pipe::Receiver::from_owned_fd(commands).unwrap();
            let replies = tokio::net::unix::pipe::Sender::from_owned_fd(replies).unwrap();
            let site = Site { cookies: vec![cookie("ct0", "a", "x.test")], ..Site::default() };
            fake(commands, replies, site, Log::default()).await;
        });
    }

    /// `log_in` starts the browser with the pipe on its fds 3 and 4, a fresh profile and an app window, and
    /// removes the profile when the browser closes.
    #[cfg(unix)]
    #[tokio::test]
    async fn the_browser_gets_a_pipe_and_a_fresh_profile() {
        use std::os::unix::fs::PermissionsExt;
        let dir = crate::testing::temp_dir("fake-browser");
        let exe = std::env::current_exe().unwrap();
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$@" > '{d}/args'
for a; do case $a in --user-data-dir=*) [ -d "${{a#*=}}" ] && touch '{d}/profile';; esac; done
BEEPER_FAKE_BROWSER=1 exec '{e}' --exact browser_login::tests::fake_browser_process --quiet
"#,
            d = dir.display(),
            e = exe.display()
        );
        let browser = dir.join("fake-chromium");
        std::fs::write(&browser, script).unwrap();
        std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o755)).unwrap();
        let fields = vec![field("ct0", "cookie", "ct0", true)];
        let cookies = Cookies { url: "https://x.test/login".into(), fields, ..Cookies::default() };
        let outcome = log_in(browser, BrowserLogin::new(&cookies, "discord", "token")).await.expect("the login");
        assert!(outcome.finished && outcome.fields["ct0"] == "a");
        let args = std::fs::read_to_string(dir.join("args")).unwrap();
        let args: Vec<&str> = args.lines().collect();
        let flags = ["--remote-debugging-pipe", "--no-first-run", "--window-size=800,800", "--app=data:text/html,"];
        for a in flags {
            assert!(args.contains(&a), "{a} in {args:?}");
        }
        assert!(dir.join("profile").exists(), "the profile is there while the browser runs");
        let profile = args.iter().find_map(|a| a.strip_prefix("--user-data-dir=")).unwrap();
        assert!(!Path::new(profile).exists(), "the profile is removed");
    }
}
