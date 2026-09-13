//! Proofs that the node API is the product.
//! The dashboard is not required. A second HTTP client is a valid frontend.

use keel_node::{router, serve_listener, Node};
use keel_types::{ContentId, InputRef, JobSpec, Millicredits, Pubkey};
use serde_json::Value;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

async fn spawn_node() -> (SocketAddr, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Arc::new(Node::open(dir.path()).expect("open node"));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let app = router(node);
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    (addr, dir)
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

async fn json(url: &str) -> Value {
    http().get(url).send().await.expect("get").json().await.expect("json")
}

#[tokio::test]
async fn proof_job_via_http_without_dashboard() {
    let (addr, _dir) = spawn_node().await;
    let base = format!("http://{addr}");

    // BYO frontend #1: never fetch GET /
    let health = json(&format!("{base}/v0/health")).await;
    assert_eq!(health["ok"], true);

    let account = http()
        .post(format!("{base}/v0/accounts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let hex = account["account"].as_str().unwrap().to_string();

    let minted = http()
        .post(format!("{base}/v0/credits/mint"))
        .json(&serde_json::json!({
            "account": hex,
            "amount_millicredits": 5000,
            "rail_ref": "mock:test-1"
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(minted["balance_millicredits"], 5000);

    // mint is idempotent
    let minted2 = http()
        .post(format!("{base}/v0/credits/mint"))
        .json(&serde_json::json!({
            "account": hex,
            "amount_millicredits": 5000,
            "rail_ref": "mock:test-1"
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(minted2["balance_millicredits"], 5000);

    let blob = http()
        .post(format!("{base}/v0/blobs"))
        .body(b"fake-gguf-weights".to_vec())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let cid = blob["cid"].as_str().unwrap();
    assert!(cid.starts_with("sha256:"));

    let fetched = http()
        .get(format!(
            "{base}/v0/blobs/{}",
            cid.trim_start_matches("sha256:")
        ))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&fetched[..], b"fake-gguf-weights");

    let model = ContentId::from_hex(cid).unwrap();
    let payer = Pubkey::from_hex(&hex).unwrap();
    let spec = JobSpec {
        schema: JobSpec::v0().into(),
        model,
        input: InputRef::Inline {
            text: "hello keel".into(),
        },
        max_millicredits: Millicredits(100),
        payer,
        nonce: "n1".into(),
    };
    let submitted = http()
        .post(format!("{base}/v0/jobs"))
        .json(&spec)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(submitted["status"], "specified");
    let job_id = submitted["job_id"].as_str().unwrap();
    let job_hex = job_id.trim_start_matches("sha256:");

    let ran = http()
        .post(format!("{base}/v0/jobs/{job_hex}/run"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(ran["status"], "succeeded");
    let out = ran["result"]["output"]["text"].as_str().unwrap();
    assert!(out.starts_with("keel-mock:"), "{out}");

    let after = json(&format!("{base}/v0/credits/{hex}")).await;
    assert!(after["balance_millicredits"].as_i64().unwrap() < 5000);
    assert_eq!(after["held_millicredits"], 0);

    // BYO frontend #2: another client, same JSON
    let jobs = json(&format!("{base}/v0/jobs")).await;
    assert_eq!(jobs["jobs"][0]["status"], "succeeded");
}

#[tokio::test]
async fn proof_dashboard_is_only_a_client() {
    let (addr, _dir) = spawn_node().await;
    let html = http()
        .get(format!("http://{addr}/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains("/v0/status"));
    assert!(html.contains("/v0/jobs"));
    assert!(html.contains("Kill this tab"));

    let catalog = json(&format!("http://{addr}/v0")).await;
    assert!(catalog["resources"]["status"].as_str().unwrap().contains("/v0/status"));

    let cors = http()
        .request(
            reqwest::Method::OPTIONS,
            format!("http://{addr}/v0/status"),
        )
        .header("origin", "http://localhost:5173")
        .header("access-control-request-method", "GET")
        .send()
        .await
        .unwrap();
    assert!(
        cors.headers()
            .get("access-control-allow-origin")
            .is_some(),
        "BYO frontends on another origin need CORS"
    );
}

async fn wait_health(base: &str) {
    for _ in 0..80 {
        if reqwest::get(format!("{base}/v0/health")).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("node at {base} did not become healthy");
}

async fn spawn_advertised() -> (SocketAddr, Arc<Node>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Arc::new(Node::open(dir.path()).expect("open node"));
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_node = node.clone();
    tokio::spawn(async move {
        serve_listener(listener, serve_node).await.expect("serve");
    });
    wait_health(&format!("http://{addr}")).await;
    (addr, node, dir)
}

#[tokio::test]
async fn proof_two_nodes_fetch_cid() {
    let (addr_a, _node_a, _da) = spawn_advertised().await;
    let (addr_b, _node_b, _db) = spawn_advertised().await;
    let a = format!("http://{addr_a}");
    let b = format!("http://{addr_b}");

    let payload = b"two-node-cid-bytes";
    let put = http()
        .post(format!("{a}/v0/blobs"))
        .body(payload.to_vec())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let cid = put["cid"].as_str().unwrap().to_string();
    let hex = cid.trim_start_matches("sha256:");

    let seeders = json(&format!("{a}/v0/seeders/{hex}")).await;
    let list = seeders["seeders"].as_array().expect("seeders");
    assert!(!list.is_empty(), "node A must announce a seeder on put");
    for s in list {
        let rec = serde_json::json!({
            "file_cid": cid,
            "multiaddrs": [s["multiaddr"]],
            "expires_at": s["expires_at"].as_u64().unwrap_or(u64::MAX)
        });
        let ok = http()
            .post(format!("{b}/v0/seeders"))
            .json(&rec)
            .send()
            .await
            .unwrap();
        assert!(ok.status().is_success(), "{}", ok.status());
    }

    let fetched = http()
        .get(format!("{b}/v0/blobs/{hex}"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&fetched[..], payload);

    let listed = json(&format!("{b}/v0/blobs")).await;
    assert!(listed["blobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["cid"] == cid));
}

async fn spawn_with_visibility(
    vis: keel_types::PeerVisibility,
) -> (SocketAddr, Arc<Node>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let node = Arc::new(Node::open(dir.path()).expect("open node"));
    node.set_visibility(vis);
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let addr = listener.local_addr().expect("addr");
    let serve_node = node.clone();
    tokio::spawn(async move {
        serve_listener(listener, serve_node).await.expect("serve");
    });
    wait_health(&format!("http://{addr}")).await;
    (addr, node, dir)
}

#[tokio::test]
async fn proof_public_peers_sync_then_fetch_cid() {
    use keel_types::PeerVisibility;
    let (addr_a, _na, _da) = spawn_with_visibility(PeerVisibility::Public).await;
    let (addr_b, node_b, _db) = spawn_with_visibility(PeerVisibility::Public).await;
    let a = format!("http://{addr_a}");
    let b = format!("http://{addr_b}");

    let payload = b"public-peer-fetch";
    http()
        .post(format!("{a}/v0/blobs"))
        .body(payload.to_vec())
        .send()
        .await
        .unwrap();
    let cid = keel_types::ContentId::of_bytes(payload);

    node_b.add_bootstrap(&a);
    let synced = http()
        .post(format!("{b}/v0/peers/sync"))
        .json(&serde_json::json!({ "urls": [&a] }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert!(synced["ingested"].as_u64().unwrap() >= 1, "{synced}");

    let public = json(&format!("{b}/v0/peers")).await;
    let peers = public["peers"].as_array().unwrap();
    assert!(
        peers.iter().any(|p| p["body"]["visibility"] == "public"),
        "{public}"
    );

    let fetched = http()
        .get(format!("{b}/v0/blobs/{}", cid.to_hex()))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&fetched[..], payload);
}

#[tokio::test]
async fn proof_invite_only_not_on_public_list() {
    use keel_types::PeerVisibility;
    let (addr_pub, _np, _dp) = spawn_with_visibility(PeerVisibility::Public).await;
    let (addr_c, _nc, _dc) = spawn_with_visibility(PeerVisibility::Invite).await;
    let (addr_d, _nd, _dd) = spawn_with_visibility(PeerVisibility::Invite).await;
    let pub_api = format!("http://{addr_pub}");
    let c = format!("http://{addr_c}");
    let d = format!("http://{addr_d}");

    let invite = http()
        .post(format!("{c}/v0/peers/invite"))
        .json(&serde_json::json!({ "once": true }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(invite["kind"], "peer.invite");

    let accepted = http()
        .post(format!("{d}/v0/peers/accept"))
        .json(&invite)
        .send()
        .await
        .unwrap();
    assert!(accepted.status().is_success(), "{}", accepted.status());

    let c_public = json(&format!("{c}/v0/peers")).await;
    let c_list = c_public["peers"].as_array().cloned().unwrap_or_default();
    let c_id = json(&format!("{c}/v0/identity")).await;
    let c_pk = c_id["pubkey"].as_str().unwrap();
    assert!(
        !c_list.iter().any(|p| p["from"] == c_pk),
        "invite-only node must not list itself as public: {c_public}"
    );

    let known_d = json(&format!("{d}/v0/peers/known")).await;
    assert!(
        known_d["peers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["from"] == c_pk),
        "{known_d}"
    );

    http()
        .post(format!("{pub_api}/v0/peers/sync"))
        .json(&serde_json::json!({ "urls": [&c] }))
        .send()
        .await
        .unwrap();
    let pub_peers = json(&format!("{pub_api}/v0/peers")).await;
    assert!(
        !pub_peers["peers"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["from"] == c_pk),
        "public directory must not learn invite-only node from its /v0/peers: {pub_peers}"
    );

    let mut bad = invite.clone();
    bad["body"]["multiaddrs"] = serde_json::json!(["/ip4/10.0.0.1/tcp/1/http"]);
    let tamper = http()
        .post(format!("{d}/v0/peers/accept"))
        .json(&bad)
        .send()
        .await
        .unwrap();
    assert_eq!(tamper.status(), 409, "tampered invite must fail verify");
}
