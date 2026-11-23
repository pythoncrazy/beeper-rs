//! FLOE, the segmented AES-GCM encryption of the `com.beeper.floe.v1` attachments that bridges like WhatsApp's send
//! (MSC4571), as Desktop's SDK decrypts them with mautrix-go's `crypto/floe` and `crypto/attachment.FLOEFile`.
//!
//! The `file` object of the event holds the key and the FLOE header, in unpadded standard base64, and the SHA-256 of
//! the ciphertext, which is the FLOE body alone: segments of 4 KiB, each its own AES-256-GCM message under a key that
//! HKDF-SHA-384 derives from the segment number, and a shorter last segment that says it is the last.

use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::{digest, hkdf};
use serde_json::Value;

/// The `v` of a FLOE file in the `file` object of an event.
pub(crate) const VERSION: &str = "com.beeper.floe.v1";

/// `FLOE_IV_LEN`: the length of the random IV of the header.
const IV_LEN: usize = 32;
/// `AEAD_IV_LEN` of AES-GCM.
const NONCE_LEN: usize = 12;
/// `AEAD_TAG_LEN` of AES-GCM.
const TAG_LEN: usize = 16;
/// The length that says whether a segment is the last, before its nonce.
const MARK_LEN: usize = 4;
/// The `HKDF-EXPAND-SHA-384` output of the message key.
const MESSAGE_KEY_LEN: usize = 48;
/// The length of the header tag that proves the key.
const HEADER_TAG_LEN: usize = 32;

/// FLOE's parameters: AES-GCM-256 and HKDF-SHA-384 with a 32-byte IV, and the length of an encrypted segment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Params {
    /// `ENC_SEG_LEN`.
    seg_len: u32,
    /// The number of low bits of the segment number that share a derived key, 20 for AES-GCM.
    rotation: u32,
}

/// `GCM256_IV256_4K`, the only parameters of MSC4571, which mautrix-go's `FLOEParams` hardcodes.
pub(crate) const ATTACHMENT: Params = Params { seg_len: 4096, rotation: 20 };

impl Params {
    /// `PARAM_ENCODE`: the AEAD (0, AES-GCM-256), the hash (0, SHA-384), `ENC_SEG_LEN` and `FLOE_IV_LEN`.
    fn encoded(self) -> [u8; 10] {
        let mut e = [0; 10];
        e[2..6].copy_from_slice(&self.seg_len.to_be_bytes());
        e[6..].copy_from_slice(&(IV_LEN as u32).to_be_bytes());
        e
    }

    fn header_len(self) -> usize {
        self.encoded().len() + IV_LEN + HEADER_TAG_LEN
    }

    fn seg_len(self) -> usize {
        self.seg_len as usize
    }

    /// `FLOE_KDF`: HKDF-Expand with SHA-384 of `key`, over the encoded parameters, the IV, the purpose and the AAD.
    /// Like Go's `hkdf.Expand`, the key is the pseudorandom key as it is, whatever its length.
    fn kdf(self, key: &[u8], iv: &[u8], aad: &[u8], purpose: &[&[u8]], out: &mut [u8]) -> Result<(), String> {
        struct Len(usize);
        impl hkdf::KeyType for Len {
            fn len(&self) -> usize {
                self.0
            }
        }
        let encoded = self.encoded();
        let info = [&[&encoded[..], iv][..], purpose, &[aad]].concat();
        let prk = hkdf::Prk::new_less_safe(hkdf::HKDF_SHA384, key);
        let okm = prk.expand(&info, Len(out.len())).map_err(|_| "FLOE key derivation failed")?;
        okm.fill(out).map_err(|_| "FLOE key derivation failed".to_string())
    }
}

/// Whether two byte strings are equal, in a time that depends on their length only.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0, |d, (x, y)| d | (x ^ y)) == 0
}

/// mautrix-go's `FloeDecryptor`: the state of a decryption, one segment after the other.
pub(crate) struct Decryptor {
    params: Params,
    message_key: [u8; MESSAGE_KEY_LEN],
    iv: [u8; IV_LEN],
    aad: Vec<u8>,
    counter: u64,
    done: bool,
    /// The derived key of the masked segment number that it is for, which changes once in 2^20 segments.
    key: Option<(u64, LessSafeKey)>,
}

