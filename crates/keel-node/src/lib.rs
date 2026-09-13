//! Local node: content-addressed blobs, millicredits, CID runner, operator HTTP.

mod blob;
mod http;
mod ln;
mod lnd;
mod node;

pub use blob::FsArtifactStore;
pub use http::{router, serve, serve_listener, serve_with, DEFAULT_BIND};
pub use ln::{LightningBook, PayIntentRequest, PayIntentView, PaySettleRequest};
pub use node::Node;

pub const SCHEMA_SQL: &str = include_str!("../../../sql/schema.sql");
