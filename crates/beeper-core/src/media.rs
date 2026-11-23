//! Desktop's `prepareOutgoingMedia`: before the upload to a room with room features, ffmpeg converts videos and audio
//! files, and the `image` codecs resize or compress images, so that the room takes them.
use crate::models::{
    FileRule, FileRules, FILE_AUDIO, FILE_FILE, FILE_GIF, FILE_IMAGE, FILE_STICKER, FILE_VIDEO, FILE_VOICE,
};
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

/// Desktop's absolute upload limit `Tn`.
const ABSOLUTE_MAX: u64 = 500 * 1024 * 1024;
/// Desktop's `Xn.Rejected` level of a MIME type.
const REJECTED: i8 = -2;

/// The facts of a file that Desktop's `checkMediaCompatibility` (`Cc`) reads.
#[derive(Debug, Clone, PartialEq)]
pub struct Facts {
    pub mime: String,
    pub size: u64,
    pub dims: Option<(u32, u32)>,
    /// The length in seconds.
    pub duration: Option<f64>,
    pub sent: Sent,
}

/// Desktop's `isVoiceNote` and `isSticker`: a file that goes as a voice note or a sticker, and not by its MIME type.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Sent {
    File,
    Voice,
    Sticker,
}

/// What Desktop's `Cc` does with a file in a room.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    SendAsIs,
    /// The width and height, or the length, are necessary for a limit of the room.
    MissingInfo,
    Reject(String),
    /// Desktop's `RESIZE_ONLY`: the largest width and height.
    Resize(u32, u32),
    /// Desktop's `COMPRESS_ONLY`.
    Compress,
    /// The MIME type to convert to, the strong compression, and the largest width and height.
    Transcode {
        mime: &'static str,
        strong: bool,
        dims: Option<(u32, u32)>,
    },
}

impl Action {
    fn name(&self) -> &'static str {
        match self {
            Action::SendAsIs => "SEND_AS_IS",
            Action::MissingInfo => "MISSING_INFO",
            Action::Reject(_) => "REJECT",
            Action::Resize(..) => "RESIZE_ONLY",
            Action::Compress => "COMPRESS_ONLY",
            Action::Transcode { .. } => "TRANSCODE",
        }
    }
}

/// Desktop's `Ji`: the known top type of a MIME type.
fn top(mime: &str) -> Option<&'static str> {
    let t = mime.split_once('/')?.0;
    ["image", "audio", "video", "text", "font", "model", "application"].into_iter().find(|k| k.eq_ignore_ascii_case(t))
}

/// The lower-case MIME type without its parameters.
fn base(mime: &str) -> String {
    mime.to_lowercase().split(';').next().expect("split gives one part").to_string()
}

/// Desktop's `Ds`: the highest level of the MIME type, its `type/*` and `*/*` in the rule.
fn best(mime: &str, rule: Option<&FileRule>) -> Option<i8> {
    let wild = top(mime).map(|t| format!("{t}/*"));
    let keys = [Some(mime), wild.as_deref(), Some("*/*")];
    keys.into_iter().flatten().filter_map(|k| rule?.mime_types.get(k).copied()).max()
}

/// Desktop's `Yi`: the first level of the MIME type, the type without parameters, `type/*` or `*/*` is `min` or more.
fn supports(mime: &str, types: &BTreeMap<String, i8>, min: i8) -> bool {
    let wild = format!("{}/*", mime.split('/').next().expect("split gives one part"));
    let short = mime.split(';').next().expect("split gives one part").trim();
    let level = [mime, short, &wild, "*/*"].into_iter().find_map(|k| types.get(k).copied());
    level.is_some_and(|l| l >= min)
}

/// Desktop's `Px`: the animated sticker types.
pub fn animated(mime: &str) -> bool {
    matches!(mime, "video/lottie+json" | "application/x-tgsticker" | "image/tgs")
}

/// Desktop's `nw`: the room takes `mime` as a sticker. An animated sticker needs full support.
pub fn takes_sticker(rules: &FileRules, mime: &str) -> bool {
    let (mime, rule) = (base(mime), rules.get(FILE_STICKER));
    rule.is_some_and(|r| r.mime_types.is_empty() || supports(&mime, &r.mime_types, if animated(&mime) { 2 } else { 1 }))
}

/// Desktop's `cw`: the first conversion target of the top type that the room lists.
fn target(mime: &str, listed: &[&str]) -> Option<&'static str> {
    let t = top(mime)?;
    let targets: &[&'static str] = match t {
        "image" => &["image/jpeg", "image/png", "image/webp"],
        "video" => &["video/mp4"],
        "audio" => &["audio/aac", "audio/mpeg", "audio/mp4", "audio/x-caf", "video/mp4"],
        _ => return None,
    };
    let wild = format!("{t}/*");
    targets.iter().copied().find(|a| listed.iter().any(|r| r == a || *r == "*/*" || *r == wild))
}

/// Desktop's `rP`: whether the chat takes text as the caption of a file, which a GIF asks of images and a voice note
/// of audio when its own kind does not say. Otherwise the text goes as a message of its own before the file.
pub fn takes_caption(rules: &FileRules, mime: &str, gif: bool, voice: bool) -> bool {
    let base = base(mime);
    let kind = match top(&base) {
        _ if voice => FILE_VOICE,
        _ if gif || base == "image/gif" => FILE_GIF,
        Some("image") => FILE_IMAGE,
        Some("audio") => FILE_AUDIO,
        Some("video") => FILE_VIDEO,
        _ => FILE_FILE,
    };
    let fallback = match kind {
        FILE_GIF => Some(FILE_IMAGE),
        FILE_VOICE => Some(FILE_AUDIO),
        _ => None,
    };
    let level = rules.get(kind).and_then(|r| r.caption).or_else(|| rules.get(fallback?)?.caption);
    level.unwrap_or(0) > 0
}

