//! Library client. The CLI is a consumer of this crate, not a second protocol.

mod client;

pub use client::{Client, ClientError};
pub use keel_node::{
    router, serve, serve_listener, serve_with, FsArtifactStore, Node, SCHEMA_SQL, DEFAULT_BIND,
};
pub use keel_types::*;
