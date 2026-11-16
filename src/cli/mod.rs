//! The client commands of the upstream Beeper CLI (`@beeper/cli` 0.6.2, oclif): the same topics, commands, args,
//! flags, defaults, text output, `--json` envelope, and exit codes. They talk to any Desktop API server through
//! [`BeeperClient`].

mod accounts;
mod chats;
pub mod commands;
mod export;
mod messages;
mod render;
mod stream;
mod targets;

use beeper_api::client::{ApiError, BeeperClient};
use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;

pub const BIN: &str = env!("CARGO_BIN_NAME");

/// How a flag takes its value, like the oclif flag types.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Kind {
    Bool,
    /// A boolean flag that also has a `--no-` form. It is unset when the user gives neither form.
    AllowNo,
    Str,
    Int,
    /// A string flag that the user can repeat. Like oclif, it also takes the next words that are not flags.
    Many,
}

pub struct Flag {
    pub name: &'static str,
    pub char: Option<char>,
    pub kind: Kind,
    pub desc: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
    pub options: &'static [&'static str],
}

impl Flag {
    const fn new(name: &'static str, kind: Kind, desc: &'static str) -> Self {
        Self { name, char: None, kind, desc, required: false, default: None, options: &[] }
    }
    pub const fn b(name: &'static str, desc: &'static str) -> Self {
        Self::new(name, Kind::Bool, desc)
    }
    pub const fn no(name: &'static str, desc: &'static str) -> Self {
        Self::new(name, Kind::AllowNo, desc)
    }
    pub const fn s(name: &'static str, desc: &'static str) -> Self {
        Self::new(name, Kind::Str, desc)
    }
    pub const fn i(name: &'static str, desc: &'static str) -> Self {
        Self::new(name, Kind::Int, desc)
    }
    pub const fn many(name: &'static str, desc: &'static str) -> Self {
        Self::new(name, Kind::Many, desc)
    }
    pub const fn c(mut self, char: char) -> Self {
        self.char = Some(char);
        self
    }
    pub const fn req(mut self) -> Self {
        self.required = true;
        self
    }
    pub const fn def(mut self, default: &'static str) -> Self {
        self.default = Some(default);
        self
    }
    pub const fn opts(mut self, options: &'static [&'static str]) -> Self {
        self.options = options;
        self
    }
}

pub struct Arg {
    pub name: &'static str,
    pub desc: &'static str,
    pub required: bool,
    pub options: &'static [&'static str],
}

pub const fn arg(name: &'static str, desc: &'static str) -> Arg {
    Arg { name, desc, required: true, options: &[] }
}

pub const fn opt_arg(name: &'static str, desc: &'static str) -> Arg {
    Arg { name, desc, required: false, options: &[] }
}

pub type Run = fn(&mut Ctx) -> Result<(), CliError>;

pub struct Cmd {
    pub id: &'static str,
    pub summary: &'static str,
    pub description: &'static str,
    pub args: &'static [Arg],
    pub flags: &'static [Flag],
    pub examples: &'static [&'static str],
    pub run: Run,
}

/// The flags of every command (`lib/command.js` `baseFlags`).
pub const GLOBAL: &[Flag] = &[
    Flag::s("base-url", "Beeper Desktop API base URL (overrides --target)"),
    Flag::s("target", "Named Beeper target to use for this command").c('t'),
    Flag::b("debug", "Print SDK debug logging on stderr"),
    Flag::b("events", "Emit NDJSON lifecycle events on stderr (long-running commands)"),
    Flag::b("full", "Disable text-output truncation; print full IDs and bodies"),
    Flag::b("json", "Print machine-readable JSON envelope on stdout"),
    Flag::b("quiet", "Suppress spinners and success lines (errors still print). Honored with or without --json.")
        .c('q'),
    Flag::b("read-only", "Reject commands that would modify Beeper or local CLI state (or set BEEPER_READONLY=1)"),
    Flag::s("timeout", "Maximum time to wait, such as 30s, 2m, or 1h"),
    Flag::b("yes", "Skip interactive confirmation prompts").c('y'),
];

/// An error of a command. `Bug` is a plain JavaScript `Error` upstream (a parse error, an API error, a check of the
/// command), which prints a panel. `Abort` is a `CLIError` or `AbortError` with its class name and exit code.
#[derive(Debug, PartialEq)]
pub struct CliError {
    pub msg: String,
    pub code: i32,
    pub class: Option<&'static str>,
    pub hint: Option<String>,
    /// The first line of the stack in the error panel: `Error: ` before the message, or nothing for a system error.
    pub stack: &'static str,
}

