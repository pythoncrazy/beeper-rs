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

/// A stub Desktop API on a free port: `route` answers each request from its method, path, and body, and the stub
/// keeps a log of the requests.
struct Stub {
    base: String,
    log: std::sync::Arc<std::sync::Mutex<Vec<String>>>,
}

fn stub(route: impl Fn(&str, &str, &str) -> (u16, Value) + Send + 'static) -> Stub {
    use std::io::{BufRead, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let requests = log.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(&stream);
            let (mut head, mut line) = (String::new(), String::new());
            while reader.read_line(&mut line).unwrap() > 2 {
                head.push_str(&std::mem::take(&mut line));
            }
            let length = head
                .lines()
                .find_map(|l| l.to_lowercase().strip_prefix("content-length: ").map(|n| n.trim().parse().unwrap()))
                .unwrap_or(0);
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
            let mut words = head.split(' ');
            let (method, path) = (words.next().unwrap().to_string(), words.next().unwrap().to_string());
            let body = String::from_utf8(body).unwrap();
            let token = head.lines().find_map(|l| l.strip_prefix("authorization: Bearer ")).unwrap_or("-").to_string();
            requests.lock().unwrap().push(format!("{method} {path} {token} {body}").trim_end().to_string());
            let (status, reply) = route(&method, &path, &body);
            let reply = if status == 204 { String::new() } else { reply.to_string() };
            let head = format!(
                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                reply.len()
            );
            (&stream).write_all(format!("{head}{reply}").as_bytes()).unwrap();
        }
    });
    Stub { base, log }
}

impl Stub {
    /// The CLI against the stub, without a token, with the programs in `bin` before the others.
    fn command(&self, config: &std::path::Path, bin: &std::path::Path, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beeper-rs"));
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        command
            .args(args)
            .args(["--base-url", &self.base])
            .env("BEEPER_CLI_CONFIG_DIR", config)
            .env("PATH", path)
            .env_remove("BEEPER_ACCESS_TOKEN")
            .env_remove("BEEPER_READONLY");
        command
    }

    fn requests(&self) -> Vec<String> {
        std::mem::take(&mut self.log.lock().unwrap())
    }
}

/// The value of `key` in the query of `url`.
fn query_value(url: &str, key: &str) -> String {
    let query = url.split_once('?').unwrap().1;
    let value = query.split('&').find_map(|p| p.strip_prefix(&format!("{key}="))).unwrap();
    beeper_core::codec::url_decode(value).unwrap()
}

/// An executable script `name` in `dir`.
fn script(dir: &std::path::Path, name: &str, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), text).unwrap();
    #[cfg(unix)]
    std::fs::set_permissions(dir.join(name), std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
}

