//! Safety-first facts and actions for developer disk cleanup.
//!
//! The index is evidence. Grants are authorization. The sink is the final
//! authority and must re-observe every predicate before changing the filesystem.

pub mod attribution;
pub mod docker;
pub mod entities;
pub mod execution;
pub mod extractor;
pub mod fs_events;
pub mod git;
pub mod grants;
pub mod ledger;
pub mod measurement;
pub mod occupancy;
pub mod report;
pub mod scan;
pub mod store;
pub mod volume;

pub use entities::*;
pub use grants::*;
pub use ledger::*;
pub use report::{Report, report};
pub use scan::*;
pub use store::*;
