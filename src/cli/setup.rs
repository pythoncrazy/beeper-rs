//! `setup` (`commands/setup.js`) and `auth email` (`lib/setup-login.js`, `lib/local-desktop.js`): they make a target
//! ready, with a token from the browser sign-in, from the session of the app on this device, or from an email
//! sign-in.
//!
//! beeper-rs is both Beeper Desktop and Beeper Server, so it has nothing to install: the local session is the one in
//! the data folder of beeper-rs, and `--install` only keeps upstream's check for `--yes`.

use super::commands::{cmd, readiness_with};
use super::oauth::{ensure_desktop_token, find_local_desktop, login_with_pkce};
use super::targets::{create_profile_target, SERVER_ENV};
use super::{
    config, confirm, interactive, iso_in, js_string, list_targets, obj, profiles, prompt, read_target, target_path, truthy, update_config,
    write_event, write_json, CliError, Cmd, Ctx, Flag, Res, BIN,
};
use beeper_api::client::BeeperClient;
use serde_json::{json, Map, Value};
use std::path::PathBuf;
use std::time::{Duration, Instant};

type Target = Map<String, Value>;

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        flags: &[
            Flag::s("code", "Email verification code").req(),
            Flag::s("setup-request-id", "Setup request ID from auth email start").req(),
            Flag::s("username", "Username to use if setup creates a new account"),
            Flag::b("yes", "Accept required registration prompts non-interactively"),
        ],
        ..cmd("auth email response", "Finish email sign-in with a verification code", email_response)
    },
    Cmd {
        flags: &[Flag::s("email", "Email address to sign in with").req()],
        ..cmd("auth email start", "Start email sign-in for a target", email_start)
    },
    Cmd {
        flags: &[
            Flag::b("local", "Use the local Beeper Desktop session on this device"),
            Flag::b("oauth", "Authorize the target with browser OAuth/PKCE"),
            Flag::s("remote", "Connect to a remote Beeper Desktop or Server URL"),
            Flag::b("server", "Set up a local Beeper Server target"),
            Flag::b("desktop", "Set up a local Beeper Desktop target"),
            Flag::b("install", "Allow installing missing managed runtime"),
            Flag::s("channel", "Install release channel").opts(&["stable", "nightly"]).def("stable"),
            SERVER_ENV,
            Flag::s("email", "Sign in with an email address"),
            Flag::s("username", "Username to use if setup creates a new account"),
        ],
        examples: &[
            "beeper setup --local",
            "beeper setup --oauth",
            "beeper setup --remote https://my-beeper.example.com",
            "beeper setup --server --install",
            "beeper setup --desktop --install",
        ],
        ..cmd("setup", "Make the selected target ready for messaging", setup)
    },
];

fn id(t: &Target) -> String {
    js_string(&t["id"])
}

fn base(t: &Target) -> String {
    js_string(&t["baseURL"])
}

/// A target's `name ?? id`.
fn name(t: &Target) -> String {
    t.get("name").filter(|n| !n.is_null()).map_or_else(|| id(t), js_string)
}

/// The prompts and the progress lines show only to someone at a terminal.
fn chatty(c: &Ctx) -> bool {
    !c.json() && interactive()
}

fn write_target(t: &Target) -> Res {
    write_json(&target_path(&id(t)), t)
}

/// `saveTargetAuth`: the config keeps the auth of the `custom` target of `--base-url`, and a stored target its own.
fn save_target_auth(t: &Target, auth: &Value) -> Res {
    if id(t) == "custom" {
        return update_config(|config| {
            config.insert("baseURL".into(), t["baseURL"].clone());
            config.insert("auth".into(), auth.clone());
        });
    }
    let mut stored = t.clone();
    stored.insert("auth".into(), auth.clone());
    write_target(&stored)
}

/// `publicTarget`: the target without its token.
fn public_target(t: &Target) -> Value {
    let mut rest = t.clone();
    if let Some(auth) = rest.shift_remove("auth").filter(truthy) {
        let public = obj([("source", auth["source"].clone()), ("tokenType", auth["tokenType"].clone())]);
        rest.insert("auth".into(), Value::Object(public));
    }
    Value::Object(rest)
}

fn with_auth(t: &Target, auth: &Value) -> Target {
    let mut t = t.clone();
    t.insert("auth".into(), auth.clone());
    t
}

/// `evaluateReadiness` of the target `t` with `token`.
fn readiness_of(c: &mut Ctx, t: &Target, token: Option<String>) -> Res<Value> {
    c.set_target(t.clone());
    readiness_with(c, token)
}

/// `evaluateReadiness` of `t` with the token that the target or the environment gives.
fn stored_readiness(c: &mut Ctx, t: &Target) -> Res<Value> {
    c.set_target(t.clone());
    let token = c.stored_token()?;
    readiness_with(c, token)
}

fn set_default_target(id: &str) -> Res {
    update_config(|config| {
        if config.get("defaultTarget").is_none_or(Value::is_null) {
            config.insert("defaultTarget".into(), json!(id));
        }
    })
}

