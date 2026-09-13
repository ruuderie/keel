use crate::canonical::{canonical_json, ContentIdError};
use ed25519_dalek::{Signature as EdSig, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use std::fmt;
use thiserror::Error;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ContentId(pub [u8; 32]);

impl ContentId {
    pub fn of_bytes(bytes: &[u8]) -> Self {
        ContentId(Sha256::digest(bytes).into())
    }

    pub fn of_canonical<T: Serialize>(value: &T) -> Result<Self, ContentIdError> {
        Ok(Self::of_bytes(&canonical_json(value)?))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn to_hex_prefixed(&self) -> String {
        format!("sha256:{}", hex::encode(self.0))
    }

    pub fn from_hex(raw: &str) -> Result<Self, String> {
        parse_prefixed(raw).or_else(|_| parse_raw32(raw).map(ContentId))
    }
}

impl fmt::Debug for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex_prefixed())
    }
}

impl fmt::Display for ContentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex_prefixed())
    }
}

impl Serialize for ContentId {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_hex_prefixed())
    }
}

impl<'de> Deserialize<'de> for ContentId {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        ContentId::from_hex(&raw).map_err(serde::de::Error::custom)
    }
}

fn parse_raw32(raw: &str) -> Result<[u8; 32], String> {
    let v = hex::decode(raw).map_err(|e| e.to_string())?;
    v.try_into()
        .map_err(|_| "expected 32 bytes".to_string())
}

fn parse_prefixed(raw: &str) -> Result<ContentId, String> {
    let hex = raw
        .strip_prefix("sha256:")
        .ok_or_else(|| "ContentId must be sha256:<hex>".to_string())?;
    Ok(ContentId(parse_raw32(hex)?))
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pubkey(pub [u8; 32]);

impl Pubkey {
    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }

    pub fn from_hex(raw: &str) -> Result<Self, String> {
        Ok(Pubkey(parse_raw32(raw)?))
    }

    pub fn verify(&self, msg: &[u8], sig: &Signature) -> Result<(), IdentityError> {
        let vk = VerifyingKey::from_bytes(&self.0).map_err(|_| IdentityError::BadKey)?;
        let ed = EdSig::from_bytes(&sig.0);
        vk.verify(msg, &ed).map_err(|_| IdentityError::BadSig)
    }
}

impl fmt::Debug for Pubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Display for Pubkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl Serialize for Pubkey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Pubkey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        Pubkey::from_hex(&raw).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Signature(pub [u8; 64]);

impl Serialize for Signature {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&hex::encode(self.0))
    }
}

impl<'de> Deserialize<'de> for Signature {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(d)?;
        let v = hex::decode(&raw).map_err(serde::de::Error::custom)?;
        let arr: [u8; 64] = v
            .try_into()
            .map_err(|_| serde::de::Error::custom("expected 64-byte signature"))?;
        Ok(Signature(arr))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Signed<T> {
    pub body: T,
    pub from: Pubkey,
    pub ts: u64,
    pub sig: Signature,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdentityError {
    #[error("invalid ed25519 key")]
    BadKey,
    #[error("signature verification failed")]
    BadSig,
    #[error("io: {0}")]
    Io(String),
}

/// Node/operator Ed25519 identity. Seed is 32 bytes on disk.
pub struct Identity {
    signing: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        Self {
            signing: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            signing: SigningKey::from_bytes(&seed),
        }
    }

    pub fn load_or_create(path: &std::path::Path) -> Result<Self, IdentityError> {
        if path.exists() {
            let bytes = std::fs::read(path).map_err(|e| IdentityError::Io(e.to_string()))?;
            let seed: [u8; 32] = bytes.try_into().map_err(|_| IdentityError::BadKey)?;
            Ok(Self::from_seed(seed))
        } else {
            let id = Self::generate();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir).map_err(|e| IdentityError::Io(e.to_string()))?;
            }
            std::fs::write(path, id.signing.to_bytes())
                .map_err(|e| IdentityError::Io(e.to_string()))?;
            Ok(id)
        }
    }

    pub fn seed_bytes(&self) -> [u8; 32] {
        self.signing.to_bytes()
    }

    pub fn pubkey(&self) -> Pubkey {
        Pubkey(self.signing.verifying_key().to_bytes())
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        Signature(self.signing.sign(msg).to_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_verify_and_tamper() {
        let id = Identity::generate();
        let msg = b"keel envelope";
        let sig = id.sign(msg);
        id.pubkey().verify(msg, &sig).unwrap();
        let mut bad = *msg;
        bad[0] ^= 1;
        assert!(id.pubkey().verify(&bad, &sig).is_err());
    }

    #[test]
    fn content_id_stable_for_same_bytes() {
        let a = ContentId::of_bytes(b"keel v0");
        let b = ContentId::of_bytes(b"keel v0");
        assert_eq!(a, b);
        assert_ne!(a, ContentId::of_bytes(b"keel v1"));
    }
}
