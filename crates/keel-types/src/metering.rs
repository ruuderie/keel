use crate::identity::{ContentId, Pubkey};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Millicredits(pub i64);

impl Millicredits {
    pub fn saturating_add(self, other: Self) -> Self {
        Millicredits(self.0.saturating_add(other.0))
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CreditKind {
    Mint,
    Hold,
    Release,
    Burn,
    Expire,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditMovement {
    pub account: Pubkey,
    pub kind: CreditKind,
    pub amount: Millicredits,
    pub cause_type: CauseType,
    pub cause_id: String,
    pub idempotency_key: String,
}

impl CreditMovement {
    pub fn hold_key(job_id: &ContentId) -> String {
        format!("hold:{job_id}")
    }

    pub fn burn_key(job_id: &ContentId) -> String {
        format!("burn:{job_id}")
    }

    pub fn release_key(job_id: &ContentId) -> String {
        format!("release:{job_id}")
    }

    pub fn mint_key(rail: &str, rail_ref: &str) -> String {
        format!("mint:{rail}:{rail_ref}")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CauseType {
    SettlementReceipt,
    InferenceJob,
    HoldExpiry,
    ManualAdjust,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct UsageMeter {
    pub job_id: ContentId,
    pub work: Work,
    pub billed: Millicredits,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Work {
    Tokens {
        prompt: u32,
        completion: u32,
        tokenizer: ContentId,
    },
    Time {
        billable_ms: u64,
    },
    Job,
}

/// Prepaid compute inventory. Not money: no FX, no interest, no negative spend.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CreditAccount {
    pub balance: Millicredits,
    pub held: Millicredits,
}

impl CreditAccount {
    pub fn apply(&mut self, m: &CreditMovement) -> Result<(), CreditError> {
        let n = m.amount.0;
        if n <= 0 {
            return Err(CreditError::NonPositive);
        }
        match m.kind {
            CreditKind::Mint => self.balance.0 += n,
            CreditKind::Hold => {
                if self.balance.0 < n {
                    return Err(CreditError::Insufficient);
                }
                self.balance.0 -= n;
                self.held.0 += n;
            }
            CreditKind::Burn => {
                if self.held.0 < n {
                    return Err(CreditError::InsufficientHold);
                }
                self.held.0 -= n;
            }
            CreditKind::Release | CreditKind::Expire => {
                if self.held.0 < n {
                    return Err(CreditError::InsufficientHold);
                }
                self.held.0 -= n;
                self.balance.0 += n;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CreditError {
    #[error("amount must be positive")]
    NonPositive,
    #[error("insufficient free millicredits")]
    Insufficient,
    #[error("insufficient held millicredits")]
    InsufficientHold,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn acct(pk: u8) -> Pubkey {
        let mut b = [0u8; 32];
        b[0] = pk;
        Pubkey(b)
    }

    fn cid() -> ContentId {
        ContentId([7u8; 32])
    }

    #[test]
    fn mint_hold_burn_release() {
        let mut a = CreditAccount::default();
        let pk = acct(1);
        let job = cid();
        a.apply(&CreditMovement {
            account: pk,
            kind: CreditKind::Mint,
            amount: Millicredits(1000),
            cause_type: CauseType::SettlementReceipt,
            cause_id: "ln:abc".into(),
            idempotency_key: CreditMovement::mint_key("btc_lightning", "ph"),
        })
        .unwrap();
        a.apply(&CreditMovement {
            account: pk,
            kind: CreditKind::Hold,
            amount: Millicredits(400),
            cause_type: CauseType::InferenceJob,
            cause_id: job.to_string(),
            idempotency_key: CreditMovement::hold_key(&job),
        })
        .unwrap();
        a.apply(&CreditMovement {
            account: pk,
            kind: CreditKind::Burn,
            amount: Millicredits(250),
            cause_type: CauseType::InferenceJob,
            cause_id: job.to_string(),
            idempotency_key: CreditMovement::burn_key(&job),
        })
        .unwrap();
        a.apply(&CreditMovement {
            account: pk,
            kind: CreditKind::Release,
            amount: Millicredits(150),
            cause_type: CauseType::InferenceJob,
            cause_id: job.to_string(),
            idempotency_key: CreditMovement::release_key(&job),
        })
        .unwrap();
        assert_eq!(a, CreditAccount {
            balance: Millicredits(750),
            held: Millicredits(0),
        });
    }

    #[test]
    fn cannot_hold_more_than_balance() {
        let mut a = CreditAccount::default();
        let err = a
            .apply(&CreditMovement {
                account: acct(1),
                kind: CreditKind::Hold,
                amount: Millicredits(1),
                cause_type: CauseType::InferenceJob,
                cause_id: "x".into(),
                idempotency_key: "hold:x".into(),
            })
            .unwrap_err();
        assert_eq!(err, CreditError::Insufficient);
    }
}
