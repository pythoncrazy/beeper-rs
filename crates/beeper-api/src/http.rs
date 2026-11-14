use crate::codec::{base64_encode_standard, sha1, url_decode, url_encode};
use serde_json::{Map, Value};
use std::collections::HashMap;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusCode(pub u16);

impl StatusCode {
    pub const OK: Self = Self(200);
    pub const CREATED: Self = Self(201);
    pub const NO_CONTENT: Self = Self(204);
    pub const PARTIAL_CONTENT: Self = Self(206);
    pub const FOUND: Self = Self(302);
    pub const NOT_MODIFIED: Self = Self(304);
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const NOT_FOUND: Self = Self(404);
    pub const CONFLICT: Self = Self(409);
    pub const PAYLOAD_TOO_LARGE: Self = Self(413);
    pub const RANGE_NOT_SATISFIABLE: Self = Self(416);
    pub const HEADERS_TOO_LARGE: Self = Self(431);
    pub const BAD_GATEWAY: Self = Self(502);

    /// The reason phrase of Node's `http.STATUS_CODES`.
    pub fn reason(self) -> &'static str {
        match self.0 {
            100 => "Continue",
            101 => "Switching Protocols",
            200 => "OK",
            201 => "Created",
            202 => "Accepted",
            204 => "No Content",
            206 => "Partial Content",
            301 => "Moved Permanently",
            302 => "Found",
            303 => "See Other",
            304 => "Not Modified",
            307 => "Temporary Redirect",
            308 => "Permanent Redirect",
            400 => "Bad Request",
            401 => "Unauthorized",
            403 => "Forbidden",
            404 => "Not Found",
            405 => "Method Not Allowed",
            406 => "Not Acceptable",
            409 => "Conflict",
            410 => "Gone",
            413 => "Payload Too Large",
            415 => "Unsupported Media Type",
            416 => "Range Not Satisfiable",
            422 => "Unprocessable Entity",
            429 => "Too Many Requests",
            431 => "Request Header Fields Too Large",
            500 => "Internal Server Error",
            501 => "Not Implemented",
            502 => "Bad Gateway",
            503 => "Service Unavailable",
            504 => "Gateway Timeout",
            code => panic!("add the reason phrase of HTTP status {code}"),
        }
    }
}

pub struct HttpMessage {
    pub start_line: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpMessage {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers.get(name).map(String::as_str)
    }

    pub fn header_is(&self, name: &str, value: &str) -> bool {
        self.header(name).is_some_and(|v| v.eq_ignore_ascii_case(value))
    }

    /// The method of a request.
    pub fn method(&self) -> &str {
        self.start_line.split(' ').next().expect("a first item")
    }

    /// The path of a request target, without the query.
    pub fn path(&self) -> &str {
        let target = self.start_line.split(' ').nth(1).unwrap_or_default();
        target.split('?').next().expect("a first item")
    }
}

/// The weak `ETag` of Express for a body.
pub fn weak_etag(body: &[u8]) -> String {
    format!("W/\"{:x}-{}\"", body.len(), &base64_encode_standard(&sha1(body))[..27])
}

/// An IMF-fixdate for the `Date` header, like `Sun, 06 Nov 1994 08:49:37 GMT`.
pub fn http_date(secs: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let (days, s) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = beeper_core::config::civil_from_days(days);
    let (weekday, month) = (DAYS[days.rem_euclid(7) as usize], MONTHS[m as usize - 1]);
    format!("{weekday}, {d:02} {month} {y} {:02}:{:02}:{:02} GMT", s / 3600, s / 60 % 60, s % 60)
}

/// Why a message was not read: the peer closed the connection (`None`), or the server refuses the request with
/// this status, like Node's HTTP parser and Express's body parsers.
pub type Stop = Option<StatusCode>;

const BAD: Stop = Some(StatusCode::BAD_REQUEST);

/// The `maxHeaderSize` of Node: the longest head of a message, chunk-size line, or trailer section.
const MAX_HEAD: usize = 16 * 1024;

