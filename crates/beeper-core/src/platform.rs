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
const MIME_TYPES: [(&str, &[&str]); 24] = [
    ("image/png", &["png"]),
    ("image/jpeg", &["jpeg", "jpg", "jpe"]),
    ("image/gif", &["gif"]),
    ("image/webp", &["webp"]),
    ("image/heic", &["heic"]),
    ("image/svg+xml", &["svg", "svgz"]),
    ("video/mp4", &["mp4", "mp4v", "mpg4", "m4v"]),
    ("video/quicktime", &["qt", "mov"]),
    ("video/webm", &["webm"]),
    ("video/x-matroska", &["mkv", "mk3d", "mks"]),
    ("video/x-msvideo", &["avi"]),
    ("video/3gpp", &["3gp", "3gpp"]),
    ("audio/mpeg", &["mpga", "mp2", "mp2a", "mp3", "m2a", "m3a"]),
    ("audio/mp4", &["m4a", "mp4a"]),
    ("audio/ogg", &["oga", "ogg", "spx", "opus"]),
    ("audio/wav", &["wav"]),
    ("audio/aac", &["aac"]),
    ("audio/x-flac", &["flac"]),
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

/// Shuts down the TCP connections of this process to `peers`, like Go's `CloseIdleConnections` with the requests in
/// flight canceled: a request that waits on one of them fails at once, and its client connects again for the next.
/// The connections show in `/proc/self/net/tcp` and `tcp6` by the inode of their socket. Gives how many it shut down.
#[cfg(target_os = "linux")]
pub fn shutdown_tcp_to(peers: &std::collections::HashSet<std::net::SocketAddr>) -> usize {
    let inodes: std::collections::HashSet<u64> = ["tcp", "tcp6"]
        .iter()
        .filter_map(|t| std::fs::read_to_string(format!("/proc/self/net/{t}")).ok())
        .flat_map(|table| table.lines().skip(1).filter_map(tcp_row).collect::<Vec<_>>())
        .filter(|(remote, _)| peers.contains(remote))
        .map(|(_, inode)| inode)
        .collect();
    let fds = std::fs::read_dir("/proc/self/fd").into_iter().flatten().flatten();
    let socket = |fd: &std::fs::DirEntry| {
        let link = std::fs::read_link(fd.path()).ok()?;
        let inode = link.to_str()?.strip_prefix("socket:[")?.strip_suffix(']')?.parse().ok()?;
        inodes.contains(&inode).then(|| fd.file_name().to_str()?.parse::<i32>().ok()).flatten()
    };
    let fds: Vec<i32> = fds.filter_map(|fd| socket(&fd)).collect();
    // SAFETY: shutdown(2) only ends the connection; the descriptor stays open for its owner, which closes it.
    fds.iter().filter(|&&fd| unsafe { libc::shutdown(fd, libc::SHUT_RDWR) } == 0).count()
}

/// The remote address and the socket inode of a row of `/proc/net/tcp` or `tcp6`, whose addresses are the bytes of
/// the address in network order, printed as host-order 32-bit words, and the port in hex. An IPv4 address mapped to
/// IPv6 gives its IPv4 address.
#[cfg(target_os = "linux")]
fn tcp_row(row: &str) -> Option<(std::net::SocketAddr, u64)> {
    let cols: Vec<&str> = row.split_whitespace().collect();
    let (addr, port) = cols.get(2)?.split_once(':')?;
    let words = (0..addr.len() / 8).map(|i| u32::from_str_radix(&addr[i * 8..i * 8 + 8], 16).map(u32::to_le_bytes));
    let bytes: Vec<u8> = words.collect::<Result<Vec<_>, _>>().ok()?.concat();
    let ip = match <[u8; 16]>::try_from(bytes.as_slice()) {
        Ok(v6) => std::net::Ipv6Addr::from(v6).to_canonical(),
        Err(_) => std::net::Ipv4Addr::from(<[u8; 4]>::try_from(bytes.as_slice()).ok()?).into(),
    };
    Some((std::net::SocketAddr::new(ip, u16::from_str_radix(port, 16).ok()?), cols.get(9)?.parse().ok()?))
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::single_instance;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;

    #[test]
    fn a_later_launch_hands_its_arguments_to_the_first() {
        let dir = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/single-instance-test"));
        let first = single_instance(dir, &[]).expect("the first launch runs");
        let profile = single_instance(&dir.join("profile"), &[]).expect("a launch with another data dir runs");
        assert!(single_instance(dir, &["beeper://focus".into()]).is_none(), "a later launch stops");
        let args: Vec<String> = serde_json::from_reader(first.accept().expect("the later launch").0).unwrap();
        assert_eq!(args, ["beeper://focus"]);
        // A stop closes the socket in all processes. A child that another test starts has a copy of the socket until
        // it runs its program, so a drop does not stop the socket. A shutdown does.
        UnixStream::from(OwnedFd::from(first)).shutdown(std::net::Shutdown::Both).expect("stop the socket");
        drop(profile);
        assert!(single_instance(dir, &[]).is_some(), "a launch after a stop replaces the old socket");
    }

    /// A connection to a peer of the list ends at once for both ends, also while its reader waits; one to another
    /// peer stays.
    #[test]
    fn the_connections_to_the_peers_shut_down() {
        use std::io::{Read, Write};
        let (reset, other) =
            (std::net::TcpListener::bind("127.0.0.1:0").unwrap(), std::net::TcpListener::bind("[::1]:0"));
        let mut client = std::net::TcpStream::connect(reset.local_addr().unwrap()).unwrap();
        let mut server = reset.accept().unwrap().0;
        let other = other.expect("an IPv6 loopback");
        let mut kept = std::net::TcpStream::connect(other.local_addr().unwrap()).unwrap();
        let waiting = std::thread::spawn(move || client.read(&mut [0; 8]).map_err(|e| e.kind()));
        std::thread::sleep(std::time::Duration::from_millis(50));
        let peers = std::collections::HashSet::from([reset.local_addr().unwrap()]);
        assert_eq!(super::shutdown_tcp_to(&peers), 1);
        assert_eq!(waiting.join().unwrap(), Ok(0), "the waiting read ends");
        assert_eq!(server.read(&mut [0; 8]).unwrap(), 0, "the peer sees the end");
        kept.write_all(b"ping").unwrap();
        let mut buf = [0; 4];
        other.accept().unwrap().0.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"ping");
    }

    #[test]
    fn tcp_rows_give_the_remote_address_and_the_inode() {
        let v4 = "   3: 0100007F:A1B2 0100007F:5CF3 01 00000000:00000000 00:00000000 00000000  1000        0 4242 1 0";
        let mapped =
            "   0: 0000000000000000FFFF00000100007F:A1B2 0000000000000000FFFF00000200A8C0:01BB 01 00000000:00000000 \
                      00:00000000 00000000  1000        0 77 1 0";
        let v6 =
            "   1: 00000000000000000000000001000000:A1B2 B80D0120000000000000000001000000:01BB 01 0:0 0:0 0 1000 0 9";
        assert_eq!(super::tcp_row(v4), Some(("127.0.0.1:23795".parse().unwrap(), 4242)));
        assert_eq!(super::tcp_row(mapped), Some(("192.168.0.2:443".parse().unwrap(), 77)));
        assert_eq!(super::tcp_row(v6), Some(("[2001:db8::1]:443".parse().unwrap(), 9)));
        assert_eq!(super::tcp_row("  sl  local_address rem_address   st tx_queue rx_queue"), None);
    }
}
