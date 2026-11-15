//! Desktop's `showOpenDialog`: the XDG desktop portal file chooser, and the files it gives.
use crate::{decode_media, PendingMedia};
use beeper_core::models::{FileRule, FileRules, FILE_AUDIO, FILE_FILE, FILE_GIF, FILE_IMAGE, FILE_VIDEO};
use std::path::{Path, PathBuf};

pub const PORTAL: &str = "org.freedesktop.portal.Desktop";

/// What Desktop's `showOpenDialog` and `showSaveDialog` pick.
#[derive(Debug, Clone, PartialEq)]
pub enum Pick {
    Files,
    /// One image, with Desktop's "Images" filter.
    Image,
    Folder,
    /// The path to save a file with this name.
    Save(String),
}

/// The paths that the user picks in the file chooser of the portal at `dest`, or none on cancel.
#[cfg(target_os = "linux")]
pub async fn open_files(dest: String, title: String, pick: Pick) -> Result<Vec<PathBuf>, String> {
    use iced::futures::StreamExt;
    use std::collections::HashMap;
    use zbus::zvariant::{OwnedValue, Value};
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let s = |e: zbus::Error| format!("Could not open the file chooser: {e}");
    let bus = crate::notify::bus().await?;
    let token = format!("beeper_rs_{}", NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
    let me = bus.unique_name().expect("a session bus connection has a name");
    let path =
        format!("/org/freedesktop/portal/desktop/request/{}/{token}", me.trim_start_matches(':').replace('.', "_"));
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface("org.freedesktop.portal.Request")
        .and_then(|r| r.member("Response"))
        .and_then(|r| r.path(path))
        .expect("a valid match rule")
        .build();
    let mut answers = zbus::MessageStream::for_match_rule(rule, bus, Some(1)).await.map_err(s)?;
    let mut options = HashMap::from([("handle_token", Value::from(token))]);
    let method = if matches!(pick, Pick::Save(_)) { "SaveFile" } else { "OpenFile" };
    match pick {
        Pick::Files => drop(options.insert("multiple", Value::from(true))),
        Pick::Image => {
            let globs = IMAGES.iter().map(|e| (0u32, format!("*.{e}"))).collect::<Vec<_>>();
            options.insert("filters", Value::from(vec![(crate::tr("Images").to_string(), globs)]));
        }
        Pick::Folder => drop(options.insert("directory", Value::from(true))),
        Pick::Save(name) => drop(options.insert("current_name", Value::from(name))),
    }
    let (obj, iface) = ("/org/freedesktop/portal/desktop", Some("org.freedesktop.portal.FileChooser"));
    bus.call_method(Some(dest.as_str()), obj, iface, method, &("", title, options)).await.map_err(s)?;
    let answer = answers.next().await.expect("the portal answers the request").map_err(s)?;
    let (code, results): (u32, HashMap<String, OwnedValue>) =
        answer.body().deserialize().expect("Response carries a code and the results");
    let uris = results.get("uris").map(|u| Vec::<String>::try_from(u.try_clone().expect("a string array")));
    picked(code, uris.transpose().expect("uris is a string array").unwrap_or_default())
}

/// Chromium's `navigator.onLine` from the portal's `NetworkMonitor`: whether the network is available, and then the
/// same at each change. Without the portal, the stream ends, and the app stays online.
#[cfg(target_os = "linux")]
pub fn online() -> impl iced::futures::Stream<Item = bool> {
    iced::stream::channel(4, async |mut out: iced::futures::channel::mpsc::Sender<bool>| {
        let Err(e) = watch_network(&mut out).await;
        eprintln!("network monitor: {e}");
    })
}

#[cfg(not(target_os = "linux"))]
pub fn online() -> iced::futures::stream::Empty<bool> {
    unreachable!("only the Linux build reads the network portal")
}

#[cfg(target_os = "linux")]
async fn watch_network(
    out: &mut iced::futures::channel::mpsc::Sender<bool>,
) -> Result<std::convert::Infallible, String> {
    use iced::futures::{SinkExt, StreamExt};
    let (obj, iface) = ("/org/freedesktop/portal/desktop", "org.freedesktop.portal.NetworkMonitor");
    let bus = crate::notify::bus().await?;
    let rule = zbus::MatchRule::builder().msg_type(zbus::message::Type::Signal).interface(iface);
    let rule = rule.and_then(|r| r.member("changed")).expect("a valid match rule").build();
    let mut changes = zbus::MessageStream::for_match_rule(rule, bus, None).await.map_err(|e| e.to_string())?;
    loop {
        let reply = bus.call_method(Some(PORTAL), obj, Some(iface), "GetAvailable", &()).await;
        let available = reply.map_err(|e| e.to_string())?.body().deserialize::<bool>();
        _ = out.send(available.expect("GetAvailable gives a boolean")).await;
        changes.next().await.ok_or("the session bus closed")?.map_err(|e| e.to_string())?;
    }
}

/// Desktop's image extensions that the `image` codecs of the app decode.
#[cfg(target_os = "linux")]
const IMAGES: [&str; 5] = ["jpg", "jpeg", "png", "gif", "webp"];

#[cfg(not(target_os = "linux"))]
pub async fn open_files(_dest: String, _title: String, _pick: Pick) -> Result<Vec<PathBuf>, String> {
    Err("The file chooser needs the Linux desktop portal".into())
}

/// The paths of a portal Response: code 0 is a choice, 1 a cancel and 2 an error.
#[cfg(target_os = "linux")]
fn picked(code: u32, uris: Vec<String>) -> Result<Vec<PathBuf>, String> {
    match code {
        0 => uris.iter().map(|u| file_path(u).ok_or(format!("The file chooser gave a bad file: {u}"))).collect(),
        1 => Ok(vec![]),
        _ => Err("The file chooser failed".into()),
    }
}

#[cfg(target_os = "linux")]
fn file_path(uri: &str) -> Option<PathBuf> {
    beeper_core::codec::url_decode(uri.strip_prefix("file://")?).map(PathBuf::from)
}

/// A file for the composer: an image has its size and frames, and an other file has its MIME type by its extension.
pub fn load_file(path: &Path) -> Result<PendingMedia, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Could not read {}: {e}", path.display()))?;
    let name = path.file_name().expect("a picked file has a name").to_string_lossy().into_owned();
    let reader = ::image::ImageReader::new(std::io::Cursor::new(&bytes)).with_guessed_format().expect("in memory");
    if let Some(format) = reader.format() {
        let size = reader.into_dimensions().map_err(|e| format!("Could not read {name}: {e}"))?;
        let frames = decode_media(bytes.clone())?;
        let mime = format.to_mime_type().to_string();
        return Ok(PendingMedia { is_gif: mime == "image/gif", bytes, name, mime, size, frames, duration: None });
    }
    let mime = beeper_core::platform::mime_type_of(&name).unwrap_or("application/octet-stream");
    // Desktop's `processAttachment` measures a video, and logs and attaches it without the size when it cannot.
    let measure = mime.starts_with("video/") || mime.starts_with("audio/");
    let ((w, h), duration) = match measure.then(|| probe(path)).transpose() {
        Ok(m) => m.unwrap_or_default(),
        Err(e) => {
            eprintln!("processAttachment: could not measure {name}, attaching without dimensions: {e}");
            Default::default()
        }
    };
    Ok(PendingMedia { bytes, name, mime: mime.into(), is_gif: false, size: (w, h), frames: vec![], duration })
}

