//! Desktop SDK's encrypted global account data (MSC4483): the content of an account data event, without its
//! plaintext fields, encrypted like a secret storage secret (mautrix's `ssss.Key.Encrypt`) with a 32-byte key that
//! secret storage keeps as `dev.zirco.msc4483.account_data.key`.

use ctr::cipher::{KeyIvInit, StreamCipher};
use ring::hkdf;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{json, Map, Value};

/// Desktop SDK's `SecretEncryptedAccountDataKey`: the secret storage name of the key.
pub const KEY_SECRET: &str = "dev.zirco.msc4483.account_data.key";
/// Desktop SDK's `plaintextAccountDataFields`, which stay next to `encrypted`.
const PLAINTEXT: [&str; 1] = ["com.beeper.revision_id"];

/// The 64 bytes of mautrix's `DeriveKeysSHA256`.
struct Keys;

impl hkdf::KeyType for Keys {
    fn len(&self) -> usize {
        64
    }
}

/// mautrix's `DeriveKeysSHA256`: HKDF-SHA-256 of the key with a zero salt and the event type, as the AES-256 key and
/// the HMAC-SHA-256 key.
fn derive(key: &[u8; 32], kind: &str) -> ([u8; 32], hmac::Key) {
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, &[0; 32]).extract(key);
    let mut out = [0; 64];
    let info = [kind.as_bytes()];
    let okm = prk.expand(&info, Keys).expect("HKDF-SHA-256 gives 64 bytes");
    okm.fill(&mut out).expect("HKDF-SHA-256 fills 64 bytes");
    let aes = out[..32].try_into().expect("a 32-byte AES key");
    (aes, hmac::Key::new(hmac::HMAC_SHA256, &out[32..]))
}

fn aes_ctr(key: &[u8; 32], iv: &[u8], data: &mut [u8]) {
    let mut cipher = ctr::Ctr128BE::<aes::Aes256>::new_from_slices(key, iv).expect("a 256-bit key and a 128-bit IV");
    cipher.apply_keystream(data);
}

/// Desktop SDK's `IsEncryptedAccountDataPayload`: `content` has an `encrypted` ciphertext, IV and MAC.
pub fn is_encrypted(content: &Value) -> bool {
    ["ciphertext", "iv", "mac"].iter().all(|k| content["encrypted"][k].as_str().is_some_and(|s| !s.is_empty()))
}

/// A new random key, like Desktop SDK's `generateAndStoreEncryptedAccountDataKey`.
pub fn new_key() -> [u8; 32] {
    let mut key = [0; 32];
    SystemRandom::new().fill(&mut key).expect("random bytes");
    key
}

/// Desktop SDK's `encryptedAccountDataKeyFromBytes` of the secret, which is the unpadded base64 of the key.
pub fn key_from_secret(secret: &str) -> Result<[u8; 32], String> {
    let bytes = vodozemac::base64_decode(secret.trim()).map_err(|e| e.to_string())?;
    let len = bytes.len();
    bytes.try_into().map_err(|_| format!("encrypted account data key unavailable: expected 32 bytes, got {len}"))
}

/// Desktop SDK's `encryptAccountDataPayload` of the account data `content` of the type `kind`: its plaintext fields,
/// and `encrypted` with the rest.
pub fn encrypt(key: &[u8; 32], kind: &str, content: &Value) -> Value {
    let mut iv = [0; 16];
    SystemRandom::new().fill(&mut iv).expect("random bytes");
    // mautrix's `GenA256CTRIV` clears bit 63 of the counter.
    iv[8] &= 0x7f;
    seal(key, kind, content, iv)
}

fn seal(key: &[u8; 32], kind: &str, content: &Value, iv: [u8; 16]) -> Value {
    let mut secret = content.as_object().cloned().unwrap_or_default();
    let mut out: Map<String, Value> = PLAINTEXT.iter().filter_map(|f| Some((f.to_string(), secret.remove(*f)?))).collect();
    let (aes, mac) = derive(key, kind);
    // mautrix encrypts the unpadded base64 of the data, like a secret storage secret.
    let mut payload = vodozemac::base64_encode(Value::Object(secret).to_string()).into_bytes();
    aes_ctr(&aes, &iv, &mut payload);
    let tag = hmac::sign(&mac, &payload);
    let encrypted = json!({
        "ciphertext": vodozemac::base64_encode(&payload),
        "iv": vodozemac::base64_encode(iv),
        "mac": vodozemac::base64_encode(tag.as_ref()),
    });
    out.insert("encrypted".into(), encrypted);
    Value::Object(out)
}