impl CliError {
    pub fn bug(msg: impl Into<String>) -> Self {
        Self { msg: msg.into(), code: 1, class: None, hint: None, stack: "Error: " }
    }
    pub fn abort(msg: impl Into<String>, code: i32) -> Self {
        Self { msg: msg.into(), code, class: Some("AbortError"), hint: None, stack: "Error: " }
    }
    /// A file system error of Node, whose stack starts with the message.
    pub fn system(msg: impl Into<String>) -> Self {
        Self { stack: "", ..Self::bug(msg) }
    }
    fn usage(msg: impl Into<String>) -> Self {
        Self::bug(format!("{}\nSee more help with --help", msg.into()))
    }
}

impl From<ApiError> for CliError {
    fn from(e: ApiError) -> Self {
        Self::bug(e.0)
    }
}

pub type Res<T = ()> = Result<T, CliError>;

pub const NOT_FOUND: i32 = 5;
pub const AMBIGUOUS: i32 = 6;

#[derive(Clone, Debug, PartialEq)]
enum Val {
    Bool(bool),
    Str(String),
    Int(i64),
    Many(Vec<String>),
}

/// The command for `words`: an exact id, else the same words in another order like oclif's `flexibleTaxonomy`. The
/// command takes the longest match; the other words are its arguments.
pub fn find(words: &[String]) -> Option<(&'static Cmd, usize)> {
    let ids = commands::all().map(|c| (c, c.id.split(' ').collect::<Vec<_>>()));
    let matches = |id: &[&str], n: usize| {
        let mut given: Vec<&str> = words[..n].iter().map(String::as_str).collect();
        let mut id = id.to_vec();
        given.sort_unstable();
        id.sort_unstable();
        given == id
    };
    (1..=words.len().min(3))
        .rev()
        .find_map(|n| ids.clone().find(|(_, id)| id.len() == n && matches(id, n)).map(|(c, _)| (c, n)))
}

/// Whether `word` starts a client command or a topic, so `main` hands the command line to [`run`].
pub fn is_cli(word: &str) -> bool {
    commands::all().any(|c| c.id.split(' ').next() == Some(word)) || commands::topic(word).is_some()
}

/// The flag of `cmd` that `pred` selects. A command flag replaces the global flag of the same name.
fn flag_spec(cmd: &Cmd, pred: impl Fn(&Flag) -> bool) -> Option<&Flag> {
    let globals = GLOBAL.iter().filter(|g| cmd.flags.iter().all(|f| f.name != g.name));
    cmd.flags.iter().chain(globals).find(|f| pred(f))
}