fn step_event(c: &Ctx, step: &str, target: Option<String>) {
    if c.on("events") {
        write_event("setup_step", Value::Object(obj([("step", json!(step)), ("target", json!(target))])));
    }
}

fn progress(c: &Ctx, message: &str) {
    if chatty(c) && !c.on("quiet") {
        println!("{message}...");
    }
}

/// `accountName` of `lib/local-desktop.js`.
fn account_name(item: &Value) -> Option<String> {
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    let network = &item["network"];
    [&item["network"], &network["displayName"], &network["name"], &item["displayName"], &item["name"]]
        .into_iter()
        .chain([&item["bridge"]["type"], &item["accountID"], &item["id"]])
        .find_map(text)
}

/// `connectedAccountSummary`: the names of up to 8 accounts of the target.
fn account_summary(c: &Ctx, t: &Target, token: &str) -> Vec<String> {
    let Ok(client) = BeeperClient::try_new(&base(t), token) else { return Vec::new() };
    let rows = match c.block(client.call("GET", "/v1/accounts", None)) {
        Ok(Value::Array(rows)) => rows,
        Ok(mut page) => page["items"].as_array_mut().map(std::mem::take).unwrap_or_default(),
        Err(_) => return Vec::new(),
    };
    rows.iter().filter_map(account_name).take(8).collect()
}

/// The result of a sign-in: `{accounts, authSource, readiness, target}`.
fn signed_in(c: &mut Ctx, t: &Target, auth: &Value) -> Res<Value> {
    write_target(t)?;
    save_target_auth(t, auth)?;
    let token = auth["accessToken"].as_str().unwrap_or_default().to_string();
    let readiness = readiness_of(c, t, Some(token.clone()))?;
    let (accounts, target) = (account_summary(c, t, &token), public_target(&with_auth(t, auth)));
    Ok(json!({ "accounts": accounts, "authSource": auth["source"], "readiness": readiness, "target": target }))
}

// ─── setup ─────────────────────────────────────────────────────────────────────

fn setup(c: &mut Ctx) -> Res {
    c.writable()?;
    let remote = c.str("remote").map(str::to_string);
    let (server, desktop) = (c.on("server"), c.on("desktop"));
    if [remote.is_some(), server, desktop].iter().filter(|b| **b).count() > 1 {
        return Err(CliError::bug("Specify at most one of --remote, --server, or --desktop"));
    }
    let (local, oauth, email) = (c.on("local"), c.on("oauth"), c.str("email").is_some());
    if [local, oauth, email].iter().filter(|b| **b).count() > 1 {
        return Err(CliError::bug("Specify at most one of --local, --oauth, or --email"));
    }
    if (local || oauth) && (remote.is_some() || server || desktop) {
        let msg = "Use --local or --oauth with an existing target, not with --remote, --server, or --desktop.";
        return Err(CliError::bug(msg));
    }
    step_event(c, "start", c.str("target").map(str::to_string));
    if let Some(url) = remote {
        return setup_remote(c, &url);
    }
    if server || desktop {
        return setup_managed(c, if server { "server" } else { "desktop" });
    }
    let target = setup_target(c)?;
    let result = match (local, oauth, email) {
        (true, _, _) => commit_local(prepare_local(c, &target)?)?,
        (_, true, _) => setup_oauth(c, &target, None)?,
        (_, _, true) => setup_email(c, &target)?,
        _ => return setup_default(c, &target),
    };
    print_result(c, result)
}

/// `setupTarget`: `--base-url`, `-t`, the default target, the stored `desktop` target, or a new `desktop` target.
fn setup_target(c: &mut Ctx) -> Res<Target> {
    if let Some(url) = c.str("base-url") {
        return Ok(obj([("id", json!("custom")), ("type", json!("desktop")), ("baseURL", json!(url))]));
    }
    if let Some(name) = c.str("target") {
        let unknown = || CliError::bug(format!("Unknown Beeper target \"{name}\". Run `{BIN} targets list`."));
        return read_target(name)?.ok_or_else(unknown);
    }
    if let Some(id) = config()?.get("defaultTarget").and_then(Value::as_str) {
        if let Some(t) = read_target(id)? {
            return Ok(t);
        }
    }
    match read_target("desktop")? {
        Some(t) => Ok(t),
        None => default_desktop_target(c),
    }
}

/// `defaultDesktopTarget`: the `desktop` target on the port where Desktop answers.
fn default_desktop_target(c: &Ctx) -> Res<Target> {
    let preferred = config()?.get("baseURL").and_then(Value::as_str).unwrap_or("http://127.0.0.1:23373").to_string();
    let found = find_local_desktop(c, &preferred, true, Duration::from_millis(300)).ok();
    let target = obj([
        ("id", json!("desktop")),
        ("type", json!("desktop")),
        ("name", json!("Beeper Desktop")),
        ("baseURL", json!(found.map_or("http://127.0.0.1:23373".into(), |f| f.0))),
        ("managed", json!(false)),
        ("runtime", json!({ "install": "desktop", "port": 23373 })),
    ]);
    write_target(&target)?;
    set_default_target("desktop")?;
    Ok(target)
}

