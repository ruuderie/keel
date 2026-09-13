//! In-process Lightning: real BOLT11 + payment_hash. Settle is an incoming HTLC (preimage).

use async_trait::async_trait;
use bitcoin::hashes::{sha256, Hash};
use bitcoin::secp256k1::{Secp256k1, SecretKey};
use keel_types::{PaymentIntent, Rail, SettlementRail};
use lightning_invoice::{Bolt11Invoice, Currency, InvoiceBuilder};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Clone, Debug)]
pub struct PendingInvoice {
    pub payment_hash: [u8; 32],
    pub preimage: [u8; 32],
    pub bolt11: String,
    pub millicredits: i64,
    pub account: String,
    pub amount_msat: u64,
    pub settled: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PayIntentRequest {
    pub account: String,
    pub millicredits_on_confirm: i64,
    #[serde(default = "one_sat")]
    pub amount_sats: u64,
}

fn one_sat() -> u64 {
    1
}

#[derive(Clone, Debug, Deserialize)]
pub struct PaySettleRequest {
    pub payment_hash: String,
    pub preimage: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct PayIntentView {
    pub id: String,
    pub rail: &'static str,
    pub bolt11: String,
    pub payment_hash: String,
    pub millicredits_on_confirm: i64,
    pub amount_atomic: u64,
    pub status: String,
}

pub struct LightningBook {
    secp: Secp256k1<bitcoin::secp256k1::All>,
    key: SecretKey,
    pending: Mutex<HashMap<String, PendingInvoice>>,
}

impl LightningBook {
    pub fn new() -> Self {
        let secp = Secp256k1::new();
        let mut seed = [0u8; 32];
        seed[0] = 42;
        getrandom::getrandom(&mut seed).ok();
        if seed.iter().all(|b| *b == 0) {
            seed[0] = 3;
        }
        let key = SecretKey::from_slice(&seed).unwrap_or_else(|_| {
            SecretKey::from_slice(&[3u8; 32]).expect("nonzero")
        });
        Self {
            secp,
            key,
            pending: Mutex::new(HashMap::new()),
        }
    }

    pub fn create_invoice(
        &self,
        account: &str,
        millicredits: i64,
        amount_sats: u64,
    ) -> Result<PayIntentView, String> {
        let mut preimage = [0u8; 32];
        getrandom::getrandom(&mut preimage).map_err(|e| e.to_string())?;
        let payment_hash: [u8; 32] = Sha256::digest(preimage).into();
        let hash = sha256::Hash::from_slice(&payment_hash).map_err(|e| e.to_string())?;
        let secret = lightning_types::payment::PaymentSecret(preimage);
        let amount_msat = amount_sats.saturating_mul(1000).max(1000);
        let bolt11 = InvoiceBuilder::new(Currency::Regtest)
            .description("keel millicredit mint".into())
            .amount_milli_satoshis(amount_msat)
            .payment_hash(hash)
            .payment_secret(secret)
            .current_timestamp()
            .min_final_cltv_expiry_delta(144)
            .build_signed(|msg| self.secp.sign_ecdsa_recoverable(msg, &self.key))
            .map_err(|e| e.to_string())?
            .to_string();

        let parsed: Bolt11Invoice = bolt11.parse().map_err(|e| format!("parse bolt11: {e}"))?;
        let ph = hex::encode(payment_hash);
        let view = PayIntentView {
            id: ph.clone(),
            rail: "btc_lightning",
            bolt11: parsed.to_string(),
            payment_hash: ph.clone(),
            millicredits_on_confirm: millicredits,
            amount_atomic: amount_sats.max(1),
            status: "open".into(),
        };
        self.pending.lock().unwrap().insert(
            ph,
            PendingInvoice {
                payment_hash,
                preimage,
                bolt11: view.bolt11.clone(),
                millicredits,
                account: account.to_string(),
                amount_msat,
                settled: false,
            },
        );
        Ok(view)
    }

    pub fn get(&self, payment_hash: &str) -> Option<PendingInvoice> {
        self.pending.lock().unwrap().get(payment_hash).cloned()
    }

    /// Incoming HTLC: preimage must hash to payment_hash. Not `rail: mock`.
    pub fn settle(&self, payment_hash: &str, preimage_hex: &str) -> Result<PendingInvoice, String> {
        let preimage = hex::decode(preimage_hex).map_err(|e| e.to_string())?;
        let pre: [u8; 32] = preimage
            .try_into()
            .map_err(|_| "preimage must be 32 bytes".to_string())?;
        let expect: [u8; 32] = Sha256::digest(pre).into();
        let want = hex::decode(payment_hash).map_err(|e| e.to_string())?;
        if expect.as_slice() != want.as_slice() {
            return Err("preimage does not match payment_hash".into());
        }
        let mut g = self.pending.lock().unwrap();
        let inv = g.get_mut(payment_hash).ok_or("unknown invoice")?;
        inv.settled = true;
        Ok(inv.clone())
    }

    pub fn test_preimage(&self, payment_hash: &str) -> Option<String> {
        self.pending
            .lock()
            .unwrap()
            .get(payment_hash)
            .map(|p| hex::encode(p.preimage))
    }
}

impl Default for LightningBook {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SettlementRail for LightningBook {
    fn rail(&self) -> Rail {
        Rail::BtcLightning
    }

    async fn create_intent(&self, intent: &PaymentIntent) -> Result<serde_json::Value, String> {
        let view = self.create_invoice(
            &intent.mint_account.to_hex(),
            intent.millicredits_on_confirm.0,
            intent.amount_atomic,
        )?;
        serde_json::to_value(view).map_err(|e| e.to_string())
    }

    async fn is_confirmed(&self, rail_ref: &str) -> Result<bool, String> {
        Ok(self.get(rail_ref).map(|p| p.settled).unwrap_or(false))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bolt11_roundtrip_and_preimage() {
        let book = LightningBook::new();
        let view = book.create_invoice("aa", 100, 1).unwrap();
        assert!(view.bolt11.to_lowercase().starts_with("lnbcrt"));
        let parsed: Bolt11Invoice = view.bolt11.parse().unwrap();
        assert_eq!(hex::encode(parsed.payment_hash()), view.payment_hash);
        let pre = book.test_preimage(&view.payment_hash).unwrap();
        book.settle(&view.payment_hash, &pre).unwrap();
        assert!(book.get(&view.payment_hash).unwrap().settled);
    }
}