async fn fill<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut Vec<u8>) -> Result<(), Stop> {
    buf.reserve(8192);
    stream.read_buf(buf).await.ok().filter(|&n| n > 0).ok_or(None).map(drop)
}

async fn fill_to<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut Vec<u8>, end: usize) -> Result<(), Stop> {
    while buf.len() < end {
        fill(stream, buf).await?;
    }
    Ok(())
}

/// The position of the next CRLF at or after `from`.
async fn line_end<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut Vec<u8>, from: usize) -> Result<usize, Stop> {
    loop {
        if let Some(i) = buf[from..].windows(2).position(|w| w == b"\r\n") {
            return Ok(from + i);
        }
        if buf.len() - from > MAX_HEAD {
            return Err(BAD);
        }
        fill(stream, buf).await?;
    }
}

/// Decodes a chunked body that starts at `pos`, and returns it with the position after its trailer section.
async fn read_chunked<R: AsyncRead + Unpin>(
    stream: &mut R,
    buf: &mut Vec<u8>,
    mut pos: usize,
    limit: usize,
) -> Result<(Vec<u8>, usize), Stop> {
    let mut body = Vec::new();
    loop {
        let eol = line_end(stream, buf, pos).await?;
        let size = std::str::from_utf8(&buf[pos..eol]).ok().map(|l| l.split(';').next().expect("a first item"));
        let size = size.filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_hexdigit()));
        let size = size.and_then(|s| usize::from_str_radix(s, 16).ok()).ok_or(BAD)?;
        pos = eol + 2;
        if size == 0 {
            break;
        }
        if body.len() + size > limit {
            return Err(Some(StatusCode::PAYLOAD_TOO_LARGE));
        }
        fill_to(stream, buf, pos + size + 2).await?;
        if &buf[pos + size..pos + size + 2] != b"\r\n" {
            return Err(BAD);
        }
        body.extend_from_slice(&buf[pos..pos + size]);
        pos += size + 2;
    }
    let trailer = pos;
    loop {
        let eol = line_end(stream, buf, pos).await?;
        let last = eol == pos;
        pos = eol + 2;
        if last {
            return Ok((body, pos));
        }
        if pos - trailer > MAX_HEAD {
            return Err(BAD);
        }
    }
}

/// Reads one message. `limit` gives the largest body that the server accepts for the message head.
pub async fn read_message<S: AsyncRead + AsyncWrite + Unpin>(
    stream: &mut S,
    buf: &mut Vec<u8>,
    limit: impl FnOnce(&HttpMessage) -> usize,
) -> Result<HttpMessage, Stop> {
    let bare_lf = |head: &[u8]| head.iter().enumerate().any(|(i, &b)| b == b'\n' && (i == 0 || head[i - 1] != b'\r'));
    let head_end = loop {
        match buf.windows(4).position(|w| w == b"\r\n\r\n") {
            Some(pos) if pos + 4 > MAX_HEAD => return Err(Some(StatusCode::HEADERS_TOO_LARGE)),
            Some(pos) => break pos + 4,
            None if bare_lf(buf) => return Err(BAD),
            None if buf.len() > MAX_HEAD => return Err(Some(StatusCode::HEADERS_TOO_LARGE)),
            None => fill(stream, buf).await?,
        }
    };
    let head = &buf[..head_end - 4];
    if bare_lf(head) {
        return Err(BAD);
    }
    let mut lines = std::str::from_utf8(head).map_err(|_| BAD)?.split("\r\n");
    let start_line = lines.next().expect("a first line").to_string();
    let mut headers = HashMap::new();
    for line in lines {
        let (k, v) = line.split_once(':').ok_or(BAD)?;
        if k.is_empty() || k.contains(|c: char| c.is_ascii_whitespace()) {
            return Err(BAD);
        }
        if headers.insert(k.to_ascii_lowercase(), v.trim().to_string()).is_some()
            && k.eq_ignore_ascii_case("content-length")
        {
            return Err(BAD);
        }
    }
    let mut msg = HttpMessage { start_line, headers, body: Vec::new() };
    let limit = limit(&msg);
    if msg.header_is("expect", "100-continue") {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").await.map_err(|_| None)?;
    }
    let end = match (msg.header("transfer-encoding"), msg.header("content-length")) {
        (Some(_), Some(_)) => return Err(BAD),
        (Some(te), None) => {
            if !te.rsplit(',').next().expect("a last item").trim().eq_ignore_ascii_case("chunked") {
                return Err(BAD);
            }
            let (body, end) = read_chunked(stream, buf, head_end, limit).await?;
            msg.body = body;
            end
        }
        (None, len) => {
            let len = len.unwrap_or("0");
            let len: usize = len.bytes().all(|b| b.is_ascii_digit()).then(|| len.parse().ok()).flatten().ok_or(BAD)?;
            if len > limit {
                return Err(Some(StatusCode::PAYLOAD_TOO_LARGE));
            }
            let end = head_end.checked_add(len).ok_or(BAD)?;
            fill_to(stream, buf, end).await?;
            msg.body = buf[head_end..end].to_vec();
            end
        }
    };
    buf.drain(..end);
    Ok(msg)
}

