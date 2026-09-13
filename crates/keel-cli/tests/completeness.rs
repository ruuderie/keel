//! Completeness: two nodes, signed index, CID fetch, Lightning mint, CID runner.
//! Never fetch GET /. CLI JSON equals GET /v0/status.

use keel_node::{serve_listener, Node};
use keel_runner::fixture_weights;
use keel_sdk::{
    ArtifactFile, ArtifactManifest, Client, ContentId, HardwareHint, IndexEntry, InputRef, JobSpec,
    Millicredits, ModelIndex, Pubkey,
};
use serde_json::Value;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

async fn wait_health(base: &str) {
    for _ in 0..80 {
        if reqwest::get(format!("{base}/v0/health")).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("node at {base} did not become healthy");
}

async fn spawn() -> (String, Arc<Node>) {
    let path = tempfile::tempdir().unwrap().keep();
    let node = Arc::new(Node::open(&path).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let serve = node.clone();
    tokio::spawn(async move {
        serve_listener(listener, serve).await.unwrap();
    });
    let api = format!("http://{addr}");
    wait_health(&api).await;
    (api, node)
}

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

async fn json(url: &str) -> Value {
    http()
        .get(url)
        .send()
        .await
        .expect("get")
        .json()
        .await
        .expect("json")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn proof_v0_completeness() {
    let (a, _node_a) = spawn().await;
    let (b, node_b) = spawn().await;

    let weights = fixture_weights();
    let put = http()
        .post(format!("{a}/v0/blobs"))
        .body(weights.clone())
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let model = ContentId::from_hex(put["cid"].as_str().unwrap()).unwrap();
    let model_hex = model.to_hex();

    let manifest = ArtifactManifest {
        schema: ArtifactManifest::v0().into(),
        name: "cid-weights".into(),
        license: "MIT".into(),
        files: vec![ArtifactFile {
            path: "weights.bin".into(),
            cid: model,
            size_bytes: weights.len() as u64,
            media: Some("keelw001".into()),
        }],
        hardware: HardwareHint {
            min_vram_mb: 0,
            quant: None,
            backend: vec!["cid-weights".into()],
        },
        signatures: vec![],
    };
    let art = http()
        .post(format!("{a}/v0/artifacts"))
        .json(&manifest)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let artifact_cid = ContentId::from_hex(art["cid"].as_str().unwrap()).unwrap();

    let index = ModelIndex {
        schema: ModelIndex::v0().into(),
        seq: 1,
        prev: None,
        entries: vec![IndexEntry {
            alias: "cid-weights".into(),
            artifact_cid,
        }],
    };
    let published = http()
        .post(format!("{a}/v0/indexes"))
        .json(&index)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(published["seq"], 1);
    let publisher = published["publisher"].as_str().unwrap().to_string();

    let head = json(&format!("{a}/v0/indexes/{publisher}/head")).await;
    assert_eq!(head["body"]["seq"], 1);
    let copied = http()
        .post(format!("{b}/v0/indexes"))
        .json(&head)
        .send()
        .await
        .unwrap();
    assert!(copied.status().is_success(), "{}", copied.status());
    let b_head = json(&format!("{b}/v0/indexes/{publisher}/head")).await;
    assert_eq!(b_head["body"]["entries"][0]["artifact_cid"], art["cid"]);

    let seeders = json(&format!("{a}/v0/seeders/{model_hex}")).await;
    for s in seeders["seeders"].as_array().unwrap() {
        let rec = serde_json::json!({
            "file_cid": model.to_string(),
            "multiaddrs": [s["multiaddr"]],
            "expires_at": s["expires_at"].as_u64().unwrap_or(u64::MAX)
        });
        http()
            .post(format!("{b}/v0/seeders"))
            .json(&rec)
            .send()
            .await
            .unwrap();
    }

    let fetched = http()
        .get(format!("{b}/v0/blobs/{model_hex}"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&fetched[..], weights.as_slice());

    let acct = http()
        .post(format!("{b}/v0/accounts"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let hex = acct["account"].as_str().unwrap().to_string();

    let intent = http()
        .post(format!("{b}/v0/pay/intent"))
        .json(&serde_json::json!({
            "account": hex,
            "millicredits_on_confirm": 5000,
            "amount_sats": 1
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let bolt11 = intent["bolt11"].as_str().unwrap();
    assert!(bolt11.to_lowercase().starts_with("lnbcrt"), "{bolt11}");
    let ph = intent["payment_hash"].as_str().unwrap().to_string();
    let pre = node_b.test_preimage(&ph).expect("in-process preimage");
    let settled = http()
        .post(format!("{b}/v0/pay/settle"))
        .json(&serde_json::json!({
            "payment_hash": ph,
            "preimage": pre
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(settled["balance_millicredits"], 5000);

    let moves = json(&format!("{b}/v0/credits/{hex}/movements")).await;
    let key = moves["movements"][0]["idempotency_key"].as_str().unwrap();
    assert!(
        key.starts_with("mint:btc_lightning:"),
        "{key} must mint on payment_hash, not mock"
    );

    let spec = JobSpec {
        schema: JobSpec::v0().into(),
        model,
        input: InputRef::Inline {
            text: "hello keel".into(),
        },
        max_millicredits: Millicredits(100),
        payer: Pubkey::from_hex(&hex).unwrap(),
        nonce: "complete".into(),
    };
    let submitted = http()
        .post(format!("{b}/v0/jobs"))
        .json(&spec)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    let job_hex = submitted["job_id"]
        .as_str()
        .unwrap()
        .trim_start_matches("sha256:");
    http()
        .post(format!("{b}/v0/jobs/{job_hex}/accept"))
        .send()
        .await
        .unwrap();
    let ran = http()
        .post(format!("{b}/v0/jobs/{job_hex}/run"))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(ran["status"], "succeeded", "{ran}");
    let out = ran["result"]["output"]["text"].as_str().unwrap();
    assert!(out.starts_with("cid-weights:"), "{out}");
    assert!(!out.starts_with("keel-mock:"), "{out}");

    let bin = env!("CARGO_BIN_EXE_keel");
    let cli = Command::new(bin)
        .args(["--api", &b, "status"])
        .output()
        .unwrap();
    assert!(
        cli.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&cli.stderr)
    );
    let via_cli: Value = serde_json::from_slice(&cli.stdout).unwrap();
    let via_http = Client::new(&b).status().await.unwrap();
    assert_eq!(via_cli["keel"], via_http["keel"]);
    assert_eq!(via_cli["blobs"], via_http["blobs"]);
    assert_eq!(via_cli["jobs"], via_http["jobs"]);

    let still = http()
        .get(format!("{b}/v0/blobs/{model_hex}"))
        .send()
        .await
        .unwrap()
        .bytes()
        .await
        .unwrap();
    assert_eq!(&still[..], weights.as_slice());
}