fn setup_command(t: &Target) -> String {
    match id(t).as_str() {
        "desktop" => format!("{BIN} setup"),
        id => format!("{BIN} setup -t {id}"),
    }
}

fn action(id: &str, command: impl Into<String>) -> Value {
    json!({ "id": id, "command": command.into() })
}

/// `setupDetailForReadiness`.
fn detail(readiness: &Value, t: &Target) -> Option<String> {
    let flag = if id(t) == "desktop" { String::new() } else { format!(" -t {}", id(t)) };
    Some(match readiness["state"].as_str().unwrap_or_default() {
        "needs-login" => format!("Sign in to Beeper Desktop, then run `{BIN} setup` again."),
        "needs-verification" | "verification-in-progress" => "Continue verification to finish setup.".into(),
        "needs-recovery-key" | "needs-secrets" => format!("Run `{BIN} verify recovery-key{flag}`."),
        "needs-cross-signing-setup" => format!("Run `{BIN} verify reset-recovery-key{flag}`."),
        "needs-first-sync" | "initializing" => {
            format!("Beeper is still syncing. You can rerun `{BIN} setup` at any time.")
        }
        _ => return readiness["message"].as_str().map(str::to_string),
    })
}

/// What `setup` found of Desktop on this device (`detectDesktopSetup`). beeper-rs is always installed.
enum Detected {
    SessionFound(Box<Local>),
    InstalledNotRunning,
    RunningSignedOut(Value),
    SessionUnreadable(String, Value),
}

fn setup_default(c: &mut Ctx, target: &Target) -> Res {
    let setup_cmd = setup_command(target);
    if chatty(c) && !c.on("quiet") {
        // Upstream draws the Beeper logo above this.
        println!("Setup\n");
        if id(target) != "desktop" || c.str("target").is_some() {
            println!("Continuing setup for {}.\n", name(target));
        }
    }
    if target["type"] == "desktop" {
        match detect(c, target) {
            Detected::SessionFound(local) => {
                if c.on("yes") {
                    let result = commit_local(*local)?;
                    return print_result(c, result);
                }
                if !chatty(c) {
                    return c.print(&session_found(&local, &setup_cmd));
                }
                println!("Found Beeper Desktop on this device.\n");
                let ready = local.readiness["state"] == "ready";
                let status = if ready { "signed in and ready".into() } else { js_string(&local.readiness["state"]) };
                println!("Status: {status}");
                if let Some(user) = local.session["user_id"].as_str().filter(|u| !u.is_empty()) {
                    println!("Signed in as: {user}");
                }
                if !local.accounts.is_empty() {
                    println!("Connected accounts: {}", local.accounts.join(", "));
                }
                println!();
                if confirm("Use this Desktop session for CLI access?", true)? {
                    let result = commit_local(*local)?;
                    return print_result(c, result);
                }
                let message = match ready {
                    true => "Beeper Desktop is ready".into(),
                    false => format!("Setup paused: {}", js_string(&local.readiness["state"])),
                };
                let data = json!({ "target": public_target(&local.target), "readiness": local.readiness });
                return c.success(&message, detail(&local.readiness, &local.target).as_deref(), data);
            }
            detected if !chatty(c) => return c.print(&state_output(&detected, target)),
            Detected::InstalledNotRunning => {
                println!("Found Beeper Desktop on this device.\n\nStatus: installed, not running\n");
                if c.on("yes") || confirm("Launch Beeper Desktop now?", true)? {
                    return launch_and_poll(c, target, &setup_cmd);
                }
            }
            Detected::RunningSignedOut(_) => {
                println!("Found Beeper Desktop on this device.\n\nStatus: running, signed out\n");
                if c.on("yes") || confirm("Open Beeper Desktop so you can sign in?", true)? {
                    return launch_and_poll(c, target, &setup_cmd);
                }
            }
            Detected::SessionUnreadable(reason, _) => {
                let status = "signed in, but CLI could not read the local session";
                println!("Found Beeper Desktop on this device.\n\nStatus: {status}\n");
                println!("You can still connect through Beeper Desktop.");
                if c.on("debug") {
                    println!("\n{reason}");
                }
                println!();
                if c.on("yes") || confirm("Connect through Beeper Desktop instead?", true)? {
                    let result = setup_oauth(c, target, None)?;
                    return print_result(c, result);
                }
            }
        }
    }
    let readiness = stored_readiness(c, target)?;
    let unreachable = readiness["state"] == "target-unreachable";
    if unreachable && target["type"] != "desktop" {
        if !chatty(c) {
            return c.print(&broken_output(target, &readiness));
        }
        if handle_broken(c, target, &readiness)? {
            return Ok(());
        }
    }
    let ask = "Beeper Desktop is not reachable. Launch it now?";
    if unreachable && target["type"] == "desktop" && chatty(c) && (c.on("yes") || confirm(ask, true)?) {
        return launch_and_poll(c, target, &setup_cmd);
    }
    let data = json!({ "target": public_target(target), "readiness": readiness });
    if !chatty(c) {
        return c.print(&data);
    }
    let message = match readiness["state"] == "ready" {
        true => "Target ready".into(),
        false => format!("Setup paused: {}", js_string(&readiness["state"])),
    };
    c.success(&message, detail(&readiness, target).as_deref(), data)
}