/// Parses the words after the command id like the oclif parser, with its error text.
fn parse(cmd: &'static Cmd, argv: &[String]) -> Res<(HashMap<&'static str, Val>, Vec<String>)> {
    let mut vals: HashMap<&'static str, Val> = HashMap::new();
    let (mut args, mut unknown) = (Vec::new(), Vec::new());
    let mut words = argv.iter().cloned().peekable();
    let is_flag = |w: &str| {
        let name = w.split_once('=').map_or(w, |p| p.0);
        name.strip_prefix("--").is_some_and(|n| flag_spec(cmd, |f| f.name == n).is_some())
            || name.strip_prefix('-').is_some_and(|c| {
                c.len() == 1 && flag_spec(cmd, |f| f.char.is_some_and(|ch| c.starts_with(ch))).is_some()
            })
    };
    while let Some(word) = words.next() {
        if word == "--" {
            args.extend(words.by_ref());
            break;
        }
        let (name, inline) = match word.split_once('=') {
            Some((n, v)) if n.starts_with("--") => (n.to_string(), Some(v.to_string())),
            _ => (word.clone(), None),
        };
        let spec = match (name.strip_prefix("--"), name.strip_prefix('-')) {
            (Some(long), _) => flag_spec(cmd, |f| f.name == long).map(|f| (f, true)).or_else(|| {
                long.strip_prefix("no-")
                    .and_then(|n| flag_spec(cmd, |f| f.name == n && f.kind == Kind::AllowNo))
                    .map(|f| (f, false))
            }),
            (None, Some(short)) if !short.is_empty() => {
                let ch = short.chars().next().expect("a short flag has a char");
                flag_spec(cmd, |f| f.char == Some(ch)).map(|f| (f, true))
            }
            _ => None,
        };
        let Some((flag, on)) = spec else {
            match word.starts_with('-') && word != "-" {
                true => unknown.push(word),
                false => args.push(word),
            }
            continue;
        };
        let short_inline = (!name.starts_with("--") && name.len() > 2).then(|| name[2..].to_string());
        let val = match flag.kind {
            Kind::Bool | Kind::AllowNo => Val::Bool(on),
            _ => {
                if flag.kind != Kind::Many && vals.contains_key(flag.name) {
                    return Err(CliError::bug(format!("Flag --{} can only be specified once", flag.name)));
                }
                let value = inline.or(short_inline).or_else(|| words.next_if(|w| !is_flag(w)));
                let shown = if name.starts_with("--") { name.clone() } else { format!("--{}", flag.name) };
                let value = value.ok_or_else(|| match flag.options {
                    [] => CliError::bug(format!("Flag {shown} expects a value")),
                    options => {
                        CliError::bug(format!("Flag {shown} expects one of these values: {}", options.join(", ")))
                    }
                })?;
                let mut values = vec![value];
                if flag.kind == Kind::Many {
                    values.extend(std::iter::from_fn(|| words.next_if(|w| !w.starts_with('-'))));
                }
                if let Some(bad) =
                    values.iter().find(|v| !flag.options.is_empty() && !flag.options.contains(&v.as_str()))
                {
                    let msg = format!("Expected --{}={bad} to be one of: {}", flag.name, flag.options.join(", "));
                    return Err(CliError::usage(msg));
                }
                match flag.kind {
                    Kind::Int => Val::Int(int(&values[0]).ok_or_else(|| {
                        let msg =
                            format!("Parsing --{} \n\tExpected an integer but received: {}", flag.name, values[0]);
                        CliError::usage(msg)
                    })?),
                    Kind::Many => match vals.remove(flag.name) {
                        Some(Val::Many(all)) => Val::Many([all, values].concat()),
                        _ => Val::Many(values),
                    },
                    _ => Val::Str(values.remove(0)),
                }
            }
        };
        vals.insert(flag.name, val);
    }
    if let Some(word) = unknown.first() {
        return Err(CliError::usage(format!("Nonexistent flag: {word}")));
    }
    if args.len() > cmd.args.len() {
        let extra = &args[cmd.args.len()..];
        let s = if extra.len() > 1 { "s" } else { "" };
        return Err(CliError::usage(format!("Unexpected argument{s}: {}", extra.join(", "))));
    }
    if args.len() < cmd.args.len() {
        args.extend(stdin());
    }
    let missing: Vec<&Arg> = cmd.args.iter().skip(args.len()).filter(|a| a.required).collect();
    if !missing.is_empty() {
        let s = if missing.len() > 1 { "s" } else { "" };
        let width = missing.iter().map(|a| a.name.len()).max().expect("a missing arg");
        let lines: Vec<String> = missing.iter().map(|a| format!("{:width$}  {}", a.name, a.desc)).collect();
        let note = cmd.flags.iter().find(|f| f.kind == Kind::Many).map_or(String::new(), |f| {
            format!(
                "\nNote: --{} allows multiple values. Because of this you need to provide all arguments before providing that flag.\nAlternatively, you can use \"--\" to signify the end of the flags and the beginning of arguments.",
                f.name
            )
        });
        return Err(CliError::usage(format!(
            "Missing {} required arg{s}:\n{}\n{note}",
            missing.len(),
            lines.join("\n")
        )));
    }
    for (a, value) in cmd.args.iter().zip(&args) {
        if !a.options.is_empty() && !a.options.contains(&value.as_str()) {
            return Err(CliError::usage(format!("Expected {value} to be one of: {}", a.options.join(", "))));
        }
    }
    let missing: Vec<String> = cmd
        .flags
        .iter()
        .filter(|f| f.required && !vals.contains_key(f.name))
        .map(|f| format!("  Missing required flag {}", f.name))
        .collect();
    if !missing.is_empty() {
        let s = if missing.len() > 1 { "s" } else { "" };
        return Err(CliError::usage(format!("The following error{s} occurred:\n{}", missing.join("\n"))));
    }
    for f in cmd.flags.iter().chain(GLOBAL) {
        if let (Some(d), false) = (f.default, vals.contains_key(f.name)) {
            let val = match f.kind {
                Kind::Bool | Kind::AllowNo => Val::Bool(d == "true"),
                Kind::Int => Val::Int(int(d).expect("an integer default")),
                Kind::Str => Val::Str(d.into()),
                Kind::Many => Val::Many(vec![d.into()]),
            };
            vals.insert(f.name, val);
        }
    }
    Ok((vals, args))
}