impl Decryptor {
    /// `NewFloeDecryptor`: checks that the header has the parameters and the tag of `key` before anything decrypts.
    pub(crate) fn new(params: Params, key: &[u8], aad: &[u8], header: &[u8]) -> Result<Self, String> {
        if key.len() != 32 {
            return Err(format!("key has incorrect length of {} rather than 32", key.len()));
        }
        if header.len() != params.header_len() {
            return Err("invalid header length".into());
        }
        let encoded = params.encoded();
        let (head, rest) = header.split_at(encoded.len());
        let (iv, tag) = rest.split_at(IV_LEN);
        if head != encoded {
            return Err("invalid header parameters".into());
        }
        let mut expected = [0; HEADER_TAG_LEN];
        params.kdf(key, iv, aad, &[b"HEADER_TAG:"], &mut expected)?;
        if !same(&expected, tag) {
            return Err("invalid header tag".into());
        }
        let mut message_key = [0; MESSAGE_KEY_LEN];
        params.kdf(key, iv, aad, &[b"MESSAGE_KEY:"], &mut message_key)?;
        let iv = iv.try_into().expect("the IV length");
        Ok(Self { params, message_key, iv, aad: aad.to_vec(), counter: 0, done: false, key: None })
    }

    /// `DERIVE_KEY`: the AES-GCM key of the segment `counter`, derived once for each masked segment number.
    fn key(&mut self) -> Result<&LessSafeKey, String> {
        let masked = self.counter & !((1u64 << self.params.rotation) - 1);
        if self.key.as_ref().is_none_or(|(m, _)| *m != masked) {
            let mut raw = [0; 32];
            let n = masked.to_be_bytes();
            self.params.kdf(&self.message_key, &self.iv, &self.aad, &[b"DEK:", &n], &mut raw)?;
            let key = UnboundKey::new(&aead::AES_256_GCM, &raw).map_err(|_| "bad AES-GCM key")?;
            self.key = Some((masked, LessSafeKey::new(key)));
        }
        Ok(&self.key.as_ref().expect("derived above").1)
    }

    /// `DecryptSegment` or, with `last`, `DecryptLastSegment`: appends the plaintext of `segment` to `out`.
    pub(crate) fn segment(&mut self, segment: &[u8], last: bool, out: &mut Vec<u8>) -> Result<(), String> {
        if self.done {
            return Err("decryptor closed".into());
        }
        let len = segment.len();
        let mark = u32::from_be_bytes(segment.get(..MARK_LEN).ok_or("segment too short")?.try_into().expect("4"));
        match last {
            false if len != self.params.seg_len() => return Err("invalid encrypted segment length".into()),
            false if mark != u32::MAX => return Err("unexpected last segment".into()),
            false if self.counter == (1 << 40) - 1 => return Err("counter overflow".into()),
            true if len < MARK_LEN + NONCE_LEN + TAG_LEN || len > self.params.seg_len() => {
                return Err(format!("invalid encrypted segment length {len}"))
            }
            true if mark as usize != len => return Err(format!("invalid last segment length: {mark}")),
            _ => {}
        }
        let mut aad = [0; 9];
        aad[..8].copy_from_slice(&self.counter.to_be_bytes());
        aad[8] = u8::from(last);
        let nonce = Nonce::try_assume_unique_for_key(&segment[MARK_LEN..MARK_LEN + NONCE_LEN]).expect("12 bytes");
        let key = self.key()?;
        let start = out.len();
        out.extend_from_slice(&segment[MARK_LEN + NONCE_LEN..]);
        match key.open_in_place(nonce, Aad::from(aad), &mut out[start..]) {
            Ok(plain) => {
                let n = plain.len();
                out.truncate(start + n);
            }
            Err(_) => {
                out.truncate(start);
                return Err(format!("segment {} failed authentication", self.counter));
            }
        }
        self.counter += 1;
        self.done = last;
        Ok(())
    }
}

