use std::fmt::Write;

pub fn url_encode(input: &str) -> String {
    input.bytes().fold(String::with_capacity(input.len()), |mut s, b| {
        match b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
            true => s.push(b as char),
            false => write!(s, "%{b:02X}").unwrap(),
        }
        s
    })
}

pub fn url_decode(input: &str) -> Option<String> {
    let (mut out, mut bytes) = (Vec::with_capacity(input.len()), input.as_bytes());
    while let [b, rest @ ..] = bytes {
        bytes = rest;
        out.push(match b {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(..2)?).ok()?;
                bytes = &bytes[2..];
                u8::from_str_radix(hex, 16).ok()?
            }
            b'+' => b' ',
            &b => b,
        });
    }
    String::from_utf8(out).ok()
}
