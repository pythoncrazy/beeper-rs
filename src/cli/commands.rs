//! The command table: each row is one command id of the upstream CLI with its args, flags, and handler.

use super::render;
use super::{
    arg, config, config_dir, js_string, obj, opt_arg, write_json, CliError, Cmd, Ctx, Flag, Kind, Res, BIN, GLOBAL,
};
use serde_json::{json, Value};

/// The topics: the name, the description, and the command that runs for the topic alone.
const TOPICS: &[(&str, &str, Option<&str>)] = &[
    ("api", "Call raw Desktop API endpoints", None),
    ("auth", "Inspect and clear stored authentication", None),
    ("chats", "List, search, manage, and modify chats", Some("chats list")),
    ("config", "Manage local CLI configuration", None),
    ("messages", "List, search, show, edit, delete, and export messages", None),
    ("send", "Send text, files, and reactions", None),
];

pub type Topic = (&'static str, &'static str, Option<&'static str>);

pub fn topic(name: &str) -> Option<&'static Topic> {
    TOPICS.iter().find(|t| t.0 == name)
}

/// Every command, topic by topic.
pub fn all() -> impl Iterator<Item = &'static Cmd> + Clone {
    [CORE, super::chats::COMMANDS, super::messages::COMMANDS].into_iter().flatten()
}

const NO_AUTH: Flag = Flag::b("no-auth", "Call a public API path without a bearer token");
const API_JSON: Flag = Flag::no("json", "Print JSON").def("true");
const CONFIG_KEYS: &[&str] = &["baseURL", "auth", "defaultTarget", "defaultAccount"];

pub const fn cmd(id: &'static str, summary: &'static str, run: super::Run) -> Cmd {
    Cmd { id, summary, description: "", args: &[], flags: &[], examples: &[], run }
}

const CORE: &[Cmd] = &[
    Cmd {
        args: &[arg("path", "API path, for example /v1/info")],
        flags: &[API_JSON, NO_AUTH],
        ..cmd("api get", "Call a raw Desktop API GET path", api_get)
    },
    Cmd {
        args: &[arg("path", "API path, for example /v1/messages/{chatID}/send")],
        flags: &[Flag::s("body", "JSON request body").def("{}"), API_JSON, NO_AUTH],
        ..cmd("api post", "Call a raw Desktop API POST path with a JSON body", api_post)
    },
    Cmd {
        args: &[
            super::Arg { options: &["GET", "POST", "PUT", "PATCH", "DELETE"], ..arg("method", "HTTP method") },
            arg("path", "API path, for example /v1/info"),
        ],
        flags: &[Flag::s("body", "JSON request body"), API_JSON, NO_AUTH],
        ..cmd("api request", "Call a raw Desktop API path with any supported HTTP method", api_request)
    },
    cmd("auth status", "Show stored auth for the selected target", auth_status),
    Cmd {
        args: &[super::Arg { options: CONFIG_KEYS, ..opt_arg("key", "Optional config key to print") }],
        ..cmd("config get", "Print CLI configuration", config_get)
    },
    cmd("config path", "Print the CLI config path", config_path),
    cmd("config reset", "Reset CLI configuration", config_reset),
    Cmd {
        args: &[
            super::Arg { options: &["defaultTarget", "defaultAccount"], ..arg("key", "Config key to set") },
            arg("value", "Config value (pass \"\" to clear)"),
        ],
        ..cmd("config set", "Set a CLI configuration value", config_set)
    },
    cmd("docs", "Open Beeper CLI docs", |c| c.print(&json!({ "url": "https://developers.beeper.com/desktop-api-reference" }))),
    Cmd {
        description: "Active reachability check plus readiness diagnostics. Exits non-zero when the target is not ready. For a cheap snapshot use `beeper status`.",
        ..cmd("doctor", "Probe the target live and report diagnostics", doctor)
    },
    Cmd {
        description: "Read-only readiness snapshot for the selected target. For active reachability checks and diagnostics, run `beeper doctor`.",
        ..cmd("status", "Show selected target and setup readiness", status)
    },
    cmd("version", "Print CLI version", |c| {
        c.print(&json!({ "name": BIN, "version": beeper_core::config::APP_VERSION }))
    }),
];

