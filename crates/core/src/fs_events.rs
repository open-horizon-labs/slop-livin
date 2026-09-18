//! In-process FSEvents replay: "which directories under a root changed
//! since event id N?", answered without a helper process.
//!
//! Ports the design from mole `integrate` (`cmd/fswatch`,
//! `internal/fswatch`, `docs/fsevents-helper.md`), collapsed into one
//! crate instead of a separate cgo binary: this crate is already macOS
//! product code (unlike Mole's pure-Go release binaries), so there is no
//! `CGO_ENABLED=0` constraint to protect by keeping FSEvents in a
//! sidecar. `fsevent-sys` gives raw CoreServices bindings; [`macos::replay`]
//! is the small safe wrapper around them.
//!
//! The client never treats a failed replay as an error: [`FsEventsSource::replay`]
//! always returns a [`FsEventsPlan`], and every way a replay can fail to
//! earn an incremental refresh is a named [`RefreshRefusal`] instead of an
//! `Err`. A refusal is not a bug; it is this design refusing to guess.

use crate::entities::{Confidence, FactMeta};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Every way a replay can fail to earn an incremental refresh. Two rules
/// hold across all of them: a refusal never carries a usable change list,
/// and the caller always falls back to a full walk.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum RefreshRefusal {
    /// First observation of this root, or a stored id this build has never
    /// written: nothing to replay from.
    NoStoredEventId,
    /// The stored id is ahead of the stream's current id -- the volume's
    /// FSEvents history was reset and everything in between is
    /// unaccounted for.
    EventIdFromFuture,
    /// The root does not canonicalize to itself, or the root's device
    /// differs from the device the stored id was recorded against: the
    /// stored id addresses a different filesystem than the one being
    /// asked about.
    RootMismatch,
    /// No FSEvents history at all is available for this volume (no
    /// current event id, stream creation failed, or the volume is not
    /// journaled).
    FseventsdUnavailable,
    /// The replay could not be completed as a confident, complete account
    /// of what changed: `MustScanSubDirs`, `UserDropped`/`KernelDropped`,
    /// `EventIdsWrapped`, or the run loop budget expired before FSEvents
    /// reported `HistoryDone`. A prefix of the changes is not an answer.
    HelperInconclusive,
    /// The changed-directory count exceeds the configured fraction of
    /// previously known directories: re-walking that much of the tree
    /// piecemeal costs more than a full walk would.
    TooManyChanges,
    /// The previous observation of this root is too recent (within the
    /// same coarse timestamp granularity the growth store uses) for a
    /// replay to be meaningful: FSEvents' own persisted log can lag a
    /// write by longer than that gap, so a replay this soon after the
    /// baseline cannot yet distinguish "nothing changed" from "the
    /// change has not been logged yet". Decided by the caller
    /// (`growth::observe_tracked`), not by [`FsEventsSource::replay`]
    /// itself.
    TooSoon,
    /// This build has no FSEvents implementation for the current
    /// platform (non-macOS). Always refuses; there is no fallback stream
    /// to try.
    UnsupportedPlatform,
}

impl RefreshRefusal {
    /// Stable, lowercase snake_case reason code used in report coverage
    /// notes and the `observe` log line (`mode=full reason=<this>`).
    pub fn as_str(self) -> &'static str {
        match self {
            RefreshRefusal::NoStoredEventId => "no_stored_event_id",
            RefreshRefusal::EventIdFromFuture => "event_id_from_future",
            RefreshRefusal::RootMismatch => "root_mismatch",
            RefreshRefusal::FseventsdUnavailable => "fseventsd_unavailable",
            RefreshRefusal::HelperInconclusive => "helper_inconclusive",
            RefreshRefusal::TooManyChanges => "too_many_changes",
            RefreshRefusal::TooSoon => "too_soon",
            RefreshRefusal::UnsupportedPlatform => "unsupported_platform",
        }
    }
}

/// The two scalars a caller persists alongside whatever it publishes, and
/// passes back in on the next call. Mirrors mole's `fswatch.Request`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FsEventsState {
    /// The `current_event_id` from the plan that accompanied the last
    /// observation. `None` means "nothing stored", always a full refresh.
    pub event_id: Option<u64>,
    /// The device (`st_dev`) the root lived on when `event_id` was
    /// recorded. A root that now resolves to a different device makes the
    /// stored id meaningless.
    pub device: Option<u64>,
    /// `now()` (whole seconds) as of the observation that recorded
    /// `event_id`. See [`RefreshRefusal::TooSoon`].
    pub last_observed_at: Option<u64>,
}

