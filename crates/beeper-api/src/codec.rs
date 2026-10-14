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

fn md_pad(data: &[u8]) -> Vec<u8> {
    let mut msg = data.to_vec();
    msg.push(0x80);
    msg.resize(msg.len() + (64 + 56 - msg.len() % 64) % 64, 0);
    msg.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    msg
}

fn be_words<const N: usize>(chunk: &[u8; 64]) -> [u32; N] {
    let mut w = [0u32; N];
    for (i, c) in chunk.as_chunks::<4>().0.iter().enumerate() {
        w[i] = u32::from_be_bytes(*c);
    }
    w
}

fn be_bytes<const W: usize, const B: usize>(state: [u32; W]) -> [u8; B] {
    let mut out = [0u8; B];
    for (o, s) in out.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *o = s.to_be_bytes();
    }
    out
}

#[rustfmt::skip]
const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state = [0x6a09e667u32, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19];
    for chunk in md_pad(data).as_chunks::<64>().0 {
        let mut w = be_words::<64>(chunk);
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16].wrapping_add(s0).wrapping_add(w[i - 7]).wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let [a, b, c, d, e, f, g, h] = v;
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t1 = h.wrapping_add(s1).wrapping_add((e & f) ^ (!e & g)).wrapping_add(SHA256_K[i]).wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let t2 = s0.wrapping_add((a & b) ^ (a & c) ^ (b & c));
            v = [t1.wrapping_add(t2), a, b, c, d.wrapping_add(t1), e, f, g];
        }
        for (s, x) in state.iter_mut().zip(v) {
            *s = s.wrapping_add(x);
        }
    }
    be_bytes(state)
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state = [0x67452301u32, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    for chunk in md_pad(data).as_chunks::<64>().0 {
        let mut w = be_words::<80>(chunk);
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let mut v = state;
        for (i, wi) in w.into_iter().enumerate() {
            let [a, b, c, d, e] = v;
            let (f, k) = match i / 20 {
                0 => ((b & c) | (!b & d), 0x5a827999),
                1 => (b ^ c ^ d, 0x6ed9eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            v = [
                a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(wi),
                a,
                b.rotate_left(30),
                c,
                d,
            ];
        }
        for (s, x) in state.iter_mut().zip(v) {
            *s = s.wrapping_add(x);
        }
    }
    be_bytes(state)
}

const B64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode_with(data: &[u8], url: bool) -> String {
    let sym = |v: u32| match (B64[v as usize & 63], url) {
        (b'+', true) => '-',
        (b'/', true) => '_',
        (c, _) => c as char,
    };
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let n = chunk.iter().enumerate().fold(0u32, |n, (i, &b)| n | (b as u32) << (16 - 8 * i));
        for i in 0..=chunk.len() {
            out.push(sym(n >> (18 - 6 * i)));
        }
        if !url {
            out.push_str(&"=="[..3 - chunk.len()]);
        }
    }
    out
}

pub fn base64_encode_standard(data: &[u8]) -> String {
    base64_encode_with(data, false)
}

pub fn base64_encode_url_nopad(data: &[u8]) -> String {
    base64_encode_with(data, true)
}

pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let (mut buf, mut bits) = (0u32, 0u32);
    for b in input.trim_end_matches('=').bytes() {
        buf = (buf << 6) | B64.iter().position(|&c| c == b)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}
