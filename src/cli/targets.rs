//! The `targets` topic and `auth logout`: the targets and tokens in the CLI config folder, and the local runtimes of
//! managed targets.

use super::commands::{cmd, live_status};
use super::profiles;
use super::{
    arg, config, config_dir, js_string, list_targets, obj, opt_arg, read_target, target_path, truthy, update_config, write_json, CliError,
    Cmd, Ctx, Flag, Res, BIN,
};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};

const NAME: &str = "Target name. Defaults to the selected target.";
const NAME_ARG: &[super::Arg] = &[opt_arg("name", NAME)];
const DEFAULT: Flag = Flag::b("default", "Set this target as the default after creation");
pub const SERVER_ENV: Flag =
    Flag::s("server-env", "Server environment. Staging forces nightly.").opts(&["production", "staging"]).def("production");

pub const COMMANDS: &[Cmd] = &[
    cmd("auth logout", "Clear stored authentication", logout),
    Cmd {
        args: &[opt_arg("name", "Target name (default: \"desktop\")")],
        flags: &[Flag::i("port", "TCP port the managed Desktop will expose its API on"), DEFAULT, SERVER_ENV],
        ..cmd("targets add desktop", "Add a managed Beeper Desktop target", |c| add_managed(c, "desktop"))
    },
    Cmd {
        args: &[arg("name", "Local name for the target"), arg("url", "Base URL of the remote Desktop or Server API")],
        flags: &[DEFAULT],
        ..cmd("targets add remote", "Add a remote Beeper Desktop or Server target", add_remote)
    },
    Cmd {
        args: &[opt_arg("name", "Target name (default: \"server\")")],
        flags: &[Flag::i("port", "TCP port the managed Server will expose its API on"), DEFAULT, SERVER_ENV],
        ..cmd("targets add server", "Add a managed Beeper Server target", |c| add_managed(c, "server"))
    },
    Cmd { args: NAME_ARG, ..cmd("targets disable", "Disable a local Beeper Server target at login", disable) },
    Cmd { args: NAME_ARG, ..cmd("targets enable", "Enable a local Beeper Server target at login", enable) },
    cmd("targets list", "List configured Beeper targets", list),
    Cmd {
        args: NAME_ARG,
        flags: &[
            Flag::i("lines", "Lines to print from each log file").def("200"),
            Flag::i("files", "Desktop log files to print, newest first").def("5"),
            Flag::b("all", "Print all matching log files instead of only recent files"),
        ],
        ..cmd("targets logs", "Print logs for a local Beeper Desktop or Server install", logs)
    },
    Cmd { args: &[arg("name", "Target name")], ..cmd("targets remove", "Remove a target", remove) },
    Cmd { args: NAME_ARG, ..cmd("targets restart", "Restart a local Beeper Server target", restart) },
    Cmd { args: NAME_ARG, ..cmd("targets show", "Show target details", show) },
    Cmd { args: NAME_ARG, ..cmd("targets start", "Start a local Server target or open Beeper Desktop", start) },
    Cmd { args: NAME_ARG, ..cmd("targets status", "Check endpoint and process reachability for a target", status) },
    Cmd { args: NAME_ARG, ..cmd("targets stop", "Stop a local Beeper Server target", stop) },
    Cmd { args: &[arg("name", "Target name")], ..cmd("targets use", "Set the default target", use_target) },
];

/// The targets commands select the target of the argument, else of the flags.
fn named(c: &mut Ctx) -> Res<Map<String, Value>> {
    if let (Some(name), None) = (c.arg(0).map(str::to_string), c.str("base-url")) {
        c.set_target(super::resolve_target(Some(&name))?);
    }
    Ok(c.target()?.clone())
}

fn managed(t: &Map<String, Value>) -> bool {
    t.get("managed").is_some_and(truthy)
}

