use crate::blob::FsArtifactStore;
use crate::ln::{LightningBook, PayIntentRequest, PayIntentView};
use crate::SCHEMA_SQL;
use keel_runner::{default_runner, InferenceRunner};
use keel_types::{
    ArtifactManifest, ArtifactStore, CauseType, ContentId, CreditAccount, CreditKind,
    CreditMovement, Envelope, FilterList, Identity, InputRef, JobResult, JobSpec, JobStatus,
    Kind, Millicredits, ModelIndex, PeerAdvertisement, PeerInvite, PeerVisibility, Pubkey,
    RunnerAdvertisement, SeederRecord, UsageMeter, Work,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum NodeError {
    #[error("{0}")]
    Msg(String),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}

impl NodeError {
    pub fn status_code(&self) -> u16 {
        match self {
            Self::NotFound(_) => 404,
            Self::Conflict(_) => 409,
            _ => 400,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StatusView {
    pub keel: u32,
    pub data_dir: String,
    pub blobs: i64,
    pub jobs: i64,
    pub accounts: i64,
    pub millicredits_free: i64,
    pub millicredits_held: i64,
    pub pubkey: String,
    pub advertise: Option<String>,
    pub visibility: String,
    pub peers_public: i64,
    pub peers_invite: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct BlobView {
    pub cid: String,
    pub size_bytes: i64,
    pub stored_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct AccountView {
    pub account: String,
    pub balance_millicredits: i64,
    pub held_millicredits: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MovementView {
    pub id: String,
    pub account: String,
    pub kind: String,
    pub amount_millicredits: i64,
    pub cause_type: String,
    pub cause_id: String,
    pub idempotency_key: String,
    pub created_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    pub job_id: String,
    pub status: String,
    pub hold_millicredits: i64,
    pub spec: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct SeederView {
    pub file_cid: String,
    pub multiaddr: String,
    pub announced_at: i64,
    pub expires_at: i64,
    pub from_key: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct MintRequest {
    pub account: String,
    pub amount_millicredits: i64,
    pub rail_ref: String,
    #[serde(default = "mock_rail")]
    pub rail: String,
}

fn mock_rail() -> String {
    "mock".into()
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PeerRedeemRequest {
    pub invite: Envelope<PeerInvite>,
    pub advertisement: Envelope<PeerAdvertisement>,
}

pub struct Node {
    pub data_dir: PathBuf,
    identity: Identity,
    blobs: FsArtifactStore,
    db: Mutex<Connection>,
    advertise: Mutex<Option<String>>,
    lightning: LightningBook,
    http: reqwest::Client,
    runner: Box<dyn InferenceRunner>,
    visibility: Mutex<PeerVisibility>,
    bootstrap: Mutex<Vec<String>>,
}

impl Node {
    pub fn open(data_dir: impl AsRef<Path>) -> Result<Self, NodeError> {
        let data_dir = data_dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&data_dir).map_err(|e| NodeError::Msg(e.to_string()))?;
        let db_path = data_dir.join("node.sqlite");
        let db = Connection::open(&db_path)?;
        db.execute_batch(SCHEMA_SQL)?;
        let identity = Identity::load_or_create(&data_dir.join("identity.key"))
            .map_err(|e| NodeError::Msg(e.to_string()))?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(8))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap_or_else(|_| reqwest::Client::new());
        Ok(Self {
            blobs: FsArtifactStore::new(&data_dir),
            data_dir,
            identity,
            db: Mutex::new(db),
            advertise: Mutex::new(None),
            lightning: LightningBook::new(),
            http,
            runner: default_runner(),
            visibility: Mutex::new(PeerVisibility::Invite),
            bootstrap: Mutex::new(Vec::new()),
        })
    }

    pub fn identity_pubkey(&self) -> Pubkey {
        self.identity.pubkey()
    }

    pub fn identity_view(&self) -> serde_json::Value {
        serde_json::json!({ "pubkey": self.identity.pubkey().to_hex() })
    }

    pub fn set_advertise_base(&self, base: impl Into<String>) {
        *self.advertise.lock().unwrap() = Some(base.into().trim_end_matches('/').to_string());
    }

    pub fn advertise_base(&self) -> Option<String> {
        self.advertise.lock().unwrap().clone()
    }

    pub fn visibility(&self) -> PeerVisibility {
        *self.visibility.lock().unwrap()
    }

    pub fn set_visibility(&self, vis: PeerVisibility) {
        *self.visibility.lock().unwrap() = vis;
    }

    pub fn add_bootstrap(&self, url: impl Into<String>) {
        let u = url.into().trim_end_matches('/').to_string();
        if u.is_empty() {
            return;
        }
        let mut g = self.bootstrap.lock().unwrap();
        if !g.iter().any(|x| x == &u) {
            g.push(u);
        }
    }

    pub fn publish_self(&self) -> Result<serde_json::Value, NodeError> {
        let Some(base) = self.advertise_base() else {
            return Ok(serde_json::json!({ "ok": false, "reason": "no advertise address" }));
        };
        let vis = self.visibility();
        let ad = PeerAdvertisement {
            schema: PeerAdvertisement::v0().into(),
            multiaddrs: vec![http_to_multiaddr(&base)],
            visibility: vis,
            expires_at: (Self::now() + 86400) as u64,
        };
        let env = self.sign_envelope(Kind::PeerAnnounce, ad)?;
        self.store_peer_envelope(&env, vis)?;
        Ok(serde_json::to_value(&env)?)
    }

    pub fn sign_envelope<T: serde::Serialize>(
        &self,
        kind: Kind,
        body: T,
    ) -> Result<Envelope<T>, NodeError> {
        Envelope::sign(kind, body, &self.identity, Self::now() as u64)
            .map_err(|e| NodeError::Msg(e.to_string()))
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, NodeError> {
        self.db
            .lock()
            .map_err(|_| NodeError::Msg("db lock poisoned".into()))
    }

    fn now() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub fn status(&self) -> Result<StatusView, NodeError> {
        let db = self.lock()?;
        let blobs: i64 = db.query_row("SELECT COUNT(*) FROM blobs", [], |r| r.get(0))?;
        let jobs: i64 = db.query_row("SELECT COUNT(*) FROM jobs", [], |r| r.get(0))?;
        let accounts: i64 =
            db.query_row("SELECT COUNT(*) FROM credit_accounts", [], |r| r.get(0))?;
        let millicredits_free: i64 = db
            .query_row(
                "SELECT COALESCE(SUM(balance_millicredits),0) FROM credit_accounts",
                [],
                |r| r.get(0),
            )?;
        let millicredits_held: i64 = db
            .query_row(
                "SELECT COALESCE(SUM(held_millicredits),0) FROM credit_accounts",
                [],
                |r| r.get(0),
            )?;
        let peers_public: i64 = db.query_row(
            "SELECT COUNT(*) FROM peers WHERE visibility = 'public'",
            [],
            |r| r.get(0),
        )?;
        let peers_invite: i64 = db.query_row(
            "SELECT COUNT(*) FROM peers WHERE visibility = 'invite'",
            [],
            |r| r.get(0),
        )?;
        Ok(StatusView {
            keel: keel_types::KEEL_VERSION,
            data_dir: self.data_dir.display().to_string(),
            blobs,
            jobs,
            accounts,
            millicredits_free,
            millicredits_held,
            pubkey: self.identity.pubkey().to_hex(),
            advertise: self.advertise_base(),
            visibility: self.visibility().as_str().into(),
            peers_public,
            peers_invite,
        })
    }

    pub async fn put_blob(&self, bytes: &[u8]) -> Result<BlobView, NodeError> {
        let cid = self
            .blobs
            .put(bytes)
            .await
            .map_err(NodeError::Msg)?;
        let now = Self::now();
        let size = bytes.len() as i64;
        {
            let db = self.lock()?;
            db.execute(
                "INSERT OR IGNORE INTO blobs (cid, size_bytes, stored_at) VALUES (?1, ?2, ?3)",
                params![cid.to_string(), size, now],
            )?;
        }
        if let Some(base) = self.advertise_base() {
            let _ = self.record_seeder(&cid, &base, now + 86400);
        }
        Ok(BlobView {
            cid: cid.to_string(),
            size_bytes: size,
            stored_at: now,
        })
    }

    pub async fn get_blob(&self, cid: &ContentId) -> Result<Vec<u8>, NodeError> {
        if let Ok(bytes) = self.blobs.get(cid).await {
            return Ok(bytes);
        }
        let seeders = self.list_seeders(Some(cid))?;
        let self_base = self.advertise_base();
        for s in seeders {
            let Some(base) = http_base(&s.multiaddr) else {
                continue;
            };
            if self_base.as_deref() == Some(base.as_str()) {
                continue;
            }
            let url = format!("{base}/v0/blobs/{}", cid.to_hex());
            let Ok(resp) = self.http.get(&url).send().await else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(bytes) = resp.bytes().await else {
                continue;
            };
            if ContentId::of_bytes(&bytes) != *cid {
                continue;
            }
            let _ = self.put_blob(&bytes).await;
            return Ok(bytes.to_vec());
        }
        let self_base = self.advertise_base();
        for base in self.peer_http_bases()? {
            if self_base.as_deref() == Some(base.as_str()) {
                continue;
            }
            let url = format!("{base}/v0/blobs/{}", cid.to_hex());
            let Ok(resp) = self.http.get(&url).send().await else {
                continue;
            };
            if !resp.status().is_success() {
                continue;
            }
            let Ok(bytes) = resp.bytes().await else {
                continue;
            };
            if ContentId::of_bytes(&bytes) != *cid {
                continue;
            }
            let _ = self.put_blob(&bytes).await;
            return Ok(bytes.to_vec());
        }
        Err(NodeError::NotFound(cid.to_string()))
    }

    pub fn list_blobs(&self) -> Result<Vec<BlobView>, NodeError> {
        let db = self.lock()?;
        let mut stmt =
            db.prepare("SELECT cid, size_bytes, stored_at FROM blobs ORDER BY stored_at DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(BlobView {
                cid: r.get(0)?,
                size_bytes: r.get(1)?,
                stored_at: r.get(2)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn new_account(&self) -> Result<AccountView, NodeError> {
        let mut raw = [0u8; 32];
        getrandom::getrandom(&mut raw).map_err(|e| NodeError::Msg(e.to_string()))?;
        let pk = Pubkey(raw);
        let db = self.lock()?;
        db.execute(
            "INSERT INTO credit_accounts (pubkey, balance_millicredits, held_millicredits) VALUES (?1, 0, 0)",
            params![pk.to_hex()],
        )?;
        Ok(AccountView {
            account: pk.to_hex(),
            balance_millicredits: 0,
            held_millicredits: 0,
        })
    }

    pub fn list_accounts(&self) -> Result<Vec<AccountView>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare(
            "SELECT pubkey, balance_millicredits, held_millicredits FROM credit_accounts ORDER BY pubkey",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(AccountView {
                account: r.get(0)?,
                balance_millicredits: r.get(1)?,
                held_millicredits: r.get(2)?,
            })
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn get_account(&self, account: &str) -> Result<AccountView, NodeError> {
        let db = self.lock()?;
        db.query_row(
            "SELECT pubkey, balance_millicredits, held_millicredits FROM credit_accounts WHERE pubkey = ?1",
            params![account],
            |r| {
                Ok(AccountView {
                    account: r.get(0)?,
                    balance_millicredits: r.get(1)?,
                    held_millicredits: r.get(2)?,
                })
            },
        )
        .optional()?
        .ok_or_else(|| NodeError::NotFound(account.into()))
    }

    pub fn list_movements(&self, account: Option<&str>) -> Result<Vec<MovementView>, NodeError> {
        let db = self.lock()?;
        if let Some(account) = account {
            let mut stmt = db.prepare(
                "SELECT id, account, kind, amount, cause_type, cause_id, idempotency_key, created_at
                 FROM credit_movements WHERE account = ?1 ORDER BY created_at DESC LIMIT 200",
            )?;
            let rows = stmt.query_map(params![account], map_movement)?;
            return Ok(rows.flatten().collect());
        }
        let mut stmt = db.prepare(
            "SELECT id, account, kind, amount, cause_type, cause_id, idempotency_key, created_at
             FROM credit_movements ORDER BY created_at DESC LIMIT 200",
        )?;
        let rows = stmt.query_map([], map_movement)?;
        Ok(rows.flatten().collect())
    }

    pub fn mint(&self, req: &MintRequest) -> Result<AccountView, NodeError> {
        let pk = Pubkey::from_hex(&req.account).map_err(NodeError::Msg)?;
        let key = CreditMovement::mint_key(&req.rail, &req.rail_ref);
        self.apply_credit(CreditMovement {
            account: pk,
            kind: CreditKind::Mint,
            amount: Millicredits(req.amount_millicredits),
            cause_type: CauseType::SettlementReceipt,
            cause_id: req.rail_ref.clone(),
            idempotency_key: key,
        })
    }

    fn apply_credit(&self, m: CreditMovement) -> Result<AccountView, NodeError> {
        let account = m.account.to_hex();
        let mut db = self.lock()?;
        let tx = db.transaction()?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM credit_movements WHERE account = ?1 AND idempotency_key = ?2",
                params![account, m.idempotency_key],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            let view = tx.query_row(
                "SELECT pubkey, balance_millicredits, held_millicredits FROM credit_accounts WHERE pubkey = ?1",
                params![account],
                |r| {
                    Ok(AccountView {
                        account: r.get(0)?,
                        balance_millicredits: r.get(1)?,
                        held_millicredits: r.get(2)?,
                    })
                },
            )?;
            tx.commit()?;
            return Ok(view);
        }

        let mut acct = {
            let row = tx
                .query_row(
                    "SELECT balance_millicredits, held_millicredits FROM credit_accounts WHERE pubkey = ?1",
                    params![account],
                    |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)),
                )
                .optional()?;
            match row {
                Some((b, h)) => CreditAccount {
                    balance: Millicredits(b),
                    held: Millicredits(h),
                },
                None => {
                    return Err(NodeError::NotFound(account));
                }
            }
        };
        acct.apply(&m).map_err(|e| NodeError::Conflict(e.to_string()))?;
        tx.execute(
            "UPDATE credit_accounts SET balance_millicredits = ?1, held_millicredits = ?2 WHERE pubkey = ?3",
            params![acct.balance.0, acct.held.0, account],
        )?;
        let id = movement_id(&account, &m.idempotency_key);
        let kind = match m.kind {
            CreditKind::Mint => "mint",
            CreditKind::Hold => "hold",
            CreditKind::Release => "release",
            CreditKind::Burn => "burn",
            CreditKind::Expire => "expire",
        };
        let cause = match m.cause_type {
            CauseType::SettlementReceipt => "settlement_receipt",
            CauseType::InferenceJob => "inference_job",
            CauseType::HoldExpiry => "hold_expiry",
            CauseType::ManualAdjust => "manual_adjust",
        };
        tx.execute(
            "INSERT INTO credit_movements (id, account, kind, amount, cause_type, cause_id, idempotency_key, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                id,
                account,
                kind,
                m.amount.0,
                cause,
                m.cause_id,
                m.idempotency_key,
                Self::now()
            ],
        )?;
        tx.commit()?;
        Ok(AccountView {
            account,
            balance_millicredits: acct.balance.0,
            held_millicredits: acct.held.0,
        })
    }

    pub fn submit_job(&self, spec: JobSpec) -> Result<JobView, NodeError> {
        let job_id = spec.job_id().map_err(|e| NodeError::Msg(e.to_string()))?;
        let spec_json = serde_json::to_string(&spec)?;
        let spec_cid = ContentId::of_bytes(spec_json.as_bytes());
        let now = Self::now();
        {
            let db = self.lock()?;
            db.execute(
                "INSERT OR IGNORE INTO jobs (job_id, spec_cid, spec_json, status, hold_millicredits, updated_at)
                 VALUES (?1, ?2, ?3, ?4, 0, ?5)",
                params![
                    job_id.to_string(),
                    spec_cid.to_string(),
                    spec_json,
                    JobStatus::Specified.as_str(),
                    now
                ],
            )?;
        }
        self.get_job(&job_id)
    }

    pub fn get_job(&self, job_id: &ContentId) -> Result<JobView, NodeError> {
        let db = self.lock()?;
        db.query_row(
            "SELECT job_id, status, hold_millicredits, spec_json, result_json, updated_at FROM jobs WHERE job_id = ?1",
            params![job_id.to_string()],
            map_job,
        )
        .optional()?
        .ok_or_else(|| NodeError::NotFound(job_id.to_string()))
    }

    pub fn list_jobs(&self) -> Result<Vec<JobView>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare(
            "SELECT job_id, status, hold_millicredits, spec_json, result_json, updated_at FROM jobs ORDER BY updated_at DESC",
        )?;
        let rows = stmt.query_map([], map_job)?;
        Ok(rows.flatten().collect())
    }

    /// Mock path kept for old tests: if still Specified, accept (hold) then run the CID runner.
    pub async fn run_job(&self, job_id: &ContentId) -> Result<JobView, NodeError> {
        self.expire_stale()?;
        let existing = self.get_job(job_id)?;
        if existing.status == JobStatus::Succeeded.as_str() {
            return Ok(existing);
        }
        if existing.status == JobStatus::Specified.as_str() {
            self.accept_job(job_id)?;
        }
        let existing = self.get_job(job_id)?;
        if existing.status != JobStatus::HoldActive.as_str() {
            return Err(NodeError::Conflict(format!(
                "job {} is {}, need hold_active",
                job_id, existing.status
            )));
        }
        let spec: JobSpec = serde_json::from_value(existing.spec.clone())?;
        let model_bytes = self.get_blob(&spec.model).await?;
        let input_bytes = match &spec.input {
            InputRef::Inline { text } => text.as_bytes().to_vec(),
            InputRef::Cid { cid } => self.get_blob(cid).await?,
        };
        self.set_status(job_id, JobStatus::HoldActive, JobStatus::Running, None)?;
        let hold = existing.hold_millicredits;
        let mut result = if keel_runner::decode_weights(&model_bytes).is_ok() {
            self.runner
                .infer(&spec, &model_bytes, &input_bytes)
                .map_err(|e| NodeError::Msg(e.to_string()))?
        } else {
            labeled_mock_result(*job_id, &input_bytes, hold)
        };
        let billed = result.meter.billed.0.min(hold).max(0);
        result.meter.billed = Millicredits(billed);
        if billed > 0 {
            self.apply_credit(CreditMovement {
                account: spec.payer,
                kind: CreditKind::Burn,
                amount: Millicredits(billed),
                cause_type: CauseType::InferenceJob,
                cause_id: job_id.to_string(),
                idempotency_key: CreditMovement::burn_key(job_id),
            })?;
        }
        let rest = hold - billed;
        if rest > 0 {
            self.apply_credit(CreditMovement {
                account: spec.payer,
                kind: CreditKind::Release,
                amount: Millicredits(rest),
                cause_type: CauseType::InferenceJob,
                cause_id: job_id.to_string(),
                idempotency_key: CreditMovement::release_key(job_id),
            })?;
        }
        let result_json = serde_json::to_string(&result)?;
        {
            let db = self.lock()?;
            db.execute(
                "UPDATE jobs SET status = ?1, result_json = ?2, hold_millicredits = 0, updated_at = ?3 WHERE job_id = ?4",
                params![
                    JobStatus::Succeeded.as_str(),
                    result_json,
                    Self::now(),
                    job_id.to_string()
                ],
            )?;
        }
        self.get_job(job_id)
    }

    pub fn accept_job(&self, job_id: &ContentId) -> Result<JobView, NodeError> {
        self.expire_stale()?;
        let existing = self.get_job(job_id)?;
        if existing.status == JobStatus::HoldActive.as_str() {
            return Ok(existing);
        }
        let spec: JobSpec = serde_json::from_value(existing.spec.clone())?;
        let _ = self.get_account(&spec.payer.to_hex())?;
        if existing.status == JobStatus::Specified.as_str() {
            self.set_status(job_id, JobStatus::Specified, JobStatus::Submitted, None)?;
        }
        let input_len = match &spec.input {
            InputRef::Inline { text } => text.len(),
            InputRef::Cid { cid } => std::fs::metadata(self.blobs.path_for(cid))
                .map(|m| m.len() as usize)
                .unwrap_or(1),
        };
        let quote = mock_quote(input_len, spec.max_millicredits.0);
        self.apply_credit(CreditMovement {
            account: spec.payer,
            kind: CreditKind::Hold,
            amount: Millicredits(quote),
            cause_type: CauseType::InferenceJob,
            cause_id: job_id.to_string(),
            idempotency_key: CreditMovement::hold_key(job_id),
        })?;
        let exp = Self::now() + 900;
        {
            let db = self.lock()?;
            db.execute(
                "UPDATE jobs SET status = ?1, hold_millicredits = ?2, expires_at = ?3, updated_at = ?4 WHERE job_id = ?5",
                params![
                    JobStatus::HoldActive.as_str(),
                    quote,
                    exp,
                    Self::now(),
                    job_id.to_string()
                ],
            )?;
        }
        self.get_job(job_id)
    }

    pub fn expire_stale(&self) -> Result<usize, NodeError> {
        let now = Self::now();
        let ids: Vec<(String, i64, String)> = {
            let db = self.lock()?;
            let mut stmt = db.prepare(
                "SELECT job_id, hold_millicredits, spec_json FROM jobs WHERE status = 'hold_active' AND expires_at IS NOT NULL AND expires_at < ?1",
            )?;
            let rows = stmt.query_map(params![now], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, String>(2)?))
            })?;
            rows.flatten().collect()
        };
        let n = ids.len();
        for (job_id, hold, spec_json) in ids {
            if hold > 0 {
                if let Ok(spec) = serde_json::from_str::<JobSpec>(&spec_json) {
                    let cid = ContentId::from_hex(&job_id).unwrap_or(ContentId([0u8; 32]));
                    let _ = self.apply_credit(CreditMovement {
                        account: spec.payer,
                        kind: CreditKind::Expire,
                        amount: Millicredits(hold),
                        cause_type: CauseType::HoldExpiry,
                        cause_id: job_id.clone(),
                        idempotency_key: format!("expire:{cid}"),
                    });
                }
            }
            let db = self.lock()?;
            db.execute(
                "UPDATE jobs SET status = ?1, hold_millicredits = 0, updated_at = ?2 WHERE job_id = ?3",
                params![JobStatus::Expired.as_str(), now, job_id],
            )?;
        }
        Ok(n)
    }

    fn record_seeder(&self, cid: &ContentId, base: &str, expires_at: i64) -> Result<(), NodeError> {
        let pk = self.identity.pubkey().to_hex();
        let multiaddr = http_to_multiaddr(base);
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO seeders (file_cid, multiaddr, announced_at, expires_at, from_key) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![cid.to_string(), multiaddr, Self::now(), expires_at, pk],
        )?;
        Ok(())
    }

    pub fn put_seeder(&self, rec: SeederRecord) -> Result<(), NodeError> {
        let pk = self.identity.pubkey().to_hex();
        let db = self.lock()?;
        for m in rec.multiaddrs {
            db.execute(
                "INSERT OR REPLACE INTO seeders (file_cid, multiaddr, announced_at, expires_at, from_key) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    rec.file_cid.to_string(),
                    m,
                    Self::now(),
                    rec.expires_at as i64,
                    pk
                ],
            )?;
        }
        Ok(())
    }

    pub fn list_seeders(&self, cid: Option<&ContentId>) -> Result<Vec<SeederView>, NodeError> {
        let db = self.lock()?;
        if let Some(cid) = cid {
            let mut stmt = db.prepare(
                "SELECT file_cid, multiaddr, announced_at, expires_at, from_key FROM seeders WHERE file_cid = ?1",
            )?;
            let rows = stmt.query_map(params![cid.to_string()], map_seeder)?;
            return Ok(rows.flatten().collect());
        }
        let mut stmt = db.prepare(
            "SELECT file_cid, multiaddr, announced_at, expires_at, from_key FROM seeders ORDER BY announced_at DESC",
        )?;
        let rows = stmt.query_map([], map_seeder)?;
        Ok(rows.flatten().collect())
    }

    pub fn put_artifact(&self, m: ArtifactManifest) -> Result<serde_json::Value, NodeError> {
        let cid = m.artifact_cid().map_err(|e| NodeError::Msg(e.to_string()))?;
        let body = serde_json::to_vec(&m)?;
        let now = Self::now();
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO documents (cid, kind, body, stored_at) VALUES (?1, ?2, ?3, ?4)",
            params![cid.to_string(), "artifact", body, now],
        )?;
        for f in &m.files {
            db.execute(
                "INSERT OR REPLACE INTO artifact_files (artifact_cid, path, file_cid, size_bytes) VALUES (?1, ?2, ?3, ?4)",
                params![cid.to_string(), f.path, f.cid.to_string(), f.size_bytes as i64],
            )?;
        }
        Ok(serde_json::json!({ "cid": cid.to_string(), "manifest": m }))
    }

    pub fn get_artifact(&self, cid: &ContentId) -> Result<serde_json::Value, NodeError> {
        let db = self.lock()?;
        let body: Vec<u8> = db
            .query_row(
                "SELECT body FROM documents WHERE cid = ?1 AND kind = 'artifact'",
                params![cid.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| NodeError::NotFound(cid.to_string()))?;
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn list_artifacts(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt =
            db.prepare("SELECT cid, stored_at FROM documents WHERE kind = 'artifact' ORDER BY stored_at DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "cid": r.get::<_, String>(0)?,
                "stored_at": r.get::<_, i64>(1)?,
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn indexes_for(&self, publisher: &str) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare(
            "SELECT publisher, seq, cid FROM indexes WHERE publisher = ?1 ORDER BY seq DESC",
        )?;
        let rows = stmt.query_map(params![publisher], |r| {
            Ok(serde_json::json!({
                "publisher": r.get::<_, String>(0)?,
                "seq": r.get::<_, i64>(1)?,
                "cid": r.get::<_, String>(2)?,
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn get_filter(&self, cid: &ContentId) -> Result<serde_json::Value, NodeError> {
        let db = self.lock()?;
        let (issuer, seq, body): (String, i64, String) = db
            .query_row(
                "SELECT issuer, seq, body FROM filters WHERE cid = ?1",
                params![cid.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
            .ok_or_else(|| NodeError::NotFound(cid.to_string()))?;
        Ok(serde_json::json!({
            "cid": cid.to_string(),
            "issuer": issuer,
            "seq": seq,
            "body": serde_json::from_str::<serde_json::Value>(&body).unwrap_or(serde_json::Value::Null),
        }))
    }

    pub fn publish_index(&self, env: Envelope<ModelIndex>) -> Result<serde_json::Value, NodeError> {
        env.verify().map_err(|e| NodeError::Conflict(e.to_string()))?;
        if env.kind != Kind::IndexPublish {
            return Err(NodeError::Msg("expected index.publish".into()));
        }
        let cid = ContentId::of_canonical(&env.body).map_err(|e| NodeError::Msg(e.to_string()))?;
        let body = serde_json::to_vec(&env)?;
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO documents (cid, kind, body, stored_at) VALUES (?1, ?2, ?3, ?4)",
            params![cid.to_string(), "index", body, Self::now()],
        )?;
        db.execute(
            "INSERT OR REPLACE INTO indexes (publisher, seq, cid) VALUES (?1, ?2, ?3)",
            params![env.from.to_hex(), env.body.seq as i64, cid.to_string()],
        )?;
        Ok(serde_json::json!({
            "cid": cid.to_string(),
            "publisher": env.from.to_hex(),
            "seq": env.body.seq
        }))
    }

    pub fn index_head(&self, publisher: &str) -> Result<serde_json::Value, NodeError> {
        let db = self.lock()?;
        let cid: String = db
            .query_row(
                "SELECT cid FROM indexes WHERE publisher = ?1 ORDER BY seq DESC LIMIT 1",
                params![publisher],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(|| NodeError::NotFound(publisher.into()))?;
        let body: Vec<u8> = db.query_row(
            "SELECT body FROM documents WHERE cid = ?1",
            params![cid],
            |r| r.get(0),
        )?;
        Ok(serde_json::from_slice(&body)?)
    }

    pub fn list_indexes(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare("SELECT publisher, seq, cid FROM indexes ORDER BY seq DESC")?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "publisher": r.get::<_, String>(0)?,
                "seq": r.get::<_, i64>(1)?,
                "cid": r.get::<_, String>(2)?,
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn put_filter(&self, env: Envelope<FilterList>) -> Result<serde_json::Value, NodeError> {
        env.verify().map_err(|e| NodeError::Conflict(e.to_string()))?;
        let cid = ContentId::of_canonical(&env.body).map_err(|e| NodeError::Msg(e.to_string()))?;
        let body = serde_json::to_string(&env.body)?;
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO filters (cid, issuer, seq, body) VALUES (?1, ?2, ?3, ?4)",
            params![cid.to_string(), env.from.to_hex(), env.body.seq as i64, body],
        )?;
        Ok(serde_json::json!({ "cid": cid.to_string(), "seq": env.body.seq }))
    }

    pub fn list_filters(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare("SELECT cid, issuer, seq, body FROM filters")?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "cid": r.get::<_, String>(0)?,
                "issuer": r.get::<_, String>(1)?,
                "seq": r.get::<_, i64>(2)?,
                "body": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(3)?).unwrap_or(serde_json::Value::Null),
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    pub fn put_runner(&self, env: Envelope<RunnerAdvertisement>) -> Result<serde_json::Value, NodeError> {
        env.verify().map_err(|e| NodeError::Conflict(e.to_string()))?;
        let cid = ContentId::of_canonical(&env.body).map_err(|e| NodeError::Msg(e.to_string()))?;
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO runners (from_key, advert_cid, expires_at, caps_json) VALUES (?1, ?2, ?3, ?4)",
            params![
                env.from.to_hex(),
                cid.to_string(),
                env.body.expires_at as i64,
                serde_json::to_string(&env.body)?
            ],
        )?;
        Ok(serde_json::json!({ "from": env.from.to_hex(), "cid": cid.to_string() }))
    }

    pub fn list_runners(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare("SELECT from_key, advert_cid, expires_at, caps_json FROM runners")?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "from": r.get::<_, String>(0)?,
                "cid": r.get::<_, String>(1)?,
                "expires_at": r.get::<_, i64>(2)?,
                "advert": serde_json::from_str::<serde_json::Value>(&r.get::<_, String>(3)?).unwrap_or(serde_json::Value::Null),
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    pub async fn pay_intent(&self, req: PayIntentRequest) -> Result<PayIntentView, NodeError> {
        let _ = self.get_account(&req.account)?;
        let view = if crate::lnd::configured() {
            let created = crate::lnd::add_invoice("keel millicredit mint", req.amount_sats)
                .await
                .map_err(NodeError::Msg)?;
            PayIntentView {
                id: created.payment_hash.clone(),
                rail: "btc_lightning",
                bolt11: created.bolt11,
                payment_hash: created.payment_hash,
                millicredits_on_confirm: req.millicredits_on_confirm,
                amount_atomic: req.amount_sats,
                status: "open".into(),
            }
        } else {
            self.lightning
                .create_invoice(&req.account, req.millicredits_on_confirm, req.amount_sats)
                .map_err(NodeError::Msg)?
        };
        {
            let db = self.lock()?;
            db.execute(
                "INSERT INTO payment_intents (id, rail, amount_atomic, millicredits, payment_hash, bolt11, mint_account, status, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![
                    view.id,
                    "btc_lightning",
                    view.amount_atomic as i64,
                    view.millicredits_on_confirm,
                    view.payment_hash,
                    view.bolt11,
                    req.account,
                    "open",
                    Self::now()
                ],
            )?;
        }
        Ok(view)
    }

    pub async fn pay_settle(
        &self,
        payment_hash: &str,
        preimage_hex: &str,
    ) -> Result<AccountView, NodeError> {
        let (account, millicredits) = if self.lightning.get(payment_hash).is_some() {
            let inv = self
                .lightning
                .settle(payment_hash, preimage_hex)
                .map_err(NodeError::Conflict)?;
            (inv.account, inv.millicredits)
        } else if crate::lnd::configured() {
            let settled = crate::lnd::is_settled(payment_hash)
                .await
                .map_err(NodeError::Msg)?;
            if !settled {
                return Err(NodeError::Conflict("lnd invoice not settled".into()));
            }
            let db = self.lock()?;
            db.query_row(
                "SELECT mint_account, millicredits FROM payment_intents WHERE payment_hash = ?1",
                params![payment_hash],
                |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)),
            )
            .optional()?
            .ok_or_else(|| NodeError::NotFound(payment_hash.into()))?
        } else {
            return Err(NodeError::NotFound(payment_hash.into()));
        };
        {
            let db = self.lock()?;
            db.execute(
                "UPDATE payment_intents SET status = 'confirmed' WHERE payment_hash = ?1",
                params![payment_hash],
            )?;
            db.execute(
                "INSERT OR IGNORE INTO settlement_receipts (id, intent_id, rail_ref) VALUES (?1, ?2, ?3)",
                params![payment_hash, payment_hash, payment_hash],
            )?;
        }
        self.mint(&MintRequest {
            account,
            amount_millicredits: millicredits,
            rail_ref: payment_hash.to_string(),
            rail: "btc_lightning".into(),
        })
    }

    pub fn test_preimage(&self, payment_hash: &str) -> Option<String> {
        self.lightning.test_preimage(payment_hash)
    }

    pub fn list_intents(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        let db = self.lock()?;
        let mut stmt = db.prepare(
            "SELECT id, rail, amount_atomic, millicredits, payment_hash, bolt11, mint_account, status FROM payment_intents ORDER BY created_at DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(serde_json::json!({
                "id": r.get::<_, String>(0)?,
                "rail": r.get::<_, String>(1)?,
                "amount_atomic": r.get::<_, i64>(2)?,
                "millicredits": r.get::<_, i64>(3)?,
                "payment_hash": r.get::<_, Option<String>>(4)?,
                "bolt11": r.get::<_, Option<String>>(5)?,
                "account": r.get::<_, String>(6)?,
                "status": r.get::<_, String>(7)?,
            }))
        })?;
        Ok(rows.flatten().collect())
    }

    fn store_peer_envelope(
        &self,
        env: &Envelope<PeerAdvertisement>,
        vis: PeerVisibility,
    ) -> Result<(), NodeError> {
        env.verify().map_err(|e| NodeError::Conflict(e.to_string()))?;
        if env.kind != Kind::PeerAnnounce {
            return Err(NodeError::Msg("expected peer.announce".into()));
        }
        if env.body.expires_at as i64 <= Self::now() {
            return Err(NodeError::Conflict("peer advertisement expired".into()));
        }
        let addrs = serde_json::to_string(&env.body.multiaddrs)?;
        let envelope_json = serde_json::to_string(env)?;
        let db = self.lock()?;
        db.execute(
            "INSERT OR REPLACE INTO peers (pubkey, visibility, multiaddrs_json, expires_at, advertised_at, envelope_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                env.from.to_hex(),
                vis.as_str(),
                addrs,
                env.body.expires_at as i64,
                Self::now(),
                envelope_json
            ],
        )?;
        Ok(())
    }

    pub fn ingest_public_peer(
        &self,
        env: Envelope<PeerAdvertisement>,
    ) -> Result<serde_json::Value, NodeError> {
        if env.body.visibility != PeerVisibility::Public {
            return Err(NodeError::Conflict(
                "invite-only advertisements must not be posted to the public peer list".into(),
            ));
        }
        self.store_peer_envelope(&env, PeerVisibility::Public)?;
        Ok(serde_json::json!({ "from": env.from.to_hex(), "visibility": "public" }))
    }

    pub fn list_public_peer_envelopes(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        self.list_peer_envelopes(Some(PeerVisibility::Public))
    }

    pub fn list_known_peers(&self) -> Result<Vec<serde_json::Value>, NodeError> {
        self.list_peer_envelopes(None)
    }

    fn list_peer_envelopes(
        &self,
        only: Option<PeerVisibility>,
    ) -> Result<Vec<serde_json::Value>, NodeError> {
        let now = Self::now();
        let db = self.lock()?;
        let sql = if only.is_some() {
            "SELECT envelope_json FROM peers WHERE visibility = ?1 AND expires_at > ?2 ORDER BY advertised_at DESC"
        } else {
            "SELECT envelope_json FROM peers WHERE expires_at > ?1 ORDER BY advertised_at DESC"
        };
        let mut stmt = db.prepare(sql)?;
        let rows = match only {
            Some(v) => stmt
                .query_map(params![v.as_str(), now], |r| r.get::<_, String>(0))?
                .flatten()
                .collect::<Vec<_>>(),
            None => stmt
                .query_map(params![now], |r| r.get::<_, String>(0))?
                .flatten()
                .collect::<Vec<_>>(),
        };
        Ok(rows
            .into_iter()
            .filter_map(|s| serde_json::from_str(&s).ok())
            .collect())
    }

    pub fn peer_http_bases(&self) -> Result<Vec<String>, NodeError> {
        let now = Self::now();
        let db = self.lock()?;
        let mut stmt = db.prepare("SELECT multiaddrs_json FROM peers WHERE expires_at > ?1")?;
        let rows = stmt.query_map(params![now], |r| r.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows.flatten() {
            let addrs: Vec<String> = serde_json::from_str(&row).unwrap_or_default();
            for m in addrs {
                if let Some(base) = http_base(&m) {
                    if !out.contains(&base) {
                        out.push(base);
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn create_invite(&self, once: bool) -> Result<Envelope<PeerInvite>, NodeError> {
        let Some(base) = self.advertise_base() else {
            return Err(NodeError::Msg("node has no advertise address yet".into()));
        };
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).map_err(|e| NodeError::Msg(e.to_string()))?;
        let invite_id = hex::encode(raw);
        let expires_at = (Self::now() + 7 * 86400) as u64;
        {
            let db = self.lock()?;
            db.execute(
                "INSERT INTO peer_invites (invite_id, once, expires_at, redeemed, created_at) VALUES (?1, ?2, ?3, 0, ?4)",
                params![invite_id, if once { 1 } else { 0 }, expires_at as i64, Self::now()],
            )?;
        }
        let body = PeerInvite {
            schema: PeerInvite::v0().into(),
            invite_id,
            multiaddrs: vec![http_to_multiaddr(&base)],
            expires_at,
            once,
        };
        self.sign_envelope(Kind::PeerInvite, body)
    }

    pub fn accept_invite(
        &self,
        env: Envelope<PeerInvite>,
    ) -> Result<serde_json::Value, NodeError> {
        env.verify().map_err(|e| NodeError::Conflict(e.to_string()))?;
        if env.kind != Kind::PeerInvite {
            return Err(NodeError::Msg("expected peer.invite".into()));
        }
        if env.body.expires_at as i64 <= Self::now() {
            return Err(NodeError::Conflict("invite expired".into()));
        }
        let addrs = serde_json::to_string(&env.body.multiaddrs)?;
        let envelope_json = serde_json::to_string(&env)?;
        {
            let db = self.lock()?;
            db.execute(
                "INSERT OR REPLACE INTO peers (pubkey, visibility, multiaddrs_json, expires_at, advertised_at, envelope_json)
                 VALUES (?1, 'invite', ?2, ?3, ?4, ?5)",
                params![
                    env.from.to_hex(),
                    addrs,
                    env.body.expires_at as i64,
                    Self::now(),
                    envelope_json
                ],
            )?;
        }
        Ok(serde_json::json!({
            "from": env.from.to_hex(),
            "visibility": "invite",
            "multiaddrs": env.body.multiaddrs
        }))
    }

    pub async fn accept_invite_and_redeem(
        &self,
        env: Envelope<PeerInvite>,
    ) -> Result<serde_json::Value, NodeError> {
        let accepted = self.accept_invite(env.clone())?;
        let ad_val = self.publish_self()?;
        let ad: Envelope<PeerAdvertisement> = serde_json::from_value(ad_val.clone())?;
        let req = PeerRedeemRequest {
            invite: env.clone(),
            advertisement: ad,
        };
        let mut redeemed = false;
        for m in &env.body.multiaddrs {
            let Some(base) = http_base(m) else {
                continue;
            };
            let Ok(resp) = self
                .http
                .post(format!("{base}/v0/peers/redeem"))
                .json(&req)
                .send()
                .await
            else {
                continue;
            };
            if resp.status().is_success() {
                redeemed = true;
            }
        }
        Ok(serde_json::json!({
            "accepted": accepted,
            "redeemed": redeemed
        }))
    }

    pub fn redeem_invite(&self, req: PeerRedeemRequest) -> Result<serde_json::Value, NodeError> {
        req.invite
            .verify()
            .map_err(|e| NodeError::Conflict(e.to_string()))?;
        req.advertisement
            .verify()
            .map_err(|e| NodeError::Conflict(e.to_string()))?;
        if req.invite.from != self.identity.pubkey() {
            return Err(NodeError::Conflict("invite was not issued by this node".into()));
        }
        if req.invite.body.expires_at as i64 <= Self::now() {
            return Err(NodeError::Conflict("invite expired".into()));
        }
        let id = req.invite.body.invite_id.clone();
        {
            let db = self.lock()?;
            let row: Option<(i64, i64, i64)> = db
                .query_row(
                    "SELECT once, expires_at, redeemed FROM peer_invites WHERE invite_id = ?1",
                    params![id],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((once, exp, redeemed)) = row else {
                return Err(NodeError::NotFound(format!("invite {id}")));
            };
            if exp <= Self::now() {
                return Err(NodeError::Conflict("invite expired".into()));
            }
            if once == 1 && redeemed == 1 {
                return Err(NodeError::Conflict("invite already used".into()));
            }
            if once == 1 {
                db.execute(
                    "UPDATE peer_invites SET redeemed = 1 WHERE invite_id = ?1",
                    params![id],
                )?;
            }
        }
        self.store_peer_envelope(&req.advertisement, PeerVisibility::Invite)?;
        Ok(serde_json::json!({
            "from": req.advertisement.from.to_hex(),
            "visibility": "invite"
        }))
    }

    pub async fn sync_public_peers(&self) -> Result<serde_json::Value, NodeError> {
        let mut urls = self.bootstrap.lock().unwrap().clone();
        for env in self.list_public_peer_envelopes()? {
            if let Some(addrs) = env.get("body").and_then(|b| b.get("multiaddrs")).and_then(|a| a.as_array()) {
                for m in addrs {
                    if let Some(s) = m.as_str() {
                        if let Some(base) = http_base(s) {
                            if !urls.contains(&base) {
                                urls.push(base);
                            }
                        }
                    }
                }
            }
        }
        let self_base = self.advertise_base();
        let mut ingested = 0u32;
        for base in urls {
            if self_base.as_deref() == Some(base.as_str()) {
                continue;
            }
            let Ok(resp) = self.http.get(format!("{base}/v0/peers")).send().await else {
                continue;
            };
            let Ok(v) = resp.json::<serde_json::Value>().await else {
                continue;
            };
            let Some(list) = v.get("peers").and_then(|p| p.as_array()) else {
                continue;
            };
            for item in list {
                if let Ok(env) =
                    serde_json::from_value::<Envelope<PeerAdvertisement>>(item.clone())
                {
                    if self.ingest_public_peer(env).is_ok() {
                        ingested += 1;
                    }
                }
            }
        }
        Ok(serde_json::json!({ "ingested": ingested }))
    }

    fn set_status(
        &self,
        job_id: &ContentId,
        from: JobStatus,
        to: JobStatus,
        hold: Option<i64>,
    ) -> Result<(), NodeError> {
        from.transition(to).map_err(|e| NodeError::Conflict(e.to_string()))?;
        let db = self.lock()?;
        let current: String = db.query_row(
            "SELECT status FROM jobs WHERE job_id = ?1",
            params![job_id.to_string()],
            |r| r.get(0),
        )?;
        if current != from.as_str() {
            return Err(NodeError::Conflict(format!(
                "job {} is {current}, expected {}",
                job_id,
                from.as_str()
            )));
        }
        if let Some(h) = hold {
            db.execute(
                "UPDATE jobs SET status = ?1, hold_millicredits = ?2, updated_at = ?3 WHERE job_id = ?4",
                params![to.as_str(), h, Self::now(), job_id.to_string()],
            )?;
        } else {
            db.execute(
                "UPDATE jobs SET status = ?1, updated_at = ?2 WHERE job_id = ?3",
                params![to.as_str(), Self::now(), job_id.to_string()],
            )?;
        }
        Ok(())
    }
}

fn labeled_mock_result(job_id: ContentId, input: &[u8], billed: i64) -> JobResult {
    let digest = ContentId::of_bytes(input);
    JobResult {
        job_id,
        status: JobStatus::Succeeded,
        output: InputRef::Inline {
            text: format!("keel-mock:{digest}"),
        },
        meter: UsageMeter {
            job_id,
            work: Work::Job,
            billed: Millicredits(billed.max(1)),
        },
    }
}

pub fn http_to_multiaddr(base: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.starts_with("/ip4/") || base.starts_with("/ip6/") {
        return base.to_string();
    }
    if let Some(rest) = base.strip_prefix("http://") {
        return match rest.split_once(':') {
            Some((host, port)) => format!("/ip4/{host}/tcp/{port}/http"),
            None => format!("/ip4/{rest}/tcp/80/http"),
        };
    }
    if let Some(rest) = base.strip_prefix("https://") {
        return match rest.split_once(':') {
            Some((host, port)) => format!("/ip4/{host}/tcp/{port}/https"),
            None => format!("/ip4/{rest}/tcp/443/https"),
        };
    }
    base.to_string()
}

pub fn http_base(multiaddr: &str) -> Option<String> {
    let trimmed = multiaddr.trim_end_matches('/');
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Some(trimmed.to_string());
    }
    let parts: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if parts.len() >= 4 && (parts[0] == "ip4" || parts[0] == "ip6") && parts[2] == "tcp" {
        let proto = if parts.get(4) == Some(&"https") {
            "https"
        } else {
            "http"
        };
        return Some(format!("{proto}://{}:{}", parts[1], parts[3]));
    }
    None
}

fn map_seeder(r: &rusqlite::Row<'_>) -> rusqlite::Result<SeederView> {
    Ok(SeederView {
        file_cid: r.get(0)?,
        multiaddr: r.get(1)?,
        announced_at: r.get(2)?,
        expires_at: r.get(3)?,
        from_key: r.get(4)?,
    })
}

fn mock_quote(input_len: usize, max: i64) -> i64 {
    let q = (input_len as i64).max(1);
    if max <= 0 {
        q
    } else {
        q.min(max)
    }
}

fn movement_id(account: &str, key: &str) -> String {
    let mut h = Sha256::new();
    h.update(account.as_bytes());
    h.update(key.as_bytes());
    hex::encode(h.finalize())
}

fn map_movement(r: &rusqlite::Row<'_>) -> rusqlite::Result<MovementView> {
    Ok(MovementView {
        id: r.get(0)?,
        account: r.get(1)?,
        kind: r.get(2)?,
        amount_millicredits: r.get(3)?,
        cause_type: r.get(4)?,
        cause_id: r.get(5)?,
        idempotency_key: r.get(6)?,
        created_at: r.get(7)?,
    })
}

fn map_job(r: &rusqlite::Row<'_>) -> rusqlite::Result<JobView> {
    let spec_s: String = r.get(3)?;
    let result_s: Option<String> = r.get(4)?;
    Ok(JobView {
        job_id: r.get(0)?,
        status: r.get(1)?,
        hold_millicredits: r.get(2)?,
        spec: serde_json::from_str(&spec_s).unwrap_or(serde_json::Value::Null),
        result: result_s.and_then(|s| serde_json::from_str(&s).ok()),
        updated_at: r.get(5)?,
    })
}
