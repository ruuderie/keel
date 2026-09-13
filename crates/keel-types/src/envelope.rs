use crate::canonical::canonical_json;
use crate::identity::{Identity, IdentityError, Pubkey, Signature};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const KEEL_VERSION: u32 = 0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[serde(rename = "peer.announce")]
    PeerAnnounce,
    #[serde(rename = "peer.invite")]
    PeerInvite,
    #[serde(rename = "artifact.announce")]
    ArtifactAnnounce,
    #[serde(rename = "artifact.want")]
    ArtifactWant,
    #[serde(rename = "seeder.announce")]
    SeederAnnounce,
    #[serde(rename = "index.publish")]
    IndexPublish,
    #[serde(rename = "filter.publish")]
    FilterPublish,
    #[serde(rename = "runner.announce")]
    RunnerAnnounce,
    #[serde(rename = "job.submit")]
    JobSubmit,
    #[serde(rename = "job.accept")]
    JobAccept,
    #[serde(rename = "job.result")]
    JobResult,
    #[serde(rename = "credit.hold")]
    CreditHold,
    #[serde(rename = "credit.burn")]
    CreditBurn,
    #[serde(rename = "credit.release")]
    CreditRelease,
    #[serde(rename = "pay.intent")]
    PayIntent,
    #[serde(rename = "pay.receipt")]
    PayReceipt,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Envelope<T> {
    pub keel: u32,
    pub kind: Kind,
    pub body: T,
    pub from: Pubkey,
    pub ts: u64,
    pub sig: Signature,
}

#[derive(Clone, Serialize)]
pub struct UnsignedEnvelope<T> {
    pub keel: u32,
    pub kind: Kind,
    pub body: T,
    pub from: Pubkey,
    pub ts: u64,
}

impl<T: Serialize> Envelope<T> {
    pub fn sign(kind: Kind, body: T, identity: &Identity, ts: u64) -> Result<Self, crate::canonical::ContentIdError> {
        let from = identity.pubkey();
        let unsigned = UnsignedEnvelope {
            keel: KEEL_VERSION,
            kind,
            body,
            from,
            ts,
        };
        let digest = signing_digest(&unsigned)?;
        let sig = identity.sign(&digest);
        Ok(Self {
            keel: unsigned.keel,
            kind: unsigned.kind,
            body: unsigned.body,
            from,
            ts,
            sig,
        })
    }

    pub fn verify(&self) -> Result<(), IdentityError>
    where
        T: Serialize,
    {
        let unsigned = UnsignedEnvelope {
            keel: self.keel,
            kind: self.kind,
            body: &self.body,
            from: self.from,
            ts: self.ts,
        };
        let digest = signing_digest(&unsigned).map_err(|_| IdentityError::BadSig)?;
        self.from.verify(&digest, &self.sig)
    }
}

fn signing_digest<T: Serialize>(unsigned: &T) -> Result<[u8; 32], crate::canonical::ContentIdError> {
    let canon = canonical_json(unsigned)?;
    Ok(Sha256::digest(&canon).into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::Identity;

    #[test]
    fn envelope_tamper_fails() {
        let id = Identity::generate();
        let mut env = Envelope::sign(Kind::IndexPublish, serde_json::json!({"seq": 1}), &id, 1).unwrap();
        env.verify().unwrap();
        env.body = serde_json::json!({"seq": 2});
        assert!(env.verify().is_err());
    }

    #[test]
    fn envelope_stable_digest() {
        let id = Identity::from_seed([7u8; 32]);
        let a = Envelope::sign(Kind::SeederAnnounce, "x", &id, 42).unwrap();
        let b = Envelope::sign(Kind::SeederAnnounce, "x", &id, 42).unwrap();
        assert_eq!(a.sig.0, b.sig.0);
    }
}