/// oclif's `readStdin`: piped input fills the first missing argument, if it arrives within 10 ms.
fn stdin() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = tx.send(std::io::stdin().read_to_string(&mut text).map(|_| text));
    });
    let text = rx.recv_timeout(std::time::Duration::from_millis(10)).ok()?.ok()?;
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

fn int(s: &str) -> Option<i64> {
    let digits = s.strip_prefix('-').unwrap_or(s);
    (!digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())).then(|| s.parse().ok()).flatten()
}

/// The state of one command: its parsed flags, the selected target, and the lazily built client.
pub struct Ctx {
    pub cmd: &'static Cmd,
    vals: HashMap<&'static str, Val>,
    pub args: Vec<String>,
    rt: tokio::runtime::Runtime,
    client: Option<BeeperClient>,
    target: Option<Map<String, Value>>,
}

impl Ctx {
    fn spec(&self, name: &str) -> &'static Flag {
        flag_spec(self.cmd, |f| f.name == name).unwrap_or_else(|| panic!("{} has no flag --{name}", self.cmd.id))
    }
    pub fn str(&self, name: &str) -> Option<&str> {
        assert!(matches!(self.spec(name).kind, Kind::Str), "--{name} is a string flag");
        match self.vals.get(name) {
            Some(Val::Str(s)) => Some(s),
            _ => None,
        }
    }
    pub fn int(&self, name: &str) -> Option<i64> {
        assert_eq!(self.spec(name).kind, Kind::Int, "--{name} is an integer flag");
        match self.vals.get(name) {
            Some(Val::Int(n)) => Some(*n),
            _ => None,
        }
    }
    pub fn on(&self, name: &str) -> bool {
        self.opt_bool(name).unwrap_or(false)
    }
    pub fn opt_bool(&self, name: &str) -> Option<bool> {
        assert!(matches!(self.spec(name).kind, Kind::Bool | Kind::AllowNo), "--{name} is a boolean flag");
        match self.vals.get(name) {
            Some(Val::Bool(b)) => Some(*b),
            _ => None,
        }
    }
    pub fn many(&self, name: &str) -> &[String] {
        assert_eq!(self.spec(name).kind, Kind::Many, "--{name} is a repeatable flag");
        match self.vals.get(name) {
            Some(Val::Many(v)) => v,
            _ => &[],
        }
    }
    pub fn arg(&self, i: usize) -> Option<&str> {
        assert!(i < self.cmd.args.len(), "{} has no argument {i}", self.cmd.id);
        self.args.get(i).map(String::as_str)
    }
    pub fn json(&self) -> bool {
        self.on("json")
    }

    pub fn block<T>(&self, f: impl std::future::Future<Output = T>) -> T {
        self.rt.block_on(f)
    }

    /// `ensureWritable`: `--read-only` or `BEEPER_READONLY` rejects a command that changes state.
    pub fn writable(&self) -> Res {
        let env = std::env::var("BEEPER_READONLY").unwrap_or_default().to_lowercase();
        match self.on("read-only") || ["1", "true", "yes", "on"].contains(&env.as_str()) {
            true => Err(CliError {
                class: Some("CLIError"),
                ..CliError::abort("read-only mode: command would modify Beeper or local CLI state", 2)
            }),
            false => Ok(()),
        }
    }

    /// `resolveTarget`: `--base-url`, `-t`, `BEEPER_TARGET`, the default target, the only target, the built-in
    /// `desktop` target.
    pub fn target(&mut self) -> Res<&Map<String, Value>> {
        if self.target.is_none() {
            let target = match self.str("base-url") {
                Some(url) => obj([("id", json!("custom")), ("type", json!("desktop")), ("baseURL", json!(url))]),
                None => resolve_target(self.str("target"))?,
            };
            self.target = Some(target);
        }
        Ok(self.target.as_ref().expect("the target is set"))
    }

    /// Selects `target` for the next requests.
    pub fn set_target(&mut self, target: Map<String, Value>) {
        (self.target, self.client) = (Some(target), None);
    }

    pub fn base_url(&mut self) -> Res<String> {
        Ok(self.target()?["baseURL"].as_str().expect("a target has a base URL").to_string())
    }

    /// `requireToken`: `BEEPER_ACCESS_TOKEN`, else the stored token of the target.
    pub fn token(&mut self) -> Res<String> {
        let base = self.base_url()?;
        let stored = self.target()?.get("auth").and_then(|a| a["accessToken"].as_str()).map(str::to_string);
        match std::env::var("BEEPER_ACCESS_TOKEN").ok().filter(|t| !t.is_empty()).or(stored) {
            Some(token) => Ok(token),
            None => self.no_token(&base),
        }
    }

    /// `createClient`: a client of the target with its token.
    pub fn client(&mut self) -> Res<&BeeperClient> {
        if self.client.is_none() {
            let (base, token) = (self.base_url()?, self.token()?);
            let mut client = BeeperClient::try_new(&base, token).map_err(CliError::bug)?;
            client.debug = self.on("debug");
            self.client = Some(client);
        }
        Ok(self.client.as_ref().expect("the client is set"))
    }

    /// `ensureDesktopToken` without a stored token: a signed-out target stops with exit 3. beeper-rs has no browser
    /// OAuth flow, so a signed-in target asks for a token.
    fn no_token(&self, base: &str) -> Res<String> {
        let probe = BeeperClient::try_new(base, "").map_err(CliError::bug)?;
        let reachable = self.block(probe.call("GET", "/v1/info", None)).is_ok();
        if !reachable {
            return Err(CliError::abort(format!("Could not find a running Beeper Desktop API on {base}."), 4));
        }
        let state = self.block(probe.call("GET", "/v1/app/setup", None)).ok();
        if state.is_some_and(|s| s["state"] == "needs-login") {
            let msg = "Beeper Desktop is not signed in. Open Beeper Desktop and sign in, then rerun this command.";
            return Err(CliError::abort(msg, 3));
        }
        let msg = "No access token for this target. Set BEEPER_ACCESS_TOKEN to a Desktop API token.";
        Err(CliError::abort(msg, 3))
    }

    pub fn call(&mut self, method: &str, path: &str, body: Option<Value>) -> Res<Value> {
        let client = self.client()?.clone();
        Ok(self.block(client.call(method, path, body.as_ref()))?)
    }

    pub fn get(&mut self, path: &str) -> Res<Value> {
        self.call("GET", path, None)
    }

    /// The items of a cursor list like the SDK page iterator, which asks for the next page at `oldestCursor`.
    pub fn collect(
        &mut self,
        path: &str,
        query: &[(&str, String)],
        limit: usize,
        keep: impl Fn(&Value) -> bool,
    ) -> Res<Vec<Value>> {
        let mut items = Vec::new();
        let mut cursor: Option<String> = None;
        while items.len() < limit {
            let mut q: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
            q.retain(|(k, _)| cursor.is_none() || *k != "cursor");
            q.extend(cursor.as_deref().map(|c| ("cursor", c)));
            let page = self.get(&beeper_api::http::query_string(path, &q))?;
            let list = page["items"].as_array().cloned().unwrap_or_default();
            items.extend(list.into_iter().filter(&keep).take(limit - items.len()));
            match (page["hasMore"].as_bool(), page["oldestCursor"].as_str()) {
                (Some(true), Some(next)) if !next.is_empty() => cursor = Some(next.to_string()),
                _ => break,
            }
        }
        Ok(items)
    }

    /// `withSpinner`: runs `run`, then shows its result line on stderr like the last frame of the ink spinner, before
    /// the output of the command, unless `--quiet`.
    pub fn spinner<T>(
        &mut self,
        label: &str,
        run: impl FnOnce(&mut Self) -> Res<T>,
        done: impl Fn(&T) -> String,
    ) -> Res<T> {
        let quiet = self.on("quiet");
        let result = run(self);
        match (&result, quiet) {
            (_, true) => {}
            (Ok(v), false) => eprintln!("✓ {}", done(v)),
            (Err(_), false) => eprintln!("✗ {label}"),
        }
        result
    }

    /// `printData`: the envelope with `--json`, else the text card of the value.
    pub fn print(&self, value: &Value) -> Res {
        match self.json() {
            // `JSON.stringify` drops the `data` of an `undefined` value.
            true if value.is_null() => println!("{}", pretty(&json!({ "success": true, "error": null }))),
            true => envelope(value),
            false => render::print(&render::value(value)),
        }
        Ok(())
    }

    /// `printList`: the envelope with `--json`, else the rows, or the empty state when there are none.
    pub fn print_list(&self, items: &[Value], empty: render::Empty) -> Res {
        match self.json() {
            true => envelope(&Value::Array(items.to_vec())),
            false => render::print(&render::list(items, Some(&empty))),
        }
        Ok(())
    }

    /// `printSuccess`: the envelope of the message and its data with `--json`, else a check line unless `--quiet`.
    pub fn success(&self, message: &str, detail: Option<&str>, data: Value) -> Res {
        let mut all = obj([("message", json!(message))]);
        all.extend(detail.map(|d| ("detail".to_string(), json!(d))));
        all.extend(data.as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.clone())));
        match (self.json(), self.on("quiet")) {
            (true, _) => envelope(&Value::Object(all)),
            (false, true) => {}
            (false, false) => render::print(&render::success(message, detail)),
        }
        Ok(())
    }

    /// `printIDs`: one selector a line.
    pub fn print_ids(&self, items: &[Value]) -> Res {
        for item in items {
            let keys = ["localChatID", "rowID", "id", "chatID", "messageID"];
            if let Some(id) = keys.iter().map(|k| &item[k]).find(|v| !v.is_null()).filter(|v| truthy(v)) {
                println!("{}", js_string(id));
            }
        }
        Ok(())
    }
}

pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("JSON values serialize")
}

/// An object from pairs, without the `Null` values like `JSON.stringify` drops `undefined`.
pub fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Map<String, Value> {
    pairs.into_iter().filter(|(_, v)| !v.is_null()).map(|(k, v)| (k.to_string(), v)).collect()
}

/// `String(value)` of JavaScript.
pub fn js_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => {
            a.iter().map(|v| if v.is_null() { String::new() } else { js_string(v) }).collect::<Vec<_>>().join(",")
        }
        Value::Object(_) => "[object Object]".into(),
        v => v.to_string(),
    }
}

/// The truth value of JavaScript.
pub fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().is_some_and(|f| f != 0.0),
        Value::String(s) => !s.is_empty(),
        _ => true,
    }
}

fn envelope(data: &Value) {
    println!("{}", pretty(&json!({ "success": true, "data": data, "error": null })));
}

/// The folder of `config.json` and `targets/`. It is `BEEPER_CLI_CONFIG_DIR` like upstream, else `cli` in the data
/// folder of beeper-rs: the upstream CLI owns `~/.beeper`, and two programs that write one file can lose data.
pub fn config_dir() -> PathBuf {
    std::env::var_os("BEEPER_CLI_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| beeper_core::platform::current_desktop_data_dir(None).join("cli"))
}

pub fn read_json(path: &std::path::Path) -> Res<Option<Map<String, Value>>> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map(Some).map_err(|e| CliError::bug(format!("{}: {e}", path.display())))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(CliError::bug(format!("{}: {e}", path.display()))),
    }
}

