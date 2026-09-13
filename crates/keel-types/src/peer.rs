//! How nodes learn each other's addresses. The CID is still identity; this is only reachability.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeerVisibility {
    /// Signed ad may be gossiped. Anyone who asks a public node can learn this address.
    Public,
    /// Address is not listed on GET /v0/peers. Reachable only with a signed invite.
    Invite,
}

impl PeerVisibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Invite => "invite",
        }
    }

    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "public" => Ok(Self::Public),
            "invite" => Ok(Self::Invite),
            other => Err(format!("peer visibility must be public or invite, not {other}")),
        }
    }
}

/// Signed by the node key. Public ads are the gossip object; invite ads must not be POSTed to /v0/peers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    pub schema: String,
    pub multiaddrs: Vec<String>,
    pub visibility: PeerVisibility,
    pub expires_at: u64,
}

impl PeerAdvertisement {
    pub fn v0() -> &'static str {
        "keel.peer/0"
    }
}

/// Bearer capability. Possession + valid signature is permission to add this peer privately.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PeerInvite {
    pub schema: String,
    pub invite_id: String,
    pub multiaddrs: Vec<String>,
    pub expires_at: u64,
    #[serde(default)]
    pub once: bool,
}

impl PeerInvite {
    pub fn v0() -> &'static str {
        "keel.invite/0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Envelope, Kind};
    use crate::identity::Identity;

    #[test]
    fn invite_tamper_fails() {
        let id = Identity::generate();
        let body = PeerInvite {
            schema: PeerInvite::v0().into(),
            invite_id: "ab".into(),
            multiaddrs: vec!["/ip4/127.0.0.1/tcp/7420/http".into()],
            expires_at: 9_999_999_999,
            once: true,
        };
        let mut env = Envelope::sign(Kind::PeerInvite, body, &id, 1).unwrap();
        env.verify().unwrap();
        env.body.multiaddrs.push("/ip4/10.0.0.1/tcp/1/http".into());
        assert!(env.verify().is_err());
    }
}