// ─── status, doctor ────────────────────────────────────────────────────────────

/// `nextAppStep`: the command that moves the target on from `state`.
fn next_step(state: &str, target: &str) -> Option<String> {
    let t = if target == "desktop" { String::new() } else { format!(" -t {target}") };
    Some(match state {
        "ready" => return None,
        "needs-login" => format!("Run: {BIN} setup{t}"),
        "needs-verification" => format!("Run: {BIN} verify{t}"),
        "needs-secrets" | "needs-recovery-key" => format!("Run: {BIN} verify recovery-key{t}"),
        "needs-cross-signing-setup" => format!("Run: {BIN} verify reset-recovery-key{t}"),
        s => format!("Waiting for app state: {s}"),
    })
}

/// The states of `normalizeReadinessState` and the commands of `actionsForState`.
const STATES: &[(&str, &[&str])] = &[
    ("no-target", &["targets add desktop", "targets add remote"]),
    ("target-unreachable", &["targets status", "targets start", "doctor"]),
    ("needs-login", &["setup", "auth status"]),
    ("login-in-progress", &["setup", "auth status"]),
    ("initializing", &["setup", "status"]),
    ("needs-cross-signing-setup", &["verify reset-recovery-key"]),
    ("needs-verification", &["verify", "verify list", "verify sas", "verify qr-scan"]),
    ("verification-in-progress", &["verify", "verify list", "verify sas", "verify qr-scan"]),
    ("needs-recovery-key", &["verify recovery-key"]),
    ("needs-secrets", &["verify recovery-key"]),
    ("needs-first-sync", &["setup", "status"]),
    ("ready", &["chats list", "messages list", "send text"]),
    ("error", &["doctor", "setup"]),
];

/// `appRequest`: a raw request with the token of the target, and its error text.
fn app_request(c: &mut Ctx, method: &str, path: &str, body: Option<Value>, auth: bool) -> Res<Value> {
    let base = c.base_url()?;
    let stored = c.target()?.get("auth").and_then(|a| a["accessToken"].as_str()).map(str::to_string);
    let token = std::env::var("BEEPER_ACCESS_TOKEN").ok().filter(|t| !t.is_empty()).or(stored).filter(|_| auth);
    let client = beeper_api::client::BeeperClient::try_new(&base, token.unwrap_or_default()).map_err(CliError::bug)?;
    let (status, bytes) = c
        .block(client.fetch(method, path, body.as_ref()))
        .map_err(|_| CliError::bug("Unable to connect. Is the computer able to access the url?"))?;
    let text = String::from_utf8_lossy(&bytes);
    match status {
        400.. => Err(CliError::bug(format!("{method} {path} failed: {status} {text}"))),
        _ if text.is_empty() => Ok(json!({})),
        _ => serde_json::from_str(&text).map_err(|e| CliError::bug(e.to_string())),
    }
}

/// `evaluateReadiness`.
fn readiness(c: &mut Ctx) -> Res<Value> {
    let id = js_string(&c.target()?["id"]);
    Ok(match app_request(c, "GET", "/v1/app/setup", None, true) {
        Ok(app) => {
            let raw = js_string(&app["state"]);
            let state = match STATES.iter().find(|s| s.0 == raw) {
                Some(s) => s.0,
                None if app["verification"]["state"].is_string() && raw != "ready" => "verification-in-progress",
                None => "error",
            };
            let actions = STATES.iter().find(|s| s.0 == state).expect("a known state").1;
            Value::Object(obj([
                ("state", json!(state)),
                ("app", app),
                ("actions", json!(actions)),
                ("message", json!(next_step(&raw, &id))),
            ]))
        }
        Err(e) => json!({ "state": "target-unreachable", "actions": STATES[1].1, "message": e.msg }),
    })
}