/// Desktop's `Cc` without the Data Saving Mode, which cloud uploads do not use.
pub fn decide(f: &Facts, rules: &FileRules) -> Action {
    if f.size > ABSOLUTE_MAX {
        return Action::Reject(format!("File size ({}) exceeds absolute limit ({ABSOLUTE_MAX})", f.size));
    }
    let base = base(&f.mime);
    let kind_top = top(&base);
    if rules.is_empty() {
        return Action::SendAsIs;
    }
    let mut kind = match kind_top {
        _ if f.sent == Sent::Sticker => FILE_STICKER,
        _ if f.sent == Sent::Voice => FILE_VOICE,
        _ if base == "image/gif" => FILE_GIF,
        Some("image") => FILE_IMAGE,
        Some("audio") => FILE_AUDIO,
        Some("video") => FILE_VIDEO,
        _ => FILE_FILE,
    };
    let fallback = |kind| match kind {
        FILE_GIF | FILE_STICKER => FILE_IMAGE,
        FILE_VOICE => FILE_AUDIO,
        _ => kind,
    };
    let file = rules.get(FILE_FILE);
    let mut rule = rules.get(kind);
    if rule.is_none() && kind != FILE_FILE {
        let Some(r) = rules.get(fallback(kind)) else {
            return Action::Reject(format!("Capability type {kind} not listed in room features"));
        };
        (kind, rule) = (fallback(kind), Some(r));
    }
    let too_big = rule.and_then(|r| r.max_size).or(file.and_then(|r| r.max_size)).is_some_and(|m| f.size > m);
    let (mut resize, mut dims) = (false, f.dims);
    let (max_w, max_h) = (rule.and_then(|r| r.max_width), rule.and_then(|r| r.max_height));
    if (matches!(kind_top, Some("image" | "video")) || kind == FILE_GIF) && (max_w.is_some() || max_h.is_some()) {
        let Some((w0, h0)) = f.dims else { return Action::MissingInfo };
        let scale = |n: u32, to: u32, from: u32| (f64::from(n) * f64::from(to) / f64::from(from)).round() as u32;
        let (mut w, mut h) = (w0, h0);
        if let Some(m) = max_w.filter(|&m| w > m) {
            (resize, h, w) = (true, scale(h, m, w), m);
        }
        if let Some(m) = max_h.filter(|&m| h > m) {
            let scaled = scale(w0, m, h);
            (resize, h) = (true, m);
            if max_w.is_none() || w == scaled {
                w = scaled;
            }
        }
        dims = Some((w, h));
    }
    let mut long = false;
    if matches!(kind_top, Some("audio" | "video")) || kind == FILE_VOICE {
        if let Some(max) = rule.and_then(|r| r.max_duration) {
            let Some(d) = f.duration else { return Action::MissingInfo };
            long = d > max;
        }
    }
    let over = resize || long || too_big;
    let checked = if matches!(kind, FILE_VOICE | FILE_AUDIO) && f.mime.contains(';') { &f.mime } else { &base };
    let mut level = best(checked, rule);
    if level.is_none_or(|l| l < 1) && matches!(kind, FILE_VOICE | FILE_GIF) {
        let m = if kind == FILE_VOICE { checked } else { &base };
        if let Some(l) = best(m, rules.get(fallback(kind))).filter(|&l| l >= 1) {
            (level, kind) = (Some(l), fallback(kind));
        }
    }
    if level.is_some_and(|l| l >= 1) {
        return match (over, dims) {
            (false, _) => Action::SendAsIs,
            (true, Some((w, h))) if resize => Action::Resize(w, h),
            (true, _) => Action::Compress,
        };
    }
    let listed: Vec<&str> =
        rule.into_iter().flat_map(|r| &r.mime_types).filter(|(_, &l)| l >= 1).map(|(m, _)| m.as_str()).collect();
    let gif_mp4 = base == "image/gif" && kind == FILE_GIF && best("video/mp4", rules.get(FILE_GIF)) >= Some(1);
    if let Some(mime) = if gif_mp4 { Some("video/mp4") } else { target(&base, &listed) } {
        return Action::Transcode { mime, strong: over, dims: dims.filter(|_| resize) };
    }
    if level == Some(REJECTED) {
        let why = "explicitly rejected and cannot transcode to supported format for";
        return Action::Reject(format!("MIME type {checked} {why} {kind}"));
    }
    if best(&base, file) >= Some(1) {
        return match file.and_then(|r| r.max_size).filter(|&m| f.size > m) {
            Some(m) => {
                Action::Reject(format!("Cannot transcode {base} for {kind}, and exceeds m.file size limit ({m})"))
            }
            None => Action::SendAsIs,
        };
    }
    let why = "no suitable transcoding target found, and fallback to m.file not possible/supported.";
    Action::Reject(format!("MIME type {base} is not supported for {kind}, {why}"))
}

/// Desktop's `pe` media classes that `prepareOutgoingMedia` converts.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    Video,
    Audio,
    Image,
}

/// Desktop's `dr`: the class by the MIME type, then by the extension.
fn class(name: &str, mime: &str) -> Option<Class> {
    let m = base(mime);
    let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let has = |list: &[&str]| list.contains(&ext.as_str());
    match () {
        _ if m.starts_with("video/") => Some(Class::Video),
        _ if m.starts_with("audio/") => Some(Class::Audio),
        _ if m.starts_with("image/") => Some(Class::Image),
        _ if has(&["mp4", "mov", "avi", "mkv", "webm", "flv", "wmv", "3gp", "m4v"]) => Some(Class::Video),
        _ if has(&["mp3", "aac", "wav", "ogg", "flac", "m4a", "opus", "caf"]) => Some(Class::Audio),
        _ if has(&["jpg", "jpeg", "png", "gif", "webp", "bmp", "tiff", "heic", "heif"]) => Some(Class::Image),
        _ => None,
    }
}

