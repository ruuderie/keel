//! HTTP client for a Keel node. CLI and BYO frontends use the same paths.

use keel_types::{ContentId, JobSpec};
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ClientError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error("{status}: {message}")]
    Api { status: u16, message: String },
    #[error("{0}")]
    Msg(String),
}

#[derive(Clone)]
pub struct Client {
    base: String,
    http: reqwest::Client,
}

impl Client {
    pub fn new(base: impl Into<String>) -> Self {
        Self {
            base: base.into().trim_end_matches('/').to_string(),
            http: reqwest::Client::new(),
        }
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    async fn json(&self, res: reqwest::Response) -> Result<Value, ClientError> {
        let status = res.status();
        let v = res.json::<Value>().await?;
        if !status.is_success() {
            let message = v
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("request failed")
                .to_string();
            return Err(ClientError::Api {
                status: status.as_u16(),
                message,
            });
        }
        Ok(v)
    }

    pub async fn health(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/health", self.base)).send().await?)
            .await
    }

    pub async fn status(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/status", self.base)).send().await?)
            .await
    }

    pub async fn api_index(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0", self.base)).send().await?)
            .await
    }

    pub async fn put_blob(&self, bytes: &[u8]) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/blobs", self.base))
                .header("content-type", "application/octet-stream")
                .body(bytes.to_vec())
                .send()
                .await?,
        )
        .await
    }

    pub async fn get_blob(&self, cid: &ContentId) -> Result<Vec<u8>, ClientError> {
        let res = self
            .http
            .get(format!("{}/v0/blobs/{}", self.base, cid.to_hex()))
            .send()
            .await?;
        if !res.status().is_success() {
            return Err(ClientError::Api {
                status: res.status().as_u16(),
                message: "blob not found".into(),
            });
        }
        Ok(res.bytes().await?.to_vec())
    }

    pub async fn list_blobs(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/blobs", self.base)).send().await?)
            .await
    }

    pub async fn new_account(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/accounts", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_accounts(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/accounts", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn mint(
        &self,
        account: &str,
        amount_millicredits: i64,
        rail_ref: &str,
    ) -> Result<Value, ClientError> {
        self.mint_on_rail(account, amount_millicredits, rail_ref, "mock")
            .await
    }

    pub async fn mint_on_rail(
        &self,
        account: &str,
        amount_millicredits: i64,
        rail_ref: &str,
        rail: &str,
    ) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/credits/mint", self.base))
                .json(&serde_json::json!({
                    "account": account,
                    "amount_millicredits": amount_millicredits,
                    "rail_ref": rail_ref,
                    "rail": rail
                }))
                .send()
                .await?,
        )
        .await
    }

    pub async fn get_account(&self, account: &str) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/credits/{account}", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_movements(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/movements", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn submit_job(&self, spec: &JobSpec) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/jobs", self.base))
                .json(spec)
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_jobs(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/jobs", self.base)).send().await?)
            .await
    }

    pub async fn get_job(&self, job_id: &ContentId) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/jobs/{}", self.base, job_id.to_hex()))
                .send()
                .await?,
        )
        .await
    }

    pub async fn run_job(&self, job_id: &ContentId) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/jobs/{}/run", self.base, job_id.to_hex()))
                .send()
                .await?,
        )
        .await
    }

    pub async fn accept_job(&self, job_id: &ContentId) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/jobs/{}/accept", self.base, job_id.to_hex()))
                .send()
                .await?,
        )
        .await
    }

    pub async fn expire_jobs(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/jobs/expire", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn identity(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/identity", self.base)).send().await?)
            .await
    }

    pub async fn put_artifact(&self, body: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/artifacts", self.base))
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn get_artifact(&self, cid: &ContentId) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/artifacts/{}", self.base, cid.to_hex()))
                .send()
                .await?,
        )
        .await
    }

    pub async fn publish_index(&self, body: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/indexes", self.base))
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn index_head(&self, publisher: &str) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/indexes/{publisher}/head", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_indexes(&self, publisher: Option<&str>) -> Result<Value, ClientError> {
        let url = match publisher {
            Some(p) => format!("{}/v0/indexes/{p}", self.base, p = p),
            None => format!("{}/v0/indexes", self.base),
        };
        self.json(self.http.get(url).send().await?).await
    }

    pub async fn put_seeder(&self, body: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/seeders", self.base))
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_seeders(&self, cid: Option<&ContentId>) -> Result<Value, ClientError> {
        let url = match cid {
            Some(c) => format!("{}/v0/seeders/{}", self.base, c.to_hex()),
            None => format!("{}/v0/seeders", self.base),
        };
        self.json(self.http.get(url).send().await?).await
    }

    pub async fn put_filter(&self, body: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/filters", self.base))
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn get_filter(&self, cid: &ContentId) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/filters/{}", self.base, cid.to_hex()))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_filters(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/filters", self.base)).send().await?)
            .await
    }

    pub async fn put_runner(&self, body: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/runners", self.base))
                .json(body)
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_runners(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/runners", self.base)).send().await?)
            .await
    }

    pub async fn pay_intent(
        &self,
        account: &str,
        millicredits_on_confirm: i64,
        amount_sats: u64,
    ) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/pay/intent", self.base))
                .json(&serde_json::json!({
                    "account": account,
                    "millicredits_on_confirm": millicredits_on_confirm,
                    "amount_sats": amount_sats
                }))
                .send()
                .await?,
        )
        .await
    }

    pub async fn pay_settle(&self, payment_hash: &str, preimage: &str) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/pay/settle", self.base))
                .json(&serde_json::json!({
                    "payment_hash": payment_hash,
                    "preimage": preimage
                }))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_intents(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/pay/intents", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn list_public_peers(&self) -> Result<Value, ClientError> {
        self.json(self.http.get(format!("{}/v0/peers", self.base)).send().await?)
            .await
    }

    pub async fn list_known_peers(&self) -> Result<Value, ClientError> {
        self.json(
            self.http
                .get(format!("{}/v0/peers/known", self.base))
                .send()
                .await?,
        )
        .await
    }

    pub async fn ingest_peer(&self, env: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/peers", self.base))
                .json(env)
                .send()
                .await?,
        )
        .await
    }

    pub async fn sync_peers(&self, urls: &[String]) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/peers/sync", self.base))
                .json(&serde_json::json!({ "urls": urls }))
                .send()
                .await?,
        )
        .await
    }

    pub async fn create_invite(&self, once: bool) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/peers/invite", self.base))
                .json(&serde_json::json!({ "once": once }))
                .send()
                .await?,
        )
        .await
    }

    pub async fn accept_invite(&self, invite: &Value) -> Result<Value, ClientError> {
        self.json(
            self.http
                .post(format!("{}/v0/peers/accept", self.base))
                .json(invite)
                .send()
                .await?,
        )
        .await
    }
}
