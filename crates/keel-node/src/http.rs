use crate::ln::{PayIntentRequest, PaySettleRequest};
use crate::node::{MintRequest, Node, NodeError, PeerRedeemRequest};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use keel_types::{
    ArtifactManifest, ContentId, Envelope, JobSpec, Kind, PeerAdvertisement, PeerInvite,
    PeerVisibility, SeederRecord,
};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::CorsLayer;

pub const DEFAULT_BIND: &str = "127.0.0.1:7420";

#[derive(Clone)]
pub struct AppState {
    pub node: Arc<Node>,
}

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/v0", get(api_index))
        .route("/v0/health", get(health))
        .route("/v0/status", get(status))
        .route("/v0/identity", get(identity))
        .route("/v0/blobs", get(list_blobs).post(put_blob))
        .route("/v0/blobs/{cid}", get(get_blob))
        .route("/v0/accounts", get(list_accounts).post(new_account))
        .route("/v0/accounts/{account}", get(get_account))
        .route("/v0/movements", get(list_all_movements))
        .route("/v0/credits/{account}", get(get_account_path))
        .route("/v0/credits/{account}/movements", get(list_movements))
        .route("/v0/credits/mint", post(mint))
        .route("/v0/jobs", get(list_jobs).post(submit_job))
        .route("/v0/jobs/expire", post(expire_jobs))
        .route("/v0/jobs/{id}", get(get_job))
        .route("/v0/jobs/{id}/accept", post(accept_job))
        .route("/v0/jobs/{id}/run", post(run_job))
        .route("/v0/artifacts", get(list_artifacts).post(put_artifact))
        .route("/v0/artifacts/{cid}", get(get_artifact))
        .route("/v0/indexes", get(list_indexes).post(post_index))
        .route("/v0/indexes/{publisher}/head", get(index_head))
        .route("/v0/indexes/{publisher}", get(indexes_for))
        .route("/v0/seeders", get(list_seeders).post(put_seeder))
        .route("/v0/seeders/{cid}", get(get_seeders))
        .route("/v0/filters", get(list_filters).post(put_filter))
        .route("/v0/filters/{cid}", get(get_filter))
        .route("/v0/runners", get(list_runners).post(put_runner))
        .route("/v0/peers", get(list_public_peers).post(ingest_peer))
        .route("/v0/peers/known", get(list_known_peers))
        .route("/v0/peers/sync", post(sync_peers))
        .route("/v0/peers/invite", post(create_invite))
        .route("/v0/peers/accept", post(accept_invite))
        .route("/v0/peers/redeem", post(redeem_invite))
        .route("/v0/pay/intent", post(pay_intent))
        .route("/v0/pay/settle", post(pay_settle))
        .route("/v0/pay/intents", get(list_intents))
        .with_state(AppState { node })
        .layer(CorsLayer::permissive())
}

pub async fn serve(bind: SocketAddr, data_dir: PathBuf) -> anyhow::Result<()> {
    serve_with(bind, data_dir, PeerVisibility::Invite, Vec::new()).await
}

pub async fn serve_with(
    bind: SocketAddr,
    data_dir: PathBuf,
    visibility: PeerVisibility,
    bootstrap: Vec<String>,
) -> anyhow::Result<()> {
    let node = Arc::new(Node::open(data_dir)?);
    node.set_visibility(visibility);
    for b in bootstrap {
        node.add_bootstrap(b);
    }
    let listener = tokio::net::TcpListener::bind(bind).await?;
    serve_listener(listener, node).await
}

pub async fn serve_listener(
    listener: tokio::net::TcpListener,
    node: Arc<Node>,
) -> anyhow::Result<()> {
    let addr = listener.local_addr()?;
    node.set_advertise_base(format!("http://{addr}"));
    let _ = node.publish_self();
    let sync = node.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let _ = sync.sync_public_peers().await;
        }
    });
    axum::serve(listener, router(node)).await?;
    Ok(())
}

async fn dashboard() -> Html<&'static str> {
    Html(include_str!("dashboard.html"))
}

