use crate::config::{resolve_data_dir_for_os, resolve_original_data_dir_for_os};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OsKind {
    MacOs,
    Windows,
    Linux,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisplayServerKind {
    Wayland,
    X11,
    Quartz,
    Win32,
    Headless,
}

impl OsKind {
    pub fn current() -> Self {
        match std::env::consts::OS {
            "macos" | "ios" => Self::MacOs,
            "windows" => Self::Windows,
            "linux" | "freebsd" | "openbsd" | "netbsd" => Self::Linux,
            other => panic!("unsupported OS: {other}"),
        }
    }

    pub fn node_platform_str(&self) -> &'static str {
        match self {
            Self::MacOs => "darwin",
            Self::Windows => "win32",
            Self::Linux => "linux",
        }
    }

    pub fn detect_display_server(&self, env_vars: &HashMap<String, String>) -> DisplayServerKind {
        match self {
            Self::MacOs => DisplayServerKind::Quartz,
            Self::Windows => DisplayServerKind::Win32,
            Self::Linux if env_vars.contains_key("WAYLAND_DISPLAY") => DisplayServerKind::Wayland,
            Self::Linux if env_vars.contains_key("DISPLAY") => DisplayServerKind::X11,
            Self::Linux => DisplayServerKind::Headless,
        }
    }
}

pub fn resolve_home_dir(env_vars: &HashMap<String, String>) -> PathBuf {
    PathBuf::from(env_vars.get("HOME").or_else(|| env_vars.get("USERPROFILE")).expect("HOME or USERPROFILE is set"))
}

/// The MIME types of Desktop's `mime-types` package for the files of chats, with their extensions. The first
/// extension is the one that Desktop adds to a file name without one.
const MIME_TYPES: [(&str, &[&str]); 19] = [
    ("image/png", &["png"]),
    ("image/jpeg", &["jpeg", "jpg", "jpe"]),
    ("image/gif", &["gif"]),
    ("image/webp", &["webp"]),
    ("image/heic", &["heic"]),
    ("image/svg+xml", &["svg", "svgz"]),
    ("video/mp4", &["mp4", "mp4v", "mpg4", "m4v"]),
    ("video/quicktime", &["qt", "mov"]),
    ("video/webm", &["webm"]),
    ("audio/mpeg", &["mpga", "mp2", "mp2a", "mp3", "m2a", "m3a"]),
    ("audio/mp4", &["m4a", "mp4a"]),
    ("audio/ogg", &["oga", "ogg", "spx", "opus"]),
    ("audio/wav", &["wav"]),
    ("application/pdf", &["pdf"]),
    ("application/zip", &["zip"]),
    ("application/json", &["json", "map"]),
    ("text/plain", &["txt", "text", "conf", "def", "list", "log", "in", "ini"]),
    ("text/html", &["html", "htm", "shtml"]),
    ("text/csv", &["csv"]),
];

/// The MIME type of a file name by its extension, like `mime.lookup`.
pub fn mime_type_of(name: &str) -> Option<&'static str> {
    let ext = Path::new(name).extension()?.to_str()?.to_ascii_lowercase();
    MIME_TYPES.iter().find(|(_, exts)| exts.contains(&ext.as_str())).map(|(m, _)| *m)
}

/// Whether the text is a MIME type that Matrix media can carry.
pub fn is_mime(text: &str) -> bool {
    text.parse::<mime::Mime>().is_ok()
}

/// The file extension of a MIME type, like `mime.extension`.
pub fn extension_of(mime: &str) -> Option<&'static str> {
    MIME_TYPES.iter().find(|(m, _)| *m == mime).map(|(_, exts)| exts[0])
}

pub fn path_to_file_url(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.starts_with('/') {
        format!("file://{raw}")
    } else {
        format!("file:///{}", raw.replace('\\', "/"))
    }
}

pub fn file_url_to_path(url: &str) -> Option<PathBuf> {
    let stripped = url.strip_prefix("file://")?;
    let bytes = stripped.as_bytes();
    if bytes.len() >= 3 && bytes[0] == b'/' && bytes[1].is_ascii_alphabetic() && bytes[2] == b':' {
        return Some(PathBuf::from(&stripped[1..]));
    }
    Some(PathBuf::from(stripped))
}

