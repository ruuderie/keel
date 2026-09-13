use crate::identity::ContentId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub schema: String,
    pub name: String,
    pub license: String,
    pub files: Vec<ArtifactFile>,
    pub hardware: HardwareHint,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub signatures: Vec<ManifestSig>,
}

impl ArtifactManifest {
    pub fn v0() -> &'static str {
        "keel.artifact/0"
    }

    /// Identity excludes signatures so attestations can be added later.
    pub fn artifact_cid(&self) -> Result<ContentId, crate::canonical::ContentIdError> {
        let v = serde_json::to_value(self)?;
        let stripped = crate::canonical::without_key(v, "signatures");
        ContentId::of_canonical(&stripped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::{Pubkey, Signature};

    fn sample() -> ArtifactManifest {
        ArtifactManifest {
            schema: ArtifactManifest::v0().into(),
            name: "example".into(),
            license: "MIT".into(),
            files: vec![ArtifactFile {
                path: "weights.gguf".into(),
                cid: ContentId([1u8; 32]),
                size_bytes: 12,
                media: Some("gguf".into()),
            }],
            hardware: HardwareHint {
                min_vram_mb: 8192,
                quant: Some("Q4_K_M".into()),
                backend: vec!["llama.cpp".into()],
            },
            signatures: vec![],
        }
    }

    #[test]
    fn signatures_do_not_change_artifact_cid() {
        let mut m = sample();
        let a = m.artifact_cid().unwrap();
        m.signatures.push(ManifestSig {
            issuer: Pubkey([2u8; 32]),
            sig: Signature([3u8; 64]),
        });
        let b = m.artifact_cid().unwrap();
        assert_eq!(a, b);
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ArtifactFile {
    pub path: String,
    pub cid: ContentId,
    pub size_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HardwareHint {
    pub min_vram_mb: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quant: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub backend: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManifestSig {
    pub issuer: crate::identity::Pubkey,
    pub sig: crate::identity::Signature,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SeederRecord {
    pub file_cid: ContentId,
    pub multiaddrs: Vec<String>,
    pub expires_at: u64,
}