/// Writes `value` like upstream: 2-space JSON, a final newline, mode 600.
pub fn write_json(path: &std::path::Path, value: &Map<String, Value>) -> Res {
    let io = |e: std::io::Error| CliError::bug(format!("{}: {e}", path.display()));
    std::fs::create_dir_all(path.parent().expect("a config file has a folder")).map_err(io)?;
    let text = format!("{}\n", serde_json::to_string_pretty(value).expect("JSON values serialize"));
    std::fs::write(path, text).map_err(io)?;
    #[cfg(unix)]
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(io)?;
    Ok(())
}

pub fn config() -> Res<Map<String, Value>> {
    Ok(read_json(&config_dir().join("config.json"))?.unwrap_or_default())
}

fn resolve_target(name: Option<&str>) -> Res<Map<String, Value>> {
    let config = config()?;
    let env = std::env::var("BEEPER_TARGET").ok();
    let read = |id: &str| read_json(&config_dir().join("targets").join(format!("{id}.json")));
    let with_auth = |mut t: Map<String, Value>| {
        let shared = t.get("auth").is_none() && t["type"] == "desktop" && config.contains_key("auth");
        if shared && config.get("baseURL").is_none_or(|b| *b == t["baseURL"]) {
            t.insert("auth".into(), config["auth"].clone());
        }
        t
    };
    let builtin = || {
        let base = std::env::var("BEEPER_DESKTOP_BASE_URL").ok().filter(|b| !b.is_empty()).map(Value::from);
        let base = base.or_else(|| config.get("baseURL").cloned()).unwrap_or_else(|| json!("http://127.0.0.1:23373"));
        let auth = config.get("auth").cloned().unwrap_or(Value::Null);
        obj([
            ("id", json!("desktop")),
            ("type", json!("desktop")),
            ("name", json!("Beeper Desktop")),
            ("baseURL", base),
            ("auth", auth),
        ])
    };
    if let Some(id) = name
        .map(str::to_string)
        .or(env)
        .or_else(|| config.get("defaultTarget").and_then(|v| v.as_str()).map(str::to_string))
    {
        return match (read(&id)?, id == "desktop") {
            (Some(t), _) => Ok(with_auth(t)),
            (None, true) => Ok(builtin()),
            (None, false) => {
                Err(CliError::abort(format!("Unknown Beeper target \"{id}\". Run `{BIN} targets list`."), NOT_FOUND))
            }
        };
    }
    let mut all = list_targets()?;
    if all.len() == 1 {
        return Ok(with_auth(all.remove(0)));
    }
    Ok(read("desktop")?.map_or_else(builtin, with_auth))
}

