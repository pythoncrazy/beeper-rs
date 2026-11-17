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
        // The app ignores unknown switches like Chromium, and a command rejects them like oclif.
        (&["serve", "--bogus"][..], "Nonexistent flag: --bogus", 2),
        (&["--port", "abc"], "Parsing --port: Expected an integer from 1 to 65535 but received: abc", 2),
        (&["--data-dir"], "Flag --data-dir expects a value", 2),
    ] {
        assert_eq!(run(args), (Some(code), String::new(), format!("Error: {error}\n{usage}")), "{args:?}");
    }
    let not_found = " ›   Warning: chatz is not a beeper-rs command.\n ›   Error: Run beeper-rs help for a list of available commands.\n";
    assert_eq!(run(&["chatz"]), (Some(127), String::new(), not_found.into()));
    let (code, help, _) = run(&["help"]);
    assert!(code == Some(0) && help.starts_with("Beeper chat app"), "{help}");
    assert!(
        help.contains("\n  chats     List, search, manage, and modify chats\n"),
        "the root help lists the client topics: {help}"
    );
    let (code, help, _) = run(&["help", "send", "text"]);
    assert!(
        code == Some(0) && help.starts_with("Send a text message\n\nUSAGE\n  $ beeper-rs send text --to <value>"),
        "{help}"
    );
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
    let dir = beeper_core::testing::temp_dir(&format!("cli-{name}"));
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(1).enable_all().build().unwrap();
    let listener = rt.block_on(tokio::net::TcpListener::bind("127.0.0.1:0")).unwrap();
    let port = listener.local_addr().unwrap().port();
    let cfg = CentralConfig { demo: true, ..CentralConfig::new(dir.join("data"), "127.0.0.1".into(), port, None) };
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
    fn command(&self, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beeper-rs"));
        command
            .args(args)
            .args(["--base-url", &self.base])
            .env("BEEPER_CLI_CONFIG_DIR", &self.config)
            .env("BEEPER_ACCESS_TOKEN", &self.token)
            .env_remove("BEEPER_READONLY");
        command
    }

    fn run(&self, args: &[&str]) -> Out {
        Out::of(self.command(args).output().unwrap())
    }
}

