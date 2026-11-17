//! `accounts add` (`commands/accounts/add.js`) and the guided bridge login of `lib/account-login.js`: a login session
//! of a bridge, whose steps show a code, ask for fields, or ask for cookies until the account is connected.

use super::accounts::{bridge_names, bridges, matching_bridges};
use super::{interactive, js_string, obj, prompt, CliError, Ctx, Res, BIN};
use beeper_core::codec::url_encode;
use serde_json::{json, Map, Value};

/// `parseKeyValueFlags`: `name=value` words.
fn key_values(values: &[String], flag: &str) -> Res<Map<String, Value>> {
    let pair = |v: &String| match v.split_once('=') {
        Some((k, value)) if !k.is_empty() => Ok((k.to_string(), json!(value))),
        _ => Err(CliError::bug(format!("{flag} must use name=value form."))),
    };
    values.iter().map(pair).collect()
}

/// `resolveAccountType`.
fn resolve_account_type(items: Vec<Value>, input: &str) -> Res<Value> {
    let mut matches = matching_bridges(items, input);
    match matches.len() {
        1 => Ok(matches.remove(0)),
        0 => Err(CliError::bug(format!(
            "Unknown bridge \"{input}\". Run `{BIN} bridges list` to list available bridges."
        ))),
        _ => Err(CliError::bug(format!("Account type {input} is ambiguous. Use one of: {}", bridge_names(&matches)))),
    }
}

/// `statusLabel`.
fn status_label(b: &Value) -> Option<String> {
    match b["status"].as_str().unwrap_or_default() {
        "available" => None,
        "connected" => Some(format!("{} Connected", js_string(&b["displayName"]))),
        status => Some(status.replace('_', " ")),
    }
}

fn multiple(b: &Value) -> &'static str {
    if super::truthy(&b["supportsMultipleAccounts"]) {
        "multiple allowed"
    } else {
        "single account"
    }
}

/// `printAvailableAccounts`: the bridges by provider, with the command that connects each available one.
fn print_available(items: &[Value]) {
    println!("Choose a bridge to connect an account:\n");
    let sections =
        [("On-Device Accounts", "local"), ("Beeper Cloud Accounts", "cloud"), ("Self-Hosted Accounts", "self-hosted")];
    for (title, provider) in sections {
        let accounts: Vec<&Value> = items.iter().filter(|b| b["provider"] == provider).collect();
        if accounts.is_empty() {
            continue;
        }
        println!("{title}");
        for b in accounts {
            let status = b["statusText"].as_str().map(str::to_string).or_else(|| status_label(b));
            let status = status.map_or(String::new(), |s| format!(" - {s}"));
            let (name, id) = (js_string(&b["displayName"]), js_string(&b["id"]));
            println!("  {name} ({id}) - {}{status}", multiple(b));
            if b["status"] == "available" {
                println!("    {BIN} accounts add {id}");
            }
        }
        println!();
    }
    println!("Run `{BIN} bridges list` for the scriptable catalog or `{BIN} bridges show <bridge>` for login flows.");
}

/// `chooseAccountType` and `chooseLoginFlow`: asks for a number or an ID of `items` until one matches.
fn choose(title: &str, question: &str, retry: &str, items: &[(String, String)]) -> Res<String> {
    println!("{title}");
    for (i, (_, label)) in items.iter().enumerate() {
        println!("  {}. {label}", i + 1);
    }
    loop {
        let answer = prompt(question)?;
        let number = answer.parse::<usize>().ok().filter(|n| (1..=items.len()).contains(n));
        if let Some(n) = number {
            return Ok(items[n - 1].0.clone());
        }
        if let Some((id, _)) = items.iter().find(|(id, _)| *id == answer) {
            return Ok(id.clone());
        }
        if answer.is_empty() && !interactive() {
            return Err(CliError::bug("No answer at the end of the input."));
        }
        println!("{retry}");
    }
}

/// `runGuidedAccountLogin` options.
struct Guide {
    fields: Map<String, Value>,
    cookies: Map<String, Value>,
    non_interactive: bool,
    webview: bool,
}