/// One replay request: a canonical root and the state persisted from the
/// last observation of it.
#[derive(Debug, Clone)]
pub struct FsEventsRequest {
    /// Must already be canonical (absolute, symlinks resolved). FSEvents
    /// answers in canonical paths; a non-canonical root would make every
    /// path in the reply appear to sit outside the tree the caller thinks
    /// it asked about.
    pub root: PathBuf,
    pub since: FsEventsState,
}

/// The verdict: either a complete, incremental account of what changed,
/// or a refusal naming why a full walk is required instead. There is no
/// third "error" case -- a failed replay is a refusal, never an `Err`.
#[derive(Debug, Clone)]
pub struct FsEventsPlan {
    pub incremental: bool,
    pub refusal: Option<RefreshRefusal>,
    /// Meaningful only when `incremental` is true. Every reported path
    /// contributes itself and its parent directory (mirrors mole's
    /// `changeSet`), deduplicated, so a creation (visible only from the
    /// parent's listing) and a deletion (nothing left to walk) are both
    /// covered.
    pub changed_dirs: Vec<PathBuf>,
    /// The id to persist for the next call. Meaningful even on a
    /// refusal: one forced full refresh re-anchors the stream rather than
    /// condemning every later run to be full as well. `0` when the
    /// platform could not tell us one at all.
    pub current_event_id: u64,
    /// The device the root lived on for this replay, persisted alongside
    /// `current_event_id` so the next call can detect a root that moved
    /// to a different volume.
    pub device: Option<u64>,
}

impl FsEventsPlan {
    fn refuse(reason: RefreshRefusal, current_event_id: u64, device: Option<u64>) -> Self {
        Self {
            incremental: false,
            refusal: Some(reason),
            changed_dirs: Vec::new(),
            current_event_id,
            device,
        }
    }

    fn ok(changed_dirs: Vec<PathBuf>, current_event_id: u64, device: Option<u64>) -> Self {
        Self {
            incremental: true,
            refusal: None,
            changed_dirs,
            current_event_id,
            device,
        }
    }

    /// The reason string for coverage notes / log lines, on either branch.
    pub fn reason_str(&self) -> &'static str {
        self.refusal.map(|r| r.as_str()).unwrap_or("incremental")
    }
}

/// The seam a real observation drives, and tests replace with canned
/// event batches so no test depends on the live `fseventsd`.
pub trait FsEventsSource: Send + Sync {
    fn replay(&self, request: &FsEventsRequest) -> FsEventsPlan;
}

/// Returns the platform's real source on macOS, and the always-refusing
/// stub everywhere else.
pub fn platform_source() -> Box<dyn FsEventsSource> {
    #[cfg(target_os = "macos")]
    {
        Box::new(macos::MacOsFsEventsSource)
    }
    #[cfg(not(target_os = "macos"))]
    {
        Box::new(UnsupportedPlatformSource)
    }
}

/// The non-macOS fallback: always refuses, naming the platform as the
/// cause, never a bug in the replay itself.
pub struct UnsupportedPlatformSource;

impl FsEventsSource for UnsupportedPlatformSource {
    fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
        FsEventsPlan::refuse(RefreshRefusal::UnsupportedPlatform, 0, None)
    }
}

/// Adds `path` and its parent directory to `changes`, provided each is
/// within `root`. FSEvents reports a path whose meaning (the changed item
/// itself, or the directory containing it) depends on flags this design
/// deliberately does not trust for that judgement (see
/// `docs/fsevents-helper.md`'s "every path contributes itself and its
/// parent"); adding both costs one extra listing on re-walk and closes
/// both a creation and a deletion.
fn add_with_parent(changes: &mut std::collections::HashSet<PathBuf>, root: &Path, path: &Path) {
    for candidate in [
        Some(path.to_path_buf()),
        path.parent().map(Path::to_path_buf),
    ]
    .into_iter()
    .flatten()
    {
        if candidate == root || candidate.starts_with(root) {
            changes.insert(candidate);
        }
    }
}