#[test]
fn cli_signs_in_with_oauth_pkce_like_upstream() {
    let dir = beeper_core::testing::temp_dir("cli-oauth");
    // The browser of the test writes down the page that the CLI opens.
    let (bin, opened) = (dir.join("bin"), dir.join("opened.txt"));
    script(&bin, "xdg-open", &format!("#!/bin/sh\necho \"$1\" > {}\n", opened.display()));
    let challenge = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
    let wanted = challenge.clone();
    let desktop = stub(move |method, path, body| match (method, path) {
        ("GET", "/v1/info") => (200, json!({})),
        ("GET", "/v1/app/setup") => (401, json!({ "message": "Unauthorized" })),
        ("POST", "/oauth/register") => (201, json!({ "client_id": "cli_stub" })),
        ("POST", "/oauth/token") => {
            let form = format!("?{body}");
            let verifier = query_value(&form, "code_verifier");
            let s256 = beeper_api::codec::base64_encode_url_nopad(&beeper_api::codec::sha256(verifier.as_bytes()));
            match s256 == *wanted.lock().unwrap() && query_value(&form, "code") == "code_1" {
                true => (
                    200,
                    json!({ "access_token": "tok_oauth", "token_type": "Bearer", "expires_in": 3600, "scope": "read write" }),
                ),
                false => (400, json!({ "error": "invalid_grant" })),
            }
        }
        ("GET", "/v1/accounts") => (200, json!([{ "accountID": "acc_stub", "network": "Stub" }])),
        _ => (404, json!({ "message": "Not found" })),
    });
    let config = dir.join("cli");
    let mut cli = desktop.command(&config, &bin, &["accounts", "list", "--json"]);
    let cli = cli.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
    let start = std::time::Instant::now();
    while std::fs::read_to_string(&opened).map_or(true, |u| !u.ends_with('\n')) {
        assert!(start.elapsed() < std::time::Duration::from_secs(10), "the CLI opens the authorization page");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let url = std::fs::read_to_string(&opened).unwrap().trim().to_string();
    let authorize =
        format!("{}/oauth/authorize?client_id=cli_stub&redirect_uri=http%3A%2F%2F127.0.0.1%3A", desktop.base);
    assert!(url.starts_with(&authorize), "{url}");
    assert_eq!(
        (query_value(&url, "scope"), query_value(&url, "code_challenge_method")),
        ("read write".into(), "S256".into())
    );
    *challenge.lock().unwrap() = query_value(&url, "code_challenge");
    // The browser follows the redirect of the approved page to the callback of the CLI.
    let redirect = query_value(&url, "redirect_uri");
    let state = beeper_core::codec::url_encode(&query_value(&url, "state"));
    let page = beeper_api::client::BeeperClient::new(redirect.trim_end_matches("/callback"), "");
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    let (status, html) = rt.block_on(page.fetch("GET", &format!("/callback?code=code_1&state={state}"), None)).unwrap();
    assert_eq!((status, String::from_utf8(html).unwrap().contains("You can close this tab")), (200, true));
    let out = Out::of(cli.wait_with_output().unwrap());
    assert_eq!(out.json()["data"], json!([{ "accountID": "acc_stub", "network": "Stub" }]), "{}", out.stderr);
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(config.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["baseURL"], desktop.base);
    let auth = &saved["auth"];
    assert_eq!(
        (auth["accessToken"].clone(), auth["clientID"].clone(), auth["source"].clone(), auth["tokenType"].clone()),
        (json!("tok_oauth"), json!("cli_stub"), json!("desktop-oauth"), json!("Bearer"))
    );
    assert!(auth["expiresAt"].as_str().unwrap().ends_with('Z'));
    let requests = desktop.requests();
    let paths: Vec<&str> = requests.iter().map(|r| r.split(' ').nth(1).unwrap()).collect();
    assert_eq!(paths, ["/v1/info", "/v1/app/setup", "/oauth/register", "/oauth/token", "/v1/accounts"]);
    assert!(requests[2].contains("{\"client_name\":\"Beeper CLI\",\"grant_types\":[\"authorization_code\"]"));
    let exchange = "POST /oauth/token - grant_type=authorization_code&client_id=cli_stub&code=code_1&code_verifier=";
    assert!(requests[3].starts_with(exchange), "{}", requests[3]);
    assert_eq!(requests[4], "GET /v1/accounts tok_oauth");

    // A non-interactive `setup --oauth` needs --yes to open the browser, and a signed-out Desktop stops the sign-in.
    let out = Out::of(desktop.command(&dir.join("other"), &bin, &["setup", "--oauth", "--json"]).output().unwrap());
    let error: Value = serde_json::from_str(&out.stderr).unwrap();
    assert_eq!(error["error"], "OAuth setup requires an interactive terminal or --yes to open the browser.");
    let signed_out = stub(|_, path, _| match path {
        "/v1/info" => (200, json!({})),
        _ => (200, json!({ "state": "needs-login" })),
    });
    let out = Out::of(signed_out.command(&dir.join("other"), &bin, &["chats", "list", "--json"]).output().unwrap());
    let error: Value = serde_json::from_str(&out.stderr).unwrap();
    let msg = "Beeper Desktop is not signed in. Open Beeper Desktop and sign in, then rerun this command.";
    assert_eq!((out.code, error["error"].clone(), error["kind"].clone()), (3, json!(msg), json!("abort")));
}

#[test]
fn cli_signs_in_by_email_and_registers() {
    let dir = beeper_core::testing::temp_dir("cli-email");
    let desktop = stub(|_, path, _| match path {
        "/v1/app/setup/start" => (200, json!({ "setupRequestID": "req_1", "signInMethods": ["email"] })),
        "/v1/app/setup/email" => (204, Value::Null),
        "/v1/app/setup/response" => (
            200,
            json!({ "registrationRequired": true, "setupRequestID": "req_1", "leadToken": "lead", "usernameSuggestions": ["alex"] }),
        ),
        "/v1/app/setup/register" => {
            (200, json!({ "matrix": { "accessToken": "tok_email" }, "session": { "state": "ready" } }))
        }
        "/v1/app/setup" => (200, json!({ "state": "ready" })),
        "/v1/accounts" => (200, json!({ "items": [{ "network": "WhatsApp" }] })),
        _ => (404, json!({ "message": "Not found" })),
    });
    let (config, bin) = (dir.join("cli"), dir.join("bin"));
    let run = |args: &[&str]| Out::of(desktop.command(&config, &bin, args).output().unwrap());
    let started = run(&["auth", "email", "start", "--email", "a@b.c", "--json"]);
    assert_eq!(started.json()["data"], json!({ "setupRequestID": "req_1" }));
    let email = "POST /v1/app/setup/email - {\"setupRequestID\":\"req_1\",\"email\":\"a@b.c\"}";
    assert_eq!(desktop.requests(), ["POST /v1/app/setup/start -", email]);
    let refused = run(&["auth", "email", "response", "--code", "123", "--setup-request-id", "req_1", "--json"]);
    let error: Value = serde_json::from_str(&refused.stderr).unwrap();
    assert_eq!(error["error"], "Registration requires --yes to accept the Beeper terms in non-interactive setup.");
    let args = ["--code", "123", "--setup-request-id", "req_1", "--username", "alex", "--yes", "--json"];
    let done = run(&[&["auth", "email", "response"][..], &args].concat()).json()["data"].clone();
    let auth = json!({ "source": "manual", "tokenType": "Bearer" });
    let target = json!({ "id": "custom", "type": "desktop", "baseURL": desktop.base, "auth": auth });
    let readiness = json!({ "state": "ready", "app": { "state": "ready" }, "actions": ["chats list", "messages list", "send text"] });
    assert_eq!(
        done,
        json!({ "accounts": ["WhatsApp"], "authSource": "manual", "readiness": readiness, "target": target })
    );
    let register = desktop.requests().into_iter().find(|r| r.contains("/register")).unwrap();
    let body = "{\"acceptTerms\":true,\"leadToken\":\"lead\",\"setupRequestID\":\"req_1\",\"username\":\"alex\"}";
    assert_eq!(register, format!("POST /v1/app/setup/register - {body}"));
    let saved: Value = serde_json::from_str(&std::fs::read_to_string(config.join("config.json")).unwrap()).unwrap();
    assert_eq!(saved["auth"], json!({ "accessToken": "tok_email", "source": "manual", "tokenType": "Bearer" }));
    let setup = run(&["setup", "--email", "a@b.c", "--json"]);
    let hint = "Email setup prompts for the verification code. For automation, use `beeper-rs auth email start`";
    assert!(setup.stderr.contains(hint), "{}", setup.stderr);
}

#[test]
fn cli_adds_accounts_through_bridge_login_steps() {
    let dir = beeper_core::testing::temp_dir("cli-accounts-add");
    let sessions = "/v1/bridges/stubnet/login-sessions";
    let desktop = stub(move |_, path, body| match path {
        "/v1/bridges" => (
            200,
            json!({ "items": [{ "id": "stubnet", "type": "stubnet", "provider": "cloud", "displayName": "Stub Net", "network": "stubnet", "status": "available", "supportsMultipleAccounts": false }] }),
        ),
        "/v1/bridges/stubnet/login-flows" => {
            (200, json!({ "items": [{ "id": "qr", "name": "QR" }, { "id": "password", "name": "Password" }] }))
        }
        p if p == sessions && body.contains("\"qr\"") => (
            200,
            json!({ "loginSessionID": "ls2", "status": "in_progress", "currentStep": { "type": "display_and_wait", "stepID": "q1", "display": { "type": "code", "data": "1234-5678" } } }),
        ),
        p if p == sessions => (
            200,
            json!({ "loginSessionID": "ls1", "status": "in_progress", "currentStep": { "type": "user_input", "stepID": "s1", "instructions": "Enter your details", "fields": [{ "id": "username", "label": "Username", "type": "username" }, { "id": "region", "label": "Region", "initialValue": "eu" }] } }),
        ),
        p if p == format!("{sessions}/ls1/steps/s1") => (
            200,
            json!({ "loginSessionID": "ls1", "status": "in_progress", "currentStep": { "type": "cookies", "stepID": "c1", "url": "https://example.com/login", "fields": [{ "id": "session" }] } }),
        ),
        _ => (
            200,
            json!({ "loginSessionID": "ls1", "status": "complete", "loginID": "login_1", "currentStep": { "type": "complete", "login": { "loginID": "login_1" } } }),
        ),
    });
    let (config, bin) = (dir.join("cli"), dir.join("bin"));
    let run = |args: &[&str]| {
        let mut command = desktop.command(&config, &bin, args);
        Out::of(command.env("BEEPER_ACCESS_TOKEN", "tok").output().unwrap())
    };
    let list = run(&["accounts", "add"]).stdout;
    let cloud =
        "\n\nBeeper Cloud Accounts\n  Stub Net (stubnet) - single account\n    beeper-rs accounts add stubnet\n\n";
    assert!(list.starts_with(&format!("Choose a bridge to connect an account:{cloud}")), "{list}");
    let many = run(&["accounts", "add", "stub", "--non-interactive", "--json"]);
    assert!(
        many.stderr.contains("Multiple sign-in methods are available for Stub Net. Pass --flow."),
        "{}",
        many.stderr
    );
    let waiting = run(&["accounts", "add", "stubnet", "--flow", "qr", "--non-interactive"]).stdout;
    let step = "status: in_progress\nstep: display_and_wait\nstep_id: q1\ndisplay: code\n1234-5678\n";
    assert_eq!(waiting, step.repeat(2), "the guide shows the step, and then prints the session it stops at");
    let missing = run(&["accounts", "add", "stubnet", "--flow", "password", "--non-interactive"]);
    let hint = "Missing required field username. Pass --field username=... or run without --non-interactive.";
    assert!(missing.stderr.contains(hint), "{}", missing.stderr);
    desktop.requests();
    let flags = ["--flow", "password", "--field", "username=alex", "--cookie", "session=abc", "--non-interactive"];
    let done = run(&[&["accounts", "add", "Stub Net"][..], &flags].concat()).stdout;
    let steps = "status: in_progress\nstep: user_input\nEnter your details\nstep_id: s1\nfield username: Username (username)\nfield region: Region\nstatus: in_progress\nstep: cookies\nstep_id: c1\nurl: https://example.com/login\ncookie field session: cookie\n";
    assert!(done.starts_with(steps), "{done}");
    assert!(done.ends_with("status: complete\nlogin_id: login_1\nstep: complete\ncomplete: login_1\n"), "{done}");
    let posts: Vec<String> = desktop.requests().into_iter().filter(|r| r.starts_with("POST")).collect();
    let fields = "{\"type\":\"user_input\",\"fields\":{\"username\":\"alex\",\"region\":\"eu\"}}";
    let cookies = "{\"type\":\"cookies\",\"fields\":{\"session\":\"abc\"},\"source\":\"api\"}";
    assert_eq!(
        posts,
        [
            format!("POST {sessions} tok {{\"flowID\":\"password\"}}"),
            format!("POST {sessions}/ls1/steps/s1 tok {fields}"),
            format!("POST {sessions}/ls1/steps/c1 tok {cookies}"),
        ]
    );
}

#[test]
fn cli_setup_and_verify_follow_the_app_state() {
    let d = demo("setup");
    let data = d.config.with_file_name("data");
    // `setup` finds the signed-in session in the data folder of the app on this device.
    let found = Out::of(d.command(&["setup", "--json"]).env("BEEPER_USER_DATA_DIR", &data).output().unwrap());
    let found = found.json()["data"].clone();
    assert_eq!(found["state"], "desktop-ready");
    assert_eq!(
        found["recommendedAction"],
        json!({ "id": "use-desktop-session", "command": "beeper-rs setup -t custom --local" })
    );
    assert_eq!(found["localDesktop"]["signedInAs"], "@alex:beeper.com");
    let mut elsewhere = d.command(&["setup", "--json"]);
    elsewhere.env("BEEPER_USER_DATA_DIR", d.config.with_file_name("none"));
    let elsewhere = Out::of(elsewhere.output().unwrap()).json()["data"].clone();
    assert_eq!(elsewhere["state"], "desktop-running-session-unreadable");
    assert_eq!(elsewhere["recommendedAction"]["command"], "beeper-rs setup --oauth --yes");
    let local = d.command(&["setup", "--local", "--json"]).env("BEEPER_USER_DATA_DIR", &data).output().unwrap();
    let local = Out::of(local).json()["data"].clone();
    assert_eq!(
        (local["authSource"].clone(), local["readiness"]["state"].clone()),
        (json!("desktop-db"), json!("ready"))
    );
    assert_eq!(local["target"]["auth"], json!({ "source": "desktop-db", "tokenType": "Bearer" }));
    // The stored token of the new `desktop` target works without BEEPER_ACCESS_TOKEN.
    let mut plain = Command::new(env!("CARGO_BIN_EXE_beeper-rs"));
    plain.args(["chats", "list", "--limit", "1", "--ids", "-t", "desktop"]).env("BEEPER_CLI_CONFIG_DIR", &d.config);
    assert_eq!(Out::of(plain.env_remove("BEEPER_ACCESS_TOKEN").output().unwrap()).stdout, "local_tg_rust\n");
    let both = d.run(&["setup", "--local", "--oauth"]);
    assert!(both.stderr.contains("Specify at most one of --local, --oauth, or --email"), "{}", both.stderr);

    assert_eq!(d.run(&["verify", "list", "--json"]).json()["data"], json!([]));
    let null = "{\n  \"success\": true,\n  \"data\": null,\n  \"error\": null\n}\n";
    assert_eq!(d.run(&["verify", "show", "--json"]).stdout, null);
    assert_eq!(d.run(&["verify", "show"]).stdout, "null\n");
    assert_eq!(d.run(&["verify", "--json"]).json()["data"]["state"], "ready");
    assert_eq!(d.run(&["verify", "status", "--json"]).json()["data"]["state"], "ready");
    let approve = d.run(&["verify", "approve", "--json"]);
    let error: Value = serde_json::from_str(&approve.stderr).unwrap();
    assert_eq!(error["error"], "409 Verification action is not available in the current app state");

    // `messages context` centers on a message ID.
    let context = d.run(&["messages", "context", "--chat", "local_wa_eric", "--id", "msg_1000b", "--json"]).json();
    let ids = |k: &str| context["data"][k].as_array().unwrap().iter().map(|m| m["id"].clone()).collect::<Vec<_>>();
    assert_eq!((ids("before"), ids("after")), (vec![json!("msg_1000a")], vec![json!("msg_1001"), json!("msg_1000c")]));
}

/// Stops the managed Server of a test, also when the test fails.
struct Running<'a>(&'a dyn Fn(&[&str]) -> Out);

