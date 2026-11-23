//! The `verify` topic (`commands/verify.js`, `commands/verify/*.js`): the encrypted-messages setup of a signed-in
//! target, and the device verifications of `driveVerification` in `lib/app-state.js`.

use super::commands::{app_request, cmd, readiness};
use super::{confirm, envelope, interactive, js_string, obj, CliError, Cmd, Ctx, Flag, Res, BIN};
use beeper_core::codec::url_encode;
use serde_json::{json, Value};

const ID: Flag = Flag::s("id", "Verification request ID. Defaults to the active request.");
const SETUP: &str = "/v1/app/setup";

pub const COMMANDS: &[Cmd] = &[
    Cmd {
        flags: &[Flag::s("user", "User ID to verify against (defaults to your own account)")],
        ..cmd("verify", "Finish setup verification or verify another device", drive)
    },
    Cmd { flags: &[ID], ..cmd("verify approve", "Approve a pending device verification request", |c| action(c, "accept", None)) },
    Cmd { flags: &[ID], ..cmd("verify cancel", "Cancel an in-progress device verification", |c| action(c, "cancel", Some(json!({})))) },
    cmd("verify list", "List active verification work", list),
    Cmd {
        flags: &[ID],
        ..cmd("verify qr-confirm", "Confirm that the other device scanned your QR code", |c| action(c, "qr/confirm-scanned", None))
    },
    Cmd {
        flags: &[ID, Flag::s("payload", "Raw QR-code data scanned from the other device").req()],
        ..cmd("verify qr-scan", "Submit a scanned QR-code verification payload", qr_scan)
    },
    Cmd {
        flags: &[Flag::s("key", "Recovery key string").req()],
        ..cmd("verify recovery-key", "Unlock encrypted messages with a recovery key", recovery_key)
    },
    cmd("verify reset-recovery-key", "Create a new encrypted-messages recovery key", reset_recovery_key),
    Cmd { flags: &[ID], ..cmd("verify sas", "Start emoji verification", |c| action(c, "sas/start", None)) },
    Cmd { flags: &[ID], ..cmd("verify sas-confirm", "Confirm matching emoji verification", |c| action(c, "sas/confirm", None)) },
    cmd("verify show", "Show the current active verification request", show),
    Cmd {
        flags: &[Flag::s("user", "User ID to verify with (defaults to your own account)")],
        ..cmd("verify start", "Start a device verification request", start)
    },
    cmd("verify status", "Show encryption and device-verification readiness", |c| {
        let readiness = readiness(c)?;
        c.print(&readiness)
    }),
];

/// The SDK's `app.verifications` actions on `--id`, or on `active` like upstream.
fn action(c: &mut Ctx, path: &str, body: Option<Value>) -> Res {
    c.writable()?;
    let id = c.str("id").unwrap_or("active").to_string();
    let reply = c.call("POST", &format!("{SETUP}/verifications/{}/{path}", url_encode(&id)), body)?;
    c.print(&reply)
}

fn start(c: &mut Ctx) -> Res {
    c.writable()?;
    let body = Value::Object(obj([("userID", json!(c.str("user")))]));
    let reply = c.call("POST", &format!("{SETUP}/verifications"), Some(body))?;
    c.print(&reply)
}

fn qr_scan(c: &mut Ctx) -> Res {
    c.writable()?;
    let body = json!({ "data": c.str("payload").expect("--payload is required") });
    let reply = c.call("POST", &format!("{SETUP}/verifications/qr/scan"), Some(body))?;
    c.print(&reply)
}

fn recovery_key(c: &mut Ctx) -> Res {
    c.writable()?;
    let body = json!({ "recoveryKey": c.str("key").expect("--key is required") });
    let reply = c.call("POST", &format!("{SETUP}/verification/recovery-key"), Some(body))?;
    c.print(&reply)
}