/// Legacy pre-#29 shape kept only so the stub's original two symbols
/// (`full_refresh`, an early `RefreshRefusal` with fewer variants) do not
/// silently disappear from anyone who imported them mid-restart. Superseded
/// by [`FsEventsPlan::refuse`] internally.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub incremental: bool,
    pub refusal: Option<RefreshRefusal>,
    pub meta: FactMeta,
}

pub fn full_refresh(reason: RefreshRefusal) -> RefreshResult {
    RefreshResult {
        incremental: false,
        refusal: Some(reason),
        meta: FactMeta::now("filesystem.full-refresh", Confidence::High),
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)] // fsevent-sys itself is marked deprecated in favour of
// objc2-core-services, but it is a maintained, direct binding to the same
// stable CoreServices C API this module needs and ships today; swapping
// bindings crates is out of scope for this change.
mod macos {
    use super::*;
    use core_foundation::array::{CFArray, CFArrayRef};
    use core_foundation::base::TCFType;
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopDefaultMode};
    use core_foundation::string::CFString;
    use fsevent_sys as fs;
    use std::ffi::CStr;
    use std::os::raw::c_void;
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::MetadataExt;
    use std::time::{Duration, Instant};

    /// Bounds one replay's run loop. FSEvents replays retained history
    /// from local, in-kernel state, so a replay that has not reached
    /// `HistoryDone` in this long is not going to; refusing with
    /// `HelperInconclusive` beats blocking an observation indefinitely.
    /// Overridable for slow CI via `SLOP_LIVIN_FSEVENTS_TIMEOUT_SEC`.
    fn replay_budget() -> Duration {
        std::env::var("SLOP_LIVIN_FSEVENTS_TIMEOUT_SEC")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(6))
    }

    struct Collector {
        root: PathBuf,
        changes: std::collections::HashSet<PathBuf>,
        history_done: bool,
        hard_fail: Option<RefreshRefusal>,
    }

    extern "C" fn stream_callback(
        _stream: fs::FSEventStreamRef,
        info: *mut c_void,
        num_events: usize,
        event_paths: *mut c_void,
        event_flags: *const fs::FSEventStreamEventFlags,
        _event_ids: *const fs::FSEventStreamEventId,
    ) {
        if info.is_null() || num_events == 0 {
            return;
        }
        // SAFETY: `info` is the `Collector` this replay's caller created
        // and kept alive on the stack for the stream's entire lifetime;
        // FSEvents only invokes this callback on the run loop that same
        // stack frame is pumping.
        let collector = unsafe { &mut *(info as *mut Collector) };
        let paths = event_paths as *const *const std::os::raw::c_char;
        // SAFETY: FSEvents guarantees `num_events` valid entries in both
        // the flags array and the paths array it hands the callback.
        let flags = unsafe { std::slice::from_raw_parts(event_flags, num_events) };

        for (i, &f) in flags.iter().enumerate() {
            if (f & fs::kFSEventStreamEventFlagMustScanSubDirs) != 0
                || (f & fs::kFSEventStreamEventFlagEventIdsWrapped) != 0
            {
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::HelperInconclusive);
                continue;
            }
            if (f & fs::kFSEventStreamEventFlagRootChanged) != 0
                || (f & fs::kFSEventStreamEventFlagUnmount) != 0
            {
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::RootMismatch);
                continue;
            }
            if (f & fs::kFSEventStreamEventFlagHistoryDone) != 0 {
                collector.history_done = true;
                CFRunLoop::get_current().stop();
                continue;
            }

            // SAFETY: `paths` was validated non-null by FSEvents for
            // every one of `num_events` entries; `i` is in range.
            let cpath = unsafe { *paths.add(i) };
            if cpath.is_null() {
                // A path we cannot read is a change we cannot locate:
                // refusing is the difference between a full refresh and a
                // generation that silently omits this event.
                collector
                    .hard_fail
                    .get_or_insert(RefreshRefusal::HelperInconclusive);
                continue;
            }
            // SAFETY: FSEvents paths are NUL-terminated C strings.
            let cstr = unsafe { CStr::from_ptr(cpath) };
            let path = PathBuf::from(std::ffi::OsStr::from_bytes(cstr.to_bytes()));
            add_with_parent(&mut collector.changes, &collector.root, &path);
        }
    }

    pub struct MacOsFsEventsSource;

    impl FsEventsSource for MacOsFsEventsSource {
        fn replay(&self, request: &FsEventsRequest) -> FsEventsPlan {
            replay(&request.root, &request.since)
        }
    }

    fn replay(root: &Path, since: &FsEventsState) -> FsEventsPlan {
        // SAFETY: no arguments; a pure query of the FSEvents subsystem.
        let current = unsafe { fs::FSEventsGetCurrentEventId() };
        if current == 0 {
            return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, 0, None);
        }

        let dev = match std::fs::metadata(root) {
            Ok(meta) => meta.dev(),
            Err(_) => {
                return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, current, None);
            }
        };

        // Sanity check named in the issue: ask FSEvents for the last
        // event id it can vouch for on this device as of "now". This is
        // read-only and its only use here is to catch a device whose
        // FSEvents history log cannot possibly reach back to `since`
        // (the id looks plausible but predates everything retained).
        // SAFETY: `dev` came from a real `stat`; `CFAbsoluteTimeGetCurrent`
        // takes no arguments.
        let last_known = unsafe {
            fs::FSEventsGetLastEventIdForDeviceBeforeTime(
                dev,
                core_foundation_sys::date::CFAbsoluteTimeGetCurrent(),
            )
        };

        if let Some(stored_device) = since.device
            && stored_device != dev
        {
            return FsEventsPlan::refuse(RefreshRefusal::RootMismatch, current, Some(dev));
        }

        let Some(since_id) = since.event_id else {
            return FsEventsPlan::refuse(RefreshRefusal::NoStoredEventId, current, Some(dev));
        };
        if since_id > current {
            return FsEventsPlan::refuse(RefreshRefusal::EventIdFromFuture, current, Some(dev));
        }
        // `last_known` being 0 means FSEvents could not answer at all for
        // this device (no history yet observed); that is not by itself a
        // reason to refuse a replay FSEvents is about to attempt, so it
        // only gates the case where FSEvents can positively vouch for a
        // *later* floor than our stored id, meaning `since_id` is stale
        // history that has already rotated out.
        if last_known != 0 && since_id != 0 && since_id < last_known {
            return FsEventsPlan::refuse(RefreshRefusal::HelperInconclusive, current, Some(dev));
        }

        let mut collector = Box::new(Collector {
            root: root.to_path_buf(),
            changes: std::collections::HashSet::new(),
            history_done: false,
            hard_fail: None,
        });
        let info_ptr = collector.as_mut() as *mut Collector as *mut c_void;

        let cf_path = CFString::new(&root.to_string_lossy());
        let paths_array: CFArray<CFString> = CFArray::from_CFTypes(&[cf_path]);
        let context = fs::FSEventStreamContext {
            version: 0,
            info: info_ptr,
            retain: None,
            release: None,
            copy_description: None,
        };
        // NoDefer delivers the first batch immediately instead of after
        // the latency window; WatchRoot reports the root itself being
        // moved or replaced. Directory-level events (no FileEvents flag)
        // suffice: rows here are per artifact/worktree/dir, never
        // per-file.
        let create_flags =
            fs::kFSEventStreamCreateFlagNoDefer | fs::kFSEventStreamCreateFlagWatchRoot;

        // SAFETY: `paths_array` and `context` outlive the call; the
        // callback pointer has the exact signature FSEvents expects.
        let stream = unsafe {
            fs::FSEventStreamCreate(
                core_foundation_sys::base::kCFAllocatorDefault,
                stream_callback,
                &context,
                paths_array.as_concrete_TypeRef() as CFArrayRef,
                since_id,
                0.0,
                create_flags,
            )
        };
        if stream.is_null() {
            return FsEventsPlan::refuse(RefreshRefusal::FseventsdUnavailable, current, Some(dev));
        }

        // SAFETY: `stream` was just created and is released below on
        // every path out of this function.
        unsafe {
            fs::FSEventStreamScheduleWithRunLoop(
                stream,
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopDefaultMode,
            );
            if fs::FSEventStreamStart(stream) == 0 {
                fs::FSEventStreamInvalidate(stream);
                fs::FSEventStreamRelease(stream);
                return FsEventsPlan::refuse(
                    RefreshRefusal::FseventsdUnavailable,
                    current,
                    Some(dev),
                );
            }
        }

        let deadline = Instant::now() + replay_budget();
        while !collector.history_done && collector.hard_fail.is_none() {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            // Run in short slices so the loop condition (history_done /
            // hard_fail, both set from inside the callback which runs on
            // this same thread while this call is blocked in it) is
            // re-checked promptly rather than only after the whole
            // remaining budget elapses.
            let slice = remaining.min(Duration::from_millis(200));
            CFRunLoop::run_in_mode(unsafe { kCFRunLoopDefaultMode }, slice, true);
        }

        // SAFETY: matches the successful `FSEventStreamCreate` above.
        unsafe {
            fs::FSEventStreamStop(stream);
            fs::FSEventStreamInvalidate(stream);
            fs::FSEventStreamRelease(stream);
        }

        if let Some(reason) = collector.hard_fail {
            return FsEventsPlan::refuse(reason, current, Some(dev));
        }
        if !collector.history_done {
            // The run loop stopped (budget exhausted) without FSEvents
            // ever reporting the replay complete. Whatever was collected
            // may be a prefix of what changed, and a prefix is exactly
            // the silent partial this design refuses to report.
            return FsEventsPlan::refuse(RefreshRefusal::HelperInconclusive, current, Some(dev));
        }

        FsEventsPlan::ok(collector.changes.drain().collect(), current, Some(dev))
    }
}