/// `createProfileTarget`: a managed target of `kind` with its data folder in the CLI folder, on `port` or else the
/// first port after 23373 that no target uses.
pub fn create_profile_target(kind: &str, id: &str, server_env: &str, port: Option<i64>) -> Res<Map<String, Value>> {
    let used: Vec<i64> = list_targets()?.iter().filter_map(|t| t.get("port").and_then(Value::as_i64)).collect();
    let port = match port {
        Some(port) => port,
        None => {
            (23374..23573).find(|p| !used.contains(p)).ok_or_else(|| CliError::bug("No available default port for a new Beeper target."))?
        }
    };
    let dir = config_dir().join("profiles").join(kind).join(id);
    std::fs::create_dir_all(&dir).map_err(|e| CliError::bug(format!("{}: {e}", dir.display())))?;
    let dir = json!(dir.display().to_string());
    let target = obj([
        ("id", json!(id)),
        ("type", json!(kind)),
        ("name", json!(id)),
        ("baseURL", json!(format!("http://127.0.0.1:{port}"))),
        ("managed", json!(true)),
        ("dataDir", dir.clone()),
        ("profile", json!(id)),
        ("runtime", json!({ "install": kind, "dataDir": dir, "port": port })),
        ("serverEnv", json!(server_env)),
        ("port", json!(port)),
    ]);
    write_json(&target_path(id), &target)?;
    Ok(target)
}

/// `targets add desktop` and `targets add server`.
fn add_managed(c: &mut Ctx, kind: &str) -> Res {
    c.writable()?;
    let id = c.arg(0).unwrap_or(kind).to_string();
    if read_target(&id)?.is_some() {
        return Err(CliError::bug(format!("Target \"{id}\" already exists.")));
    }
    let env = c.str("server-env").expect("--server-env has a default").to_string();
    let target = create_profile_target(kind, &id, &env, c.int("port"))?;
    if c.on("default") {
        update_config(|config| drop(config.insert("defaultTarget".into(), json!(id))))?;
    }
    let base = js_string(&target["baseURL"]);
    c.success(&format!("Added target: {id}"), Some(&base), Value::Object(target))
}

fn start(c: &mut Ctx) -> Res {
    c.writable()?;
    let target = named(c)?;
    let (id, base) = (js_string(&target["id"]), js_string(&target["baseURL"]));
    if target["type"] == "desktop" && id != "custom" {
        let result = profiles::launch_desktop(Some(&target).filter(|t| managed(t)))?;
        return c.success("Opened Beeper Desktop", Some(&base), json!({ "target": target, "result": result }));
    }
    if !managed(&target) || target["type"] != "server" {
        return Err(CliError::bug(format!("Target \"{id}\" is not a local Beeper Server install.")));
    }
    let result = profiles::start(&target)?;
    c.success(&format!("Started target: {id}"), Some(&base), json!({ "target": target, "result": result }))
}

fn stop(c: &mut Ctx) -> Res {
    c.writable()?;
    let target = named(c)?;
    profiles::assert_server(&target)?;
    profiles::stop(&target)?;
    c.success(&format!("Stopped target: {}", js_string(&target["id"])), None, json!({ "target": target }))
}

fn restart(c: &mut Ctx) -> Res {
    c.writable()?;
    let target = named(c)?;
    profiles::assert_server(&target)?;
    drop(profiles::stop(&target));
    let result = profiles::start(&target)?;
    let (id, base) = (js_string(&target["id"]), js_string(&target["baseURL"]));
    c.success(&format!("Restarted target: {id}"), Some(&base), json!({ "target": target, "result": result }))
}

/// `targets enable` and `targets disable`.
fn at_login(c: &mut Ctx, on: bool) -> Res {
    c.writable()?;
    let target = named(c)?;
    profiles::assert_server(&target)?;
    let path = if on { profiles::enable(&target)? } else { profiles::disable(&target)? };
    let path = path.display().to_string();
    let message = format!("{} target at login: {}", if on { "Enabled" } else { "Disabled" }, js_string(&target["id"]));
    c.success(&message, Some(&path), json!({ "target": target, "path": path }))
}

fn enable(c: &mut Ctx) -> Res {
    at_login(c, true)
}

fn disable(c: &mut Ctx) -> Res {
    at_login(c, false)
}