pub fn accounts_add(c: &mut Ctx) -> Res {
    c.writable()?;
    let (guided, non_interactive) = (c.on("guided"), c.on("non-interactive"));
    let input = match c.arg(0) {
        Some(input) => input.to_string(),
        None => {
            let list = c.get("/v1/bridges")?;
            if c.json() {
                return c.print(&list);
            }
            let items = list["items"].as_array().cloned().unwrap_or_default();
            if !guided || non_interactive || !interactive() {
                print_available(&items);
                return Ok(());
            }
            let available: Vec<(String, String)> = items
                .iter()
                .filter(|b| b["status"] == "available")
                .map(|b| {
                    let id = js_string(&b["id"]);
                    (id.clone(), format!("{} ({id}) - {}", js_string(&b["displayName"]), multiple(b)))
                })
                .collect();
            if available.is_empty() {
                return Err(CliError::bug("No available bridges to connect."));
            }
            choose(
                "Choose a bridge to connect an account:",
                "Select a bridge: ",
                "Choose one of the listed bridges.",
                &available,
            )?
        }
    };
    let bridge = resolve_account_type(bridges(c)?, &input)?;
    let (id, name) = (js_string(&bridge["id"]), js_string(&bridge["displayName"]));
    if bridge["status"] != "available" {
        let suffix =
            bridge["statusText"].as_str().filter(|s| !s.is_empty()).map_or(String::new(), |s| format!(": {s}"));
        return Err(CliError::bug(format!("{name} is not available{suffix}")));
    }
    let path = format!("/v1/bridges/{}", url_encode(&id));
    let flow = match c.str("flow") {
        Some(flow) => flow.to_string(),
        None => {
            let flows = c.get(&format!("{path}/login-flows"))?["items"].as_array().cloned().unwrap_or_default();
            let flow = match flows.len() > 1 {
                true if guided && !c.json() && !non_interactive => {
                    let items: Vec<(String, String)> = flows
                        .iter()
                        .map(|f| {
                            let about = f["description"].as_str().filter(|d| !d.is_empty());
                            let about = about.map_or(String::new(), |d| format!(" - {d}"));
                            (js_string(&f["id"]), format!("{}{about}", js_string(&f["name"])))
                        })
                        .collect();
                    let retry = "Choose one of the listed sign-in methods.";
                    Some(choose("Choose how you want to sign in:", "Select a sign-in method: ", retry, &items)?)
                }
                true => {
                    let msg = format!("Multiple sign-in methods are available for {name}. Pass --flow.");
                    return Err(CliError::bug(msg));
                }
                false => flows.first().and_then(|f| f["id"].as_str()).map(str::to_string),
            };
            let flow = flow.ok_or_else(|| CliError::bug(format!("No login flows returned for {name}.")))?;
            if !c.json() && flows.len() > 1 {
                println!("Using flow {flow}");
            }
            flow
        }
    };
    let body = Value::Object(obj([("flowID", json!(flow)), ("loginID", json!(c.str("login-id")))]));
    let step = c.call("POST", &format!("{path}/login-sessions"), Some(body))?;
    let result = match guided {
        true => {
            let guide = Guide {
                fields: key_values(c.many("field"), "--field")?,
                cookies: key_values(c.many("cookie"), "--cookie")?,
                non_interactive,
                webview: c.on("webview"),
            };
            run_guided(c, &path, step, &guide)?
        }
        false => step,
    };
    match c.json() {
        true => c.print(&result),
        false => {
            print_step(&result);
            Ok(())
        }
    }
}