/// A file to upload, before and after Desktop's `prepareOutgoingMedia`.
#[derive(Debug, Clone, PartialEq)]
pub struct Upload {
    pub name: String,
    pub path: PathBuf,
    pub mime: String,
    pub dims: Option<(u32, u32)>,
    /// The length in seconds.
    pub duration: Option<f64>,
    /// Desktop's `conversionFailed`: the room does not take the file as it is, and the conversion failed.
    pub failed: bool,
}

/// Desktop's `uploadMediaForCloudRoom` before the upload: converts the file in the new folder `dir` for the room,
/// measures what is missing, and checks the result. It gives the bytes and the facts to upload, and removes `dir`.
pub fn prepare_upload(
    dir: &Path, bytes: &[u8], f: Upload, rules: &FileRules, sent: Sent,
) -> Result<(Vec<u8>, Upload), String> {
    let io = |e: std::io::Error| e.to_string();
    let path = dir.join("original").join(safe(&f.name));
    std::fs::create_dir_all(path.parent().expect("a parent folder")).map_err(io)?;
    std::fs::write(&path, bytes).map_err(io)?;
    let r = prepare(Upload { path, ..f }, rules, sent, dir).and_then(|u| {
        let u = backfill(u);
        let bytes = std::fs::read(&u.path).map_err(io)?;
        enforce(&u, bytes.len() as u64, rules, sent)?;
        Ok((bytes, u))
    });
    if let Err(e) = std::fs::remove_dir_all(dir) {
        eprintln!("Failed to clean up outgoing upload directory {}: {e}", dir.display());
    }
    r
}

/// Desktop's `_p`: the file as it is, or converted in `dir` like the room features ask.
fn prepare(f: Upload, rules: &FileRules, sent: Sent, dir: &Path) -> Result<Upload, String> {
    let mut f = Upload { dims: f.dims.filter(|&(w, h)| w > 0 && h > 0), ..f };
    let Some(class) = class(&f.name, &f.mime).filter(|_| base(&f.mime) != "image/gif") else { return Ok(f) };
    let size = std::fs::metadata(&f.path).map_err(|e| e.to_string())?.len();
    let facts = |f: &Upload| Facts { mime: f.mime.clone(), size, dims: f.dims, duration: f.duration, sent };
    let mut action = decide(&facts(&f), rules);
    if action == Action::MissingInfo {
        match measure(&f.path, class) {
            Ok((dims, duration)) => (f.dims, f.duration) = (dims.or(f.dims), duration.or(f.duration)),
            Err(e) => {
                eprintln!("prepareOutgoingMedia: Failed to get required media metadata: {e}");
                return Ok(Upload { failed: true, ..f });
            }
        }
        action = decide(&facts(&f), rules);
    }
    let (max, strong, format) = match &action {
        Action::SendAsIs | Action::MissingInfo => return Ok(f),
        Action::Reject(why) => {
            eprintln!("prepareOutgoingMedia: {} is not sendable here ({why})", f.name);
            return Ok(f);
        }
        Action::Resize(w, h) => (Some(*w.max(h)), false, None),
        Action::Compress => (None, false, None),
        Action::Transcode { mime, strong, dims } => (dims.map(|(w, h)| w.max(h)), *strong, mime.split('/').nth(1)),
    };
    let stem = Path::new(&f.name).file_stem().map_or(String::new(), |s| s.to_string_lossy().into_owned());
    let out = match class {
        Class::Video => video(&f.path, &stem, dir, strong, max, rules),
        Class::Audio => audio(&f.path, &stem, dir, audio_format(&action, rules)),
        Class::Image => image(&f.path, &stem, dir, strong, max, format, rules),
    };
    match out {
        Ok(u) => Ok(u),
        Err(e) if class == Class::Image && max.is_some() => Err(format!("Failed to resize image: {e}")),
        Err(e) => {
            eprintln!("prepareOutgoingMedia: {} processing failed, sending original: {e}", f.name);
            Ok(Upload { failed: true, ..f })
        }
    }
}

/// Desktop's `bp`: measures the width, height or length that the file does not have.
fn backfill(f: Upload) -> Upload {
    let need = match class(&f.name, &f.mime) {
        Some(Class::Video) => f.dims.is_none() || f.duration.is_none(),
        Some(Class::Audio) => f.duration.is_none(),
        Some(Class::Image) => f.dims.is_none(),
        None => false,
    };
    match need.then(|| measure(&f.path, class(&f.name, &f.mime).expect("a class"))) {
        Some(Ok((dims, duration))) => Upload { dims: f.dims.or(dims), duration: f.duration.or(duration), ..f },
        Some(Err(e)) => {
            eprintln!("backfillMediaMetadata: could not measure {}: {e}", f.name);
            f
        }
        None => f,
    }
}

/// Desktop's `vp`: the error of a file that the room still does not take after the conversion.
fn enforce(f: &Upload, size: u64, rules: &FileRules, sent: Sent) -> Result<(), String> {
    let action = decide(&Facts { mime: f.mime.clone(), size, dims: f.dims, duration: f.duration, sent }, rules);
    match action {
        Action::SendAsIs => Ok(()),
        Action::MissingInfo if !f.failed => Ok(()),
        _ if f.failed => Err(format!("Couldn't convert {} to a format this chat accepts", f.name)),
        Action::Reject(why) => Err(format!("Can't send {} to this chat: {why}", f.name)),
        a => Err(format!("Can't send {} to this chat: {} still needs {} after processing", f.name, f.mime, a.name())),
    }
}

