//! The `targets` topic and `auth logout`: the targets and tokens in the CLI config folder.

use super::commands::{cmd, live_status};
use super::{
    arg, config, config_dir, js_string, list_targets, obj, opt_arg, read_json, write_json, CliError, Cmd, Ctx, Flag,
    Res, BIN,
};
use serde_json::{json, Map, Value};

const NAME: &str = "Target name. Defaults to the selected target.";

pub const COMMANDS: &[Cmd] = &[
    cmd("auth logout", "Clear stored authentication", logout),
    Cmd {
        args: &[arg("name", "Local name for the target"), arg("url", "Base URL of the remote Desktop or Server API")],
        flags: &[Flag::b("default", "Set this target as the default after creation")],
        ..cmd("targets add remote", "Add a remote Beeper Desktop or Server target", add_remote)
    },
    cmd("targets list", "List configured Beeper targets", list),
    Cmd { args: &[arg("name", "Target name")], ..cmd("targets remove", "Remove a target", remove) },
    Cmd { args: &[opt_arg("name", NAME)], ..cmd("targets show", "Show target details", show) },
    Cmd {
        args: &[opt_arg("name", NAME)],
        ..cmd("targets status", "Check endpoint and process reachability for a target", status)
    },
    Cmd { args: &[arg("name", "Target name")], ..cmd("targets use", "Set the default target", use_target) },
];

fn target_path(id: &str) -> std::path::PathBuf {
    config_dir().join("targets").join(format!("{id}.json"))
}

fn config_path() -> std::path::PathBuf {
    config_dir().join("config.json")
}

fn update_config(change: impl FnOnce(&mut Map<String, Value>)) -> Res {
    let mut config = config()?;
    change(&mut config);
    write_json(&config_path(), &config)
}

/// `targets show` and `targets status` select the target of the argument, else of the flags.
fn named(c: &mut Ctx) -> Res<Map<String, Value>> {
    if let (Some(name), None) = (c.arg(0).map(str::to_string), c.str("base-url")) {
        c.set_target(super::resolve_target(Some(&name))?);
    }
    Ok(c.target()?.clone())
}

fn list(c: &mut Ctx) -> Res {
    let config = config()?;
    let mut targets = list_targets()?;
    if targets.is_empty() {
        let desktop =
            [("id", "desktop"), ("type", "desktop"), ("name", "Beeper Desktop"), ("baseURL", "http://127.0.0.1:23373")];
        targets.push(
            desktop
                .into_iter()
                .map(|(k, v)| (k.to_string(), json!(v)))
                .chain([("managed".into(), json!(false))])
                .collect(),
        );
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
    let target = read_json(&target_path(&name))?.ok_or_else(unknown)?;
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
    if read_json(&target_path(&name))?.is_some() {
        return Err(CliError::bug(format!("Target \"{name}\" already exists.")));
    }
    let target = obj([
        ("id", json!(name)),
        ("name", json!(name)),
        ("type", json!("remote")),
        ("baseURL", json!(url)),
        ("managed", json!(false)),
    ]);
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
