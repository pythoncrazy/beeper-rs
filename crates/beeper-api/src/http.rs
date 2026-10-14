use crate::codec::{url_decode, url_encode};
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
    pub const BAD_REQUEST: Self = Self(400);
    pub const UNAUTHORIZED: Self = Self(401);
    pub const NOT_FOUND: Self = Self(404);
    pub const CONFLICT: Self = Self(409);
    pub const RANGE_NOT_SATISFIABLE: Self = Self(416);
    pub const BAD_GATEWAY: Self = Self(502);
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
}

async fn fill<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut Vec<u8>) -> Option<()> {
    let mut tmp = [0u8; 8192];
    let n = stream.read(&mut tmp).await.ok().filter(|&n| n > 0)?;
    buf.extend_from_slice(&tmp[..n]);
    Some(())
}

pub async fn read_message<R: AsyncRead + Unpin>(stream: &mut R, buf: &mut Vec<u8>) -> Option<HttpMessage> {
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos + 4;
        }
        fill(stream, buf).await?;
    };
    let mut lines = std::str::from_utf8(&buf[..head_end - 4]).ok()?.split("\r\n");
    let start_line = lines.next()?.to_string();
    let headers: HashMap<String, String> = lines
        .map(|l| l.split_once(':').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
        .collect::<Option<_>>()?;
    let len: usize = headers.get("content-length").map_or(Some(0), |v| v.parse().ok())?;
    while buf.len() < head_end + len {
        fill(stream, buf).await?;
    }
    let body = buf[head_end..head_end + len].to_vec();
    buf.drain(..head_end + len);
    Some(HttpMessage { start_line, headers, body })
}

pub fn parse_query(raw: &str) -> Option<HashMap<String, String>> {
    raw.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            Some((url_decode(k)?, url_decode(v)?))
        })
        .collect()
}

pub fn query_string(path: &str, params: &[(&str, &str)]) -> String {
    let pairs: Vec<String> = params.iter().map(|(k, v)| format!("{}={}", url_encode(k), url_encode(v))).collect();
    format!("{path}?{}", pairs.join("&"))
}

pub async fn read_ws_frame<R: AsyncRead + Unpin>(r: &mut R) -> Option<(u8, Vec<u8>)> {
    let [b0, b1] = r.read_u16().await.ok()?.to_be_bytes();
    let len = match b1 & 0x7F {
        126 => r.read_u16().await.ok()? as usize,
        127 => r.read_u64().await.ok()? as usize,
        n => n as usize,
    };
    let mask = if b1 & 0x80 != 0 { r.read_u32().await.ok()?.to_be_bytes() } else { [0; 4] };
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload).await.ok()?;
    payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= mask[i & 3]);
    Some((b0 & 0x0F, payload))
}

pub async fn write_ws_frame<W: AsyncWrite + Unpin>(w: &mut W, payload: &[u8], masked: bool) -> std::io::Result<()> {
    let (mask_bit, mask) = if masked { (0x80, [0x37, 0xFA, 0x21, 0x3D]) } else { (0, [0; 4]) };
    let mut frame = vec![0x81];
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