/// Desktop's `getVideoMetadata`: the shown width and height of the video stream, and the length in seconds.
fn probe(path: &Path) -> Result<((u32, u32), Option<f64>), String> {
    let entries = "stream=codec_type,width,height:stream_side_data=rotation:format=duration";
    let out = std::process::Command::new("ffprobe")
        .args(["-v", "error", "-show_entries", entries, "-of", "json"])
        .arg(path)
        .output()
        .map_err(|e| format!("Could not start ffprobe: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("ffprobe writes JSON");
    let duration = v["format"]["duration"].as_str().and_then(|d| d.parse().ok());
    let streams = v["streams"].as_array().into_iter().flatten();
    let Some(s) = streams.into_iter().find(|s| s["codec_type"] == "video") else { return Ok(((0, 0), duration)) };
    let dim = |k: &str| s[k].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(0);
    let turned = s["side_data_list"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|d| d["rotation"].as_i64().is_some_and(|r| r % 180 != 0));
    let (w, h) = (dim("width"), dim("height"));
    Ok((if turned { (h, w) } else { (w, h) }, duration))
}

/// Desktop's `Oae`: the `com.beeper.room_features` file kind of a MIME type.
fn file_kind(mime: &str) -> &'static str {
    match mime.split('/').next() {
        _ if mime == "image/gif" => FILE_GIF,
        Some("image") => FILE_IMAGE,
        Some("audio") => FILE_AUDIO,
        Some("video") => FILE_VIDEO,
        _ => FILE_FILE,
    }
}

/// The levels of `mime` in the rule: by its type, then `type/*`, then `*/*`. Desktop's `SBe` takes the first, and
/// `Z1` the highest.
fn levels(mime: &str, rule: &FileRule) -> Vec<i8> {
    let top = format!("{}/*", mime.split('/').next().expect("split gives one part"));
    [mime, &top, "*/*"].iter().filter_map(|k| rule.mime_types.get(*k).copied()).collect()
}

/// Desktop's `Pae`: Desktop can convert the file to a type of the rule.
fn convertible(mime: &str, name: &str, rule: &FileRule) -> bool {
    let unlisted = rule.mime_types.get(mime).is_none_or(|&l| l == 0);
    let mp4 = rule.mime_types.get("video/mp4").is_some_and(|&l| l > 0);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_lowercase()).unwrap_or_default();
    match file_kind(mime) {
        FILE_IMAGE => unlisted,
        FILE_GIF => mp4,
        FILE_VIDEO => {
            let types = ["video/x-matroska", "video/webm", "video/x-msvideo", "video/quicktime", "video/x-flv"];
            let exts = ["mkv", "webm", "avi", "mov", "flv", "wmv", "3gp"];
            (types.contains(&mime) || exts.contains(&ext.as_str()) || unlisted) && mp4
        }
        _ => false,
    }
}