/// The FLOE part of the `file` object of an event: what [`Stream::new`] needs.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct File {
    key: Vec<u8>,
    header: Vec<u8>,
    /// The `hashes.sha256` of the ciphertext, which MSC4571 lets streams leave out.
    sha256: Option<Vec<u8>>,
}

/// Unpadded standard base64, which also takes padding, like the v2 keys.
fn base64(v: &Value, what: &str) -> Result<Vec<u8>, String> {
    let s = v.as_str().ok_or(format!("no {what}"))?;
    vodozemac::base64_decode(s).map_err(|e| format!("bad {what}: {e}"))
}

impl File {
    /// `FLOEFile.decodeKeys` of the `file` object: its `floe_key`, `floe_header` and `hashes.sha256`.
    pub(crate) fn parse(file: &Value) -> Result<Self, String> {
        let sha256 = match &file["hashes"]["sha256"] {
            Value::Null => None,
            Value::String(s) if s.is_empty() => None,
            h => match base64(h, "hash")? {
                h if h.len() == 32 => Some(h),
                _ => return Err("bad hash: not 32 bytes".into()),
            },
        };
        Ok(Self { key: base64(&file["floe_key"], "FLOE key")?, header: base64(&file["floe_header"], "FLOE header")?, sha256 })
    }
}

/// mautrix-go's `FLOEStream` of a decryption: the ciphertext comes in pieces of any length as it downloads, and each
/// whole segment decrypts as soon as a byte after it arrives, since only the end of the body tells the last segment.
pub(crate) struct Stream {
    decryptor: Decryptor,
    pending: Vec<u8>,
    started: bool,
    hash: Option<(digest::Context, Vec<u8>)>,
}

impl Stream {
    /// Fails, before any download, when the header or the key is not right.
    pub(crate) fn new(file: &File) -> Result<Self, String> {
        let decryptor = Decryptor::new(ATTACHMENT, &file.key, &[], &file.header)?;
        let hash = file.sha256.clone().map(|h| (digest::Context::new(&digest::SHA256), h));
        Ok(Self { decryptor, pending: Vec::new(), started: false, hash })
    }

    /// Decrypts into `out` each segment of `chunk` and of what came before that is not the last.
    pub(crate) fn push(&mut self, chunk: &[u8], out: &mut Vec<u8>) -> Result<(), String> {
        self.started |= !chunk.is_empty();
        if let Some((h, _)) = &mut self.hash {
            h.update(chunk);
        }
        self.pending.extend_from_slice(chunk);
        let len = self.decryptor.params.seg_len();
        let mut at = 0;
        while self.pending.len() - at > len {
            self.decryptor.segment(&self.pending[at..at + len], false, out)?;
            at += len;
        }
        self.pending.drain(..at);
        Ok(())
    }