fn status(c: &mut Ctx) -> Res {
    let target = Value::Object(c.target()?.clone());
    let readiness = readiness(c)?;
    c.print(&json!({ "target": target, "readiness": readiness }))
}

/// `targetLiveStatus`: whether `/v1/info` answers, and the app behind it.
pub fn live_status(c: &mut Ctx) -> Res<Value> {
    let base = c.base_url()?;
    let target = c.target()?.clone();
    let client = beeper_api::client::BeeperClient::try_new(&base, "").map_err(CliError::bug)?;
    let reply = c.block(async {
        tokio::time::timeout(std::time::Duration::from_secs(3), client.fetch("GET", "/v1/info", None)).await
    });
    let Ok(Ok((status, bytes))) = reply else {
        return Ok(json!({ "reachable": false, "error": format!("Could not reach {base}") }));
    };
    if status >= 300 {
        let reason = beeper_api::http::StatusCode(status).reason();
        return Ok(json!({ "reachable": false, "error": format!("{status} {reason}") }));
    }
    let info: Value = serde_json::from_slice(&bytes).map_err(|_| CliError::bug(format!("Could not reach {base}")))?;
    let (version, bundle) = (info["app"]["version"].clone(), info["app"]["bundle_id"].clone());
    let managed_server = target["type"] == "server"
        && super::truthy(&target.get("managed").cloned().unwrap_or_default())
        && info["server"]["hostname"] == "127.0.0.1"
        && info["server"]["remote_access"] == false;
    let bundle_type = bundle.as_str().and_then(|b| {
        [(".server", "server"), (".desktop", "desktop")].iter().find(|(k, _)| b.contains(k)).map(|t| t.1)
    });
    let actual = if managed_server { Some("server") } else { bundle_type };
    let kind = js_string(&target["type"]);
    if let Some(actual) = actual.filter(|a| kind != "remote" && *a != kind) {
        let error = format!("Expected {kind} target but {base} is {actual}.");
        let all = obj([
            ("reachable", json!(false)),
            ("version", version),
            ("bundleID", bundle),
            ("actualType", json!(actual)),
            ("error", json!(error)),
        ]);
        return Ok(Value::Object(all));
    }
    Ok(Value::Object(obj([
        ("reachable", json!(true)),
        ("version", version),
        ("bundleID", bundle),
        ("actualType", json!(actual)),
    ])))
}

fn doctor(c: &mut Ctx) -> Res {
    let target = live_status(c)?;
    let readiness = readiness(c)?;
    let ready = readiness["state"] == "ready";
    c.print(&json!({ "ok": ready, "checks": { "target": target, "readiness": readiness } }))?;
    match ready {
        true => Ok(()),
        // Upstream calls `this.exit(4)`, which its error handler shows as an unexpected error.
        false => Err(CliError::bug("EEXIT: 4")),
    }
}

// ─── auth, config ──────────────────────────────────────────────────────────────

fn auth_status(c: &mut Ctx) -> Res {
    let target = c.target()?.clone();
    let auth = target.get("auth").cloned().unwrap_or_default();
    let env = std::env::var("BEEPER_ACCESS_TOKEN").is_ok_and(|t| !t.is_empty());
    let source = match (env, &auth["source"]) {
        (true, _) => json!("env"),
        (false, s) if !s.is_null() => s.clone(),
        _ if auth["accessToken"].is_string() => json!("target"),
        _ => json!("none"),
    };
    let data = obj([
        ("authenticated", json!(env || super::truthy(&auth["accessToken"]))),
        ("target", target["id"].clone()),
        ("baseURL", target["baseURL"].clone()),
        ("source", source),
        ("clientID", auth["clientID"].clone()),
        ("expiresAt", auth["expiresAt"].clone()),
        ("scope", auth["scope"].clone()),
    ]);
    c.print(&Value::Object(data))
}

