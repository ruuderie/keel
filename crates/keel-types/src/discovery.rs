use crate::identity::ContentId;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ModelIndex {
    pub schema: String,
    pub seq: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prev: Option<ContentId>,
    pub entries: Vec<IndexEntry>,
}

impl ModelIndex {
    pub fn v0() -> &'static str {
        "keel.index/0"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IndexEntry {
    pub alias: String,
    pub artifact_cid: ContentId,
}

/// Opt-in. Never a protocol kill switch.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FilterList {
    pub schema: String,
    pub seq: u64,
    #[serde(default)]
    pub deny_cids: Vec<ContentId>,
    #[serde(default)]
    pub deny_aliases: Vec<String>,
}

impl FilterList {
    pub fn v0() -> &'static str {
        "keel.filter/0"
    }
}