pub fn open_url_or_path_command(os: OsKind, target: &str) -> (String, Vec<String>) {
    assert!(!target.trim().is_empty(), "target must not be empty");
    match os {
        OsKind::MacOs => ("open".to_string(), vec![target.to_string()]),
        OsKind::Linux => ("xdg-open".to_string(), vec![target.to_string()]),
        OsKind::Windows => {
            ("cmd".to_string(), vec!["/C".to_string(), "start".to_string(), String::new(), target.to_string()])
        }
    }
}

pub fn resolve_original_beeper_data_dir(
    os: OsKind,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    resolve_original_data_dir_for_os(os.node_platform_str(), profile, env_vars, home_dir)
}

pub fn current_original_beeper_data_dir(profile: Option<&str>) -> PathBuf {
    let env_map: HashMap<String, String> = std::env::vars().collect();
    resolve_original_beeper_data_dir(OsKind::current(), profile, &env_map, &resolve_home_dir(&env_map))
}

pub fn resolve_desktop_data_dir(
    os: OsKind,
    profile: Option<&str>,
    env_vars: &HashMap<String, String>,
    home_dir: &Path,
) -> PathBuf {
    resolve_data_dir_for_os(os.node_platform_str(), profile, env_vars, home_dir)
}

pub fn current_desktop_data_dir(profile: Option<&str>) -> PathBuf {
    let env_map: HashMap<String, String> = std::env::vars().collect();
    resolve_desktop_data_dir(OsKind::current(), profile, &env_map, &resolve_home_dir(&env_map))
}

/// Desktop's `requestSingleInstanceLock`, which Electron scopes to the data dir, so two profiles run side by side. Like
/// Desktop's lock, the socket is in `XDG_RUNTIME_DIR`. The first launch gets the listener. A later launch sends `args`
/// to the first launch and gets `None`. A lock file makes the check and the bind one step.
#[cfg(target_os = "linux")]
pub fn single_instance(data_dir: &Path, args: &[String]) -> Option<std::os::unix::net::UnixListener> {
    use std::io::ErrorKind::{ConnectionRefused, NotFound};
    let path = std::path::absolute(data_dir).expect("the data dir has an absolute path");
    let fnv = |h: u64, b: &u8| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3);
    let hash = path.as_os_str().as_encoded_bytes().iter().fold(0xcbf2_9ce4_8422_2325, fnv);
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map_or_else(std::env::temp_dir, PathBuf::from);
    let socket = dir.join(format!("beeper-rs-{hash:016x}"));
    let lock = std::fs::File::create(socket.with_extension("lock")).expect("create the single instance lock");
    lock.lock().expect("lock the single instance lock");
    match std::os::unix::net::UnixStream::connect(&socket) {
        Ok(stream) => {
            serde_json::to_writer(stream, args).expect("send the arguments to the running instance");
            None
        }
        Err(e) if matches!(e.kind(), NotFound | ConnectionRefused) => {
            if e.kind() == ConnectionRefused {
                std::fs::remove_file(&socket).expect("remove the socket of an instance that stopped");
            }
            Some(std::os::unix::net::UnixListener::bind(&socket).expect("bind the single instance socket"))
        }
        Err(e) => panic!("connect to {}: {e}", socket.display()),
    }
}

/// Desktop's `second-instance` event: sends the arguments of each later launch to `tx`.
#[cfg(target_os = "linux")]
pub fn forward_launches(first: std::os::unix::net::UnixListener, tx: tokio::sync::broadcast::Sender<Vec<String>>) {
    std::thread::spawn(move || {
        for stream in first.incoming() {
            match serde_json::from_reader(stream.expect("accept a later launch")) {
                Ok(args) => drop(tx.send(args)),
                Err(e) => eprintln!("read the arguments of a later launch: {e}"),
            }
        }
    });
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::single_instance;

    #[test]
    fn a_later_launch_hands_its_arguments_to_the_first() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/single-instance-test"));
        let first = single_instance(dir, &[]).expect("the first launch runs");
        let profile = single_instance(&dir.join("profile"), &[]).expect("a launch with another data dir runs");
        assert!(single_instance(dir, &["beeper://focus".into()]).is_none(), "a later launch stops");
        let args: Vec<String> = serde_json::from_reader(first.accept().expect("the later launch").0).unwrap();
        assert_eq!(args, ["beeper://focus"]);
        drop((first, profile));
        assert!(single_instance(dir, &[]).is_some(), "a launch after a stop replaces the old socket");
    }
}