/// Desktop's `oP` for an audio file that the room does not take as it is: an audio type to convert to, or the room
/// takes it as a file.
fn audio_sendable(mime: &str, size: u64, rules: &FileRules) -> bool {
    // Desktop's `uBe` conversion targets, or any audio type.
    const TARGETS: [&str; 7] = ["audio/aac", "audio/mpeg", "audio/mp4", "audio/x-caf", "video/mp4", "audio/*", "*/*"];
    let rule = &rules[FILE_AUDIO];
    let target = rule.mime_types.iter().any(|(m, &l)| l > 0 && TARGETS.contains(&m.as_str()));
    let best = |rule| levels(mime, rule).into_iter().max();
    let as_file = rules.get(FILE_FILE).is_some_and(|f| best(f) > Some(0) && f.max_size.is_none_or(|max| size <= max));
    size <= 500 << 20 && (target || (best(rule) != Some(-2) && as_file))
}

/// Desktop's `TBe`: the toast text when the chat does not accept the file.
pub fn check(f: &PendingMedia, rules: &FileRules) -> Result<(), String> {
    let mime = f.mime.as_str();
    let top = mime.split('/').next().expect("split gives one part");
    let rule = rules.get(file_kind(mime)).ok_or("This chat doesn't support sending files")?;
    let size = f.bytes.len() as u64;
    let supported = match levels(mime, rule).first() {
        Some(&l) => l > 0,
        None => convertible(mime, &f.name, rule),
    };
    let converts = match top {
        "audio" => audio_sendable(mime, size, rules),
        _ => (top == "video" || mime == "image/gif") && convertible(mime, &f.name, rule),
    };
    if !supported && !converts {
        return Err(format!("This file type ({mime}) is not supported and cannot be converted to a supported format"));
    }
    if size == 0 {
        return Err("Attachment is empty".into());
    }
    if let Some(max) = rule.max_size.filter(|&max| size > max) {
        let (size, max) = (crate::human_size(size), crate::human_size(max));
        return Err(format!("File size ({size}) exceeds the maximum allowed size ({max})"));
    }
    let (w, h) = f.size;
    if w > 0 && matches!(top, "image" | "video") {
        if let Some(max) = rule.max_width.filter(|&max| w > max) {
            return Err(format!("Image width ({w}px) exceeds maximum allowed width ({max}px)"));
        }
        if let Some(max) = rule.max_height.filter(|&max| h > max) {
            return Err(format!("Image height ({h}px) exceeds maximum allowed height ({max}px)"));
        }
    }
    match (f.duration, rule.max_duration) {
        (Some(d), Some(max)) if d > max && matches!(top, "audio" | "video") => {
            Err(format!("Media duration ({}) exceeds maximum allowed duration ({})", duration(d), duration(max)))
        }
        _ => Ok(()),
    }
}

