//! Safety-first facts and actions for developer disk cleanup.
//!
//! The index is evidence. Grants are authorization. The sink is the final
//! authority and must re-observe every predicate before changing the filesystem.

pub mod attribution;
pub mod compose;
pub mod docker;
pub mod entities;
pub mod execution;
pub mod extractor;
pub mod fs_events;
pub mod git;
pub mod grants;
pub mod growth;
pub mod ledger;
pub mod measurement;
pub mod occupancy;
pub mod render;
pub mod report;
pub mod scan;
pub mod signals;
pub mod store;
pub mod volume;
pub mod walk;

pub use entities::*;
pub use grants::*;
pub use ledger::*;
pub use report::{Report, report};
pub use scan::*;
pub use store::*;
