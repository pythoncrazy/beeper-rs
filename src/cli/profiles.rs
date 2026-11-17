//! `lib/profiles.js`: the local runtimes of managed targets. beeper-rs is both Beeper Desktop and Beeper Server, so a
//! managed target runs this program: as Desktop with Desktop's switches, or as Server with Beeper Server's flags and
//! `BEEPER_SERVER_DATA_DIR`, which make it serve the Desktop API headless. `BEEPER_SERVER_BIN` picks another Server.

use super::{config_dir, iso_in, js_string, obj, read_json, truthy, write_json, CliError, Res};
use serde_json::{json, Map, Value};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

type Target = Map<String, Value>;

fn id(t: &Target) -> String {
    js_string(&t["id"])
}

fn text<'a>(t: &'a Target, key: &str) -> Option<&'a str> {
    t.get(key).and_then(Value::as_str).filter(|v| !v.is_empty())
}

fn run_path(id: &str) -> PathBuf {
    config_dir().join("run").join("profiles").join(format!("{id}.json"))
}

pub fn log_path(id: &str) -> PathBuf {
    config_dir().join("logs").join("profiles").join(format!("{id}.log"))
}

pub fn error_log_path(id: &str) -> PathBuf {
    config_dir().join("logs").join("profiles").join(format!("{id}.err.log"))
}

fn io(path: &Path) -> impl Fn(std::io::Error) -> CliError + '_ {
    move |e| CliError::bug(format!("{}: {e}", path.display()))
}

fn remove(path: &Path) -> Res {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(io(path)(e)),
        _ => Ok(()),
    }
}

/// `assertProfile`.
fn assert_profile(t: &Target) -> Res {
    match t.get("managed").is_some_and(truthy) && text(t, "dataDir").is_some() {
        true => Ok(()),
        false => Err(CliError::bug(format!("Target \"{}\" is not a local profile.", id(t)))),
    }
}

/// `assertServerProfile`.
pub fn assert_server(t: &Target) -> Res {
    match assert_profile(t).is_ok() && t["type"] == "server" {
        true => Ok(()),
        false => Err(CliError::abort(format!("Target \"{}\" is not a local Beeper Server install.", id(t)), 2)),
    }
}

/// The path of the runtime of a managed target: this program, unless `BEEPER_SERVER_BIN` names a Server.
fn program(server: bool) -> Res<PathBuf> {
    let custom = std::env::var_os("BEEPER_SERVER_BIN").filter(|p| server && !p.is_empty());
    custom.map_or_else(|| std::env::current_exe().map_err(|e| CliError::bug(e.to_string())), |p| Ok(p.into()))
}

/// A command whose process runs on after the CLI exits, apart from the terminal's Ctrl+C (`detached: true`).
fn detached(program: &Path) -> Command {
    let mut command = Command::new(program);
    command.stdin(Stdio::null());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
}

/// `startProfile`.
pub fn start(t: &Target) -> Res<Value> {
    assert_profile(t)?;
    match t["type"] == "desktop" {
        true => launch_desktop(Some(t)),
        false => start_server(t),
    }
}

/// `launchDesktopApp`: Desktop with the port and the cloud of a managed target, in the target's data dir.
pub fn launch_desktop(t: Option<&Target>) -> Res<Value> {
    let mut command = detached(&program(false)?);
    command.arg("--no-enforce-app-location").stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(port) = t.and_then(|t| t.get("port")).filter(|p| truthy(p)) {
        command.arg(format!("--pas-port={}", js_string(port)));
    }
    if let Some(env) = t.and_then(|t| text(t, "serverEnv")) {
        command.arg(format!("--server-env={env}"));
    }
    if let Some((t, dir)) = t.and_then(|t| Some((t, text(t, "dataDir")?))) {
        let profile = text(t, "profile").map_or_else(|| id(t), str::to_string);
        command.envs([
            ("ALLOW_MULTIPLE_INSTANCES", "true"),
            ("BEEPER_PROFILE", &profile),
            ("BEEPER_USER_DATA_DIR", dir),
        ]);
    }
    command.spawn().map_err(|e| CliError::bug(e.to_string()))?;
    Ok(json!({ "id": t.map_or("desktop".into(), id), "startedAt": iso_in(0) }))
}