/// Desktop's `Pf`: seconds as "4.5s", "2m 5s" or "1h 2m 5s".
fn duration(s: f64) -> String {
    if s < 60.0 {
        return format!("{s:.1}s");
    }
    let (m, rest) = ((s / 60.0).floor() as u64, s % 60.0);
    match m < 60 {
        true => format!("{m}m {rest:.0}s"),
        false => format!("{}h {}m {rest:.0}s", m / 60, m % 60),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_os = "linux")]
    fn portal_answers_give_paths() {
        assert_eq!(picked(0, vec!["file:///tmp/a%20b.txt".into()]), Ok(vec![PathBuf::from("/tmp/a b.txt")]));
        assert_eq!(picked(1, vec![]), Ok(vec![]));
        assert!(picked(2, vec![]).is_err() && picked(0, vec!["https://a".into()]).is_err());
    }

    #[test]
    fn files_load_like_desktop() {
        let dir = std::env::temp_dir().join(format!("beeper-rs-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut png = Vec::new();
        ::image::RgbaImage::new(3, 2).write_to(&mut std::io::Cursor::new(&mut png), ::image::ImageFormat::Png).unwrap();
        std::fs::write(dir.join("a.bin"), &png).unwrap();
        std::fs::write(dir.join("b.PDF"), b"%PDF").unwrap();
        let img = load_file(&dir.join("a.bin")).unwrap();
        assert_eq!((img.mime.as_str(), img.size, img.frames.len()), ("image/png", (3, 2), 1), "images by content");
        let pdf = load_file(&dir.join("b.PDF")).unwrap();
        assert_eq!((pdf.name.as_str(), pdf.mime.as_str(), pdf.frames.len()), ("b.PDF", "application/pdf", 0));
        assert!(load_file(&dir.join("none")).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn attachments_follow_the_room_features_like_desktop() {
        let file = |name: &str, mime: &str, len: usize, size: (u32, u32), duration: Option<f64>| PendingMedia {
            bytes: vec![1; len],
            name: name.into(),
            mime: mime.into(),
            is_gif: false,
            size,
            frames: vec![],
            duration,
        };
        let beeper = beeper_core::models::beeper_files();
        let check = |f: &PendingMedia, rules: &FileRules| check(f, rules).err();
        assert_eq!(check(&file("a.png", "image/png", 9, (9, 9), None), &beeper), None);
        assert_eq!(check(&file("a.mkv", "video/x-matroska", 9, (9, 9), None), &beeper), None, "Desktop converts");
        assert_eq!(check(&file("a.png", "image/png", 0, (9, 9), None), &beeper).unwrap(), "Attachment is empty");
        let mut rules = beeper.clone();
        rules.remove(FILE_FILE);
        let pdf = file("a.pdf", "application/pdf", 2000, (0, 0), None);
        assert_eq!(check(&pdf, &rules).unwrap(), "This chat doesn't support sending files");
        let image = rules.get_mut(FILE_IMAGE).unwrap();
        (image.max_width, image.max_height) = (Some(8), Some(4));
        image.mime_types.insert("image/webp".into(), -2);
        let image = |name, mime| file(name, mime, 9, (8, 9), None);
        let height = "Image height (9px) exceeds maximum allowed height (4px)";
        assert_eq!(check(&image("a.png", "image/png"), &rules).unwrap(), height);
        let webp = "This file type (image/webp) is not supported and cannot be converted to a supported format";
        assert_eq!(check(&image("a.webp", "image/webp"), &rules).unwrap(), webp);
        assert_eq!(check(&image("a.bmp", "image/bmp"), &rules).unwrap(), height, "Desktop converts a BMP");
        rules.insert(FILE_FILE.into(), FileRule { max_size: Some(1000), ..beeper[FILE_FILE].clone() });
        let big = "File size (2 KB) exceeds the maximum allowed size (1000 B)";
        assert_eq!(check(&pdf, &rules).unwrap(), big);
        rules.get_mut(FILE_VIDEO).unwrap().max_duration = Some(10.0);
        let long = "Media duration (1m 15s) exceeds maximum allowed duration (10.0s)";
        assert_eq!(check(&file("a.mp4", "video/mp4", 9, (9, 9), Some(75.0)), &rules).unwrap(), long);
        rules.get_mut(FILE_AUDIO).unwrap().mime_types = [("audio/ogg".into(), 2)].into();
        assert_eq!(check(&file("a.flac", "audio/flac", 9, (0, 0), None), &rules), None, "a file takes it");
        rules.remove(FILE_FILE);
        let flac = "This file type (audio/flac) is not supported and cannot be converted to a supported format";
        assert_eq!(check(&file("a.flac", "audio/flac", 9, (0, 0), None), &rules).unwrap(), flac);
    }

    /// A portal on the session bus under a private name answers with a file.
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs a D-Bus session bus"]
    fn file_chooser_talks_to_the_portal() {
        use iced::futures::StreamExt;
        let name = format!("org.beeper.rs.TestPortal{}", std::process::id());
        use iced::futures::executor::block_on;
        let conn = block_on(async {
            let conn = zbus::Connection::session().await.unwrap();
            conn.request_name(name.as_str()).await.unwrap();
            conn
        });
        let mut calls = zbus::MessageStream::from(&conn);
        let serve = async {
            let call = loop {
                let m = calls.next().await.unwrap().unwrap();
                if m.header().member().is_some_and(|n| n == "OpenFile") {
                    break m;
                }
            };
            let (_, _, options): (String, String, std::collections::HashMap<String, zbus::zvariant::OwnedValue>) =
                call.body().deserialize().unwrap();
            let token = String::try_from(options["handle_token"].try_clone().unwrap()).unwrap();
            let sender = call.header().sender().unwrap().to_owned();
            let path = format!("/org/freedesktop/portal/desktop/request/{}/{token}", sender[1..].replace('.', "_"));
            let path = zbus::zvariant::ObjectPath::try_from(path).unwrap();
            conn.reply(&call.header(), &path).await.unwrap();
            let uris = zbus::zvariant::Value::from(vec!["file:///tmp/x.txt"]);
            let results = std::collections::HashMap::from([("uris", uris)]);
            let iface = "org.freedesktop.portal.Request";
            conn.emit_signal(Some(sender), &path, iface, "Response", &(0u32, results)).await.unwrap();
        };
        let (_, got) =
            block_on(iced::futures::future::join(serve, open_files(name.clone(), "Open".into(), Pick::Files)));
        assert_eq!(got, Ok(vec![PathBuf::from("/tmp/x.txt")]));
    }
}
