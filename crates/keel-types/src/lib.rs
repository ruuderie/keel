//! Keel wire types. Hash-linked documents; new version = new hash.

pub mod artifact;
pub mod canonical;
pub mod discovery;
pub mod envelope;
pub mod identity;
pub mod job;
pub mod metering;
pub mod peer;
pub mod runner;
pub mod settlement;

pub use artifact::{ArtifactFile, ArtifactManifest, HardwareHint, ManifestSig, SeederRecord};
pub use canonical::{canonical_json, ContentIdError};
pub use discovery::{FilterList, IndexEntry, ModelIndex};
pub use envelope::{Envelope, Kind, UnsignedEnvelope, KEEL_VERSION};
pub use identity::{ContentId, Identity, IdentityError, Pubkey, Signature, Signed};
pub use job::{InputRef, JobError, JobResult, JobSpec, JobStatus};
pub use metering::{
    CauseType, CreditAccount, CreditError, CreditKind, CreditMovement, Millicredits, UsageMeter,
    Work,
};
pub use peer::{PeerAdvertisement, PeerInvite, PeerVisibility};
pub use runner::{PriceBook, RunnerAdvertisement};
pub use settlement::{
    ArtifactStore, CreditWallet, IntentStatus, JobScheduler, PaymentIntent, Rail,
    SettlementRail, SettlementReceipt,
};
