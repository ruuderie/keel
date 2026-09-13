//! Optional operator Lightning via LND REST.
//!
//! Env: `KEEL_LND_REST` (e.g. `https://127.0.0.1:8080`), `KEEL_LND_MACAROON` (hex)
//! or `KEEL_LND_MACAROON_FILE`, optional `KEEL_LND_INSECURE=1` for self-signed TLS.
//! Same JSON as in-process invoices (`bolt11`, `payment_hash`). Not used by CI.

use serde_json::Value;

pub fn configured() -> bool {
    std::env::var("KEEL_LND_REST")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some()
}

fn rest_base() -> Result<String, String> {
    std::env::var("KEEL_LND_REST")
        .map(|s| s.trim_end_matches('/').to_string())
        .map_err(|_| "KEEL_LND_REST unset".into())
}

fn macaroon_hex() -> Result<String, String> {
    if let Ok(hex) = std::env::var("KEEL_LND_MACAROON") {
        if !hex.is_empty() {
            return Ok(hex);
        }
    }
    if let Ok(path) = std::env::var("KEEL_LND_MACAROON_FILE") {
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        return Ok(hex::encode(bytes));
    }
    Err("set KEEL_LND_MACAROON or KEEL_LND_MACAROON_FILE".into())
}

fn client() -> Result<reqwest::Client, String> {
    let insecure = std::env::var("KEEL_LND_INSECURE").ok().as_deref() == Some("1");
    reqwest::Client::builder()
        .danger_accept_invalid_certs(insecure)
        .build()
        .map_err(|e| e.to_string())
}

pub struct LndInvoice {
    pub bolt11: String,
    pub payment_hash: String,
}

fn b64_to_hex(raw: &str) -> Result<String, String> {
    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(raw)
        .or_else(|_| base64::engine::general_purpose::URL_SAFE.decode(raw))
        .or_else(|_| base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw))
        .map_err(|e| e.to_string())?;
    Ok(hex::encode(bytes))
}

pub async fn add_invoice(memo: &str, amount_sats: u64) -> Result<LndInvoice, String> {
    let base = rest_base()?;
    let mac = macaroon_hex()?;
    let http = client()?;
    let res = http
        .post(format!("{base}/v1/invoices"))
        .header("Grpc-Metadata-macaroon", mac)
        .json(&serde_json::json!({
            "memo": memo,
            "value": amount_sats.max(1).to_string(),
        }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("lnd addinvoice {status}: {v}"));
    }
    let bolt11 = v
        .get("payment_request")
        .and_then(|x| x.as_str())
        .ok_or("lnd: missing payment_request")?
        .to_string();
    let r_hash = v
        .get("r_hash")
        .and_then(|x| x.as_str())
        .ok_or("lnd: missing r_hash")?;
    let payment_hash = if r_hash.len() == 64 && hex::decode(r_hash).is_ok() {
        r_hash.to_string()
    } else {
        b64_to_hex(r_hash)?
    };
    Ok(LndInvoice {
        bolt11,
        payment_hash,
    })
}

pub async fn is_settled(payment_hash: &str) -> Result<bool, String> {
    let base = rest_base()?;
    let mac = macaroon_hex()?;
    let http = client()?;
    let bytes = hex::decode(payment_hash).map_err(|e| e.to_string())?;
    use base64::Engine;
    let b64 = base64::engine::general_purpose::URL_SAFE.encode(&bytes);
    let res = http
        .get(format!("{base}/v1/invoice/{b64}"))
        .header("Grpc-Metadata-macaroon", mac)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = res.json().await.map_err(|e| e.to_string())?;
    Ok(v.get("settled").and_then(|x| x.as_bool()).unwrap_or(false)
        || v.get("state").and_then(|x| x.as_str()) == Some("SETTLED"))
}
