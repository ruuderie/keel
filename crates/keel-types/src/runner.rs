use crate::identity::ContentId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerAdvertisement {
    pub schema: String,
    pub models: Vec<ContentId>,
    pub backends: Vec<String>,
    pub vram_mb: u32,
    pub price: PriceBook,
    pub multiaddrs: Vec<String>,
    pub expires_at: u64,
}

impl RunnerAdvertisement {
    pub fn v0() -> &'static str {
        "keel.runner/0"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PriceBook {
    pub millicredits_per_k_completion: u32,
    pub millicredits_per_k_prompt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub millicredits_per_second: Option<u32>,
}