/// Parses a query string like the `qs` parser of Express: a repeated key or a `key[]` key gives an array of strings,
/// and a part with bad percent-encoding stays as it is.
pub fn parse_query(raw: &str) -> Map<String, Value> {
    let decode = |s: &str| url_decode(s).unwrap_or_else(|| s.replace('+', " "));
    let mut query = Map::new();
    for pair in raw.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let (k, v) = (decode(k), Value::String(decode(v)));
        let (key, array) = k.strip_suffix("[]").map_or((k.as_str(), false), |key| (key, true));
        match query.get_mut(key) {
            Some(Value::Array(values)) => values.push(v),
            Some(first) => *first = Value::Array(vec![first.take(), v]),
            None => drop(query.insert(key.into(), if array { Value::Array(vec![v]) } else { v })),
        }
    }
    query
}

pub fn query_string(path: &str, params: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect();
    format!("{path}?{}", pairs.join("&"))
}

/// The close code of a frame larger than the `max` of [`read_ws_frame`], like the `ws` package.
const WS_TOO_BIG: u16 = 1009;

/// Reads one frame as `(opcode, payload)`. `Err(Some(code))` means that the reader must close the connection with
/// this code.
pub async fn read_ws_frame<R: AsyncRead + Unpin>(r: &mut R, max: usize) -> Result<(u8, Vec<u8>), Option<u16>> {
    let [b0, b1] = r.read_u16().await.map_err(|_| None)?.to_be_bytes();
    let len = match b1 & 0x7F {
        126 => r.read_u16().await.map_err(|_| None)? as u64,
        127 => r.read_u64().await.map_err(|_| None)?,
        n => n as u64,
    };
    if len > max as u64 {
        return Err(Some(WS_TOO_BIG));
    }
    let mask = if b1 & 0x80 != 0 { r.read_u32().await.map_err(|_| None)?.to_be_bytes() } else { [0; 4] };
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).await.map_err(|_| None)?;
    payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i & 3]);
    Ok((b0 & 0x0F, payload))
}

/// Writes one final frame with `opcode`: 0x1 for text, 0x8 for close.
pub async fn write_ws_frame<W: AsyncWrite + Unpin>(
    w: &mut W,
    opcode: u8,
    payload: &[u8],
    masked: bool,
) -> std::io::Result<()> {
    let (mask_bit, mask) = if masked { (0x80, [0x37, 0xFA, 0x21, 0x3D]) } else { (0, [0; 4]) };
    let mut frame = vec![0x80 | opcode];
    match payload.len() {
        n if n < 126 => frame.push(mask_bit | n as u8),
        n if n <= 0xFFFF => frame.extend([&[mask_bit | 126][..], &(n as u16).to_be_bytes()].concat()),
        n => frame.extend([&[mask_bit | 127][..], &(n as u64).to_be_bytes()].concat()),
    }
    if masked {
        frame.extend(mask);
    }
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i & 3]));
    w.write_all(&frame).await
}