async fn api_index() -> Json<Value> {
    Json(json!({
        "keel": keel_types::KEEL_VERSION,
        "note": "This JSON is the operator API. The HTML dashboard at / only GETs these routes. Bring your own frontend by speaking the same paths.",
        "resources": {
            "health": "GET /v0/health",
            "status": "GET /v0/status",
            "identity": "GET /v0/identity",
            "blobs": "GET|POST /v0/blobs",
            "blob": "GET /v0/blobs/{sha256hex}",
            "accounts": "GET|POST /v0/accounts",
            "account": "GET /v0/accounts/{hex_pubkey}",
            "mint": "POST /v0/credits/mint",
            "movements": "GET /v0/movements",
            "credit": "GET /v0/credits/{hex_pubkey}",
            "jobs": "GET|POST /v0/jobs",
            "job": "GET /v0/jobs/{sha256hex}",
            "accept": "POST /v0/jobs/{sha256hex}/accept",
            "run": "POST /v0/jobs/{sha256hex}/run",
            "expire": "POST /v0/jobs/expire",
            "artifacts": "GET|POST /v0/artifacts",
            "artifact": "GET /v0/artifacts/{sha256hex}",
            "indexes": "GET|POST /v0/indexes",
            "index": "GET /v0/indexes/{publisher}",
            "index_head": "GET /v0/indexes/{publisher}/head",
            "seeders": "GET|POST /v0/seeders",
            "seeder": "GET /v0/seeders/{sha256hex}",
            "filters": "GET|POST /v0/filters",
            "filter": "GET /v0/filters/{sha256hex}",
            "runners": "GET|POST /v0/runners",
            "peers": "GET|POST /v0/peers",
            "peers_known": "GET /v0/peers/known",
            "peers_sync": "POST /v0/peers/sync",
            "peer_invite": "POST /v0/peers/invite",
            "peer_accept": "POST /v0/peers/accept",
            "peer_redeem": "POST /v0/peers/redeem",
            "pay_intent": "POST /v0/pay/intent",
            "pay_settle": "POST /v0/pay/settle",
            "pay_intents": "GET /v0/pay/intents"
        }
    }))
}

async fn health() -> Json<Value> {
    Json(json!({ "ok": true, "keel": keel_types::KEEL_VERSION }))
}

async fn status(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.status()?)?))
}

async fn identity(State(st): State<AppState>) -> Json<Value> {
    Json(st.node.identity_view())
}

async fn list_blobs(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "blobs": st.node.list_blobs()? })))
}

async fn put_blob(State(st): State<AppState>, body: Bytes) -> Result<Json<Value>, ApiError> {
    let view = st.node.put_blob(&body).await?;
    Ok(Json(serde_json::to_value(view)?))
}

async fn get_blob(
    State(st): State<AppState>,
    Path(cid): Path<String>,
) -> Result<Response, ApiError> {
    let cid = ContentId::from_hex(&cid).map_err(NodeError::Msg)?;
    let bytes = st.node.get_blob(&cid).await?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    headers.insert(
        header::HeaderName::from_static("x-keel-cid"),
        header::HeaderValue::from_str(&cid.to_string())
            .unwrap_or(header::HeaderValue::from_static("ok")),
    );
    Ok((headers, bytes).into_response())
}

async fn list_accounts(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "accounts": st.node.list_accounts()? })))
}

async fn new_account(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.new_account()?)?))
}

async fn get_account(
    State(st): State<AppState>,
    Path(account): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.get_account(&account)?)?))
}

async fn get_account_path(
    State(st): State<AppState>,
    Path(account): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.get_account(&account)?)?))
}

async fn list_movements(
    State(st): State<AppState>,
    Path(account): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "movements": st.node.list_movements(Some(&account))? })))
}

async fn list_all_movements(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "movements": st.node.list_movements(None)? })))
}

async fn mint(State(st): State<AppState>, Json(req): Json<MintRequest>) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.mint(&req)?)?))
}

async fn list_jobs(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "jobs": st.node.list_jobs()? })))
}

async fn submit_job(
    State(st): State<AppState>,
    Json(spec): Json<JobSpec>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.submit_job(spec)?)?))
}

async fn get_job(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let id = ContentId::from_hex(&id).map_err(NodeError::Msg)?;
    Ok(Json(serde_json::to_value(st.node.get_job(&id)?)?))
}

async fn accept_job(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let id = ContentId::from_hex(&id).map_err(NodeError::Msg)?;
    Ok(Json(serde_json::to_value(st.node.accept_job(&id)?)?))
}

async fn run_job(State(st): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let id = ContentId::from_hex(&id).map_err(NodeError::Msg)?;
    Ok(Json(serde_json::to_value(st.node.run_job(&id).await?)?))
}

async fn expire_jobs(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    let n = st.node.expire_stale()?;
    Ok(Json(json!({ "expired": n })))
}

async fn put_artifact(
    State(st): State<AppState>,
    Json(m): Json<ArtifactManifest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(st.node.put_artifact(m)?))
}

async fn list_artifacts(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "artifacts": st.node.list_artifacts()? })))
}

async fn get_artifact(
    State(st): State<AppState>,
    Path(cid): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let cid = ContentId::from_hex(&cid).map_err(NodeError::Msg)?;
    Ok(Json(st.node.get_artifact(&cid)?))
}

fn envelope_or_sign<T: DeserializeOwned + Serialize>(
    node: &Node,
    kind: Kind,
    v: Value,
) -> Result<Envelope<T>, NodeError> {
    if v.get("sig").is_some() {
        serde_json::from_value(v).map_err(NodeError::Json)
    } else {
        let body: T = serde_json::from_value(v).map_err(NodeError::Json)?;
        node.sign_envelope(kind, body)
    }
}

