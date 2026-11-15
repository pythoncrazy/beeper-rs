//! Desktop's `showOpenDialog`: the XDG desktop portal file chooser, and the files it gives.
use crate::{decode_media, PendingMedia};
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
        return Ok(PendingMedia { is_gif: mime == "image/gif", bytes, name, mime, size, frames });
    }
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "ogg" | "opus" => "audio/ogg",
        "wav" => "audio/wav",
        "pdf" => "application/pdf",
        "txt" => "text/plain",
        _ => "application/octet-stream",
    };
    Ok(PendingMedia { bytes, name, mime: mime.into(), is_gif: false, size: (0, 0), frames: vec![] })
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