impl Out {
    fn of(out: std::process::Output) -> Self {
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
    let missing = d.run(&["api", "get", "/v1/accounts", "--no-auth"]);
    assert_eq!(missing.code, 1);
    assert!(missing.stderr.contains("│ GET /v1/accounts failed: 401 "), "{}", missing.stderr);
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
    // The official CLI: a word after a command without arguments makes the words an unknown command.
    let unknown = d.run(&["chats", "zap"]);
    assert_eq!(unknown.code, 127);
    assert!(unknown.stderr.starts_with(" ›   Warning: chats zap is not a beeper-rs command.\n"), "{}", unknown.stderr);
    assert!(unknown.stderr.contains("Run beeper-rs help chats for"), "{}", unknown.stderr);
    assert_eq!(d.run(&["chats", "search", "a", "b"]).code, 1, "a command with arguments refuses extra ones");
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
    let reminded = d.run(&["chats", "remind", "--chat", "local_x5", "--when", "2030-01-01T00:00:00Z", "--json"]).json();
    assert_eq!(
        reminded["data"],
        json!({ "message": "Reminder set", "detail": "2030-01-01T00:00:00Z", "chatID": "local_x5", "remindAt": "2030-01-01T00:00:00Z" })
    );
    let cleared = d.run(&["chats", "unremind", "--chat", "local_x5", "--json"]).json();
    assert_eq!(cleared["data"], json!({ "message": "Reminder cleared", "chatID": "local_x5" }));
    let bad = d.run(&["chats", "disappear", "--chat", "local_x5", "--seconds", "x", "--json"]);
    assert!(bad.stderr.contains("--seconds must be a positive integer or \\\"off\\\""), "{}", bad.stderr);
}

#[test]
fn cli_lists_searches_sends_and_exports_messages() {
    let d = demo("messages");
    let ids = d.run(&["messages", "list", "--chat", "local_wa_eric", "--ids"]).stdout;
    assert_eq!(ids, "msg_1001\nmsg_1000c\nmsg_1000b\nmsg_1000a\n");
    let asc = d.run(&["messages", "list", "--chat", "Eric Migicovsky", "--limit", "2", "--asc", "--ids"]).stdout;
    assert_eq!(asc, "msg_1000c\nmsg_1001\n");
    let show = d.run(&["messages", "show", "--chat", "local_wa_eric", "--id", "msg_1000b"]).stdout;
    let (head, body) = show.split_once('\n').unwrap();
    assert!(head.starts_with("▎ you  📎 beeper-rs-v4-architecture.pdf  ♥2   "), "{show}");
    assert_eq!(unicode_width::UnicodeWidthStr::width(head), 80, "{show}");
    let text = "  Just migrated the UI to Iced with Inter + Lucide vector rendering and full\n  SQLite WAL persistence. Memory usage is under 28 MB!\n";
    assert_eq!(body, format!("{text}  msg_1000b  in !wa_eric:beeper.com\n\n"));

    let none = d.run(&["messages", "search", "zzzz"]);
    assert!(
        none.stdout.starts_with("▎ No messages matched\n  Nothing found for \"zzzz\".\n\nTry\n"),
        "{}",
        none.stdout
    );
    assert_eq!(none.stderr, "✓ 0 matches\n");
    let usage = d.run(&["messages", "search"]);
    let wrapped =
        "    AbortError: Provide a search query or at least one filter flag (--chat, \n    --sender, --media, etc.).\n";
    assert_eq!((usage.code, usage.stderr), (1, wrapped.to_string()));
    assert_eq!(d.run(&["messages", "search", "--json"]).code, 2, "with --json the exit code is the code of the error");

    let sent = d.run(&["send", "text", "--to", "local_x7", "--message", "hi", "--json"]).json();
    assert_eq!((sent["data"]["state"].clone(), sent["data"]["chatID"].clone()), (json!("accepted"), json!("local_x7")));
    let waited = d.run(&["send", "text", "--to", "local_x7", "--message", "hello there", "--wait", "--json"]).json();
    assert_eq!(
        (waited["data"]["state"].clone(), waited["data"]["message"]["text"].clone()),
        (json!("resolved"), json!("hello there"))
    );
    let file = d.config.with_file_name("note.txt");
    std::fs::write(&file, "a note\n").unwrap();
    let sent = d.run(&["send", "file", "--to", "local_x7", "--file", file.to_str().unwrap(), "--json"]).json();
    assert_eq!(sent["data"]["accepted"], true);
    let missing = d.run(&["send", "file", "--to", "local_x7", "--file", "/nonexistent/x.txt"]);
    assert!(
        missing.stderr.contains("│   ENOENT: no such file or directory, open '/nonexistent/x.txt'\n"),
        "{}",
        missing.stderr
    );
    let react =
        d.run(&["send", "react", "--to", "local_wa_eric", "--id", "msg_1001", "--reaction", "👍", "--json"]).json();
    assert_eq!(react["data"]["reactionKey"], "👍");

    let export: Value =
        serde_json::from_str(&d.run(&["messages", "export", "--chat", "local_wa_eric", "--limit", "2"]).stdout)
            .unwrap();
    assert_eq!((export["chatID"].clone(), export["count"].clone()), (json!("local_wa_eric"), json!(2)));
}

#[test]
fn cli_lists_accounts_contacts_and_bridges() {
    let d = demo("accounts");
    let ids = d.run(&["accounts", "--ids"]).stdout;
    assert_eq!(ids, "acc_whatsapp\nacc_imessage\nacc_telegram\nacc_signal\nacc_instagram\nacc_slack\n");
    let show = d.run(&["accounts", "show", "telegram"]).stdout;
    assert_eq!(show, "▎ Telegram  telegram\n  acc_telegram  bridge telegram cloud telegram\n\n");
    let missing = d.run(&["accounts", "show", "zzz", "--json"]);
    assert_eq!(missing.code, 5, "a selector without a match is a not-found error");
    let removed = d.run(&["accounts", "remove", "telegram"]);
    assert!(removed.stderr.contains("│ This Desktop API does not expose account removal.\n"), "{}", removed.stderr);

    let used = d.run(&["accounts", "use", "telegram", "--json"]).json();
    assert_eq!(used["data"], json!({ "message": "Default account: acc_telegram", "accountID": "acc_telegram" }));
    let default = d.run(&["accounts", "list", "--json"]).json();
    assert_eq!(default["data"][2]["default"], true, "the default account has the default mark");
    let chats = d.run(&["chats", "list", "--json"]).json();
    assert!(
        chats["data"].as_array().unwrap().iter().all(|c| c["accountID"] == "acc_telegram"),
        "the default account limits the chats"
    );
    d.run(&["accounts", "use", ""]);

    let search = d.run(&["contacts", "search", "tulir"]);
    assert_eq!(search.stdout, "▎ Tulir Asokan\n  @tulir  tulir@example.com\n  @tulir:beeper.com  on acc_telegram\n\n");
    assert_eq!(search.stderr, "✓ 1 match across 6 accounts\n");
    let contact = d.run(&["contacts", "show", "@tulir:beeper.com", "--account", "telegram", "--json"]).json();
    assert_eq!(
        (contact["data"]["accountID"].clone(), contact["data"]["contact"]["fullName"].clone()),
        (json!("acc_telegram"), json!("Tulir Asokan"))
    );
    let listed = d.run(&["contacts", "--limit", "2", "--ids"]).stdout;
    assert_eq!(listed.lines().count(), 2, "{listed}");
    assert_eq!(d.run(&["bridges", "list", "--provider", "nowhere"]).code, 1, "a bad option is a parse error");
}

#[test]
fn cli_watches_the_websocket_and_runs_rpc_lines() {
    use std::io::{BufRead, Write};
    let d = demo("stream");
    let mut watch = d
        .command(&["watch", "--json", "-c", "!wa_eric:beeper.com"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let mut lines = std::io::BufReader::new(watch.stdout.take().unwrap()).lines();
    let mut next = || serde_json::from_str::<Value>(&lines.next().unwrap().unwrap()).unwrap();
    assert_eq!(next()["type"], "ready");
    assert_eq!(
        next(),
        json!({ "type": "subscriptions.updated", "chatIDs": ["!wa_eric:beeper.com"], "app": { "state": false } })
    );
    d.run(&["send", "text", "--to", "local_wa_eric", "--message", "during watch"]);
    let event = next();
    assert_eq!(
        (event["type"].clone(), event["entries"][0]["text"].clone()),
        (json!("message.upserted"), json!("during watch"))
    );
    // Windows has no SIGINT to send to another process.
    #[cfg(unix)]
    {
        assert!(Command::new("kill").args(["-INT", &watch.id().to_string()]).status().unwrap().success());
        assert_eq!(watch.wait().unwrap().code(), Some(0), "Ctrl+C closes the socket and exits");
    }
    #[cfg(not(unix))]
    {
        watch.kill().unwrap();
        watch.wait().unwrap();
    }

    let mut rpc =
        d.command(&["rpc"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let lines = [
        format!("{{\"id\":1,\"command\":\"accounts --ids --base-url {}\"}}", d.base),
        "{\"id\":2,\"args\":[\"rpc\"]}".into(),
        "{\"id\":3,\"command\":\"say \\\"hi\"}".into(),
        "{\"id\":4}".into(),
    ];
    rpc.stdin.take().unwrap().write_all(lines.join("\n").as_bytes()).unwrap();
    let out = Out::of(rpc.wait_with_output().unwrap());
    let replies: Vec<Value> = out.stdout.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    let accounts = "acc_whatsapp\nacc_imessage\nacc_telegram\nacc_signal\nacc_instagram\nacc_slack\n";
    let want = [
        json!({ "id": 1, "ok": true, "code": 0, "signal": null, "stdout": accounts, "stderr": "" }),
        json!({ "id": 2, "ok": false, "error": "Unsupported nested command: rpc" }),
        json!({ "id": 3, "ok": false, "error": "Unclosed \" quote" }),
        json!({ "id": 4, "ok": false, "error": "Expected args, argv, or command" }),
    ];
    assert_eq!(replies, want);
}

#[test]
fn cli_manages_targets_and_downloads_and_exports() {
    let d = demo("targets");
    let added = d.run(&["targets", "add", "remote", "home", &d.base, "--default"]).stdout;
    assert_eq!(added, format!("✓ Added target: home  {}\n", d.base));
    assert_eq!(d.run(&["targets", "add", "remote", "home", &d.base]).code, 1, "a target name is unique");
    // Without --base-url, the default target selects the server.
    let plain = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beeper-rs"));
        command.args(args).env("BEEPER_CLI_CONFIG_DIR", &d.config).env("BEEPER_ACCESS_TOKEN", &d.token);
        Out::of(command.output().unwrap())
    };
    let status = plain(&["status"]).stdout;
    assert_eq!(status, format!("▎ Ready  ready\n  target      home\n  endpoint    {}\n", d.base));
    let list = plain(&["targets", "--json"]).json();
    let row = &list["data"][0];
    assert_eq!(
        (row["id"].clone(), row["default"].clone(), row["reachable"].clone()),
        (json!("home"), json!(true), json!(true))
    );
    assert_eq!(plain(&["chats", "list", "--limit", "1", "--ids"]).stdout, "local_tg_rust\n");
    let logout = plain(&["auth", "logout"]);
    assert!(
        logout.stderr.contains("│ auth logout cannot clear BEEPER_ACCESS_TOKEN from the environment;"),
        "{}",
        logout.stderr
    );
    assert_eq!(plain(&["targets", "remove", "home"]).stdout, "✓ Removed target: home\n");
    assert!(!d.config.join("targets/home.json").exists());

    let file = d.config.with_file_name("note.txt");
    std::fs::write(&file, "a note\n").unwrap();
    let sent =
        d.run(&["send", "file", "--to", "local_x7", "--file", file.to_str().unwrap(), "--wait", "--json"]).json();
    let url = sent["data"]["message"]["attachments"][0]["srcURL"].as_str().unwrap().to_string();
    assert_eq!(d.run(&["media", "download", &url, "-o", "-"]).stdout, "a note\n");
    let out = d.config.with_file_name("media");
    let saved = d.run(&["media", "download", &url, "-o", out.to_str().unwrap(), "--json"]).json();
    let path = saved["data"]["path"].as_str().unwrap();
    assert_eq!(std::fs::read_to_string(path).unwrap(), "a note\n");

    let dir = d.config.with_file_name("export");
    let export = d.run(&["export", "-o", dir.to_str().unwrap(), "--chat", "local_x0", "--no-attachments"]);
    assert_eq!(export.stdout, format!("Exported 1 chats, 1 messages, 0 attachments to {}\n", dir.display()));
    assert!(
        export.stderr.ends_with(
            "[1/1] Family (WhatsApp) complete: 1 messages, 0 attachments\nDone: 1 chats, 1 messages, 0 attachments\n"
        ),
        "{}",
        export.stderr
    );
    let chat = dir.join("chats/wa_family_beeper.com");
    let markdown = std::fs::read_to_string(chat.join("messages.markdown")).unwrap();
    assert!(markdown.starts_with("# Family\n\n- Chat ID: `!wa_family:beeper.com`\n- Account ID: `acc_whatsapp`\n- Network: WhatsApp\n- Type: group\n- Messages: 1\n\n## Messages\n\n### Mom - "), "{markdown}");
    let html = std::fs::read_to_string(chat.join("messages.html")).unwrap();
    assert!(
        html.contains("      <article class=\"message\">\n        <header><strong>Mom</strong><time datetime="),
        "{html}"
    );
    let manifest: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("manifest.json")).unwrap()).unwrap();
    assert_eq!((manifest["chatCount"].clone(), manifest["messageCount"].clone()), (json!(1), json!(1)));
    let again = d.run(&["export", "-o", dir.to_str().unwrap(), "--chat", "local_x0", "--no-attachments"]);
    assert!(
        again.stderr.contains("[1/1] Family (WhatsApp) already complete\n"),
        "the state file resumes: {}",
        again.stderr
    );
}