/// `runGuidedAccountLogin`: shows each step, and answers it from the flags or from prompts.
fn run_guided(c: &mut Ctx, path: &str, mut session: Value, guide: &Guide) -> Res<Value> {
    loop {
        print_step(&session);
        if ["complete", "cancelled", "failed"].iter().any(|s| session["status"] == *s) {
            return Ok(session);
        }
        let step = session["currentStep"].clone();
        let Some(step_id) = step["stepID"].as_str() else {
            return Err(CliError::bug("Account login session did not include a current step."));
        };
        let submit = format!(
            "{path}/login-sessions/{}/steps/{}",
            url_encode(&js_string(&session["loginSessionID"])),
            url_encode(step_id)
        );
        let fields = step["fields"].as_array().cloned().unwrap_or_default();
        let body = match step["type"].as_str().unwrap_or_default() {
            "display_and_wait" => {
                if guide.non_interactive {
                    return Ok(session);
                }
                println!("waiting for this step to complete...");
                json!({ "type": "display_and_wait" })
            }
            "user_input" => {
                let mut values = Map::new();
                for field in &fields {
                    let id = js_string(&field["id"]);
                    let initial = field.get("initialValue").filter(|v| !v.is_null());
                    let value = match (guide.fields.get(&id), guide.non_interactive, initial) {
                        (Some(v), _, _) => v.clone(),
                        (None, true, Some(initial)) => initial.clone(),
                        (None, true, None) => {
                            return Err(CliError::bug(format!(
                                "Missing required field {id}. Pass --field {id}=... or run without --non-interactive."
                            )))
                        }
                        (None, false, _) => {
                            let label = field["label"].as_str().map_or(id.clone(), str::to_string);
                            let initial = initial.filter(|v| super::truthy(v)).map(js_string);
                            let fallback = initial.as_ref().map_or(String::new(), |v| format!(" [{v}]"));
                            let answer = prompt(&format!("{label}{fallback}: "))?;
                            json!(Some(answer).filter(|a| !a.is_empty()).or(initial).unwrap_or_default())
                        }
                    };
                    values.insert(id, value);
                }
                json!({ "type": "user_input", "fields": values })
            }
            "cookies" => {
                if guide.webview {
                    let msg = "beeper-rs cannot open a WebView to collect cookie fields.";
                    if guide.non_interactive {
                        return Err(CliError::bug(msg));
                    }
                    println!("webview: {msg}");
                }
                let mut values = Map::new();
                for field in &fields {
                    let id = js_string(&field["id"]);
                    let value = match (guide.cookies.get(&id), guide.non_interactive) {
                        (Some(v), _) => v.clone(),
                        (None, true) => {
                            return Err(CliError::bug(format!(
                                "Missing required cookie {id}. Pass --cookie {id}=... or run without --non-interactive."
                            )))
                        }
                        (None, false) => json!(prompt_secret(&format!("{id}: "))?),
                    };
                    values.insert(id, value);
                }
                json!({ "type": "cookies", "fields": values, "source": "api" })
            }
            other => return Err(CliError::bug(format!("Unsupported account login step: {other}"))),
        };
        session = c.call("POST", &submit, Some(body))?;
    }
}

/// `promptSecret`: a prompt that does not echo at a terminal.
fn prompt_secret(label: &str) -> Res<String> {
    if !interactive() {
        return prompt(label);
    }
    let stty = |arg: &str| std::process::Command::new("stty").arg(arg).status();
    drop(stty("-echo"));
    let answer = prompt(label);
    drop(stty("echo"));
    println!();
    answer
}