/// Upstream asks for the new key before it checks that someone can confirm it.
fn reset_recovery_key(c: &mut Ctx) -> Res {
    c.writable()?;
    let reset = c.call("POST", &format!("{SETUP}/verification/recovery-key/reset"), Some(json!({})))?;
    let key = reset["recoveryKey"].clone();
    if (c.json() || !interactive()) && !c.on("yes") {
        let msg = "Resetting the recovery key requires --yes in non-interactive mode so the new key can be confirmed.";
        return Err(CliError::bug(msg));
    }
    if !c.on("yes") {
        eprintln!("New recovery key:\n{}", js_string(&key));
        if !confirm("I saved this recovery key. Use it for this account?", true)? {
            return Err(CliError::bug("Recovery key reset cancelled."));
        }
    }
    let body = json!({ "recoveryKey": key });
    let confirmed = c.call("POST", &format!("{SETUP}/verification/recovery-key/reset/confirm"), Some(body))?;
    c.print(&json!({ "recoveryKey": key, "session": confirmed["session"] }))
}

/// `getAppState` with the stored token.
fn app_state(c: &mut Ctx) -> Res<Value> {
    let token = c.stored_token()?;
    app_request(c, "GET", SETUP, None, token)
}

fn list(c: &mut Ctx) -> Res {
    let state = app_state(c)?;
    let items: Vec<Value> = state.get("verification").filter(|v| super::truthy(v)).cloned().into_iter().collect();
    c.print(&Value::Array(items))
}

fn show(c: &mut Ctx) -> Res {
    let state = app_state(c)?;
    // `printData(null)` shows the `null` that `printData(undefined)` drops.
    match (state.get("verification").cloned().unwrap_or_default(), c.json()) {
        (Value::Null, true) => envelope(&Value::Null),
        (Value::Null, false) => println!("null"),
        (verification, _) => return c.print(&verification),
    }
    Ok(())
}

fn drive(c: &mut Ctx) -> Res {
    c.writable()?;
    let user = c.str("user").map(str::to_string);
    let state = drive_verification(c, user)?;
    c.print(&state)
}

/// `driveVerification`: starts a verification of this device with `user` and takes each step that this side can
/// take, until the app is ready or waits for the other device. Returns the app state.
pub fn drive_verification(c: &mut Ctx, user: Option<String>) -> Res<Value> {
    let token = c.stored_token()?;
    let mut state = app_request(c, "GET", SETUP, None, token.clone())?;
    if state["state"] == "ready" {
        return Ok(state);
    }
    if state["state"] == "needs-login" {
        let msg = format!("Target is not signed in. Run `{BIN} setup` after signing in to Beeper Desktop.");
        return Err(CliError::bug(msg));
    }
    loop {
        let verification = state.get("verification").cloned().unwrap_or_default();
        let actions = verification["availableActions"].as_array().cloned().unwrap_or_default();
        let has = |action: &str| actions.iter().any(|a| a == action);
        let id = verification["id"].as_str().filter(|id| !id.is_empty()).map(url_encode);
        let step = |c: &mut Ctx, path: String, body: Option<Value>| {
            app_request(c, "POST", &path, body, token.clone()).map(|reply| reply["session"].clone())
        };
        state = match id {
            _ if !super::truthy(&verification) => {
                let body = Value::Object(obj([("userID", json!(user))]));
                step(c, format!("{SETUP}/verifications"), Some(body))?
            }
            Some(id) if has("accept") => step(c, format!("{SETUP}/verifications/{id}/accept"), None)?,
            Some(id) if has("sas.start") => step(c, format!("{SETUP}/verifications/{id}/sas/start"), None)?,
            Some(id) if has("sas.confirm") => {
                if !c.on("yes") {
                    let sas = &verification["sas"];
                    let shown = ["emojis", "decimals"].iter().map(|k| &sas[*k]).find(|v| !v.is_null());
                    let shown = shown.map_or("(no SAS data)".into(), js_string);
                    println!("Verify that this matches on the other device:\n{shown}");
                    if !confirm("Do they match?", false)? {
                        return Err(CliError::bug("Verification cancelled."));
                    }
                }
                step(c, format!("{SETUP}/verifications/{id}/sas/confirm"), None)?
            }
            Some(id) if has("qr.confirmScanned") => step(c, format!("{SETUP}/verifications/{id}/qr/confirm-scanned"), None)?,
            _ => return Ok(state),
        };
    }
}