/// The width and height, and the length in seconds, of a file.
type Measured = (Option<(u32, u32)>, Option<f64>);

/// Desktop's `Qf`: the width and height of a video or an image, and the length of a video or an audio file.
fn measure(path: &Path, class: Class) -> Result<Measured, String> {
    match class {
        Class::Video => video_meta(path).map(|m| (Some(m.display), Some(m.duration))),
        Class::Audio => audio_duration(path).map(|d| (None, Some(d))),
        Class::Image => image_size(path).map(|d| (Some(d), None)),
    }
}

/// Desktop's `Jf`: the size that fits in a square of `max`, with the same ratio.
fn fit((w, h): (u32, u32), max: u32) -> (u32, u32) {
    let scale = |n: u32, of: u32| (f64::from(n) * f64::from(max) / f64::from(of)).round() as u32;
    match () {
        _ if w <= max && h <= max => (w, h),
        _ if w >= h => (max, scale(h, w)),
        _ => (scale(w, h), max),
    }
}

/// A file name without the characters that Desktop replaces in output paths.
fn safe(name: &str) -> String {
    name.replace(['/', '\\', ':'], "_")
}

fn run(program: &str, args: &[&std::ffi::OsStr]) -> Result<Vec<u8>, String> {
    let out = std::process::Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    match out.status.success() {
        true => Ok(out.stdout),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

fn ffprobe(args: &[&str], path: &Path) -> Result<Value, String> {
    let mut all: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).collect();
    all.push(path.as_os_str());
    Ok(serde_json::from_slice(&run("ffprobe", &all)?).expect("ffprobe writes JSON"))
}

/// A number that ffprobe writes as a JSON string.
fn number(v: &Value) -> Option<f64> {
    v.as_str().and_then(|s| s.parse().ok())
}

/// Desktop's `getDecodableCodecs`: the codecs of `ffmpeg -decoders`, read once. A failed read gives no codecs, and the
/// next call reads again.
fn decoders() -> HashSet<String> {
    static DECODERS: std::sync::Mutex<Option<HashSet<String>>> = std::sync::Mutex::new(None);
    let mut known = DECODERS.lock().expect("the decoder list");
    if known.is_none() {
        let args = ["-hide_banner", "-decoders"].map(std::ffi::OsStr::new);
        let out = match run("ffmpeg", &args) {
            Ok(out) => String::from_utf8_lossy(&out).into_owned(),
            Err(e) => {
                eprintln!("getDecodableCodecs: could not read decoders from ffmpeg: {e}");
                return HashSet::new();
            }
        };
        // The list starts after the line of dashes, and the second word of each line is the codec.
        let dashes = |l: &&str| !l.trim().is_empty() && l.trim().chars().all(|c| c == '-');
        let lines = out.lines().skip_while(|l| !dashes(l)).skip(1);
        *known = Some(lines.filter_map(|l| l.split_whitespace().nth(1)).map(str::to_string).collect());
    }
    known.clone().expect("the decoders were read")
}

/// Desktop's `canDecodeMedia`: ffmpeg can decode each video and audio stream of the file. Without the decoder list
/// Desktop does not block the file, and a file that ffprobe cannot read is not decodable.
pub fn can_decode(path: &Path) -> bool {
    let decoders = decoders();
    if decoders.is_empty() {
        return true;
    }
    let v = match ffprobe(&["-v", "error", "-show_entries", "stream=codec_name,codec_type", "-of", "json"], path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("canDecodeMedia: ffprobe could not read {}: {e}", path.display());
            return false;
        }
    };
    let streams = v["streams"].as_array().into_iter().flatten();
    let media = streams.filter(|s| matches!(s["codec_type"].as_str(), Some("video" | "audio")));
    media.into_iter().all(|s| s["codec_name"].as_str().is_none_or(|c| decoders.contains(c)))
}

/// Desktop's `getVideoMetadata` result.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoMeta {
    /// The width and height of the first video stream.
    pub size: (u32, u32),
    /// The width and height that the video shows with its rotation.
    pub display: (u32, u32),
    /// The length in seconds.
    pub duration: f64,
    pub rotation: i64,
    /// The MIME type by the extension.
    pub mime: &'static str,
}

/// Desktop's `getVideoMetadata` with ffprobe. A file without a video stream has the size 0 by 0.
pub fn video_meta(path: &Path) -> Result<VideoMeta, String> {
    let v = ffprobe(&["-v", "error", "-select_streams", "v:0", "-show_streams", "-show_format", "-of", "json"], path)?;
    let s = &v["streams"][0];
    let duration = number(&s["duration"]).or(number(&v["format"]["duration"])).unwrap_or(0.0);
    let side = s["side_data_list"].as_array().into_iter().flatten();
    let rotation = side.filter_map(|d| d["rotation"].as_i64()).find(|&r| r != 0).unwrap_or(0);
    let dim = |k: &str| s[k].as_u64().map_or(0, |n| u32::try_from(n).expect("a video size fits in u32"));
    let size = (dim("width"), dim("height"));
    let display = if matches!(rotation.abs(), 90 | 270) { (size.1, size.0) } else { size };
    let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mime = match ext.as_str() {
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        "avi" => "video/x-msvideo",
        "mkv" => "video/x-matroska",
        _ => "video/mp4",
    };
    Ok(VideoMeta { size, display, duration, rotation, mime })
}

/// Desktop's `getAudioMetadata`: the length in seconds of the first audio stream, or of the file.
pub fn audio_duration(path: &Path) -> Result<f64, String> {
    let entries = ["-show_entries", "stream=duration", "-show_entries", "format=duration"];
    let v = ffprobe(&[&["-v", "error", "-select_streams", "a:0"][..], &entries, &["-of", "json"]].concat(), path)?;
    Ok(number(&v["streams"][0]["duration"]).or(number(&v["format"]["duration"])).unwrap_or(0.0))
}

