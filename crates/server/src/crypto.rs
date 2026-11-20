//! Random tokens, hashing, and encryption of provider tokens at rest.

use aes_gcm::aead::{Aead, KeyInit};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use sha2::{Digest, Sha256};

/// A random secret with a readable prefix, e.g. "agent_3fA…".
pub fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}_{}", URL_SAFE_NO_PAD.encode(bytes))
}

pub fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// Compare two strings in time that does not depend on where they differ.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub struct Sealed {
    pub ciphertext: Vec<u8>,
    pub nonce: Vec<u8>,
}

pub fn seal(key: &[u8; 32], plaintext: &[u8]) -> anyhow::Result<Sealed> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("bad key: {e}"))?;
    let mut nonce = [0u8; 12];
    rand::rng().fill_bytes(&mut nonce);
    let ciphertext = cipher.encrypt(Nonce::from_slice(&nonce), plaintext).map_err(|e| anyhow::anyhow!("encrypt: {e}"))?;
    Ok(Sealed { ciphertext, nonce: nonce.to_vec() })
}

pub fn open(key: &[u8; 32], ciphertext: &[u8], nonce: &[u8]) -> anyhow::Result<Vec<u8>> {
    let cipher = Aes256Gcm::new_from_slice(key).map_err(|e| anyhow::anyhow!("bad key: {e}"))?;
    if nonce.len() != 12 {
        anyhow::bail!("stored nonce has the wrong length");
    }
    cipher.decrypt(Nonce::from_slice(nonce), ciphertext).map_err(|_| anyhow::anyhow!("stored token could not be decrypted; was TOKEN_KEY changed?"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_round_trips_and_detects_tampering() {
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        let mut other = key;
        other[0] ^= 1;
        let s = seal(&key, b"refresh-token").unwrap();
        assert_eq!(open(&key, &s.ciphertext, &s.nonce).unwrap(), b"refresh-token");
        let mut bad = s.ciphertext.clone();
        bad[0] ^= 1;
        assert!(open(&key, &bad, &s.nonce).is_err());
        assert!(open(&other, &s.ciphertext, &s.nonce).is_err());
    }

    #[test]
    fn secrets_are_unique_and_prefixed() {
        let a = new_secret("agent");
        assert!(a.starts_with("agent_") && a.len() > 40);
        assert_ne!(a, new_secret("agent"));
        assert!(constant_time_eq("abc", "abc"));
        assert!(!constant_time_eq("abc", "abd"));
        assert!(!constant_time_eq("abc", "ab"));
    }
}