/// `detectDesktopSetup`.
fn detect(c: &mut Ctx, target: &Target) -> Detected {
    progress(c, "Checking Beeper Desktop");
    progress(c, "Reading local Desktop session");
    let error = match prepare_local(c, target) {
        Ok(local) => return Detected::SessionFound(Box::new(local)),
        Err(e) => e.msg,
    };
    progress(c, "Checking Desktop readiness");
    let scan = id(target) == "desktop";
    let Ok((base, _)) = find_local_desktop(c, &base(target), scan, Duration::from_millis(500)) else {
        return Detected::InstalledNotRunning;
    };
    let mut found = target.clone();
    found.insert("baseURL".into(), json!(base));
    let readiness = readiness_of(c, &found, None).unwrap_or_default();
    match readiness["state"] == "needs-login" {
        true => Detected::RunningSignedOut(readiness),
        false => Detected::SessionUnreadable(error, readiness),
    }
}

/// `setupSessionFoundOutput`.
fn session_found(local: &Local, setup_cmd: &str) -> Value {
    let ready = local.readiness["state"] == "ready";
    let preview = obj([
        ("authSource", local.auth["source"].clone()),
        ("baseURL", local.target["baseURL"].clone()),
        ("dataDir", json!(local.dir.display().to_string())),
        ("signedInAs", local.session.get("user_id").filter(|u| truthy(u)).cloned().unwrap_or_default()),
        ("connectedAccounts", json!(local.accounts)),
    ]);
    let actions = [
        action("use-desktop-session", format!("{setup_cmd} --local")),
        action("desktop-oauth", format!("{setup_cmd} --oauth")),
        action("connect-remote", format!("{BIN} setup --remote <url>")),
    ];
    let message = match ready {
        true => "Beeper Desktop is signed in and ready.",
        false => "Beeper Desktop is signed in, but setup is not finished.",
    };
    Value::Object(obj([
        ("state", json!(if ready { "desktop-ready" } else { "desktop-session-found" })),
        ("message", json!(message)),
        ("target", public_target(&local.target)),
        ("readiness", local.readiness.clone()),
        ("localDesktop", Value::Object(preview)),
        ("recommendedAction", actions[0].clone()),
        ("availableActions", json!(actions)),
    ]))
}

/// `setupStateOutput`.
fn state_output(detected: &Detected, target: &Target) -> Value {
    let remote = action("connect-remote", format!("{BIN} setup --remote <url>"));
    let server = action("install-server", format!("{BIN} setup --server --install --yes"));
    let (state, message, readiness, reason, actions) = match detected {
        Detected::InstalledNotRunning => (
            "desktop-installed-not-running",
            "Beeper Desktop is installed but not running.",
            None,
            None,
            vec![action("launch-desktop", format!("{BIN} setup --desktop --yes")), remote, server],
        ),
        Detected::RunningSignedOut(readiness) => (
            "desktop-running-signed-out",
            "Beeper Desktop is running but not signed in.",
            Some(readiness),
            None,
            vec![action("open-desktop", format!("{BIN} setup --desktop --yes")), remote],
        ),
        Detected::SessionUnreadable(reason, readiness) => (
            "desktop-running-session-unreadable",
            "Beeper Desktop is running, but CLI could not read the local session.",
            Some(readiness),
            Some(reason),
            vec![action("desktop-oauth", format!("{BIN} setup --oauth --yes")), remote],
        ),
        Detected::SessionFound(_) => unreachable!("a found session has its own output"),
    };
    Value::Object(obj([
        ("state", json!(state)),
        ("message", json!(message)),
        ("detail", json!(reason)),
        ("target", public_target(target)),
        ("readiness", readiness.cloned().unwrap_or_default()),
        ("recommendedAction", actions[0].clone()),
        ("availableActions", json!(actions)),
    ]))
}

/// `currentTargetBrokenOutput`.
fn broken_output(target: &Target, readiness: &Value) -> Value {
    let retry = action("retry-current", format!("{BIN} setup -t {}", id(target)));
    json!({
        "state": "current-target-unreachable",
        "message": format!("Beeper CLI is set up for {}, but it is not reachable.", name(target)),
        "target": public_target(target),
        "readiness": readiness,
        "recommendedAction": retry,
        "availableActions": [
            retry,
            action("use-desktop", format!("{BIN} setup --desktop")),
            action("install-server", format!("{BIN} setup --server --install --yes")),
            action("connect-remote", format!("{BIN} setup --remote <url>")),
        ],
    })
}