pub fn list_targets() -> Res<Vec<Map<String, Value>>> {
    let dir = config_dir().join("targets");
    let mut names: Vec<PathBuf> =
        std::fs::read_dir(&dir).map_or(Vec::new(), |d| d.flatten().map(|e| e.path()).collect());
    names.retain(|p| p.extension().is_some_and(|e| e == "json"));
    let mut all: Vec<_> = names.iter().filter_map(|p| read_json(p).ok().flatten()).collect();
    all.sort_by(|a, b| js_string(&a["id"]).cmp(&js_string(&b["id"])));
    Ok(all)
}

/// `writeEvent`: one NDJSON lifecycle event on stderr.
pub fn write_event(event: &str, data: Value) {
    let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("a time after 1970").as_millis();
    eprintln!("{}", json!({ "event": event, "data": data, "ts": ts as u64 }));
}

/// Prints `err` like `BeeperCommand.catch`, and returns the exit code.
fn report(err: &CliError, json: bool, events: bool) -> i32 {
    let kind = if err.class.is_none() { "bug" } else { "abort" };
    let data = obj([
        ("message", json!(err.msg)),
        ("exitCode", json!(err.code)),
        ("kind", json!(kind)),
        ("tryMessage", json!(err.hint)),
    ]);
    if events {
        let ts =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).expect("a time after 1970").as_millis();
        eprintln!("{}", json!({ "event": "error", "data": data, "ts": ts as u64 }));
        return err.code;
    }
    if json {
        let mut line = Map::from_iter([("success".into(), json!(false)), ("data".into(), Value::Null)]);
        line.insert("error".into(), json!(err.msg));
        line.extend(data.into_iter().filter(|(k, _)| k != "message"));
        eprintln!("{}", Value::Object(line));
        return err.code;
    }
    match (err.class, &err.hint) {
        (None, _) => {
            let bar = "─".repeat(60);
            let stack: Vec<String> = format!("{}{}", err.stack, err.msg).lines().map(|l| format!("│   {l}")).collect();
            let version = beeper_core::config::APP_VERSION;
            eprint!(
                "\n┌─ unexpected error {}\n│ {}\n│\n{}\n│\n│ {BIN} {version} — please report at\n│   https://github.com/pythoncrazy/beeper-rs/issues\n└{bar}\n",
                &bar[20 * 3..],
                err.msg,
                stack.join("\n")
            );
            1
        }
        (Some(_), Some(hint)) => {
            eprintln!("{}\n  hint: {hint}", err.msg);
            err.code
        }
        (Some(class), None) => {
            // oclif's `prettyPrint`: wrapped at the width of stderr less 6, and indented by 4.
            let text = render::wrap_plain(&format!("{class}: {}", err.msg), render::columns(2) - 6);
            text.iter()
                .for_each(|line| eprintln!("{}", if line.is_empty() { String::new() } else { format!("    {line}") }));
            1
        }
    }
}

/// oclif's command-not-found `warn` and `error` for the leading `words` of the command line, wrapped at the width of
/// stderr less 6 after the bang, with the help of their `topic`. Returns oclif's exit code 127.
pub fn not_a_command(words: &str, topic: Option<&str>) -> i32 {
    let help = topic.map_or("help".to_string(), |t| format!("help {t}"));
    let warn = format!("Warning: {words} is not a {BIN} command.");
    let error = format!("Error: Run {BIN} {help} for a list of available commands.");
    for line in [warn, error].iter().flat_map(|t| render::wrap_hard(t, render::columns(2) - 6)) {
        eprintln!(" ›   {line}");
    }
    127
}