fn logs(c: &mut Ctx) -> Res {
    let target = named(c)?;
    let id = js_string(&target["id"]);
    let lines = c.int("lines").expect("--lines has a default");
    if target["type"] == "remote" || id == "custom" {
        return Err(CliError::bug(format!("Target \"{id}\" is remote and has no local logs.")));
    }
    if target["type"] == "server" {
        if !managed(&target) {
            return Err(CliError::bug(format!("Target \"{id}\" is not a local Beeper Server install.")));
        }
        print_log(&profiles::log_path(&id), lines);
        print_log(&profiles::error_log_path(&id), lines);
        return Ok(());
    }
    // `desktopLogDir`: the logs of the data folder of a managed Desktop, else of the default one.
    let dir = target.get("dataDir").and_then(Value::as_str).filter(|_| managed(&target)).map(PathBuf::from);
    let dir = dir.unwrap_or_else(|| beeper_core::platform::current_desktop_data_dir(None)).join("logs");
    let mut files = log_files(&dir);
    files.sort_by_key(|f| std::cmp::Reverse(f.1));
    let count = if c.on("all") { files.len() } else { usize::try_from(c.int("files").expect("--files has a default")).unwrap_or(0) };
    files.iter().take(count).for_each(|f| print_log(&f.0, lines));
    Ok(())
}

/// The `.log` files in `dir` and its folders, with their change times.
fn log_files(dir: &Path) -> Vec<(PathBuf, std::time::SystemTime)> {
    let entries = std::fs::read_dir(dir).into_iter().flatten().flatten();
    let files = entries.flat_map(|e| match e.file_type() {
        Ok(t) if t.is_dir() => log_files(&e.path()),
        Ok(t) if t.is_file() && e.file_name().to_string_lossy().ends_with(".log") => {
            e.metadata().and_then(|m| m.modified()).map(|time| vec![(e.path(), time)]).unwrap_or_default()
        }
        _ => Vec::new(),
    });
    files.collect()
}

/// `printLogFile`: the last `lines` lines of a log with text, under its path.
fn print_log(path: &Path, lines: i64) {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    if text.is_empty() {
        return;
    }
    let parts: Vec<&str> = text.split('\n').collect();
    let skip = usize::try_from(lines).ok().filter(|n| *n > 0).map_or(0, |n| parts.len().saturating_sub(n + 1));
    let tail = parts[skip..].join("\n");
    print!("\n==> {} <==\n{tail}{}", path.display(), if tail.ends_with('\n') { "" } else { "\n" });
}

fn list(c: &mut Ctx) -> Res {
    let config = config()?;
    let mut targets = list_targets()?;
    if targets.is_empty() {
        let desktop = [("id", "desktop"), ("type", "desktop"), ("name", "Beeper Desktop"), ("baseURL", "http://127.0.0.1:23373")];
        targets.push(desktop.into_iter().map(|(k, v)| (k.to_string(), json!(v))).chain([("managed".into(), json!(false))]).collect());
    }
    let mut rows = Vec::new();
    for t in targets {
        let default = match config.get("defaultTarget").and_then(Value::as_str) {
            Some(id) => t["id"] == id,
            None => t["id"] == "desktop",
        };
        let name = t.get("name").filter(|n| !n.is_null()).unwrap_or(&t["id"]).clone();
        let mut row = obj([
            ("default", json!(default)),
            ("id", t["id"].clone()),
            ("type", t["type"].clone()),
            ("name", name),
            ("managed", t.get("managed").cloned().unwrap_or_default()),
            ("baseURL", t["baseURL"].clone()),
            ("runtime", t.get("runtime").cloned().unwrap_or_default()),
        ]);
        c.set_target(t);
        row.extend(live_status(c)?.as_object().expect("a live status is an object").clone());
        rows.push(Value::Object(row));
    }
    c.print(&Value::Array(rows))
}

fn show(c: &mut Ctx) -> Res {
    let target = named(c)?;
    c.print(&Value::Object(target))
}