fn config_get(c: &mut Ctx) -> Res {
    let mut safe = config()?;
    if let Some(Value::Object(auth)) = safe.get_mut("auth") {
        auth.insert("accessToken".into(), json!("[redacted]"));
    }
    match (c.arg(0), c.json()) {
        (Some(key), _) => c.print(safe.get(key).unwrap_or(&Value::Null)),
        (None, true) => c.print(&Value::Object(safe)),
        (None, false) => {
            render::print(&render::config_view(&safe));
            Ok(())
        }
    }
}

fn config_set(c: &mut Ctx) -> Res {
    c.writable()?;
    let (key, value) = (c.arg(0).expect("a required arg").to_string(), c.arg(1).expect("a required arg").to_string());
    let mut config = config()?;
    let next = (!value.is_empty()).then_some(value);
    match &next {
        Some(v) => config.insert(key.clone(), json!(v)),
        None => config.remove(&key),
    };
    write_json(&config_dir().join("config.json"), &config)?;
    let message = if next.is_some() { format!("Set {key}") } else { format!("Cleared {key}") };
    c.success(&message, next.as_deref(), Value::Object(obj([(key.as_str(), json!(next))])))
}

fn config_path(c: &mut Ctx) -> Res {
    let path = config_dir().join("config.json").display().to_string();
    match c.json() {
        true => println!("{}", serde_json::to_string_pretty(&json!({ "path": path })).expect("JSON values serialize")),
        false => println!("{path}"),
    }
    Ok(())
}

fn config_reset(c: &mut Ctx) -> Res {
    c.writable()?;
    match std::fs::remove_file(config_dir().join("config.json")) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(CliError::bug(e.to_string())),
        _ => {}
    }
    c.success("Config reset", None, json!({}))
}

// ─── api ───────────────────────────────────────────────────────────────────────

fn api_get(c: &mut Ctx) -> Res {
    let path = c.arg(0).expect("a required arg").to_string();
    let data = match c.on("no-auth") {
        true => app_request(c, "GET", &path, None, false)?,
        false => c.get(&path)?,
    };
    c.print(&data)
}

fn api_post(c: &mut Ctx) -> Res {
    let path = c.arg(0).expect("a required arg").to_string();
    c.writable()?;
    let raw = c.str("body").expect("--body has a default");
    let body: Value =
        serde_json::from_str(raw).map_err(|_| CliError::bug(format!("--body is not valid JSON: {raw}")))?;
    let data = match c.on("no-auth") {
        true => app_request(c, "POST", &path, Some(body), false)?,
        false => c.call("POST", &path, Some(body))?,
    };
    c.print(&data)
}

fn api_request(c: &mut Ctx) -> Res {
    let (method, path) = (c.arg(0).expect("a required arg").to_string(), c.arg(1).expect("a required arg").to_string());
    if method != "GET" {
        c.writable()?;
    }
    let body = c.str("body").filter(|b| !b.is_empty()).map(serde_json::from_str::<Value>).transpose();
    let body = body.map_err(|e| CliError::bug(format!("JSON Parse error: {e}")))?;
    let auth = !c.on("no-auth");
    let data = app_request(c, &method, &path, body, auth)?;
    c.print(&data)
}

// ─── help ──────────────────────────────────────────────────────────────────────

const WIDTH: usize = 100;

/// Greedy word wrap with trimmed rows and hard breaks, like `wrap-ansi` with `{hard: true}`.
fn wrap(text: &str, width: usize) -> Vec<String> {
    text.split('\n')
        .flat_map(|line| {
            let mut rows = vec![String::new()];
            for word in line.split(' ').filter(|w| !w.is_empty()) {
                let last = rows.last().expect("a row").chars().count();
                if last > 0 && last + 1 + word.chars().count() > width {
                    rows.push(String::new());
                }
                if last > 0 && last + 1 + word.chars().count() <= width {
                    rows.last_mut().expect("a row").push(' ');
                }
                for ch in word.chars() {
                    if rows.last().expect("a row").chars().count() == width {
                        rows.push(String::new());
                    }
                    rows.last_mut().expect("a row").push(ch);
                }
            }
            rows
        })
        .collect()
}

