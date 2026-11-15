//! `watch` (the Desktop API WebSocket) and `rpc` (newline-delimited JSON commands).

use super::commands::cmd;
use super::render;
use super::{js_string, CliError, Cmd, Ctx, Flag, Res};
use beeper_api::client::BeeperClient;
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

const TYPES: &[&str] = &["chat.upserted", "chat.deleted", "message.upserted", "message.deleted"];

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        description: "Reads JSON lines like {\"id\":1,\"command\":\"send text --to 10313 --message hello\"} or {\"id\":1,\"args\":[\"status\",\"--json\"]}.",
        ..cmd("rpc", "Run newline-delimited JSON command RPC over stdin/stdout", rpc)
    },
    Cmd {
        flags: &[
            Flag::many("chat", "Chat ID to subscribe to. Defaults to all chats.").c('c'),
            Flag::b("json", "Print raw JSON, one event per line"),
            Flag::many("include-type", "Only forward events of these types. Repeat for multiple.").opts(TYPES),
            Flag::many("exclude-type", "Drop events of these types. Repeat for multiple.").opts(TYPES),
            Flag::s("webhook", "Forward each event to this URL as a POST request (best-effort, fire-and-forget)"),
            Flag::s("webhook-secret", "HMAC-SHA256 secret. Signs payloads with X-Beeper-Signature: sha256=<hex>"),
            Flag::i("webhook-queue", "Maximum pending webhook deliveries before dropping events").def("64"),
        ],
        ..cmd("watch", "Stream Desktop API WebSocket events", watch)
    },
];

/// `http://host:port/path` as the base URL and the path.
fn split_url(url: &str) -> (String, String) {
    let rest = url.split_once("://").map_or(url, |p| p.1);
    let (host, path) = rest.split_once('/').map_or((rest, String::new()), |(h, p)| (h, format!("/{p}")));
    let scheme = url.split_once("://").map_or("http", |p| p.0);
    (format!("{scheme}://{host}"), if path.is_empty() { "/".into() } else { path })
}

/// `passesFilter`: a frame without a type always passes.
fn passes(text: &str, include: &[String], exclude: &[String]) -> bool {
    let kind = serde_json::from_str::<Value>(text).ok().and_then(|v| v["type"].as_str().map(str::to_string));
    kind.is_none_or(|k| (include.is_empty() || include.contains(&k)) && !exclude.contains(&k))
}

struct Webhook {
    client: Result<BeeperClient, String>,
    path: String,
    url: String,
    secret: Option<String>,
    max: usize,
    inflight: Arc<AtomicUsize>,
}