fn status(c: &mut Ctx) -> Res {
    let target = named(c)?;
    let live = live_status(c)?;
    let mut all = Map::from_iter([("target".to_string(), Value::Object(target))]);
    all.extend(live.as_object().expect("a live status is an object").clone());
    c.print(&Value::Object(all))?;
    // Upstream sets `process.exitCode = 1` without an error message.
    match live["reachable"] == true {
        true => Ok(()),
        false => std::process::exit(1),
    }
}

fn use_target(c: &mut Ctx) -> Res {
    c.writable()?;
    let name = c.arg(0).expect("a required arg").to_string();
    let unknown = || CliError::bug(format!("Unknown Beeper target \"{name}\". Run `{BIN} targets list`."));
    let target = read_target(&name)?.ok_or_else(unknown)?;
    update_config(|config| drop(config.insert("defaultTarget".into(), target["id"].clone())))?;
    let message = format!("Using target: {}", js_string(&target["id"]));
    c.success(&message, target["baseURL"].as_str(), Value::Object(target.clone()))
}

fn remove(c: &mut Ctx) -> Res {
    c.writable()?;
    let name = c.arg(0).expect("a required arg").to_string();
    match std::fs::remove_file(target_path(&name)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(CliError::bug(e.to_string())),
        _ => {}
    }
    update_config(|config| {
        if config.get("defaultTarget").is_some_and(|d| *d == name) {
            config.shift_remove("defaultTarget");
        }
        if name == "desktop" {
            config.shift_remove("auth");
            config.shift_remove("baseURL");
        }
    })?;
    c.success(&format!("Removed target: {name}"), None, json!({ "id": name }))
}

fn add_remote(c: &mut Ctx) -> Res {
    c.writable()?;
    let (name, url) = (c.arg(0).expect("a required arg").to_string(), c.arg(1).expect("a required arg").to_string());
    if read_target(&name)?.is_some() {
        return Err(CliError::bug(format!("Target \"{name}\" already exists.")));
    }
    let target =
        obj([("id", json!(name)), ("name", json!(name)), ("type", json!("remote")), ("baseURL", json!(url)), ("managed", json!(false))]);
    write_json(&target_path(&name), &target)?;
    if c.on("default") {
        update_config(|config| drop(config.insert("defaultTarget".into(), json!(name))))?;
    }
    c.success(&format!("Added target: {name}"), Some(&url), Value::Object(target))
}

/// `auth logout`: revokes the stored token of the target and forgets it.
fn logout(c: &mut Ctx) -> Res {
    c.writable()?;
    let target = c.target()?.clone();
    let token = target.get("auth").and_then(|a| a["accessToken"].as_str()).map(str::to_string);
    if std::env::var("BEEPER_ACCESS_TOKEN").is_ok_and(|t| !t.is_empty()) && token.is_none() {
        let msg = "auth logout cannot clear BEEPER_ACCESS_TOKEN from the environment; unset it in the calling process.";
        return Err(CliError::bug(msg));
    }
    let mut revoked = false;
    if let Some(token) = &token {
        let client = beeper_api::client::BeeperClient::try_new(&c.base_url()?, "").map_err(CliError::bug)?;
        let form = format!("token={}&token_type_hint=access_token", beeper_core::codec::url_encode(token));
        let body = Some(("application/x-www-form-urlencoded".to_string(), form.into_bytes()));
        let post = client.request_raw("POST", "/oauth/revoke", &[], body);
        let reply = c.block(async { tokio::time::timeout(std::time::Duration::from_secs(5), post).await });
        revoked = matches!(reply, Ok(Ok((200..=299, _))));
        // `clearTargetAuth`.
        match target["id"].as_str() {
            Some("custom") => update_config(|config| drop(config.shift_remove("auth")))?,
            id => {
                let mut stored = target.clone();
                stored.shift_remove("auth");
                write_json(&target_path(&js_string(&target["id"])), &stored)?;
                if id == Some("desktop") {
                    update_config(|config| drop(config.shift_remove("auth")))?;
                }
            }
        }
    }
    let detail = if token.is_some() { "local token cleared" } else { "no token was stored" };
    c.success("Logged out", Some(detail), json!({ "revoked": revoked, "hadToken": token.is_some() }))
}