/// The flags of `cmd` in oclif order: the global flags (a command flag replaces its global flag), then the rest.
fn all_flags(cmd: &Cmd) -> Vec<&Flag> {
    let globals = GLOBAL.iter().map(|g| cmd.flags.iter().find(|f| f.name == g.name).unwrap_or(g));
    globals.chain(cmd.flags.iter().filter(|f| GLOBAL.iter().all(|g| g.name != f.name))).collect()
}

/// A two-column list of an oclif help section.
fn section(header: &str, rows: &[(String, String)]) -> String {
    let left = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
    let body = rows.iter().map(|(l, r)| {
        let lines = wrap(r, WIDTH - 2 - left - 2);
        let rest = lines[1..].iter().map(|line| format!("\n  {:left$}  {line}", "")).collect::<String>();
        format!("  {l:left$}  {}{rest}", lines[0]).trim_end().to_string()
    });
    format!("{header}\n{}", body.collect::<Vec<_>>().join("\n"))
}

fn usage(cmd: &Cmd, id: &str) -> String {
    let flags = all_flags(cmd);
    let (required, optional): (Vec<&&Flag>, Vec<&&Flag>) = flags.iter().partition(|f| f.required);
    let element = |f: &Flag| {
        let name = f.char.map_or(format!("--{}", f.name), |c| format!("-{c}"));
        let value = match (f.kind, f.options) {
            (Kind::Bool | Kind::AllowNo, _) => String::new(),
            (_, []) => format!(" <value>{}", if f.kind == Kind::Many { "..." } else { "" }),
            (_, options) => format!(" {}", options.join("|")),
        };
        if f.required {
            format!("{name}{value}")
        } else {
            format!("[{name}{value}]")
        }
    };
    let args =
        cmd.args
            .iter()
            .map(|a| if a.required { a.name.to_uppercase() } else { format!("[{}]", a.name.to_uppercase()) });
    let parts: Vec<String> = args.chain(required.iter().chain(&optional).map(|f| element(f))).collect();
    // oclif measures the line with its `<%= command.id %>` template, and fills in the id later.
    let line = format!("$ {BIN} <%= command.id %> {}", parts.join(" "));
    let line = match line.chars().count() > WIDTH - 2 {
        false => line,
        true => {
            let split = line[..WIDTH - 2].rfind(' ').expect("a space in the usage");
            let rest = wrap(&line[split..], WIDTH - 4).iter().map(|l| format!("\n    {l}")).collect::<String>();
            format!("{}{rest}", &line[..split])
        }
    };
    format!("USAGE\n  {}", line.replace("<%= command.id %>", id))
}

/// The oclif help of a command.
pub fn help(cmd: &Cmd) -> String {
    help_as(cmd, cmd.id)
}