fn read_run(id: &str) -> Res<Option<Map<String, Value>>> {
    read_json(&run_path(id))
}

/// Beeper Server's flags of a managed target (`serverArgs`).
fn server_args(t: &Target) -> Vec<String> {
    let port = t.get("port").filter(|p| truthy(p)).map_or_else(
        || js_string(&t["baseURL"]).rsplit(':').next().unwrap_or_default().trim_end_matches('/').to_string(),
        js_string,
    );
    let mut args = vec!["--host=127.0.0.1".to_string(), format!("--port={port}")];
    args.push(format!("--data-dir={}", text(t, "dataDir").unwrap_or_default()));
    args.extend(text(t, "serverEnv").map(|e| format!("--server-env={e}")));
    args
}

/// `isReachable`: whether `/v1/info` of the target answers within a second.
fn reachable(t: &Target) -> bool {
    let Ok(client) = beeper_api::client::BeeperClient::try_new(&js_string(&t["baseURL"]), "") else { return false };
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("build the tokio runtime");
    let reply = rt
        .block_on(async { tokio::time::timeout(Duration::from_secs(1), client.fetch("GET", "/v1/info", None)).await });
    matches!(reply, Ok(Ok((200..=299, _))))
}

/// `startServerProfile`: a Server in the background with its logs in the CLI folder, ready within 15 seconds.
fn start_server(t: &Target) -> Res<Value> {
    let id = id(t);
    if let Some(run) = read_run(&id)? {
        if run.get("pid").and_then(Value::as_i64).is_some_and(running) && reachable(t) {
            return Ok(Value::Object(run));
        }
        remove(&run_path(&id))?;
    }
    if reachable(t) {
        let msg = format!("Profile \"{id}\" is already reachable at {}.", js_string(&t["baseURL"]));
        return Err(CliError::bug(msg));
    }
    let (log, error_log) = (log_path(&id), error_log_path(&id));
    let logs = log.parent().expect("a log has a folder");
    std::fs::create_dir_all(logs).map_err(io(logs))?;
    let append = |path: &Path| std::fs::File::options().create(true).append(true).open(path).map_err(io(path));
    let mut command = detached(&program(true)?);
    command.args(server_args(t)).stdout(append(&log)?).stderr(append(&error_log)?);
    command.env("BEEPER_SERVER_DATA_DIR", text(t, "dataDir").unwrap_or_default());
    let child = command.spawn().map_err(|e| CliError::bug(e.to_string()))?;
    let run = obj([
        ("id", json!(id)),
        ("pid", json!(child.id())),
        ("startedAt", json!(iso_in(0))),
        ("log", json!(log.display().to_string())),
        ("errorLog", json!(error_log.display().to_string())),
    ]);
    write_json(&run_path(&id), &run)?;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(15) {
        if reachable(t) {
            return Ok(Value::Object(run));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    remove(&run_path(&id))?;
    let base = js_string(&t["baseURL"]);
    Err(CliError::bug(format!("Profile \"{id}\" did not become ready at {base}. Check logs: {}", error_log.display())))
}

/// `stopProfile`: SIGTERM, then SIGKILL after 5 seconds.
pub fn stop(t: &Target) -> Res {
    assert_profile(t)?;
    if t["type"] == "desktop" {
        return Err(CliError::bug("Quit Beeper Desktop from the app."));
    }
    let id = id(t);
    let not_running = || CliError::bug(format!("Profile \"{id}\" is not running."));
    let run = read_run(&id)?.ok_or_else(not_running)?;
    let pid = run.get("pid").and_then(Value::as_i64).unwrap_or_default();
    if !running(pid) {
        remove(&run_path(&id))?;
        return Err(not_running());
    }
    signal(pid, false);
    if !wait_for_exit(pid, Duration::from_secs(5)) {
        signal(pid, true);
        wait_for_exit(pid, Duration::from_secs(2));
    }
    remove(&run_path(&id))
}

fn wait_for_exit(pid: i64, wait: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < wait {
        if !running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

/// `isRunning`: `process.kill(pid, 0)`.
#[cfg(unix)]
fn running(pid: i64) -> bool {
    libc::pid_t::try_from(pid).is_ok_and(|pid| pid > 0 && unsafe { libc::kill(pid, 0) } == 0)
}

#[cfg(unix)]
fn signal(pid: i64, kill: bool) {
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        unsafe { libc::kill(pid, if kill { libc::SIGKILL } else { libc::SIGTERM }) };
    }
}

#[cfg(not(unix))]
fn running(pid: i64) -> bool {
    let out = Command::new("tasklist").args(["/FI", &format!("PID eq {pid}"), "/NH"]).output();
    out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains(&pid.to_string()))
}

#[cfg(not(unix))]
fn signal(pid: i64, _kill: bool) {
    drop(Command::new("taskkill").args(["/F", "/PID", &pid.to_string()]).output());
}

/// The home folder of the services of the user (`process.env.HOME ?? beeperDir()`).
fn home() -> PathBuf {
    std::env::var_os("HOME").map_or_else(config_dir, PathBuf::from)
}

/// `execFileAsync`: runs `program`, and fails like Node's `execFile` when it fails.
fn exec(program: &str, args: &[&str]) -> Res {
    let out = Command::new(program).args(args).output();
    let command = format!("{program} {}", args.join(" "));
    match out {
        Ok(o) if o.status.success() => Ok(()),
        Ok(o) => Err(CliError::bug(format!("Command failed: {command}\n{}", String::from_utf8_lossy(&o.stderr)))),
        Err(e) => Err(CliError::bug(format!("spawn {program} {e}"))),
    }
}

fn unit_name(id: &str) -> String {
    format!("beeper-profile-{id}.service")
}

fn agent_label(id: &str) -> String {
    format!("com.beeper.cli.profile.{id}")
}

/// `systemdQuote`.
fn systemd_quote(v: &str) -> String {
    match v.contains(' ') {
        true => format!("\"{}\"", v.replace('"', "\\\"")),
        false => v.to_string(),
    }
}

/// `systemdUnit`: a user service that keeps the Server running.
fn systemd_unit(t: &Target, program: &Path) -> String {
    let id = id(t);
    let args: Vec<String> = std::iter::once(program.display().to_string()).chain(server_args(t)).collect();
    let exec = args.iter().map(|a| systemd_quote(a)).collect::<Vec<_>>().join(" ");
    let dir = systemd_quote(text(t, "dataDir").unwrap_or_default());
    let (log, error_log) = (log_path(&id), error_log_path(&id));
    format!(
        "[Unit]\nDescription=Beeper profile {id}\n\n[Service]\nExecStart={exec}\nRestart=always\nEnvironment=BEEPER_SERVER_DATA_DIR={dir}\nStandardOutput=append:{}\nStandardError=append:{}\n\n[Install]\nWantedBy=default.target\n",
        log.display(),
        error_log.display()
    )
}

/// `launchAgentPlist`.
fn launch_agent(t: &Target, program: &Path) -> String {
    let id = id(t);
    let xml = |v: &str| v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let args: Vec<String> = std::iter::once(program.display().to_string()).chain(server_args(t)).collect();
    let args: String = args.iter().map(|a| format!("<string>{}</string>", xml(a))).collect();
    let dir = xml(text(t, "dataDir").unwrap_or_default());
    let (log, error_log) = (xml(&log_path(&id).display().to_string()), xml(&error_log_path(&id).display().to_string()));
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict>\n<key>Label</key><string>{}</string>\n<key>ProgramArguments</key><array>{args}</array>\n<key>EnvironmentVariables</key><dict><key>BEEPER_SERVER_DATA_DIR</key><string>{dir}</string></dict>\n<key>RunAtLoad</key><true/>\n<key>KeepAlive</key><true/>\n<key>StandardOutPath</key><string>{log}</string>\n<key>StandardErrorPath</key><string>{error_log}</string>\n</dict></plist>\n",
        xml(&agent_label(&id))
    )
}

fn uid() -> String {
    #[cfg(unix)]
    return unsafe { libc::getuid() }.to_string();
    #[cfg(not(unix))]
    return "501".into();
}

fn write_service(path: &Path, text: &str) -> Res {
    let dir = path.parent().expect("a service file has a folder");
    std::fs::create_dir_all(dir).map_err(io(dir))?;
    std::fs::write(path, text).map_err(io(path))?;
    let logs = config_dir().join("logs").join("profiles");
    std::fs::create_dir_all(&logs).map_err(io(&logs))
}

/// `enableProfile`: a systemd user service on Linux, or a launch agent on macOS, that starts the Server at login.
/// Returns the path of the service file.
pub fn enable(t: &Target) -> Res<PathBuf> {
    assert_profile(t)?;
    if t["type"] != "server" {
        return Err(CliError::bug("Manage Desktop start at launch in Beeper Desktop."));
    }
    let (id, program) = (id(t), program(true)?);
    match std::env::consts::OS {
        "linux" => {
            let path = home().join(".config/systemd/user").join(unit_name(&id));
            write_service(&path, &systemd_unit(t, &program))?;
            exec("systemctl", &["--user", "daemon-reload"])?;
            exec("systemctl", &["--user", "enable", "--now", &unit_name(&id)])?;
            Ok(path)
        }
        "macos" => {
            let path = home().join("Library/LaunchAgents").join(format!("{}.plist", agent_label(&id)));
            write_service(&path, &launch_agent(t, &program))?;
            let (service, file) = (format!("gui/{}", uid()), path.display().to_string());
            drop(exec("launchctl", &["bootout", &service, &file]));
            exec("launchctl", &["bootstrap", &service, &file])?;
            exec("launchctl", &["enable", &format!("{service}/{}", agent_label(&id))])?;
            drop(exec("launchctl", &["kickstart", "-k", &format!("{service}/{}", agent_label(&id))]));
            Ok(path)
        }
        _ => Err(CliError::bug("Beeper Server is not available on Windows.")),
    }
}

/// `disableProfile`: stops the service and removes its file. Returns the path of the file.
pub fn disable(t: &Target) -> Res<PathBuf> {
    assert_profile(t)?;
    if t["type"] != "server" {
        return Err(CliError::bug("Manage Desktop start at launch in Beeper Desktop."));
    }
    let id = id(t);
    let path = match std::env::consts::OS {
        "linux" => {
            let path = home().join(".config/systemd/user").join(unit_name(&id));
            drop(exec("systemctl", &["--user", "disable", "--now", &unit_name(&id)]));
            drop(exec("systemctl", &["--user", "daemon-reload"]));
            path
        }
        "macos" => {
            let path = home().join("Library/LaunchAgents").join(format!("{}.plist", agent_label(&id)));
            let service = format!("gui/{}", uid());
            drop(exec("launchctl", &["bootout", &service, &path.display().to_string()]));
            drop(exec("launchctl", &["disable", &format!("{service}/{}", agent_label(&id))]));
            path
        }
        _ => return Err(CliError::bug("Beeper Server is not available on Windows.")),
    };
    remove(&path)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_service_runs_the_server_with_its_flags_and_data_dir() {
        let t: Target = serde_json::from_value(json!({
            "id": "home", "type": "server", "baseURL": "http://127.0.0.1:23999", "managed": true,
            "dataDir": "/data/my server", "port": 23999, "serverEnv": "staging"
        }))
        .unwrap();
        let args = ["--host=127.0.0.1", "--port=23999", "--data-dir=/data/my server", "--server-env=staging"];
        assert_eq!(server_args(&t), args);
        let unit = systemd_unit(&t, Path::new("/bin/beeper-rs"));
        assert!(unit.starts_with("[Unit]\nDescription=Beeper profile home\n\n[Service]\nExecStart=/bin/beeper-rs --host=127.0.0.1 --port=23999 \"--data-dir=/data/my server\" --server-env=staging\nRestart=always\nEnvironment=BEEPER_SERVER_DATA_DIR=\"/data/my server\"\n"), "{unit}");
        assert!(unit.ends_with("home.err.log\n\n[Install]\nWantedBy=default.target\n"), "{unit}");
        let agent = launch_agent(&t, Path::new("/bin/beeper-rs"));
        assert!(agent.contains("<key>Label</key><string>com.beeper.cli.profile.home</string>\n<key>ProgramArguments</key><array><string>/bin/beeper-rs</string><string>--host=127.0.0.1</string>"), "{agent}");
    }
}