/// `promptChoice`.
fn choose(label: &str, allowed: &[&str]) -> Res<String> {
    let answer = prompt(label)?;
    let answer = if answer.is_empty() { allowed[0].to_string() } else { answer };
    match allowed.contains(&answer.as_str()) {
        true => Ok(answer),
        false => Err(CliError::bug(format!("Choose one of: {}", allowed.join(", ")))),
    }
}

/// `handleBrokenCurrentTarget`: whether the choice finished the setup. The local Server needs no install.
fn handle_broken(c: &mut Ctx, target: &Target, readiness: &Value) -> Res<bool> {
    println!("Beeper CLI is set up for {}, but it is not reachable.\n", name(target));
    if let Some(message) = readiness["message"].as_str() {
        println!("{message}\n");
    }
    println!("What do you want to do?\n");
    println!("  1. Retry {}", name(target));
    println!("  2. Use Beeper Desktop on this device");
    println!("  3. Install local Beeper Server");
    println!("  4. Connect with Desktop API on another device\n");
    match choose("Choose [1]: ", &["1", "2", "3", "4"])?.as_str() {
        "1" => return Ok(false),
        "2" => {
            let desktop = default_desktop_target(c)?;
            setup_default(c, &desktop)?;
        }
        "3" => setup_managed(c, "server")?,
        _ => {
            let url = prompt("Desktop API URL: ")?;
            if url.is_empty() {
                return Err(CliError::bug("Remote URL is required."));
            }
            setup_remote(c, &url)?;
        }
    }
    Ok(true)
}

/// `launchAndPoll`: opens Desktop and waits up to 10 seconds for its API.
fn launch_and_poll(c: &mut Ctx, target: &Target, setup_cmd: &str) -> Res {
    step_event(c, "launch", Some(id(target)));
    if chatty(c) {
        println!("Opening Beeper Desktop...");
    }
    profiles::launch_desktop(Some(target))?;
    let start = Instant::now();
    let mut readiness = readiness_of(c, target, None)?;
    while readiness["state"] == "target-unreachable" && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(500));
        readiness = readiness_of(c, target, None)?;
    }
    let unreachable = readiness["state"] == "target-unreachable";
    let detail = match unreachable {
        true => Some(format!("Run `{setup_cmd}` again after Beeper Desktop finishes starting.")),
        false => detail(&readiness, target),
    };
    let data = json!({ "target": public_target(target), "readiness": readiness });
    c.success("Launched Beeper Desktop", detail.as_deref(), data)?;
    if chatty(c) && unreachable {
        println!("\nNext:\n  {setup_cmd}\n  {BIN} doctor");
    }
    Ok(())
}

/// `printSetupResult`: an interactive setup goes on with the verification first.
fn print_result(c: &mut Ctx, mut result: Value) -> Res {
    let state = result["readiness"]["state"].clone();
    if chatty(c) && (state == "needs-verification" || state == "verification-in-progress") {
        println!("Continuing verification...\n");
        let target = result["target"].as_object().cloned().unwrap_or_default();
        c.set_target(target.clone());
        super::verify::drive_verification(c, None)?;
        result["readiness"] = stored_readiness(c, &target)?;
    }
    if !chatty(c) {
        return c.print(&result);
    }
    let (readiness, target) = (&result["readiness"], result["target"].as_object().cloned().unwrap_or_default());
    let ready = readiness["state"] == "ready";
    let message = match ready {
        true => format!("Connected to {}", name(&target)),
        false => format!("Connected; setup paused: {}", js_string(&readiness["state"])),
    };
    let accounts: Vec<String> = result["accounts"].as_array().into_iter().flatten().map(js_string).collect();
    let accounts = (!accounts.is_empty()).then(|| format!("Connected accounts: {}", accounts.join(", ")));
    let lines: Vec<String> = accounts.into_iter().chain(detail(readiness, &target)).collect();
    c.success(&message, Some(lines.join("\n")).filter(|l| !l.is_empty()).as_deref(), result.clone())?;
    if ready {
        println!("\nNext:\n  {BIN} chats list\n  {BIN} send text --to <chat> \"hello\"");
    }
    Ok(())
}