async fn post_index(State(st): State<AppState>, Json(v): Json<Value>) -> Result<Json<Value>, ApiError> {
    let env = envelope_or_sign(&st.node, Kind::IndexPublish, v)?;
    Ok(Json(st.node.publish_index(env)?))
}

async fn list_indexes(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "indexes": st.node.list_indexes()? })))
}

async fn indexes_for(
    State(st): State<AppState>,
    Path(publisher): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "indexes": st.node.indexes_for(&publisher)? })))
}

async fn index_head(
    State(st): State<AppState>,
    Path(publisher): Path<String>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(st.node.index_head(&publisher)?))
}

async fn put_seeder(
    State(st): State<AppState>,
    Json(rec): Json<SeederRecord>,
) -> Result<Json<Value>, ApiError> {
    st.node.put_seeder(rec)?;
    Ok(Json(json!({ "ok": true })))
}

async fn list_seeders(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "seeders": st.node.list_seeders(None)? })))
}

async fn get_seeders(
    State(st): State<AppState>,
    Path(cid): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let cid = ContentId::from_hex(&cid).map_err(NodeError::Msg)?;
    Ok(Json(json!({ "seeders": st.node.list_seeders(Some(&cid))? })))
}

async fn put_filter(State(st): State<AppState>, Json(v): Json<Value>) -> Result<Json<Value>, ApiError> {
    let env = envelope_or_sign(&st.node, Kind::FilterPublish, v)?;
    Ok(Json(st.node.put_filter(env)?))
}

async fn list_filters(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "filters": st.node.list_filters()? })))
}

async fn get_filter(
    State(st): State<AppState>,
    Path(cid): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let cid = ContentId::from_hex(&cid).map_err(NodeError::Msg)?;
    Ok(Json(st.node.get_filter(&cid)?))
}

async fn put_runner(State(st): State<AppState>, Json(v): Json<Value>) -> Result<Json<Value>, ApiError> {
    let env = envelope_or_sign(&st.node, Kind::RunnerAnnounce, v)?;
    Ok(Json(st.node.put_runner(env)?))
}

async fn list_runners(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "runners": st.node.list_runners()? })))
}

async fn list_public_peers(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "peers": st.node.list_public_peer_envelopes()? })))
}

async fn list_known_peers(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "peers": st.node.list_known_peers()? })))
}

async fn ingest_peer(
    State(st): State<AppState>,
    Json(env): Json<Envelope<PeerAdvertisement>>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(st.node.ingest_public_peer(env)?))
}

#[derive(serde::Deserialize)]
struct SyncPeersBody {
    #[serde(default)]
    urls: Vec<String>,
}

async fn sync_peers(
    State(st): State<AppState>,
    body: Option<Json<SyncPeersBody>>,
) -> Result<Json<Value>, ApiError> {
    if let Some(Json(b)) = body {
        for u in b.urls {
            st.node.add_bootstrap(u);
        }
    }
    Ok(Json(st.node.sync_public_peers().await?))
}

#[derive(serde::Deserialize)]
struct InviteBody {
    #[serde(default)]
    once: bool,
}

async fn create_invite(
    State(st): State<AppState>,
    body: Option<Json<InviteBody>>,
) -> Result<Json<Value>, ApiError> {
    let once = body.map(|j| j.once).unwrap_or(false);
    let env = st.node.create_invite(once)?;
    Ok(Json(serde_json::to_value(env)?))
}

async fn accept_invite(
    State(st): State<AppState>,
    Json(env): Json<Envelope<PeerInvite>>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(st.node.accept_invite_and_redeem(env).await?))
}

async fn redeem_invite(
    State(st): State<AppState>,
    Json(req): Json<PeerRedeemRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(st.node.redeem_invite(req)?))
}

async fn pay_intent(
    State(st): State<AppState>,
    Json(req): Json<PayIntentRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(st.node.pay_intent(req).await?)?))
}

async fn pay_settle(
    State(st): State<AppState>,
    Json(req): Json<PaySettleRequest>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(serde_json::to_value(
        st.node.pay_settle(&req.payment_hash, &req.preimage).await?,
    )?))
}

async fn list_intents(State(st): State<AppState>) -> Result<Json<Value>, ApiError> {
    Ok(Json(json!({ "intents": st.node.list_intents()? })))
}

struct ApiError(NodeError);

impl From<NodeError> for ApiError {
    fn from(e: NodeError) -> Self {
        Self(e)
    }
}

impl From<serde_json::Error> for ApiError {
    fn from(e: serde_json::Error) -> Self {
        Self(NodeError::Json(e))
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let code = match self.0.status_code() {
            404 => StatusCode::NOT_FOUND,
            409 => StatusCode::CONFLICT,
            _ => StatusCode::BAD_REQUEST,
        };
        let body = Json(ErrorBody {
            error: self.0.to_string(),
        });
        (code, body).into_response()
    }
}

#[derive(Serialize)]
struct ErrorBody {
    error: String,
}