impl Webhook {
    /// `forwardWebhook`: posts the frame without waiting, and drops it when `max` posts are in flight.
    fn forward(&self, body: &str, events: bool) {
        if self.inflight.load(Ordering::SeqCst) >= self.max {
            if events {
                super::write_event("watch.webhook_drop", json!({ "reason": "queue_full", "size": 0 }));
            }
            return eprintln!("warning: webhook queue full ({}); dropped event", self.max);
        }
        let sign = |s: &String| {
            let mac = beeper_api::codec::hmac_sha256(s.as_bytes(), body.as_bytes());
            format!("sha256={}", mac.iter().map(|b| format!("{b:02x}")).collect::<String>())
        };
        let signature = self.secret.as_ref().map(sign);
        let (client, path, url, body) =
            (self.client.clone(), self.path.clone(), self.url.clone(), body.as_bytes().to_vec());
        let inflight = self.inflight.clone();
        inflight.fetch_add(1, Ordering::SeqCst);
        tokio::spawn(async move {
            let headers: Vec<(&str, &str)> = signature.iter().map(|s| ("x-beeper-signature", s.as_str())).collect();
            let post = async {
                let client = client?;
                let form = Some(("application/json".to_string(), body));
                client.request_raw("POST", &path, &headers, form).await.map_err(|e| e.0)
            };
            match tokio::time::timeout(std::time::Duration::from_secs(10), post).await {
                Ok(Ok((status, _))) if status < 300 => {}
                Ok(Ok((status, _))) => {
                    if events {
                        super::write_event("watch.webhook_error", json!({ "status": status }));
                    }
                    eprintln!("warning: webhook POST {url} returned {status}");
                }
                failed => {
                    let message = match failed {
                        Ok(Err(e)) => e,
                        _ => "The operation timed out.".into(),
                    };
                    if events {
                        super::write_event("watch.webhook_error", json!({ "message": message }));
                    }
                    eprintln!("warning: webhook POST failed: {message}");
                }
            }
            inflight.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

fn watch(c: &mut Ctx) -> Res {
    let (hook, secret) = (c.str("webhook").map(str::to_string), c.str("webhook-secret").map(str::to_string));
    if secret.is_some() && hook.is_none() {
        return Err(CliError::bug("--webhook-secret requires --webhook URL"));
    }
    let (include, exclude) = (c.many("include-type").to_vec(), c.many("exclude-type").to_vec());
    if !include.is_empty() && !exclude.is_empty() {
        return Err(CliError::bug("Use either --include-type or --exclude-type, not both."));
    }
    let token = c.token()?;
    let base = c.base_url()?;
    let info = BeeperClient::try_new(&base, "").map_err(CliError::bug)?;
    let unreachable = |_| CliError::bug("Unable to connect. Is the computer able to access the url?");
    let (status, body) = c.block(info.fetch("GET", "/v1/info", None)).map_err(unreachable)?;
    if status >= 300 {
        return Err(CliError::bug(format!("Failed to fetch /v1/info: HTTP {status}")));
    }
    let info: Value = serde_json::from_slice(&body).map_err(|e| CliError::bug(format!("JSON Parse error: {e}")))?;
    let endpoint = info["endpoints"]["ws_events"].as_str().filter(|e| !e.is_empty()).unwrap_or("/v1/ws");
    let (ws_base, path) =
        if endpoint.contains("://") { split_url(endpoint) } else { (base.clone(), endpoint.to_string()) };
    let client = BeeperClient::try_new(&ws_base, token).map_err(CliError::bug)?;
    let subscribed: Vec<String> = match c.many("chat") {
        [] => vec!["*".into()],
        chats => chats.to_vec(),
    };
    let webhook = hook.map(|url| {
        let (base, path) = split_url(&url);
        let max = usize::try_from(c.int("webhook-queue").expect("--webhook-queue has a default")).unwrap_or(0);
        Webhook { client: BeeperClient::try_new(&base, ""), path, url, secret, max, inflight: Arc::default() }
    });
    let (json_mode, events) = (c.json(), c.on("events"));
    let label = match subscribed.as_slice() {
        [all] if all == "*" => "all chats".to_string(),
        list => format!("{} chat{}", list.len(), if list.len() == 1 { "" } else { "s" }),
    };
    c.block(async move {
        let failed = || {
            if events {
                super::write_event("watch.error", json!({ "message": "WebSocket connection failed" }));
            }
            CliError::bug("WebSocket connection failed")
        };
        let mut ws = client.ws_open(&path).await.map_err(|_| failed())?;
        if events {
            super::write_event("watch.open", json!({ "subscribed": subscribed }));
        }
        if !json_mode {
            render::print(&render::stream_header(&label, &base));
        }
        ws.send(&json!({ "type": "subscriptions.set", "chatIDs": subscribed }).to_string())
            .await
            .map_err(|_| failed())?;
        let mut index = 0;
        loop {
            let frame = tokio::select! {
                frame = ws.next() => frame,
                _ = tokio::signal::ctrl_c() => {
                    let _ = ws.close(1000).await;
                    return Ok(());
                }
            };
            let text = match frame {
                Ok(text) => text,
                Err((code, reason)) => {
                    if events {
                        super::write_event("watch.close", json!({ "code": code, "reason": reason }));
                    }
                    return match (json_mode, code) {
                        (_, 1000) => Ok(()),
                        (true, _) => Err(CliError::bug(format!("WebSocket closed: {code} {reason}"))),
                        (false, _) => {
                            let reason = if reason.is_empty() { String::new() } else { format!(" {reason}") };
                            render::print(&render::stream_status(&format!("closed {code}{reason}")));
                            Ok(())
                        }
                    };
                }
            };
            if !passes(&text, &include, &exclude) {
                continue;
            }
            if events {
                super::write_event("watch.message", json!({}));
            }
            if json_mode {
                println!("{text}");
            }
            if let Some(hook) = &webhook {
                hook.forward(&text, events);
            }
            if !json_mode {
                index += 1;
                render::print(&render::stream_event(&serde_json::from_str(&text).ok(), index));
            }
        }
    })
}

/// `splitCommandLine`: words with quotes and backslash escapes, like a shell.
fn split_command_line(input: &str) -> Result<Vec<String>, String> {
    let (mut words, mut word, mut started, mut quote, mut escaped) = (Vec::new(), String::new(), false, None, false);
    for ch in input.chars() {
        match ch {
            _ if escaped => (word.push(ch), started = true, escaped = false).1,
            '\\' if quote != Some('\'') => escaped = true,
            '"' | '\'' if quote.is_none() || quote == Some(ch) => {
                started = true;
                quote = if quote.is_some() { None } else { Some(ch) };
            }
            c if quote.is_none() && c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            c => (word.push(c), started = true).1,
        }
    }
    if escaped {
        word.push('\\');
        started = true;
    }
    if let Some(q) = quote {
        return Err(format!("Unclosed {q} quote"));
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

const SIGNALS: [(i32, &str); 6] =
    [(1, "SIGHUP"), (2, "SIGINT"), (6, "SIGABRT"), (9, "SIGKILL"), (11, "SIGSEGV"), (15, "SIGTERM")];

/// One request of `rpc`: runs the command in a child process and gives its exit and output.
fn rpc_one(request: &Value) -> Result<Value, String> {
    let args = ["args", "argv"].iter().map(|k| &request[*k]).find(|v| !v.is_null());
    let args: Vec<String> = match (args, request["command"].as_str()) {
        (Some(Value::Array(list)), _) => list.iter().map(js_string).collect(),
        (Some(other), _) => vec![js_string(other)],
        (None, Some(command)) if !command.is_empty() => split_command_line(command)?,
        (None, _) => Vec::new(),
    };
    let first = args.first().ok_or("Expected args, argv, or command")?;
    // `rpc` and `shell` would nest, and the app (the GUI, `serve`, a link) does not end.
    if ["rpc", "shell", "serve"].contains(&first.as_str()) || first.starts_with('-') || first.starts_with("beeper://") {
        return Err(format!("Unsupported nested command: {first}"));
    }
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .args(&args)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    let signal = std::os::unix::process::ExitStatusExt::signal(&out.status)
        .map(|n| SIGNALS.iter().find(|s| s.0 == n).map_or(format!("SIG{n}"), |s| s.1.to_string()));
    let (stdout, stderr) = (String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    let id = request.get("id").cloned().unwrap_or_default();
    Ok(
        json!({ "id": id, "ok": out.status.success(), "code": out.status.code(), "signal": signal, "stdout": stdout, "stderr": stderr }),
    )
}

fn rpc(_: &mut Ctx) -> Res {
    for line in std::io::stdin().lines() {
        let line = line.map_err(|e| CliError::bug(e.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        let request: Result<Value, String> = serde_json::from_str(&line).map_err(|e| format!("JSON Parse error: {e}"));
        let id = request.as_ref().map_or(Value::Null, |r| r.get("id").cloned().unwrap_or_default());
        let reply =
            request.and_then(|r| rpc_one(&r)).unwrap_or_else(|error| json!({ "id": id, "ok": false, "error": error }));
        println!("{reply}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lines_split_like_upstream() {
        let words = split_command_line(r#"send text --to 10313 --message "hello there" --x 'a b' c\ d"#).unwrap();
        assert_eq!(words, ["send", "text", "--to", "10313", "--message", "hello there", "--x", "a b", "c d"]);
        assert_eq!(split_command_line("say \"hi"), Err("Unclosed \" quote".into()));
        assert_eq!(split_command_line(r#"a "" b"#).unwrap(), ["a", "", "b"]);
    }

    #[test]
    fn webhook_signatures_use_hmac_sha256() {
        // RFC 4231 test case 2.
        let mac = beeper_api::codec::hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

    #[test]
    fn filters_and_urls_follow_upstream() {
        let (include, none) = (vec!["chat.upserted".to_string()], Vec::new());
        assert!(passes(r#"{"type":"chat.upserted"}"#, &include, &none));
        assert!(!passes(r#"{"type":"ready"}"#, &include, &none));
        assert!(passes("not json", &include, &none));
        assert!(!passes(r#"{"type":"ready"}"#, &none, &["ready".into()]));
        assert_eq!(split_url("http://127.0.0.1:9/hook?x=1"), ("http://127.0.0.1:9".into(), "/hook?x=1".into()));
        assert_eq!(split_url("http://example.com"), ("http://example.com".into(), "/".into()));
    }
}