fn help_as(cmd: &Cmd, id: &str) -> String {
    let mut out = vec![cmd.summary.to_string(), usage(cmd, id)];
    if !cmd.args.is_empty() {
        let rows: Vec<(String, String)> = cmd
            .args
            .iter()
            .map(|a| {
                let name = if a.required { a.name.to_uppercase() } else { format!("[{}]", a.name.to_uppercase()) };
                let options = if a.options.is_empty() { String::new() } else { format!("({}) ", a.options.join("|")) };
                (name, format!("{options}{}", a.desc))
            })
            .collect();
        out.push(section("ARGUMENTS", &rows));
    }
    let mut flags = all_flags(cmd);
    flags.sort_by_key(|f| (f.char.is_none(), f.char, f.name));
    let rows: Vec<(String, String)> = flags
        .iter()
        .map(|f| {
            let name = if f.kind == Kind::AllowNo { format!("--[no-]{}", f.name) } else { format!("--{}", f.name) };
            let label = f.char.map_or(format!("    {name}"), |c| format!("-{c}, {name}"));
            let value = match (f.kind, f.options) {
                (Kind::Bool | Kind::AllowNo, _) => String::new(),
                (Kind::Many, []) => "=<value>...".into(),
                (_, []) => "=<value>".into(),
                (_, _) => "=<option>".into(),
            };
            let default = match (f.kind, f.default) {
                (Kind::Str | Kind::Int | Kind::Many, Some(d)) => format!("[default: {d}] "),
                _ => String::new(),
            };
            let required = if f.required { "(required) " } else { "" };
            let options =
                if f.options.is_empty() { String::new() } else { format!("\n<options: {}>", f.options.join("|")) };
            (format!("{label}{value}"), format!("{required}{default}{}{options}", f.desc))
        })
        .collect();
    out.push(section("FLAGS", &rows));
    if !cmd.description.is_empty() {
        let text = cmd.description.replace("`beeper ", &format!("`{BIN} "));
        out.push(format!(
            "DESCRIPTION\n{}",
            wrap(&text, WIDTH - 2).iter().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
        ));
    }
    if !cmd.examples.is_empty() {
        let examples = cmd
            .examples
            .iter()
            .map(|e| format!("  $ {BIN}{}", e.strip_prefix("beeper").expect("an example starts with beeper")));
        out.push(format!("EXAMPLES\n{}", examples.collect::<Vec<_>>().join("\n\n")));
    }
    let topic = cmd.id.split(' ').next().expect("an id has a word");
    if TOPICS.iter().any(|t| t.2 == Some(cmd.id)) && id == topic {
        out.push(commands_section(topic));
    }
    out.join("\n\n") + "\n"
}

fn commands_section(topic: &str) -> String {
    let rows: Vec<(String, String)> = all()
        .filter(|c| c.id.split(' ').next() == Some(topic) && c.id != topic)
        .map(|c| (c.id.to_string(), c.summary.to_string()))
        .collect();
    section("COMMANDS", &rows)
}

/// The oclif help of a topic.
pub fn topic_help(t: &Topic) -> String {
    match t.2 {
        Some(index) => {
            let cmd = all().find(|c| c.id == index).expect("the index command of a topic exists");
            help_as(cmd, t.0)
        }
        None => format!("{}\n\nUSAGE\n  $ {BIN} {} COMMAND\n\n{}\n", t.1, t.0, commands_section(t.0)),
    }
}

/// The topics and the commands without a topic, for the root help.
pub fn root_rows() -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = TOPICS.iter().map(|t| (t.0.to_string(), t.1.to_string())).collect();
    rows.extend(
        all().filter(|c| !c.id.contains(' ') && topic(c.id).is_none()).map(|c| (c.id.into(), c.summary.into())),
    );
    rows.sort();
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_follows_oclif() {
        let status = all().find(|c| c.id == "status").unwrap();
        let help = help(status);
        assert!(help.starts_with(&format!(
            "Show selected target and setup readiness\n\nUSAGE\n  $ {BIN} status [--base-url <value>] [-t <value>] [--debug] [--events] [--full]\n    [--json] [-q] [--read-only] [--timeout <value>] [-y]\n\nFLAGS\n  -q, --quiet             Suppress spinners and success lines (errors still print). Honored with or\n                          without --json.\n"
        )), "{help}");
        assert!(help.contains("\n      --timeout=<value>   Maximum time to wait, such as 30s, 2m, or 1h\n"), "{help}");
        let request = all().find(|c| c.id == "api request").unwrap();
        let help = super::help(request);
        assert!(help.contains("ARGUMENTS\n  METHOD  (GET|POST|PUT|PATCH|DELETE) HTTP method\n  PATH    API path, for example /v1/info\n"), "{help}");
        assert!(
            help.contains("\n      --[no-]json         Print JSON\n      --no-auth           Call a public API path"),
            "{help}"
        );
    }
}