/// Desktop SDK's `decryptAccountDataPayload`: the decrypted content of encrypted account data of the type `kind`,
/// with its plaintext fields.
pub fn decrypt(key: &[u8; 32], kind: &str, content: &Value) -> Result<Value, String> {
    let field = |k: &str| {
        let s = content["encrypted"][k].as_str().ok_or("invalid MSC4483 encrypted account data envelope")?;
        vodozemac::base64_decode(s).map_err(|e| format!("invalid MSC4483 encrypted account data envelope: {e}"))
    };
    let (mut payload, iv, tag) = (field("ciphertext")?, field("iv")?, field("mac")?);
    let (aes, mac) = derive(key, kind);
    hmac::verify(&mac, &payload, &tag).map_err(|_| "bad MAC of encrypted account data")?;
    if iv.len() != 16 {
        return Err("invalid MSC4483 encrypted account data envelope".into());
    }
    aes_ctr(&aes, &iv, &mut payload);
    let data = vodozemac::base64_decode(&payload).map_err(|e| e.to_string())?;
    let mut plain: Value = serde_json::from_slice(&data).map_err(|e| format!("failed to unmarshal decrypted account data: {e}"))?;
    let fields = plain.as_object_mut().ok_or("account data payload must be a JSON object")?;
    for f in PLAINTEXT {
        if let Some(v) = content.get(f) {
            fields.insert(f.into(), v.clone());
        }
    }
    Ok(plain)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The envelope keeps the revision in plaintext, and the decryption gives the content back.
    #[test]
    fn encrypted_account_data_round_trips_like_desktop() {
        let key = new_key();
        let content = json!({ "accounts": { "whatsapp": { "a": { "name": "Work" } } }, "com.beeper.revision_id": "r1" });
        let env = encrypt(&key, "com.beeper.bridge_account_overrides", &content);
        assert!(is_encrypted(&env));
        let keys: Vec<&String> = env.as_object().expect("an object").keys().collect();
        assert_eq!(keys, ["com.beeper.revision_id", "encrypted"]);
        assert_eq!(env["com.beeper.revision_id"], "r1");
        assert!(!env["encrypted"]["ciphertext"].as_str().expect("a ciphertext").ends_with('='), "unpadded base64");
        assert_eq!(decrypt(&key, "com.beeper.bridge_account_overrides", &env), Ok(content));
        assert!(decrypt(&key, "com.beeper.other", &env).is_err(), "the event type is part of the keys");
        assert!(decrypt(&new_key(), "com.beeper.bridge_account_overrides", &env).is_err());
        assert!(!is_encrypted(&json!({ "encrypted": { "KEYID": { "ciphertext": "x", "iv": "y", "mac": "z" } } })));
    }

    /// A vector computed independently with Python's `cryptography` package: HKDF-SHA-256 of the key with 32 zero
    /// bytes of salt and the event type as info, AES-256-CTR of the unpadded base64 of the JSON, and HMAC-SHA-256 of
    /// the ciphertext, all in unpadded base64.
    #[test]
    fn encrypts_a_known_vector() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        let kind = "com.beeper.bridge_account_overrides";
        let plain = json!({ "accounts": { "slackgo": { "T1-U1": { "name": "Work" } } }, "com.beeper.revision_id": "rev" });
        let env = json!({
            "com.beeper.revision_id": "rev",
            "encrypted": { "ciphertext": CIPHERTEXT, "iv": "AAECAwQFBgcICQoLDA0ODw", "mac": MAC }
        });
        assert_eq!(seal(&key, kind, &plain, std::array::from_fn(|i| i as u8)), env);
        assert_eq!(decrypt(&key, kind, &env), Ok(plain));
        assert_eq!(key_from_secret(&vodozemac::base64_encode(key)), Ok(key));
        assert!(key_from_secret("AAAA").is_err());
    }

    const CIPHERTEXT: &str = "/o3KSAtVsNxJ9pZyO02Wf2PW2k0nwDsc3I67s+lUDgisiw5RYe3nmIpBWUl3k9bLShYKjrLmgQsX+LVip8hka7GN0g";
    const MAC: &str = "jX9e3Z3lHPjkT2nzsudvv9dyMdCVEs15W/HrdDRd6cM";
}