/// Runs the client command in `argv` (the words after the program name), and returns the exit code.
pub fn run(argv: &[String]) -> i32 {
    let help = argv.iter().any(|a| a == "--help" || a == "-h");
    let topic = argv.first().map(String::as_str).and_then(commands::topic);
    let index =
        topic.and_then(|t| t.2).map(|id| (commands::all().find(|c| c.id == id).expect("a topic index exists"), 1));
    // oclif's command lookup: a word after a command without arguments makes the words an unknown command.
    let found = find(argv).or(index);
    let found = found.filter(|(c, n)| help || !c.args.is_empty() || argv.get(*n).is_none_or(|a| a.starts_with('-')));
    let Some((cmd, n)) = found else {
        if let (Some(t), true) = (topic, help) {
            return (println!("{}", commands::topic_help(t)), 0).1;
        }
        let words = argv.iter().take_while(|a| !a.starts_with('-')).cloned().collect::<Vec<_>>().join(" ");
        return not_a_command(&words, topic.map(|t| t.0));
    };
    if help {
        let topic_index = topic.is_some_and(|t| t.2 == Some(cmd.id)) && n == 1 && argv[0] != cmd.id;
        let text = topic.filter(|_| topic_index).map_or_else(|| commands::help(cmd), commands::topic_help);
        return (println!("{text}"), 0).1;
    }
    let (json, events) = (argv.iter().any(|a| a == "--json"), argv.iter().any(|a| a == "--events"));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build the tokio runtime");
    let result = parse(cmd, &argv[n..]).and_then(|(vals, args)| {
        let mut ctx = Ctx { cmd, vals, args, rt, client: None, target: None };
        (cmd.run)(&mut ctx)
    });
    match result {
        Ok(()) => 0,
        Err(e) => report(&e, json, events),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn find_takes_the_longest_id_in_any_word_order() {
        assert_eq!(find(&words("status --json")).map(|(c, n)| (c.id, n)), Some(("status", 1)));
        assert_eq!(find(&words("get config defaultTarget")).map(|(c, n)| (c.id, n)), Some(("config get", 2)));
        assert_eq!(find(&words("api get /v1/info")).map(|(c, n)| (c.id, n)), Some(("api get", 2)));
        assert!(find(&words("chatz")).is_none());
    }

    #[test]
    fn the_command_table_is_well_formed() {
        let mut ids: Vec<&str> = commands::all().map(|c| c.id).collect();
        ids.sort_unstable();
        assert!(ids.windows(2).all(|w| w[0] != w[1]), "duplicate command ids: {ids:?}");
        for c in commands::all() {
            for f in c.flags {
                assert!(!(f.required && f.default.is_some()), "{} --{} is required and has a default", c.id, f.name);
                assert!(
                    GLOBAL.iter().all(|g| g.char.is_none() || g.char != f.char),
                    "{} -{:?} is a global char",
                    c.id,
                    f.char
                );
            }
            let required: Vec<bool> = c.args.iter().map(|a| a.required).collect();
            assert!(required.windows(2).all(|w| w[0] || !w[1]), "{}: a required arg follows an optional arg", c.id);
        }
    }

    #[test]
    fn parse_reports_errors_like_oclif() {
        let cmd = find(&words("api request")).unwrap().0;
        let err = |s: &str| parse(cmd, &words(s)).unwrap_err().msg;
        assert_eq!(err("GET /x --bogus"), "Nonexistent flag: --bogus\nSee more help with --help");
        assert_eq!(err("GET /x extra"), "Unexpected argument: extra\nSee more help with --help");
        assert_eq!(err("GET /x --body"), "Flag --body expects a value");
        assert_eq!(
            err("BREW /x"),
            "Expected BREW to be one of: GET, POST, PUT, PATCH, DELETE\nSee more help with --help"
        );
        assert!(err("GET").starts_with("Missing 1 required arg:\npath  API path, for example /v1/info\n"));
        let (vals, args) = parse(cmd, &words("POST /v1/x --body={} --no-json -t home")).unwrap();
        assert_eq!(args, ["POST", "/v1/x"]);
        assert_eq!(vals["json"], Val::Bool(false));
        assert_eq!(vals["target"], Val::Str("home".into()));
        assert_eq!(vals["body"], Val::Str("{}".into()));
    }
}
