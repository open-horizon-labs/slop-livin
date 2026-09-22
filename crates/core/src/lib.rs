//! Safety-first facts and actions for developer disk cleanup.
//!
//! The index is evidence. Grants are authorization. The sink is the final
//! authority and must re-observe every predicate before changing the filesystem.

pub mod actions;
pub mod activity;
pub mod agent_json;
pub mod agents;
pub mod artifact;
pub mod assoc_store;
pub mod attribution;
pub mod build_adapters;
pub(crate) mod build_stores;
pub mod bus;
pub mod cargo_artifacts;
pub mod cargo_cleanup;
pub mod compose;
pub mod consumer_wiring;
pub mod consumers;
pub mod coverage;
pub mod docker;
pub mod ecosystem;
pub mod entities;
pub mod evidence;
pub mod execution;
pub mod external;
pub mod external_associations;
pub mod extractor;
pub mod filter;
pub mod folded_measurement;
pub mod fs_events;
pub mod git;
pub mod github;
pub mod grants;
pub mod growth;
pub mod ignore;
pub mod ledger;
pub mod locations;
pub mod measurement;
pub mod occupancy;
pub mod recheck;
pub mod reclaimability;
pub mod recovery;
pub mod render;
pub mod report;
pub mod scan;
pub mod schedule;
pub mod scope;
pub mod signals;
pub mod store;
pub mod toolchain_declarations;
pub mod tree;
pub mod volume;
pub mod walk;
pub mod work_counters;

pub use entities::*;
pub use grants::*;
pub use ledger::*;
pub use report::{Report, report};
pub use scan::*;
pub use store::*;