/// Test-only support kept as an ordinary (non-`#[cfg(test)]`) module so
/// integration tests in `crates/core/tests/*.rs` -- which link against
/// this crate rather than compiling into it -- can use it too. Small and
/// inert in a release binary: one struct, one trait impl, no I/O.
pub mod testing {
    use super::{FsEventsPlan, FsEventsRequest, FsEventsSource};

    /// A canned source: returns whatever plan it was built with,
    /// regardless of the request. Lets every refusal reason and the
    /// incremental path be exercised without depending on the live
    /// `fseventsd`.
    pub struct CannedSource(pub FsEventsPlan);

    impl FsEventsSource for CannedSource {
        fn replay(&self, _request: &FsEventsRequest) -> FsEventsPlan {
            self.0.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_source_always_refuses() {
        let source = UnsupportedPlatformSource;
        let plan = source.replay(&FsEventsRequest {
            root: PathBuf::from("/tmp"),
            since: FsEventsState {
                event_id: Some(1),
                device: Some(1),
                last_observed_at: Some(1),
            },
        });
        assert!(!plan.incremental);
        assert_eq!(plan.refusal, Some(RefreshRefusal::UnsupportedPlatform));
        assert_eq!(plan.reason_str(), "unsupported_platform");
    }

    #[test]
    fn add_with_parent_stays_within_root() {
        let root = Path::new("/root");
        let mut changes = std::collections::HashSet::new();
        add_with_parent(&mut changes, root, Path::new("/root/a/b"));
        assert!(changes.contains(Path::new("/root/a/b")));
        assert!(changes.contains(Path::new("/root/a")));

        let mut changes = std::collections::HashSet::new();
        add_with_parent(&mut changes, root, Path::new("/elsewhere/x"));
        assert!(changes.is_empty(), "a path outside root must not be added");
    }

    #[test]
    fn reason_str_covers_every_refusal() {
        for (reason, expected) in [
            (RefreshRefusal::NoStoredEventId, "no_stored_event_id"),
            (RefreshRefusal::EventIdFromFuture, "event_id_from_future"),
            (RefreshRefusal::RootMismatch, "root_mismatch"),
            (
                RefreshRefusal::FseventsdUnavailable,
                "fseventsd_unavailable",
            ),
            (RefreshRefusal::HelperInconclusive, "helper_inconclusive"),
            (RefreshRefusal::TooManyChanges, "too_many_changes"),
            (RefreshRefusal::TooSoon, "too_soon"),
            (RefreshRefusal::UnsupportedPlatform, "unsupported_platform"),
        ] {
            assert_eq!(reason.as_str(), expected);
        }
    }
}