impl Drop for Running<'_> {
    fn drop(&mut self) {
        (self.0)(&["targets", "stop", "home"]);
    }
}

#[test]
fn cli_runs_managed_server_targets_with_beeper_rs() {
    let dir = beeper_core::testing::temp_dir("cli-managed");
    let (config, home, bin) = (dir.join("cli"), dir.join("home"), dir.join("bin"));
    let systemctl = dir.join("systemctl.log");
    script(&bin, "systemctl", &format!("#!/bin/sh\necho \"$*\" >> {}\n", systemctl.display()));
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let run = |args: &[&str]| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_beeper-rs"));
        let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default());
        command
            .args(args)
            .env("BEEPER_CLI_CONFIG_DIR", &config)
            .env("HOME", &home)
            .env("PATH", path)
            // The Server of the target never reaches Beeper's servers.
            .env("BEEPER_API_URL", "http://127.0.0.1:9")
            .env("BEEPER_MATRIX_URL", "http://127.0.0.1:9")
            .env_remove("BEEPER_SERVER_BIN")
            .env_remove("BEEPER_ACCESS_TOKEN")
            .env_remove("BEEPER_READONLY");
        Out::of(command.output().unwrap())
    };
    let added = run(&["targets", "add", "server", "home", "--port", &port.to_string(), "--json"]).json();
    let (base, data_dir) =
        (format!("http://127.0.0.1:{port}"), config.join("profiles/server/home").display().to_string());
    assert_eq!(
        added["data"],
        json!({
            "message": "Added target: home", "detail": base, "id": "home", "type": "server", "name": "home",
            "baseURL": base, "managed": true, "dataDir": data_dir, "profile": "home",
            "runtime": { "install": "server", "dataDir": data_dir, "port": port }, "serverEnv": "production", "port": port
        })
    );
    assert_eq!(run(&["targets", "add", "server", "home"]).code, 1, "a target name is unique");
    let _running = Running(&run);
    assert_eq!(run(&["targets", "start", "home"]).stdout, format!("✓ Started target: home  {base}\n"));
    // This program serves the Desktop API headless in the data folder of the target, from `BEEPER_SERVER_DATA_DIR`.
    assert!(config.join("profiles/server/home/beeper-rs.db").exists());
    let status = run(&["targets", "status", "home", "--json"]).json()["data"].clone();
    assert_eq!((status["reachable"].clone(), status["actualType"].clone()), (json!(true), json!("server")));
    assert_eq!(run(&["api", "get", "/v1/app/setup", "--no-auth", "-t", "home"]).json()["data"]["state"], "needs-login");
    assert_eq!(run(&["targets", "restart", "home"]).stdout, format!("✓ Restarted target: home  {base}\n"));
    assert_eq!(run(&["targets", "stop", "home"]).stdout, "✓ Stopped target: home\n");
    assert!(run(&["targets", "stop", "home"]).stderr.contains("│ Profile \"home\" is not running.\n"));
    let log = config.join("logs/profiles/home.err.log");
    std::fs::write(&log, "one\ntwo\nthree\n").unwrap();
    assert_eq!(
        run(&["targets", "logs", "home", "--lines", "2"]).stdout,
        format!("\n==> {} <==\ntwo\nthree\n", log.display())
    );

    let enabled = run(&["targets", "enable", "home", "--json"]).json()["data"].clone();
    let unit = home.join(".config/systemd/user/beeper-profile-home.service");
    assert_eq!(enabled["path"], unit.display().to_string());
    let text = std::fs::read_to_string(&unit).unwrap();
    let program = env!("CARGO_BIN_EXE_beeper-rs");
    let exec =
        format!("ExecStart={program} --host=127.0.0.1 --port={port} --data-dir={data_dir} --server-env=production\n");
    assert!(
        text.contains(&exec) && text.contains(&format!("Environment=BEEPER_SERVER_DATA_DIR={data_dir}\n")),
        "{text}"
    );
    run(&["targets", "disable", "home"]);
    assert!(!unit.exists());
    let calls = "--user daemon-reload\n--user enable --now beeper-profile-home.service\n--user disable --now beeper-profile-home.service\n--user daemon-reload\n";
    assert_eq!(std::fs::read_to_string(&systemctl).unwrap(), calls);

    run(&["targets", "add", "desktop", "--port", &port.to_string()]);
    let desktop = run(&["targets", "stop", "desktop"]);
    let refusal = "AbortError: Target \"desktop\" is not a local Beeper Server install.";
    assert!(desktop.stderr.contains(refusal), "{}", desktop.stderr);
    let help = run(&["targets", "add", "--help"]).stdout;
    let topic = "Add a managed Beeper Desktop target\n\nUSAGE\n  $ beeper-rs targets add COMMAND\n\nCOMMANDS\n  targets add desktop";
    assert!(help.starts_with(topic), "{help}");
    let help = run(&["auth", "--help"]).stdout;
    let sections = "\nTOPICS\n  auth email  Finish email sign-in with a verification code\n\nCOMMANDS\n  auth logout  Clear stored authentication\n  auth status";
    assert!(help.contains(sections), "{help}");
}