/// `printAccountLoginStep`.
fn print_step(session: &Value) {
    let text = |v: &Value| v.as_str().filter(|s| !s.is_empty()).map(str::to_string);
    println!("status: {}", js_string(&session["status"]));
    if let Some(login) = text(&session["loginID"]) {
        println!("login_id: {login}");
    }
    let step = &session["currentStep"];
    if !super::truthy(step) {
        return;
    }
    println!("step: {}", js_string(&step["type"]));
    if let Some(instructions) = text(&step["instructions"]) {
        println!("{instructions}");
    }
    if step.get("stepID").is_some() {
        println!("step_id: {}", js_string(&step["stepID"]));
    }
    let fields = step["fields"].as_array().cloned().unwrap_or_default();
    match step["type"].as_str().unwrap_or_default() {
        "display_and_wait" => {
            let display = &step["display"];
            println!("display: {}", js_string(&display["type"]));
            match (display["type"].as_str(), text(&display["data"]), text(&display["imageURL"])) {
                (Some("qr"), Some(data), _) => println!("{}", terminal_qr(&data)),
                (Some("emoji"), _, Some(image)) => println!("image: {image}"),
                (Some("code"), Some(data), _) => println!("{data}"),
                _ => {}
            }
        }
        "user_input" => {
            for f in &fields {
                let details: Vec<String> = [&f["type"], &f["placeholder"]].into_iter().filter_map(text).collect();
                let details = if details.is_empty() { String::new() } else { format!(" ({})", details.join(" | ")) };
                let label = text(&f["label"]).unwrap_or_else(|| js_string(&f["id"]));
                println!("field {}: {label}{details}", js_string(&f["id"]));
            }
        }
        "cookies" => {
            println!("url: {}", js_string(&step["url"]));
            if let Some(agent) = text(&step["userAgent"]) {
                println!("user_agent: {agent}");
            }
            if let Some(regex) = text(&step["expectedFinalURLRegex"]) {
                println!("expected_final_url_regex: {regex}");
            }
            for f in &fields {
                println!(
                    "cookie field {}: {}",
                    js_string(&f["id"]),
                    text(&f["type"]).unwrap_or_else(|| "cookie".into())
                );
            }
            if let Some(js) = text(&step["extractJS"]) {
                println!("extract_js:\n{js}");
            }
        }
        "complete" => {
            let login = text(&step["login"]["loginID"]).or_else(|| text(&session["loginID"]));
            println!("complete: {}", login.unwrap_or_else(|| "yes".into()));
        }
        _ => {}
    }
}

/// The small terminal QR code of the `qrcode` package: two rows of modules a line, black on white, with a quiet
/// zone of one module. Text that does not fit a QR code prints as it is.
fn terminal_qr(data: &str) -> String {
    use qrcodegen::{QrCode, QrCodeEcc, QrSegment, Version};
    // The `qrcode` package keeps the error correction level that it is given.
    let segments = QrSegment::make_segments(data);
    let qr = QrCode::encode_segments_advanced(&segments, QrCodeEcc::Medium, Version::MIN, Version::MAX, None, false);
    let Ok(qr) = qr else { return data.to_string() };
    let size = qr.size();
    let (setup, reset, white, black) = ("\x1b[47m\x1b[30m", "\x1b[0m", "\x1b[37m", "\x1b[30m");
    let pixel = |x: i32, y: i32| match () {
        _ if x > size || y > size || x < -1 || y < -1 => 0,
        _ if x >= size || y >= size || x < 0 || y < 0 => 1,
        _ if qr.get_module(x, y) => 2,
        _ => 1,
    };
    let cell = |x: i32, y: i32| match (pixel(x, y), pixel(x, y + 1)) {
        (0, 0) => format!("{reset} {setup}"),
        (0, 1) => format!("{reset}{white}▄{setup}"),
        (0, _) => format!("{reset}{black}▄{setup}"),
        (1, 0) => format!("{reset}{white}▀{setup}"),
        (1, 1) => " ".into(),
        (1, _) => "▄".into(),
        (_, 0) => format!("{reset}{black}▀{setup}"),
        (_, 1) => "▀".into(),
        _ => "█".into(),
    };
    let mut out = setup.to_string();
    for y in (-1..=size).step_by(2) {
        out.extend((-1..=size).map(|x| cell(x, y)));
        out.push_str(&format!("{reset}\n{setup}"));
    }
    out + reset
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_values_need_a_name() {
        let parsed = key_values(&["a=b=c".into(), "x=".into()], "--field").unwrap();
        assert_eq!(Value::Object(parsed), json!({ "a": "b=c", "x": "" }));
        assert_eq!(key_values(&["=v".into()], "--cookie").unwrap_err().msg, "--cookie must use name=value form.");
    }

    #[test]
    fn the_terminal_qr_code_has_a_quiet_zone() {
        let qr = terminal_qr("https://beeper.com");
        let lines: Vec<&str> = qr.lines().collect();
        // A version 2 code is 25 modules, so 27 with the quiet zone, in 14 lines of two rows.
        assert_eq!(lines.len(), 15, "{qr}");
        assert!(lines[0].starts_with("\x1b[47m\x1b[30m ▄▄▄▄▄▄▄ "), "{:?}", lines[0]);
    }
}