/// `setupOAuthTarget`: a token from the browser sign-in, which looks for Desktop on the usual ports for the
/// `desktop` target.
fn setup_oauth(c: &mut Ctx, target: &Target, source: Option<&str>) -> Res<Value> {
    step_event(c, "oauth", Some(id(target)));
    if !chatty(c) && !c.on("yes") {
        return Err(CliError::bug("OAuth setup requires an interactive terminal or --yes to open the browser."));
    }
    let remote = target["type"] == "remote";
    let source = source.unwrap_or(if remote { "remote-oauth" } else { "desktop-oauth" });
    let auth = match target["type"] == "desktop" && id(target) == "desktop" {
        true => {
            let token = ensure_desktop_token(c, &base(target), true, false)?;
            json!({ "accessToken": token, "source": source, "tokenType": "Bearer" })
        }
        false => {
            let token = login_with_pkce(c, &base(target), false, source)?;
            let expires = token["expires_in"].as_i64().filter(|s| *s != 0).map(|s| iso_in(s * 1000));
            Value::Object(obj([
                ("accessToken", token["access_token"].clone()),
                ("clientID", token["clientID"].clone()),
                ("expiresAt", json!(expires)),
                ("scope", token["scope"].clone()),
                ("source", json!(source)),
                ("tokenType", token["token_type"].clone()),
            ]))
        }
    };
    signed_in(c, target, &auth)
}

/// `setupEmailTarget`: the email sign-in asks for the code at the terminal.
fn setup_email(c: &mut Ctx, target: &Target) -> Res<Value> {
    step_event(c, "email", Some(id(target)));
    let email = c.str("email").ok_or_else(|| CliError::bug("Email setup requires --email."))?.to_string();
    if !chatty(c) {
        let msg = format!(
            "Email setup prompts for the verification code. For automation, use `{BIN} auth email start` and `{BIN} auth email response`."
        );
        return Err(CliError::bug(msg));
    }
    let request = start_email(c, target, &email)?;
    let code = prompt("Email code: ")?;
    finish_email(c, target, &code, &request)
}

/// `setupRemote`: a remote target named after its host, signed in with the browser or by email.
fn setup_remote(c: &mut Ctx, url: &str) -> Res {
    let name = if let Some(name) = c.str("target") { name.to_string() } else { unique_remote_name(url)? };
    if chatty(c) {
        println!("Connecting to Desktop API on another device.\n\nName: {name}\nURL:  {url}\n");
    }
    let target =
        obj([("id", json!(name)), ("name", json!(name)), ("type", json!("remote")), ("baseURL", json!(url)), ("managed", json!(false))]);
    let result = match c.str("email").is_some() {
        true => setup_email(c, &target)?,
        false => setup_oauth(c, &target, Some("remote-oauth"))?,
    };
    write_target(&target)?;
    if c.str("target").is_none() {
        set_default_target(&name)?;
    }
    print_result(c, result)
}

/// `remoteName` and `uniqueRemoteName`: the host of the URL, with a number when a target has its name.
fn unique_remote_name(url: &str) -> Res<String> {
    let host = url.split_once("://").map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or_default());
    let host = host.map(|h| h.rsplit_once('@').map_or(h, |p| p.1)).map(|h| match h.starts_with('[') {
        true => h.split_inclusive(']').next().unwrap_or(h),
        false => h.split(':').next().unwrap_or(h),
    });
    let clean = |h: &str| {
        h.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() || "._-".contains(c) { c } else { '-' }).collect::<String>()
    };
    let base = host.map(clean).filter(|h| !h.is_empty()).unwrap_or_else(|| "remote".into());
    let ids: Vec<String> = list_targets()?.iter().map(|t| js_string(&t["id"])).collect();
    let free = std::iter::once(base.clone()).chain((2..100).map(|n| format!("{base}-{n}"))).find(|n| !ids.contains(n));
    Ok(free.unwrap_or_else(|| format!("remote-{}", chrono::Utc::now().timestamp_millis())))
}

/// `setupManaged`: a managed target of `kind` that runs here, then its readiness.
fn setup_managed(c: &mut Ctx, kind: &str) -> Res {
    if c.on("install") && !chatty(c) && !c.on("yes") {
        return Err(CliError::bug("Install requires --install --yes in non-interactive mode."));
    }
    let id = c.str("target").unwrap_or(kind).to_string();
    let env = c.str("server-env").expect("--server-env has a default").to_string();
    let target = if let Some(t) = read_target(&id)? { t } else { create_profile_target(kind, &id, &env, None)? };
    if c.str("target").is_none() {
        set_default_target(&id)?;
    }
    match profiles::start(&target) {
        Err(e) if kind == "server" => return Err(e),
        _ => {}
    }
    if c.str("email").is_some() {
        let result = setup_email(c, &target)?;
        return print_result(c, result);
    }
    let readiness = stored_readiness(c, &target)?;
    c.print(&json!({ "target": public_target(&target), "readiness": readiness }))
}

// ─── the local session ─────────────────────────────────────────────────────────

/// A signed-in session of the app on this device (`findLocalDesktopSession`), for a target.
struct Local {
    dir: PathBuf,
    session: Value,
    auth: Value,
    readiness: Value,
    accounts: Vec<String>,
    target: Target,
}

