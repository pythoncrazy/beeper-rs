//! The command line of the binary, and its client commands against a demo Desktop API server on a free port.

use beeper_api::server::serve_http;
use beeper_core::config::CentralConfig;
use beeper_core::store::{BeeperStore, StoreLock};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::process::Command;

fn run(args: &[&str]) -> (Option<i32>, String, String) {
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_beeper-rs"))
        .args(args)
        .env_remove("BEEPER_PAS_HOST")
        .env_remove("BEEPER_PAS_PORT")
        .output()
        .expect("run beeper-rs");
    let text = |b: Vec<u8>| String::from_utf8(b).expect("UTF-8 output");
    (out.status.code(), text(out.stdout), text(out.stderr))
}

#[test]
fn bad_command_lines_print_the_usage_and_exit_with_the_oclif_codes() {
    let usage = "See more help with --help\n\nUSAGE\n  $ beeper-rs [COMMAND] [FLAGS] [LINK]\n";
    for (args, error, code) in [
        (&["--bogus"][..], "Nonexistent flag: --bogus", 2),
        (&["--port", "abc"], "Parsing --port: Expected an integer from 1 to 65535 but received: abc", 2),
        (&["--data-dir"], "Flag --data-dir expects a value", 2),
    ] {
        assert_eq!(run(args), (Some(code), String::new(), format!("Error: {error}\n{usage}")), "{args:?}");
    }
    let not_found = " ›   Warning: chatz is not a beeper-rs command.\n ›   Error: Run beeper-rs help for a list of available commands.\n";
    assert_eq!(run(&["chatz"]), (Some(127), String::new(), not_found.into()));
    let (code, help, _) = run(&["help"]);
    assert!(code == Some(0) && help.starts_with("Beeper chat app"), "{help}");
    let (code, version, _) = run(&["-V"]);
    assert!(code == Some(0) && version.starts_with("beeper-rs/"), "{version}");
}

struct Demo {
    base: String,
    token: String,
    config: PathBuf,
    _rt: tokio::runtime::Runtime,
}

