-- Keel v0 local node. Tables are indexes of hash-linked documents, not identity.
-- New version = new hash. See SPEC.md §9.

CREATE TABLE IF NOT EXISTS documents (
  cid        TEXT PRIMARY KEY,
  kind       TEXT NOT NULL,
  body       BLOB NOT NULL,
  stored_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS artifact_files (
  artifact_cid TEXT NOT NULL,
  path         TEXT NOT NULL,
  file_cid     TEXT NOT NULL,
  size_bytes   INTEGER NOT NULL,
  PRIMARY KEY (artifact_cid, path)
);

CREATE TABLE IF NOT EXISTS seeders (
  file_cid     TEXT NOT NULL,
  multiaddr    TEXT NOT NULL,
  announced_at INTEGER NOT NULL,
  expires_at   INTEGER NOT NULL,
  from_key     TEXT NOT NULL,
  PRIMARY KEY (file_cid, multiaddr, from_key)
);

CREATE TABLE IF NOT EXISTS indexes (
  publisher TEXT NOT NULL,
  seq       INTEGER NOT NULL,
  cid       TEXT NOT NULL,
  PRIMARY KEY (publisher, seq)
);

CREATE TABLE IF NOT EXISTS runners (
  from_key     TEXT PRIMARY KEY,
  advert_cid   TEXT NOT NULL,
  expires_at   INTEGER NOT NULL,
  caps_json    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS jobs (
  job_id             TEXT PRIMARY KEY,
  spec_cid           TEXT NOT NULL,
  spec_json          TEXT NOT NULL,
  status             TEXT NOT NULL,
  runner_key         TEXT,
  hold_millicredits  INTEGER NOT NULL DEFAULT 0,
  result_json        TEXT,
  expires_at         INTEGER,
  updated_at         INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS credit_accounts (
  pubkey                TEXT PRIMARY KEY,
  balance_millicredits  INTEGER NOT NULL DEFAULT 0,
  held_millicredits     INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS credit_movements (
  id              TEXT PRIMARY KEY,
  account         TEXT NOT NULL,
  kind            TEXT NOT NULL,
  amount          INTEGER NOT NULL,
  cause_type      TEXT NOT NULL,
  cause_id        TEXT NOT NULL,
  idempotency_key TEXT NOT NULL,
  created_at      INTEGER NOT NULL,
  UNIQUE (account, idempotency_key)
);

CREATE TABLE IF NOT EXISTS payment_intents (
  id              TEXT PRIMARY KEY,
  rail            TEXT NOT NULL,
  amount_atomic    INTEGER NOT NULL,
  millicredits    INTEGER NOT NULL DEFAULT 0,
  payment_hash    TEXT,
  bolt11          TEXT,
  mint_account     TEXT NOT NULL,
  status          TEXT NOT NULL,
  created_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS settlement_receipts (
  id        TEXT PRIMARY KEY,
  intent_id TEXT NOT NULL,
  rail_ref  TEXT NOT NULL,
  UNIQUE (rail_ref)
);

CREATE TABLE IF NOT EXISTS filters (
  cid        TEXT PRIMARY KEY,
  issuer     TEXT NOT NULL,
  seq        INTEGER NOT NULL,
  body       TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS blobs (
  cid        TEXT PRIMARY KEY,
  size_bytes INTEGER NOT NULL,
  stored_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS peers (
  pubkey          TEXT PRIMARY KEY,
  visibility      TEXT NOT NULL,
  multiaddrs_json TEXT NOT NULL,
  expires_at      INTEGER NOT NULL,
  advertised_at   INTEGER NOT NULL,
  envelope_json   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS peer_invites (
  invite_id  TEXT PRIMARY KEY,
  once       INTEGER NOT NULL,
  expires_at INTEGER NOT NULL,
  redeemed   INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