/// A value of the `kv` table of the beeper-rs database in `dir`, which keeps Desktop's `beeperState` as `state`.
fn stored_value(dir: &std::path::Path, key: &str) -> Result<Value, String> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY;
    let db = rusqlite::Connection::open_with_flags(dir.join("beeper-rs.db"), flags).map_err(|e| e.to_string())?;
    let text: String = db.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).map_err(|_| format!("missing {key}"))?;
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// `prepareLocalDesktopSetup`: the target of the session of the app on this device.
fn prepare_local(c: &Ctx, target: &Target) -> Res<Local> {
    step_event(c, "local-desktop", Some(id(target)));
    let scan = id(target) == "desktop";
    let found = find_local_desktop(c, &base(target), scan, Duration::from_millis(500)).ok();
    let mut resolved = target.clone();
    let id = if id(target) == "custom" { "desktop".into() } else { id(target) };
    resolved.insert("id".into(), json!(id));
    resolved.insert("type".into(), json!("desktop"));
    if target.get("name").is_none_or(Value::is_null) {
        resolved.insert("name".into(), json!("Beeper Desktop"));
    }
    resolved.insert("baseURL".into(), json!(found.map_or_else(|| base(target), |f| f.0)));
    resolved.insert("managed".into(), target.get("managed").cloned().unwrap_or(json!(false)));
    // `localDesktopDataDirs`: the target's folder, `BEEPER_USER_DATA_DIR`, or the data folder of beeper-rs.
    let dir = match (resolved.get("dataDir").and_then(Value::as_str), std::env::var_os("BEEPER_USER_DATA_DIR")) {
        (Some(dir), _) => PathBuf::from(dir),
        (None, Some(dir)) => PathBuf::from(dir),
        (None, None) => beeper_core::platform::current_desktop_data_dir(None),
    };
    let fail = |e: String| CliError::bug(format!("Could not find a signed-in local Beeper Desktop session. {}: {e}", dir.display()));
    let session = stored_value(&dir, "state").map_err(fail)?;
    let token = session["access_token"].as_str().filter(|t| !t.is_empty()).map(str::to_string);
    let token = token.ok_or_else(|| fail("missing access_token".into()))?;
    let auth = json!({ "accessToken": token, "source": "desktop-db", "tokenType": "Bearer" });
    let accounts = stored_value(&dir, "bridges").ok().and_then(|v| v.as_array().cloned()).unwrap_or_default();
    let mut names: Vec<String> = Vec::new();
    for name in accounts.iter().filter_map(account_name) {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names.truncate(8);
    Ok(Local { readiness: local_readiness(&session), dir, session, auth, accounts: names, target: resolved })
}

/// Whether every flag of an object, and of its objects, is on.
fn all_true(v: &Value) -> bool {
    v.as_object().into_iter().flatten().all(|(_, v)| v.as_bool().unwrap_or_else(|| all_true(v)))
}

/// `localDesktopReadiness`: ready when the stored state finished its first sync and its encryption setup.
fn local_readiness(s: &Value) -> Value {
    let flag = |v: &Value| v.as_bool().unwrap_or(false);
    let secrets = &s["secrets"];
    // beeper-rs keeps the recovery key itself where Desktop keeps whether it has one.
    let recovery = secrets["recovery_code"].as_bool().unwrap_or_else(|| truthy(&secrets["recovery_code"]));
    let e2ee = json!({
        "initialized": flag(&s["initialized"]),
        "secretStorage": flag(&s["secret_storage"]),
        "crossSigning": flag(&s["cross_signing"]),
        "verified": flag(&s["verified"]),
        "secrets": {
            "masterKey": flag(&secrets["master_key"]),
            "selfSigningKey": flag(&secrets["self_signing_key"]),
            "userSigningKey": flag(&secrets["user_signing_key"]),
            "megolmBackupKey": flag(&secrets["megolm_backup_key"]),
            "recoveryKey": recovery,
        },
        "keyBackup": flag(&s["key_backup"]),
        "firstSyncDone": flag(&s["first_sync_done"]),
        "hasBackedUpRecoveryKey": flag(&s["has_backed_up_code"]),
    });
    let state = if all_true(&e2ee) { "ready" } else { "needs-first-sync" };
    let text = |k: &str| s[k].as_str().unwrap_or_default().to_string();
    let matrix = json!({ "userID": text("user_id"), "deviceID": text("device_id"), "homeserver": text("homeserver") });
    let (actions, message) = match state {
        "ready" => (json!(["chats list", "messages list", "send text"]), Value::Null),
        _ => (json!(["setup", "status"]), json!("Waiting for local Desktop first sync and encryption setup.")),
    };
    Value::Object(obj([
        ("state", json!(state)),
        ("app", json!({ "state": state, "matrix": matrix, "e2ee": e2ee })),
        ("actions", actions),
        ("message", message),
    ]))
}

/// `commitLocalDesktopSetup`.
fn commit_local(local: Local) -> Res<Value> {
    write_target(&local.target)?;
    save_target_auth(&local.target, &local.auth)?;
    set_default_target(&id(&local.target))?;
    Ok(json!({
        "accounts": local.accounts,
        "authSource": local.auth["source"],
        "readiness": local.readiness,
        "target": public_target(&with_auth(&local.target, &local.auth)),
    }))
}

// ─── auth email ────────────────────────────────────────────────────────────────

/// `startEmailSetup`: a sign-in request, and the code sent to `email`. Returns the request ID.
fn start_email(c: &Ctx, target: &Target, email: &str) -> Res<String> {
    let client = BeeperClient::try_new(&base(target), "").map_err(CliError::bug)?;
    let start = c.block(client.call("POST", "/v1/app/setup/start", None))?;
    let request = js_string(&start["setupRequestID"]);
    let body = json!({ "setupRequestID": request, "email": email });
    c.block(client.call("POST", "/v1/app/setup/email", Some(&body)))?;
    Ok(request)
}

/// `finishEmailSetup`: the code signs in, or registers a new account with the terms accepted. The target keeps the
/// Matrix token as its Desktop API token.
fn finish_email(c: &mut Ctx, target: &Target, code: &str, request: &str) -> Res<Value> {
    let client = BeeperClient::try_new(&base(target), "").map_err(CliError::bug)?;
    let body = json!({ "setupRequestID": request, "response": code });
    let mut output = c.block(client.call("POST", "/v1/app/setup/response", Some(&body)))?;
    if output["registrationRequired"] == true {
        let (yes, chatty) = (c.on("yes"), chatty(c));
        if !chatty && !yes {
            return Err(CliError::bug("Registration requires --yes to accept the Beeper terms in non-interactive setup."));
        }
        let username = match (c.str("username"), chatty) {
            (Some(name), _) => name.to_string(),
            (None, true) => {
                let fallback = output["usernameSuggestions"][0].as_str().unwrap_or_default().to_string();
                let suffix = if fallback.is_empty() { String::new() } else { format!(" [{fallback}]") };
                Some(prompt(&format!("Username{suffix}: "))?).filter(|n| !n.is_empty()).unwrap_or(fallback)
            }
            (None, false) => String::new(),
        };
        if username.is_empty() {
            return Err(CliError::bug("Registration requires --username."));
        }
        if !yes && !confirm("Accept the Beeper terms and create this account?", true)? {
            return Err(CliError::bug("Registration cancelled."));
        }
        let body = json!({
            "acceptTerms": true,
            "leadToken": output["leadToken"],
            "setupRequestID": output["setupRequestID"],
            "username": username,
        });
        output = c.block(client.call("POST", "/v1/app/setup/register", Some(&body)))?;
    }
    let token = output["matrix"]["accessToken"].as_str().filter(|t| !t.is_empty());
    let token = token.ok_or_else(|| CliError::bug("Setup did not return a Matrix access token."))?;
    let auth = json!({ "accessToken": token, "source": "manual", "tokenType": "Bearer" });
    signed_in(c, target, &auth)
}

fn email_start(c: &mut Ctx) -> Res {
    let target = c.target()?.clone();
    let email = c.str("email").expect("--email is required").to_string();
    let request = start_email(c, &target, &email)?;
    c.print(&json!({ "setupRequestID": request }))
}

fn email_response(c: &mut Ctx) -> Res {
    c.writable()?;
    let target = c.target()?.clone();
    let code = c.str("code").expect("--code is required").to_string();
    let request = c.str("setup-request-id").expect("--setup-request-id is required").to_string();
    let result = finish_email(c, &target, &code, &request)?;
    c.print(&result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_local_session_is_ready_after_its_encryption_setup() {
        let mut state = json!({
            "access_token": "t", "user_id": "@me:beeper.com", "device_id": "D", "homeserver": "https://matrix.beeper.com",
            "first_sync_done": true, "initialized": true, "secret_storage": true, "cross_signing": true, "verified": true,
            "key_backup": true, "has_backed_up_code": true,
            "secrets": { "master_key": true, "self_signing_key": true, "user_signing_key": true, "megolm_backup_key": true, "recovery_code": "EsT1 ..." }
        });
        let ready = local_readiness(&state);
        assert_eq!((ready["state"].clone(), ready.get("message")), (json!("ready"), None));
        assert_eq!(ready["app"]["matrix"]["userID"], "@me:beeper.com");
        state["secrets"]["recovery_code"] = json!("");
        let waiting = local_readiness(&state);
        assert_eq!(waiting["state"], "needs-first-sync");
        assert_eq!(waiting["actions"], json!(["setup", "status"]));
    }

    #[test]
    fn account_names_follow_upstream() {
        assert_eq!(account_name(&json!({ "network": "WhatsApp", "accountID": "a" })), Some("WhatsApp".into()));
        assert_eq!(account_name(&json!({ "network": { "displayName": "Signal" } })), Some("Signal".into()));
        assert_eq!(account_name(&json!({ "bridge": { "type": "telegram" }, "accountID": "a" })), Some("telegram".into()));
        assert_eq!(account_name(&json!({ "accountID": "acc_x" })), Some("acc_x".into()));
        assert_eq!(account_name(&json!(1)), None);
    }
}