/// A demo server on a free port, with its own data dir and CLI config dir.
fn demo(name: &str) -> Demo {
    let dir = std::env::temp_dir().join(format!("beeper_rs_cli_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut cfg = CentralConfig::new(dir.join("data"), "127.0.0.1".into(), port, None);
    cfg.demo = true;
    let store = StoreLock::shared(BeeperStore::from_config(cfg));
    let token = store.read().beeper_state.access_token.clone();
    rt.spawn(serve_http(listener, store));
    Demo { base: format!("http://127.0.0.1:{port}"), token, config: dir.join("cli"), _rt: rt }
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    fn json(&self) -> Value {
        serde_json::from_str(&self.stdout).unwrap_or_else(|e| panic!("{e}: {}{}", self.stdout, self.stderr))
    }
}

impl Demo {
    fn run(&self, args: &[&str]) -> Out {
        let out = Command::new(env!("CARGO_BIN_EXE_beeper-rs"))
            .args(args)
            .args(["--base-url", &self.base])
            .env("BEEPER_CLI_CONFIG_DIR", &self.config)
            .env("BEEPER_ACCESS_TOKEN", &self.token)
            .env_remove("BEEPER_READONLY")
            .output()
            .unwrap();
        let text = |b: Vec<u8>| String::from_utf8(b).unwrap();
        Out { code: out.status.code().unwrap(), stdout: text(out.stdout), stderr: text(out.stderr) }
    }
}

#[test]
fn cli_status_doctor_config_and_api_match_upstream() {
    let d = demo("core");
    let status = d.run(&["status"]);
    assert_eq!(
        (status.code, status.stdout),
        (0, format!("▎ Ready  ready\n  target      custom\n  endpoint    {}\n", d.base))
    );
    let status = d.run(&["status", "--json"]).json();
    assert_eq!((status["success"].clone(), status["error"].clone()), (json!(true), Value::Null));
    assert_eq!(status["data"]["readiness"]["actions"], json!(["chats list", "messages list", "send text"]));
    let doctor = d.run(&["doctor", "--json"]).json();
    assert_eq!(
        (doctor["data"]["ok"].clone(), doctor["data"]["checks"]["target"]["reachable"].clone()),
        (json!(true), json!(true))
    );
    let version = d.run(&["version"]).stdout;
    assert_eq!(version, format!("▎ beeper-rs\n  version  {}\n\n", beeper_core::config::APP_VERSION));

    let empty = d.run(&["config", "get"]).stdout;
    assert!(
        empty.starts_with("▎ No config set\n  The CLI is using defaults.\n\nTry\n  › beeper-rs config set baseURL"),
        "{empty}"
    );
    let set = d.run(&["config", "set", "defaultAccount", "acc_slack", "--json"]).json();
    assert_eq!(
        set["data"],
        json!({ "message": "Set defaultAccount", "detail": "acc_slack", "defaultAccount": "acc_slack" })
    );
    assert_eq!(d.run(&["config", "get", "defaultAccount", "--json"]).json()["data"], "acc_slack");
    assert_eq!(d.run(&["config", "path"]).stdout, format!("{}\n", d.config.join("config.json").display()));
    let ro = d.run(&["config", "reset", "--read-only", "--json"]);
    assert_eq!(ro.code, 2);
    let ro: Value = serde_json::from_str(&ro.stderr).unwrap();
    assert_eq!(ro["kind"], "abort");
    assert_eq!(d.run(&["config", "reset"]).stdout, "✓ Config reset\n");

    let info = d.run(&["api", "get", "/v1/info"]).json();
    assert_eq!(info["data"]["server"]["base_url"], d.base);
    let missing = d.run(&["api", "get", "/v1/nothing", "--no-auth"]);
    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("│ GET /v1/nothing failed: 404 "), "{}", missing.stderr);
}

#[test]
fn cli_reports_bad_command_lines_like_oclif() {
    let d = demo("errors");
    let bogus = d.run(&["chats", "list", "--bogus", "--json"]);
    assert_eq!(bogus.code, 1);
    let line: Value = serde_json::from_str(&bogus.stderr).unwrap();
    let want = json!({ "success": false, "data": null, "error": "Nonexistent flag: --bogus\nSee more help with --help", "exitCode": 1, "kind": "bug" });
    assert_eq!(line, want);
    let limit = d.run(&["chats", "list", "--limit", "abc"]);
    assert!(limit.stderr.contains("│ Parsing --limit \n\tExpected an integer but received: abc\n"), "{}", limit.stderr);
    let unknown = d.run(&["chats", "zap"]);
    assert_eq!(unknown.code, 1, "an unknown word after a topic is an argument of its index command");
    let unknown = d.run(&["messagez"]);
    assert_eq!(unknown.code, 127);
    let help = d.run(&["chats", "--help"]).stdout;
    assert!(help.starts_with("List chats\n\nUSAGE\n  $ beeper-rs chats [--base-url <value>]"), "{help}");
    assert!(help.contains("\nCOMMANDS\n  chats archive        Archive a chat\n"), "{help}");
}

#[test]
fn cli_lists_shows_and_changes_chats() {
    let d = demo("chats");
    let ids = d.run(&["chats", "list", "--limit", "3", "--ids"]).stdout;
    assert_eq!(ids, "local_tg_rust\nlocal_wa_eric\nlocal_im_sarah\n");
    let list = d.run(&["chats", "list", "--limit", "1"]).stdout;
    assert!(list.starts_with("▎ Beeper Rust Core Team  telegram  ★"), "{list}");
    assert!(list.ends_with("\n  local local_tg_rust  id !tg_rust:beeper.com\n\n"), "{list}");
    assert_eq!(list.lines().next().unwrap().chars().count(), 80, "the time is at the right edge of 80 columns");
    let unread = d.run(&["chats", "list", "--unread", "--json"]).json();
    assert!(unread["data"].as_array().unwrap().iter().all(|c| c["unreadCount"].as_u64().unwrap() > 0));
    let search = d.run(&["chats", "search", "zzzz"]).stdout;
    assert_eq!(search, "▎ No chats matched\n  Nothing found for \"zzzz\".\n");

    let show = d.run(&["chats", "show", "--chat", "Family"]).stdout;
    assert!(show.starts_with("▎ Family  whatsapp\n  type        group\n"), "{show}");
    assert!(show.ends_with("  id    !wa_family:beeper.com\n  local local_x0\n  acct  acc_whatsapp\n"), "{show}");
    let missing = d.run(&["chats", "show", "--chat", "nonexistentzzz", "--json"]);
    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("\"error\":\"404 Chat not found\""), "{}", missing.stderr);

    let pinned = d.run(&["chats", "pin", "--chat", "local_x5", "--json"]).json();
    assert_eq!(pinned["data"]["id"], "!sl_design:beeper.com");
    let cleared = d.run(&["chats", "unremind", "--chat", "local_x5", "--json"]).json();
    assert_eq!(cleared["data"], json!({ "message": "Reminder cleared", "chatID": "local_x5" }));
    let bad = d.run(&["chats", "disappear", "--chat", "local_x5", "--seconds", "x", "--json"]);
    assert!(bad.stderr.contains("--seconds must be a positive integer or \\\"off\\\""), "{}", bad.stderr);
}