    /// Decrypts the last segment into `out` and checks the hash of the whole ciphertext.
    pub(crate) fn finish(mut self, out: &mut Vec<u8>) -> Result<(), String> {
        if !self.started {
            return Err("no ciphertext".into());
        }
        let last = std::mem::take(&mut self.pending);
        self.decryptor.segment(&last, true, out)?;
        match self.hash.map(|(h, expected)| same(h.finish().as_ref(), &expected)) {
            Some(false) => Err("mismatching SHA-256 digest".into()),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        let s = s.trim();
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
    }

    /// The decryption of a known-answer test of the FLOE specification, segment by segment like its `decryptKat`.
    fn kat(params: Params, ct: &str, pt: &str) {
        let (ct, pt) = (hex(ct), hex(pt));
        let (header, body) = ct.split_at(params.header_len());
        let mut d = Decryptor::new(params, &[0; 32], b"This is AAD", header).expect("the header of the KAT");
        let mut out = Vec::new();
        let chunks: Vec<_> = body.chunks(params.seg_len()).collect();
        for (i, seg) in chunks.iter().enumerate() {
            d.segment(seg, i + 1 == chunks.len(), &mut out).expect("a segment of the KAT");
        }
        assert!(d.done, "the last segment closes the decryption");
        assert_eq!(out, pt);
    }

    /// The known-answer tests of github.com/Snowflake-Labs/floe-specification (`kats/`, commit 9ea198a), which the Go
    /// implementation that mautrix-go vendors as `crypto/floe`, and the Java, C++ and Rust ones, encrypted.
    #[test]
    fn floe_decrypts_the_known_answers_of_the_specification() {
        macro_rules! kat {
            ($params:expr, $name:literal) => {
                kat(
                    $params,
                    include_str!(concat!("../tests/floe-kats/", $name, "_ct.txt")),
                    include_str!(concat!("../tests/floe-kats/", $name, "_pt.txt")),
                )
            };
        }
        kat!(ATTACHMENT, "go_GCM256_IV256_4K");
        kat!(ATTACHMENT, "rust_GCM256_IV256_4K");
        kat!(Params { seg_len: 64, rotation: 20 }, "go_GCM256_IV256_64");
        // A key that rotates every 4 segments, which shows the masking of the segment number.
        kat!(Params { seg_len: 40, rotation: 2 }, "go_rotation");
        kat!(Params { seg_len: 40, rotation: 2 }, "cpp_rotation");
        // A plaintext that ends on a segment boundary: a full last segment, or an empty one after it.
        kat!(Params { seg_len: 40, rotation: 20 }, "java_lastSegAligned");
        kat!(Params { seg_len: 40, rotation: 20 }, "java_lastSegEmpty");
    }

    #[test]
    fn floe_refuses_a_wrong_key_header_or_segment() {
        let ct = hex(include_str!("../tests/floe-kats/go_GCM256_IV256_64_ct.txt"));
        let p = Params { seg_len: 64, rotation: 20 };
        let (header, body) = ct.split_at(p.header_len());
        let aad = b"This is AAD";
        let e = |r: Result<Decryptor, String>| r.err().expect("an error");
        assert_eq!(e(Decryptor::new(p, &[1; 32], aad, header)), "invalid header tag");
        assert_eq!(e(Decryptor::new(p, &[0; 32], b"other AAD", header)), "invalid header tag");
        assert_eq!(e(Decryptor::new(ATTACHMENT, &[0; 32], aad, header)), "invalid header parameters");
        assert_eq!(e(Decryptor::new(p, &[0; 33], aad, header)), "key has incorrect length of 33 rather than 32");
        assert_eq!(e(Decryptor::new(p, &[0; 32], aad, &header[1..])), "invalid header length");
        let mut d = Decryptor::new(p, &[0; 32], aad, header).expect("the header");
        let mut out = Vec::new();
        let e = d.segment(&body[..64], true, &mut out).expect_err("a segment that is not the last");
        assert_eq!(e, "invalid last segment length: 4294967295");
        let mut bad = body[..64].to_vec();
        bad[30] ^= 1;
        assert_eq!(d.segment(&bad, false, &mut out).expect_err("a changed byte"), "segment 0 failed authentication");
        assert!(out.is_empty(), "nothing of a failed segment comes out");
        d.segment(&body[..64], false, &mut out).expect("the first segment");
        let e = d.segment(&body[..64], false, &mut out).expect_err("a segment out of order");
        assert_eq!(e, "segment 1 failed authentication");
    }

    /// The example of MSC4571, a `file` that mautrix-go encrypted: its key proves its header with an empty AAD.
    #[test]
    fn the_key_of_the_msc_example_proves_its_header() {
        let file = serde_json::json!({
            "floe_header": "AAAAABAAAAAAIEAEdNga2y5bl1DLr4SWyc5LqujDxTe11MvN0/xV0PaNbDEf641UUqBZ19mMpJnNOIXK3dge5cevhV/a7rwfS6E",
            "floe_key": "igAFh5gtEMIC+jDZ1pY9u1sOgsEAnqsbbPu3+vqjFO0",
            "hashes": { "sha256": "Yb1ZnFrVyRHQYKz4MIRyJG9vFIrjdCrBGCXR9wBmlLo" },
            "url": "mxc://maunium.net/NnLIbFTdLrekiiodajhUGtCA",
            "v": "com.beeper.floe.v1"
        });
        let f = File::parse(&file).expect("the file of the example");
        assert_eq!(f.sha256.as_ref().map(Vec::len), Some(32));
        Stream::new(&f).expect("the key and the header of the example");
        let mut other = f.clone();
        other.key[0] ^= 1;
        assert_eq!(Stream::new(&other).err().expect("another key"), "invalid header tag");
        let e = File::parse(&serde_json::json!({ "floe_key": "k", "floe_header": "AAAA" })).expect_err("a bad key");
        assert!(e.starts_with("bad FLOE key"), "{e}");
    }

    /// FLOE's encryption, which the tests use to make the body of an attachment: `NewFloeEncryptor` with `iv` instead
    /// of a random one, and `EncryptSegment` and `EncryptLastSegment` with nonces that count up.
    pub(crate) fn encrypt(key: &[u8; 32], iv: &[u8; IV_LEN], plain: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let p = ATTACHMENT;
        let mut header = [&p.encoded()[..], iv].concat();
        let mut tag = [0; HEADER_TAG_LEN];
        p.kdf(key, iv, &[], &[b"HEADER_TAG:"], &mut tag).expect("the tag");
        header.extend_from_slice(&tag);
        let mut d = Decryptor::new(p, key, &[], &header).expect("the header");
        let size = p.seg_len() - MARK_LEN - NONCE_LEN - TAG_LEN;
        // Like mautrix-go's `FLOEStream`, a plaintext that ends on a segment boundary ends with a full last segment.
        let mut segments: Vec<&[u8]> = plain.chunks(size).collect();
        if plain.is_empty() {
            segments.push(&[]);
        }
        let mut body = Vec::new();
        for (i, seg) in segments.iter().enumerate() {
            let last = i + 1 == segments.len();
            let mut aad = [0; 9];
            aad[..8].copy_from_slice(&d.counter.to_be_bytes());
            aad[8] = u8::from(last);
            let mut nonce = [0; NONCE_LEN];
            nonce[4..].copy_from_slice(&d.counter.to_be_bytes());
            let mut sealed = seg.to_vec();
            let key = d.key().expect("the key");
            let n = Nonce::assume_unique_for_key(nonce);
            key.seal_in_place_append_tag(n, Aad::from(aad), &mut sealed).expect("seal");
            let len = MARK_LEN + NONCE_LEN + sealed.len();
            body.extend_from_slice(&if last { len as u32 } else { u32::MAX }.to_be_bytes());
            body.extend_from_slice(&nonce);
            body.extend_from_slice(&sealed);
            d.counter += 1;
        }
        (header, body)
    }

    #[test]
    fn a_stream_decrypts_the_body_in_pieces_of_any_length() {
        let plain: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let size = ATTACHMENT.seg_len() - 32;
        for len in [0, 1, size - 1, size, size + 1, 2 * size, plain.len()] {
            let (header, body) = encrypt(&[7; 32], &[9; IV_LEN], &plain[..len]);
            let sha256 = digest::digest(&digest::SHA256, &body).as_ref().to_vec();
            let file = File { key: vec![7; 32], header, sha256: Some(sha256) };
            for piece in [1, 1000, 4096, 4097, body.len()] {
                let mut s = Stream::new(&file).expect("the stream");
                let mut out = Vec::new();
                for c in body.chunks(piece) {
                    s.push(c, &mut out).expect("a piece");
                    assert!(out.len() <= len, "nothing more than the plaintext");
                }
                s.finish(&mut out).expect("the end");
                assert_eq!(out, &plain[..len], "{len} bytes in pieces of {piece}");
            }
            let mut s = Stream::new(&File { sha256: Some(vec![0; 32]), ..file.clone() }).expect("the stream");
            let mut out = Vec::new();
            s.push(&body, &mut out).expect("the body");
            assert_eq!(s.finish(&mut out).expect_err("another hash"), "mismatching SHA-256 digest");
            let mut s = Stream::new(&File { sha256: None, ..file.clone() }).expect("the stream");
            s.push(&body[..body.len() - 1], &mut out).expect("the body");
            assert!(s.finish(&mut out).is_err(), "a cut body fails");
        }
        let (header, _) = encrypt(&[7; 32], &[9; IV_LEN], b"x");
        let s = Stream::new(&File { key: vec![7; 32], header, sha256: None }).expect("the stream");
        assert_eq!(s.finish(&mut Vec::new()).expect_err("no body"), "no ciphertext");
    }
}
