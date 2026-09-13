//! CLI is a client of the same HTTP API as the dashboard.

use keel_node::{serve_listener, Node};
use std::process::Command;
use std::sync::Arc;
use tokio::net::TcpListener;

async fn boot() -> String {
    let dir = tempfile::tempdir().unwrap();
    // leak dir so it outlives the server task
    let path = dir.keep();
    let node = Arc::new(Node::open(&path).unwrap());
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        serve_listener(listener, node).await.unwrap();
    });
    // Server task must run on another worker; CLI Command::output blocks a thread.
    let api = format!("http://{addr}");
    for _ in 0..50 {
        if reqwest::get(format!("{api}/v0/health")).await.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    api
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_status_matches_http_status() {
    let api = boot().await;
    let bin = env!("CARGO_BIN_EXE_keel");
    let out = Command::new(bin)
        .args(["--api", &api, "status"])
        .output()
        .expect("run keel status");
    assert!(
        out.status.success(),
        "stderr {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
    assert_eq!(v["keel"], 0);
    assert_eq!(v["blobs"], 0);

    let via_http: serde_json::Value = reqwest::get(format!("{api}/v0/status"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["keel"], via_http["keel"]);
    assert_eq!(v["blobs"], via_http["blobs"]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_account_mint_visible_on_api() {
    let api = boot().await;
    let bin = env!("CARGO_BIN_EXE_keel");

    let created = Command::new(bin)
        .args(["--api", &api, "account", "new"])
        .output()
        .unwrap();
    assert!(
        created.status.success(),
        "{}",
        String::from_utf8_lossy(&created.stderr)
    );
    let acc: serde_json::Value = serde_json::from_slice(&created.stdout).unwrap();
    let hex = acc["account"].as_str().unwrap();

    let minted = Command::new(bin)
        .args([
            "--api",
            &api,
            "credit",
            "mint",
            hex,
            "2500",
            "--rail-ref",
            "cli-proof",
        ])
        .output()
        .unwrap();
    assert!(
        minted.status.success(),
        "{}",
        String::from_utf8_lossy(&minted.stderr)
    );

    let listed: serde_json::Value = reqwest::get(format!("{api}/v0/accounts"))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["accounts"][0]["balance_millicredits"], 2500);
}
