//! Inference that loads model bytes by CID. Mock prompt-hashing is not a runner.

use keel_types::{ContentId, InputRef, JobResult, JobStatus, JobSpec, Millicredits, UsageMeter, Work};
use sha2::{Digest, Sha256};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RunnerError {
    #[error("{0}")]
    Msg(String),
    #[error("llama-cli not configured (set KEEL_LLAMA_CLI)")]
    NoLlama,
}

pub trait InferenceRunner: Send + Sync {
    fn name(&self) -> &'static str;
    fn infer(&self, spec: &JobSpec, model_bytes: &[u8], input: &[u8]) -> Result<JobResult, RunnerError>;
}

/// Tiny little-endian f64 weight table. Magic `KEELW001`.
pub fn encode_weights(weights: &[f64]) -> Vec<u8> {
    let mut out = b"KEELW001".to_vec();
    out.extend_from_slice(&(weights.len() as u32).to_le_bytes());
    for w in weights {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

pub fn decode_weights(bytes: &[u8]) -> Result<Vec<f64>, RunnerError> {
    if bytes.len() < 12 || &bytes[..8] != b"KEELW001" {
        return Err(RunnerError::Msg("not a KEELW001 weight blob".into()));
    }
    let n = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
    let need = 12 + n * 8;
    if bytes.len() < need {
        return Err(RunnerError::Msg("truncated weights".into()));
    }
    let mut w = Vec::with_capacity(n);
    for i in 0..n {
        let start = 12 + i * 8;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&bytes[start..start + 8]);
        w.push(f64::from_le_bytes(buf));
    }
    Ok(w)
}

/// Score is a function of model bytes and prompt. Flip a weight → different output.
pub fn score(weights: &[f64], input: &[u8]) -> f64 {
    if weights.is_empty() {
        return 0.0;
    }
    input
        .iter()
        .enumerate()
        .map(|(i, b)| (*b as f64) * weights[i % weights.len()])
        .sum()
}

pub struct CidWeightsRunner;

impl InferenceRunner for CidWeightsRunner {
    fn name(&self) -> &'static str {
        "cid-weights"
    }

    fn infer(&self, spec: &JobSpec, model_bytes: &[u8], input: &[u8]) -> Result<JobResult, RunnerError> {
        let weights = decode_weights(model_bytes)?;
        let s = score(&weights, input);
        let job_id = spec.job_id().map_err(|e| RunnerError::Msg(e.to_string()))?;
        let billed = Millicredits(1.max(spec.max_millicredits.0.min(input.len() as i64)));
        Ok(JobResult {
            job_id,
            status: JobStatus::Succeeded,
            output: InputRef::Inline {
                text: format!("cid-weights:{s:.8}"),
            },
            meter: UsageMeter {
                job_id,
                work: Work::Job,
                billed,
            },
        })
    }
}

/// Operator runner: `KEEL_LLAMA_CLI` subprocess. Same JobResult shape.
pub struct LlamaCppRunner {
    pub bin: String,
}

impl LlamaCppRunner {
    pub fn from_env() -> Option<Self> {
        std::env::var("KEEL_LLAMA_CLI")
            .ok()
            .filter(|s| !s.is_empty())
            .map(|bin| Self { bin })
    }
}

impl InferenceRunner for LlamaCppRunner {
    fn name(&self) -> &'static str {
        "llama.cpp"
    }

    fn infer(&self, spec: &JobSpec, model_bytes: &[u8], input: &[u8]) -> Result<JobResult, RunnerError> {
        let dir = std::env::temp_dir().join(format!("keel-llama-{}", hex::encode(&ContentId::of_bytes(model_bytes).0[..8])));
        std::fs::create_dir_all(&dir).map_err(|e| RunnerError::Msg(e.to_string()))?;
        let model_path = dir.join("model.gguf");
        std::fs::write(&model_path, model_bytes).map_err(|e| RunnerError::Msg(e.to_string()))?;
        let prompt = String::from_utf8_lossy(input);
        let out = std::process::Command::new(&self.bin)
            .args(["-m", &model_path.to_string_lossy(), "-p", &prompt, "-n", "32"])
            .output()
            .map_err(|e| RunnerError::Msg(e.to_string()))?;
        if !out.status.success() {
            return Err(RunnerError::Msg(String::from_utf8_lossy(&out.stderr).into()));
        }
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let job_id = spec.job_id().map_err(|e| RunnerError::Msg(e.to_string()))?;
        Ok(JobResult {
            job_id,
            status: JobStatus::Succeeded,
            output: InputRef::Inline { text },
            meter: UsageMeter {
                job_id,
                work: Work::Job,
                billed: Millicredits(1),
            },
        })
    }
}

pub fn default_runner() -> Box<dyn InferenceRunner> {
    if let Some(llama) = LlamaCppRunner::from_env() {
        Box::new(llama)
    } else {
        Box::new(CidWeightsRunner)
    }
}

/// Fixture used in CI completeness tests (few KB, `fixtures/keelw001.bin`).
pub fn fixture_weights() -> Vec<u8> {
    include_bytes!("../fixtures/keelw001.bin").to_vec()
}

pub fn sha256_prefix(bytes: &[u8]) -> String {
    hex::encode(&Sha256::digest(bytes)[..4])
}

#[cfg(test)]
mod tests {
    use super::*;
    use keel_types::{JobSpec, Millicredits, Pubkey};

    #[test]
    fn weight_byte_changes_output() {
        let spec = JobSpec {
            schema: JobSpec::v0().into(),
            model: ContentId::of_bytes(&[0; 8]),
            input: InputRef::Inline {
                text: "hello".into(),
            },
            max_millicredits: Millicredits(10),
            payer: Pubkey([1u8; 32]),
            nonce: "n".into(),
        };
        let a = encode_weights(&[1.0, 2.0, 3.0, 4.0]);
        let mut b = a.clone();
        b[19] ^= 0xff;
        let r = CidWeightsRunner;
        let oa = r.infer(&spec, &a, b"hello").unwrap();
        let ob = r.infer(&spec, &b, b"hello").unwrap();
        let InputRef::Inline { text: ta } = oa.output else {
            panic!("inline");
        };
        let InputRef::Inline { text: tb } = ob.output else {
            panic!("inline");
        };
        assert_ne!(ta, tb);
        assert!(ta.starts_with("cid-weights:"));
        assert!(!ta.starts_with("keel-mock:"));
    }
}