/// Whether `bytes` are a HEIF file by the major brand of its `ftyp` box: file-type's `image/heic`, `image/heif` and
/// their sequences, which Desktop's `transcodeImage` decodes with libheif since Chromium and sharp cannot.
pub fn heif(bytes: &[u8]) -> bool {
    bytes.get(4..8) == Some(b"ftyp")
        && matches!(bytes.get(8..12), Some(b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1"))
}

/// Desktop's HEIC worker: the primary image of a HEIF file with its rotation, mirror and crop, as RGBA, and at most 100
/// million pixels.
fn decode_heif(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    let options =
        heic_rs::DecodeOptions::default().with_layout(heic_rs::PixelLayout::Rgba8).with_max_pixels(Some(100_000_000));
    let px = heic_rs::decode(bytes, &options).map_err(|e| format!("Could not decode the HEIF image: {e}"))?;
    let img = image::RgbaImage::from_raw(px.width, px.height, px.data).ok_or("The HEIF image has a wrong size")?;
    Ok(image::DynamicImage::ImageRgba8(img))
}

/// An image decoded upright: its EXIF orientation applied, as Chromium draws images and sharp's `rotate()` turns them.
/// A HEIF image is upright by its own transformations.
pub fn decode_upright(bytes: &[u8]) -> Result<image::DynamicImage, String> {
    use image::ImageDecoder;
    if heif(bytes) {
        return decode_heif(bytes);
    }
    let e = |e: image::ImageError| format!("Could not decode the image: {e}");
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().expect("in memory");
    let mut decoder = reader.into_decoder().map_err(e)?;
    let orientation = decoder.orientation().map_err(e)?;
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(e)?;
    img.apply_orientation(orientation);
    Ok(img)
}

/// The shown width and height of an image, after its EXIF orientation, like sharp's `autoOrient`.
fn image_size(path: &Path) -> Result<(u32, u32), String> {
    use image::ImageDecoder;
    let e = |e: image::ImageError| e.to_string();
    let reader = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format();
    let mut decoder = reader.map_err(|e| e.to_string())?.into_decoder().map_err(e)?;
    let (w, h) = decoder.dimensions();
    let turned = matches!(
        decoder.orientation().map_err(e)?,
        image::metadata::Orientation::Rotate90
            | image::metadata::Orientation::Rotate270
            | image::metadata::Orientation::Rotate90FlipH
            | image::metadata::Orientation::Rotate270FlipH
    );
    Ok(if turned { (h, w) } else { (w, h) })
}

/// Desktop's `Yf` on Linux: an H.264 MP4 that fits in the largest size of the compression or of the room.
fn video(
    src: &Path, stem: &str, dir: &Path, strong: bool, max: Option<u32>, rules: &FileRules,
) -> Result<Upload, String> {
    let meta = video_meta(src)?;
    let (audio_rate, dim) = if strong { ("96k", 848) } else { ("128k", 960) };
    let mut max = max.unwrap_or(dim);
    if let Some(r) = rules.get(FILE_VIDEO) {
        max = r.max_width.unwrap_or(max).min(r.max_height.unwrap_or(max));
    }
    let (w, h) = fit(meta.display, max);
    let turn = match meta.rotation {
        90 => "transpose=1,",
        180 => "transpose=2,transpose=2,",
        270 => "transpose=2,",
        _ => "",
    };
    let out = dir.join(format!("{}.mp4", safe(stem)));
    let filter = format!("{turn}scale={w}:{h}");
    let args = ["-c:v", "h264", "-preset", "slow", "-vf", &filter, "-crf", "26", "-c:a", "aac", "-b:a", audio_rate];
    let args = [&["-i".as_ref(), src.as_os_str(), "-y".as_ref()][..], &args.map(std::ffi::OsStr::new)].concat();
    let args = [&args[..], &["-movflags".as_ref(), "+faststart".as_ref(), out.as_os_str()]].concat();
    if let Err(e) = run("ffmpeg", &args) {
        eprintln!("FFmpeg error: {e}");
        let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
        if size < if strong { 5 << 20 } else { 50 << 20 } {
            let m = video_meta(src)?;
            let ext = src.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
            let (dims, duration) = (Some(m.size), Some(m.duration));
            return Ok(Upload {
                name: format!("{stem}{ext}"),
                path: src.into(),
                mime: m.mime.into(),
                dims,
                duration,
                failed: false,
            });
        }
        return Err(format!("Transcoding failed: {e}"));
    }
    let m = video_meta(&out).ok();
    let (dims, duration) = (m.as_ref().map(|m| m.size), m.map(|m| m.duration));
    Ok(Upload { name: format!("{stem}.mp4"), path: out, mime: "video/mp4".into(), dims, duration, failed: false })
}

/// Desktop's `audioTranscoding.audioFormats` on Linux: the extension, the MIME type and the ffmpeg arguments. The
/// `h264` format puts Desktop's dark `blackbox.jpg` (60 by 40) under the sound.
const AUDIO_FORMATS: [(&str, &str, &[&str]); 6] = [
    (".mp3", "audio/mpeg", &[]),
    (".mp4", "audio/mp4", &[]),
    (
        ".mp4",
        "video/mp4",
        &[
            "-f",
            "lavfi",
            "-i",
            "color=c=0x020202:s=60x40",
            "-c:v",
            "h264",
            "-c:a",
            "aac",
            "-vf",
            "scale=size=400x300",
            "-b:v",
            "200k",
            "-g",
            "9999",
            "-shortest",
        ],
    ),
    (".aac", "audio/aac", &["-c:a", "aac"]),
    (".caf", "audio/x-caf", &["-c:a", "libopus"]),
    (".ogg", "audio/ogg", &["-c:a", "copy"]),
];

/// Desktop's `mw`: the format of the conversion target, or Desktop's `pw` format that the room supports best.
fn audio_format(action: &Action, rules: &FileRules) -> usize {
    let target = match action {
        Action::Transcode { mime, .. } => AUDIO_FORMATS.iter().position(|f| f.1 == *mime),
        _ => None,
    };
    let Some(types) = rules.get(FILE_AUDIO).map(|r| &r.mime_types) else { return target.unwrap_or(0) };
    let by = |min| AUDIO_FORMATS.iter().position(|f| supports(f.1, types, min));
    target.or_else(|| by(2)).or_else(|| by(1)).unwrap_or(0)
}

/// Desktop's `Xf` on Linux: the audio file in the format of `AUDIO_FORMATS[format]`.
fn audio(src: &Path, stem: &str, dir: &Path, format: usize) -> Result<Upload, String> {
    let (ext, mime, extra) = AUDIO_FORMATS[format];
    let out = dir.join(format!("{}{ext}", safe(stem)));
    let args = [
        &["-i".as_ref(), src.as_os_str(), "-y".as_ref()][..],
        &extra.iter().map(std::ffi::OsStr::new).collect::<Vec<_>>(),
    ]
    .concat();
    run("ffmpeg", &[&args[..], &[out.as_os_str()]].concat()).map_err(|e| format!("Audio transcoding failed: {e}"))?;
    let duration = audio_duration(&out).ok();
    Ok(Upload { name: format!("{stem}{ext}"), path: out, mime: mime.into(), dims: None, duration, failed: false })
}

/// Desktop's `hw` with the `image` codecs instead of sharp: the image turned by its EXIF orientation, fit in the
/// largest size, as a JPEG (quality 75), PNG or WebP that the room takes. The `image` WebP encoder is lossless.
fn image(
    src: &Path, stem: &str, dir: &Path, strong: bool, max: Option<u32>, format: Option<&str>, rules: &FileRules,
) -> Result<Upload, String> {
    use image::ImageDecoder;
    let rule = rules.get(FILE_IMAGE);
    let listed = |f: &&str| rule.is_some_and(|r| supports(&format!("image/{f}"), &r.mime_types, 1));
    let format = format.or(["jpeg", "png", "webp"].into_iter().find(listed)).unwrap_or("jpeg");
    let out = dir.join(format!("{}.{}", safe(stem), if format == "jpeg" { "jpg" } else { format }));
    let e = |e: image::ImageError| e.to_string();
    let reader = image::ImageReader::open(src).map_err(|e| e.to_string())?.with_guessed_format();
    let mut decoder = reader.map_err(|e| e.to_string())?.into_decoder().map_err(e)?;
    let orientation = decoder.orientation().map_err(e)?;
    let mut img = image::DynamicImage::from_decoder(decoder).map_err(e)?;
    img.apply_orientation(orientation);
    let limit = match (max, rule) {
        (Some(m), _) => m,
        (None, r) => {
            let m = if strong { 1600 } else { 4096 };
            r.map_or(m, |r| r.max_width.unwrap_or(m).min(r.max_height.unwrap_or(m)))
        }
    };
    if img.width() > limit || img.height() > limit {
        let (w, h) = fit((img.width(), img.height()), limit);
        img = img.resize_exact(w, h, image::imageops::FilterType::Lanczos3);
    }
    let mut file = std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| e.to_string())?);
    match format {
        "png" => img.write_with_encoder(image::codecs::png::PngEncoder::new(&mut file)),
        "webp" => img.to_rgba8().write_with_encoder(image::codecs::webp::WebPEncoder::new_lossless(&mut file)),
        _ => img.to_rgb8().write_with_encoder(image::codecs::jpeg::JpegEncoder::new_with_quality(&mut file, 75)),
    }
    .map_err(e)?;
    let (name, mime) =
        (format!("{stem}.{}", out.extension().expect("an extension").to_string_lossy()), format!("image/{format}"));
    Ok(Upload { name, path: out, mime, dims: Some((img.width(), img.height())), duration: None, failed: false })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::beeper_files;

    fn rule(types: &[(&str, i8)]) -> FileRule {
        FileRule { mime_types: types.iter().map(|(m, l)| (m.to_string(), *l)).collect(), ..Default::default() }
    }

    /// Desktop's `rP`: a caption where the kind takes one, a GIF asking images and a voice note audio.
    #[test]
    fn captions_follow_the_room_features_like_desktop() {
        let with = |caption| FileRule { caption, ..rule(&[("*/*", 2)]) };
        let rules: FileRules =
            [(FILE_IMAGE.into(), with(Some(2))), (FILE_AUDIO.into(), with(Some(0))), (FILE_FILE.into(), with(None))]
                .into_iter()
                .collect();
        assert!(takes_caption(&rules, "image/jpeg", false, false));
        assert!(takes_caption(&rules, "image/gif", false, false), "a GIF without its own kind asks images");
        assert!(!takes_caption(&rules, "audio/ogg", false, true), "a voice note asks audio, which says no");
        assert!(!takes_caption(&rules, "application/pdf", false, false), "no caption level is no caption");
        assert!(!takes_caption(&FileRules::new(), "image/png", false, false));
        assert!(takes_caption(&crate::models::beeper_files(), "application/pdf", false, false), "Beeper's defaults");
    }

    /// HEIC photos that libheif and x265 made (pillow-heif 1.8.1), in 4:2:0 like a phone's, which Desktop's libheif
    /// shows with these colors away from the edge between them, where chroma upsampling differs.
    #[test]
    fn heif_photos_decode_like_libheif() {
        let near = |img: &image::RgbaImage, x, y, want: [u8; 4]| {
            let got = img.get_pixel(x, y).0;
            assert!(got.iter().zip(want).all(|(g, w)| g.abs_diff(w) <= 3), "({x}, {y}): {got:?} for {want:?}");
        };
        // Red on the left and blue on the right, with the top right quarter half transparent.
        let alpha = include_bytes!("../tests/heic/alpha.heic");
        assert!(heif(alpha));
        let img = decode_upright(alpha).expect("the HEIC photo").into_rgba8();
        assert_eq!(img.dimensions(), (64, 32));
        near(&img, 0, 0, [220, 30, 30, 255]);
        near(&img, 63, 0, [31, 30, 220, 128]);
        near(&img, 0, 31, [220, 30, 30, 255]);
        near(&img, 63, 31, [31, 30, 220, 255]);
        // Stored sideways, 32 by 64 with red on top, and turned upright by its `irot`.
        let rotated = include_bytes!("../tests/heic/rotated.heic");
        let img = decode_upright(rotated).expect("the rotated HEIC photo").into_rgba8();
        assert_eq!(img.dimensions(), (64, 32), "upright");
        near(&img, 0, 0, [220, 30, 30, 255]);
        near(&img, 63, 31, [31, 30, 220, 255]);
        let cut = &alpha[..alpha.len() / 2];
        assert!(decode_upright(cut).expect_err("a cut photo").starts_with("Could not decode the HEIF image"));
        let mut avif = alpha.to_vec();
        avif[8..12].copy_from_slice(b"avif");
        assert!(!heif(&avif) && !heif(b"\x89PNG\r\n\x1a\n"), "only HEIF brands");
    }

    /// A phone photo stores its pixels sideways with an EXIF orientation; Chromium shows it upright.
    #[test]
    fn images_decode_upright() {
        use image::ImageEncoder;
        // Two pixels side by side, red then blue, tagged "rotate 90° clockwise to show" (orientation 6).
        let exif = [0x49, 0x49, 0x2a, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0];
        let mut png = Vec::new();
        let mut encoder = image::codecs::png::PngEncoder::new(&mut png);
        encoder.set_exif_metadata(exif.to_vec()).unwrap();
        encoder.write_image(&[255, 0, 0, 0, 0, 255], 2, 1, image::ExtendedColorType::Rgb8).unwrap();
        let img = decode_upright(&png).unwrap().into_rgb8();
        assert_eq!(img.dimensions(), (1, 2), "turned upright");
        assert_eq!((img.get_pixel(0, 0).0, img.get_pixel(0, 1).0), ([255, 0, 0], [0, 0, 255]), "red on top");
        assert!(decode_upright(b"not an image").is_err());
    }

    #[test]
    fn decisions_follow_the_room_features_like_desktop() {
        let facts =
            |mime: &str, size, dims, duration| Facts { mime: mime.into(), size, dims, duration, sent: Sent::File };
        let png = |dims| facts("image/png", 9, dims, None);
        assert_eq!(decide(&png(None), &FileRules::new()), Action::SendAsIs, "a room without file features");
        assert_eq!(decide(&png(Some((9, 9))), &beeper_files()), Action::SendAsIs);
        let huge = decide(&facts("image/png", ABSOLUTE_MAX + 1, None, None), &beeper_files());
        assert_eq!(huge, Action::Reject("File size (524288001) exceeds absolute limit (524288000)".into()));
        let mut rules = beeper_files();
        let image = rules.get_mut(FILE_IMAGE).unwrap();
        (image.max_width, image.max_height) = (Some(100), Some(80));
        assert_eq!(decide(&png(None), &rules), Action::MissingInfo);
        assert_eq!(decide(&png(Some((200, 100))), &rules), Action::Resize(100, 50));
        assert_eq!(decide(&png(Some((150, 200))), &rules), Action::Resize(100, 80), "Desktop keeps the width");
        rules.get_mut(FILE_IMAGE).unwrap().max_width = None;
        assert_eq!(decide(&png(Some((100, 200))), &rules), Action::Resize(40, 80));
        rules.get_mut(FILE_IMAGE).unwrap().max_width = Some(100);
        let bmp = facts("image/bmp", 9, Some((200, 100)), None);
        assert_eq!(decide(&bmp, &rules), Action::Transcode { mime: "image/jpeg", strong: true, dims: Some((100, 50)) });
        let mkv = facts("video/x-matroska", 9, Some((9, 9)), Some(1.0));
        assert_eq!(decide(&mkv, &rules), Action::Transcode { mime: "video/mp4", strong: false, dims: None });
        rules.get_mut(FILE_VIDEO).unwrap().max_duration = Some(0.5);
        let mp4 = facts("video/mp4", 9, Some((9, 9)), None);
        assert_eq!(decide(&mp4, &rules), Action::MissingInfo);
        assert_eq!(decide(&Facts { duration: Some(1.0), ..mp4 }, &rules), Action::Compress);
        rules.insert(FILE_GIF.into(), rule(&[("video/mp4", 2)]));
        let gif = facts("image/gif", 9, Some((9, 9)), None);
        assert_eq!(decide(&gif, &rules), Action::SendAsIs, "the image features take it");
        rules.get_mut(FILE_IMAGE).unwrap().mime_types.remove("image/gif");
        assert_eq!(decide(&gif, &rules), Action::Transcode { mime: "video/mp4", strong: false, dims: None });
        rules.insert(FILE_AUDIO.into(), rule(&[("audio/ogg", 2)]));
        let flac = facts("audio/flac", 9, None, None);
        assert_eq!(decide(&flac, &rules), Action::SendAsIs, "the room takes it as a file");
        rules.remove(FILE_FILE);
        let none = "MIME type audio/flac is not supported for m.audio, no suitable transcoding target found, and \
            fallback to m.file not possible/supported.";
        assert_eq!(decide(&flac, &rules), Action::Reject(none.into()));
        rules.insert(FILE_AUDIO.into(), rule(&[("audio/*", 2), ("audio/flac", -2)]));
        assert_eq!(decide(&flac, &rules), Action::SendAsIs, "the highest level wins");
        rules.insert(FILE_AUDIO.into(), rule(&[("audio/mp4", 1), ("audio/flac", -2)]));
        let mp4_audio = Action::Transcode { mime: "audio/mp4", strong: false, dims: None };
        assert_eq!(decide(&flac, &rules), mp4_audio);
        rules.remove(FILE_VOICE);
        let voice = Facts { sent: Sent::Voice, ..facts("audio/ogg", 9, None, None) };
        assert_eq!(decide(&voice, &rules), mp4_audio, "a voice note falls back to audio");
        rules.remove(FILE_AUDIO);
        let missing = Action::Reject("Capability type org.matrix.msc3245.voice not listed in room features".into());
        assert_eq!(decide(&voice, &rules), missing);
        let sticker = Facts { sent: Sent::Sticker, ..facts("image/png", 9, Some((9, 9)), None) };
        rules.insert(FILE_IMAGE.into(), rule(&[("image/png", 2)]));
        assert_eq!(decide(&sticker, &rules), Action::SendAsIs, "a sticker falls back to images");
        rules.insert(FILE_STICKER.into(), rule(&[("image/webp", 2)]));
        let webp = Action::Transcode { mime: "image/webp", strong: false, dims: None };
        assert_eq!(decide(&sticker, &rules), webp, "a sticker follows the sticker features");
    }

    #[test]
    fn uploads_convert_like_desktop() {
        let dir = crate::testing::temp_dir("media");
        let ffmpeg = |args: &[&str], out: &Path| {
            let args: Vec<&std::ffi::OsStr> = args.iter().map(std::ffi::OsStr::new).chain([out.as_os_str()]).collect();
            run("ffmpeg", &args).expect("ffmpeg makes a test file");
            std::fs::read(out).unwrap()
        };
        let lavfi = ["-f", "lavfi", "-i", "testsrc=size=320x240:duration=1", "-f", "lavfi", "-i", "sine=duration=1"];
        let mkv = ffmpeg(&lavfi, &dir.join("a.mkv"));
        let wav = ffmpeg(&["-f", "lavfi", "-i", "sine=duration=1"], &dir.join("d.wav"));
        assert!(can_decode(&dir.join("a.mkv")) && can_decode(&dir.join("d.wav")), "ffmpeg decodes its own codecs");
        std::fs::write(dir.join("g.mkv"), b"not a video").unwrap();
        assert!(!can_decode(&dir.join("g.mkv")), "ffprobe cannot read it");
        let upload = |name: &str, mime: &str, dims| Upload {
            name: name.into(),
            path: PathBuf::new(),
            mime: mime.into(),
            dims,
            duration: None,
            failed: false,
        };
        let mut rules = FileRules::from([
            (FILE_VIDEO.into(), FileRule { max_width: Some(160), ..rule(&[("video/mp4", 2)]) }),
            (FILE_IMAGE.into(), FileRule { max_height: Some(50), ..rule(&[("image/jpeg", 2), ("image/png", 2)]) }),
            (FILE_AUDIO.into(), rule(&[("audio/mpeg", 2)])),
        ]);
        let work = dir.join("work");
        let send = |bytes: &[u8], f, rules: &FileRules| prepare_upload(&work, bytes, f, rules, Sent::File);
        let (bytes, f) = send(&mkv, upload("a.mkv", "video/x-matroska", Some((320, 240))), &rules).unwrap();
        assert_eq!((f.name.as_str(), f.mime.as_str(), f.dims), ("a.mp4", "video/mp4", Some((160, 120))));
        assert!(f.duration.is_some_and(|d| d > 0.9) && &bytes[4..8] == b"ftyp", "an MP4 with its length");
        assert!(!work.exists(), "the work folder goes away");
        let mut png = Vec::new();
        image::RgbaImage::new(300, 200).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let (bytes, f) = send(&png, upload("b.png", "image/png", Some((300, 200))), &rules).unwrap();
        assert_eq!((f.name.as_str(), f.mime.as_str(), f.dims), ("b.jpg", "image/jpeg", Some((75, 50))));
        assert_eq!(image::guess_format(&bytes).unwrap(), image::ImageFormat::Jpeg, "Desktop picks JPEG first");
        let pdf = send(b"%PDF", upload("c.pdf", "application/pdf", None), &rules).unwrap_err();
        assert!(pdf.starts_with("Can't send c.pdf to this chat: MIME type application/pdf is not supported"), "{pdf}");
        let (_, f) = send(&wav, upload("d.wav", "audio/wav", None), &rules).unwrap();
        assert_eq!((f.name.as_str(), f.mime.as_str()), ("d.mp3", "audio/mpeg"));
        assert!(f.duration.is_some_and(|d| d > 0.9));
        let broken = send(b"not a video", upload("e.mkv", "video/x-matroska", None), &rules).unwrap_err();
        assert_eq!(broken, "Couldn't convert e.mkv to a format this chat accepts");
        rules.get_mut(FILE_IMAGE).unwrap().mime_types.insert("image/png".into(), 0);
        let broken = send(b"not an image", upload("f.png", "image/png", Some((300, 200))), &rules).unwrap_err();
        assert!(broken.starts_with("Failed to resize image: "), "{broken}");
    }
}
