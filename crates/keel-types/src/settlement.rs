use crate::identity::{ContentId, Pubkey};
use crate::job::{JobSpec, JobStatus};
use crate::metering::{CreditAccount, CreditMovement, Millicredits};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Rail {
    BtcOnchain,
    BtcLightning,
    /// v1+; kept on the enum so the envelope does not change.
    Xmr,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IntentStatus {
    Open,
    Confirmed,
    Failed,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PaymentIntent {
    pub id: String,
    pub rail: Rail,
    pub amount_atomic: u64,
    pub mint_account: Pubkey,
    pub millicredits_on_confirm: Millicredits,
    pub status: IntentStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SettlementReceipt {
    pub intent_id: String,
    pub rail: Rail,
    pub rail_ref: String,
}

#[async_trait]
pub trait SettlementRail: Send + Sync {
    fn rail(&self) -> Rail;
    async fn create_intent(&self, intent: &PaymentIntent) -> Result<serde_json::Value, String>;
    async fn is_confirmed(&self, rail_ref: &str) -> Result<bool, String>;
}

#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, bytes: &[u8]) -> Result<ContentId, String>;
    async fn get(&self, cid: &ContentId) -> Result<Vec<u8>, String>;
    async fn has(&self, cid: &ContentId) -> Result<bool, String>;
}

#[async_trait]
pub trait JobScheduler: Send + Sync {
    async fn submit(&self, spec: JobSpec) -> Result<ContentId, String>;
    async fn poll(&self, job_id: &ContentId) -> Result<JobStatus, String>;
}

#[async_trait]
pub trait CreditWallet: Send + Sync {
    async fn apply(&self, m: CreditMovement) -> Result<CreditAccount, String>;
}
