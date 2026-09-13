use crate::identity::{ContentId, Pubkey};
use crate::metering::{Millicredits, UsageMeter};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSpec {
    pub schema: String,
    pub model: ContentId,
    pub input: InputRef,
    pub max_millicredits: Millicredits,
    pub payer: Pubkey,
    pub nonce: String,
}

impl JobSpec {
    pub fn v0() -> &'static str {
        "keel.job/0"
    }

    pub fn job_id(&self) -> Result<ContentId, crate::canonical::ContentIdError> {
        ContentId::of_canonical(self)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InputRef {
    Inline { text: String },
    Cid { cid: ContentId },
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Specified,
    Submitted,
    HoldActive,
    Running,
    Succeeded,
    Failed,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobResult {
    pub job_id: ContentId,
    pub status: JobStatus,
    pub output: InputRef,
    pub meter: UsageMeter,
}

#[derive(Debug, Error)]
pub enum JobError {
    #[error("status {from:?} cannot move to {to:?}")]
    BadTransition { from: JobStatus, to: JobStatus },
}

impl JobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Specified => "specified",
            Self::Submitted => "submitted",
            Self::HoldActive => "hold_active",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }

    pub fn from_str_status(s: &str) -> Result<Self, String> {
        match s {
            "specified" => Ok(Self::Specified),
            "submitted" => Ok(Self::Submitted),
            "hold_active" => Ok(Self::HoldActive),
            "running" => Ok(Self::Running),
            "succeeded" => Ok(Self::Succeeded),
            "failed" => Ok(Self::Failed),
            "expired" => Ok(Self::Expired),
            other => Err(format!("unknown job status {other}")),
        }
    }

    pub fn can_enter(self, next: Self) -> bool {
        use JobStatus::*;
        matches!(
            (self, next),
            (Specified, Submitted)
                | (Submitted, HoldActive)
                | (HoldActive, Running)
                | (Running, Succeeded)
                | (Running, Failed)
                | (HoldActive, Expired)
                | (Running, Expired)
        )
    }

    pub fn transition(self, next: Self) -> Result<Self, JobError> {
        if self.can_enter(next) {
            Ok(next)
        } else {
            Err(JobError::BadTransition {
                from: self,
                to: next,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hold_then_run_then_succeed() {
        let s = JobStatus::Specified
            .transition(JobStatus::Submitted)
            .unwrap()
            .transition(JobStatus::HoldActive)
            .unwrap()
            .transition(JobStatus::Running)
            .unwrap()
            .transition(JobStatus::Succeeded)
            .unwrap();
        assert_eq!(s, JobStatus::Succeeded);
    }

    #[test]
    fn cannot_burn_without_hold() {
        assert!(JobStatus::Submitted.transition(JobStatus::Running).is_err());
    }
}
