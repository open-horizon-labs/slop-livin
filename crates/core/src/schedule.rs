//! Opt-in scheduled observation: a per-user LaunchAgent that runs
//! `swamp observe <roots>` on a fixed interval.
//!
//! Ported from the mole `integrate` branch's `lib/manage/schedule.sh` +
//! `docs/scheduled-inventory-refresh.md` design: a scheduled job, not a
//! daemon. `launchd` starts one process, it exits. No resident process, no
//! menu bar, no notifications. Off by default, installed only by the
//! explicit `swamp schedule --every <interval>` command.
//!
//! The scheduled program is `swamp observe`, which only walks and
//! writes the growth store -- it never renders a report and has no path to
//! any destructive command (there are none in this tool).

use crate::fs_gate::{self, read::read_owned_string, store};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

pub const LABEL: &str = "com.open-horizon-labs.swamp.observe";

/// Default watchdog budget for one `observe` invocation, in seconds.
pub const DEFAULT_OBSERVE_TIMEOUT_SECS: u64 = 1800;

fn env_dir(var: &str, default: PathBuf) -> PathBuf {
    std::env::var(var).map(PathBuf::from).unwrap_or(default)
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
}

pub fn agents_dir() -> PathBuf {
    env_dir(
        "SWAMP_LAUNCH_AGENTS_DIR",
        home().join("Library/LaunchAgents"),
    )
}

pub fn plist_path() -> PathBuf {
    agents_dir().join(format!("{LABEL}.plist"))
}

pub fn log_dir() -> PathBuf {
    env_dir("SWAMP_LOG_DIR", home().join("Library/Logs/swamp"))
}

pub fn log_file() -> PathBuf {
    log_dir().join("observe.log")
}

/// Parses a human interval (`"30m"`, `"1h"`, `"12h"`, `"1d"`, or a bare
/// number of seconds) into seconds. Rejects anything unparseable rather
/// than silently defaulting to some other cadence.
pub fn parse_interval(raw: &str) -> Result<u64> {
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("missing interval; use --every 30m, 1h, 12h or 1d");
    }
    let seconds = crate::growth::parse_duration_secs(raw)
        .with_context(|| format!("invalid interval: {raw} (use a number with s, m, h or d)"))?;
    if seconds == 0 {
        bail!("interval must be greater than zero: {raw}");
    }
    Ok(seconds)
}

/// Renders seconds back into the shortest exact human form.
pub fn format_interval(seconds: u64) -> String {
    if seconds.is_multiple_of(86400) {
        format!("{}d", seconds / 86400)
    } else if seconds.is_multiple_of(3600) {
        format!("{}h", seconds / 3600)
    } else if seconds.is_multiple_of(60) {
        format!("{}m", seconds / 60)
    } else {
        format!("{seconds}s")
    }
}

fn test_mode() -> bool {
    std::env::var("SWAMP_TEST_MODE").is_ok_and(|v| v == "1")
}

/// `launchctl` indirection. Under `SWAMP_TEST_MODE=1` every call is a
/// no-op that prints what it would have run, so no test suite can ever
/// register a real job on the machine running it.
fn run_launchctl(args: &[&str]) -> Result<bool> {
    if test_mode() {
        println!("[test-mode] launchctl {}", args.join(" "));
        return Ok(true);
    }
    let out = fs_gate::spawn::run(
        fs_gate::spawn::Program::Launchctl,
        args,
        std::time::Duration::from_secs(30),
    )
    .context("spawn launchctl")?;
    Ok(out.success())
}

fn domain() -> String {
    let uid = std::env::var("SWAMP_UID").ok().unwrap_or_else(|| {
        fs_gate::spawn::run(
            fs_gate::spawn::Program::Id,
            ["-u"],
            std::time::Duration::from_secs(5),
        )
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|| "0".to_string())
    });
    format!("gui/{uid}")
}

fn load_plist(path: &Path) -> Result<()> {
    let path_str = path.to_string_lossy().to_string();
    if run_launchctl(&["bootstrap", &domain(), &path_str])? {
        return Ok(());
    }
    // Fallback to the older launchctl surface.
    run_launchctl(&["load", "-w", &path_str])?;
    Ok(())
}

fn unload_plist(path: &Path) {
    let path_str = path.to_string_lossy().to_string();
    let service = format!("{}/{LABEL}", domain());
    let _ = run_launchctl(&["bootout", &service]);
    let _ = run_launchctl(&["unload", "-w", &path_str]);
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders the LaunchAgent plist body.
pub fn render_plist(
    exe: &Path,
    roots: &[PathBuf],
    seconds: u64,
    log_path: &Path,
    swamp_dir_env: Option<&str>,
) -> String {
    let mut args = String::new();
    args.push_str(&format!(
        "        <string>{}</string>\n",
        xml_escape(&exe.to_string_lossy())
    ));
    args.push_str("        <string>observe</string>\n");
    for root in roots {
        args.push_str(&format!(
            "        <string>{}</string>\n",
            xml_escape(&root.to_string_lossy())
        ));
    }

    let env_block = match swamp_dir_env {
        Some(dir) => format!(
            "    <key>EnvironmentVariables</key>\n    <dict>\n        <key>SWAMP_DIR</key>\n        <string>{}</string>\n    </dict>\n",
            xml_escape(dir)
        ),
        None => String::new(),
    };

    let log_str = xml_escape(&log_path.to_string_lossy());

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
    <key>Label</key>\n\
    <string>{LABEL}</string>\n\
    <key>ProgramArguments</key>\n\
    <array>\n\
{args}\
    </array>\n\
    <key>StartInterval</key>\n\
    <integer>{seconds}</integer>\n\
    <key>RunAtLoad</key>\n\
    <false/>\n\
    <key>ProcessType</key>\n\
    <string>Background</string>\n\
    <key>LowPriorityIO</key>\n\
    <true/>\n\
    <key>Nice</key>\n\
    <integer>10</integer>\n\
{env_block}\
    <key>StandardOutPath</key>\n\
    <string>{log_str}</string>\n\
    <key>StandardErrorPath</key>\n\
    <string>{log_str}</string>\n\
</dict>\n\
</plist>\n"
    )
}

fn current_exe() -> Result<PathBuf> {
    std::env::current_exe().context("resolve current executable")
}

/// `swamp schedule --every <interval> <root>...`. `roots` empty (#42/#50)
/// installs `observe` with no positional roots at all: every scheduled
/// fire re-resolves the configured scope fresh (see `Command::Observe`'s
/// `roots.is_empty()` path), rather than replaying whatever roots were
/// present at install time. Passing explicit roots still freezes exactly
/// those, same as before -- an explicit root list has always replaced
/// the configured scope for one invocation, and that includes a
/// scheduled one.
pub fn install(interval_raw: &str, roots: &[PathBuf]) -> Result<String> {
    let seconds = parse_interval(interval_raw)?;
    let exe = current_exe()?;
    let plist = plist_path();
    let log = log_file();
    store::create_dir_all(log_dir()).context("create log dir")?;
    if let Some(parent) = plist.parent() {
        store::create_dir_all(parent).context("create LaunchAgents dir")?;
    }

    // Installing over an existing agent replaces it: unload first so
    // launchd never holds two generations of the same label.
    if fs_gate::exists(&plist) {
        unload_plist(&plist);
    }

    let swamp_dir_env = std::env::var("SWAMP_DIR").ok();
    let body = render_plist(&exe, roots, seconds, &log, swamp_dir_env.as_deref());
    store::write_text(store::TextFile::LaunchAgent { plist: &plist }, &body)
        .with_context(|| format!("write {}", plist.display()))?;

    load_plist(&plist)?;

    let roots_str = if roots.is_empty() {
        "(configured scope, resolved fresh on every run)".to_string()
    } else {
        roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    Ok(format!(
        "Scheduled observation every {}\n  Label: {LABEL}\n  Plist: {}\n  Log:   {}\n  Roots: {}\n  Turn it off with: swamp schedule --off\n",
        format_interval(seconds),
        plist.display(),
        log.display(),
        roots_str
    ))
}

/// `swamp schedule --off`.
pub fn uninstall() -> Result<String> {
    let plist = plist_path();
    if !fs_gate::exists(&plist) {
        unload_plist(&plist);
        return Ok("No scheduled observation is installed\n".to_string());
    }
    unload_plist(&plist);
    store::remove_text(store::TextFile::LaunchAgent { plist: &plist })
        .with_context(|| format!("remove {}", plist.display()))?;
    Ok(format!("Removed the scheduled observation ({LABEL})\n"))
}

/// Reads `StartInterval` back out of the installed plist with a tiny
/// hand-rolled scan (no plist-parsing dependency for one integer).
fn installed_interval(plist_text: &str) -> Option<u64> {
    let key_pos = plist_text.find("<key>StartInterval</key>")?;
    let rest = &plist_text[key_pos..];
    let start = rest.find("<integer>")? + "<integer>".len();
    let end = rest[start..].find("</integer>")? + start;
    rest[start..end].trim().parse().ok()
}

/// Reads `ProgramArguments` roots (every argument after `observe`) back
/// out of the installed plist.
fn installed_roots(plist_text: &str) -> Vec<String> {
    let Some(array_start) = plist_text.find("<key>ProgramArguments</key>") else {
        return Vec::new();
    };
    let rest = &plist_text[array_start..];
    let Some(open) = rest.find("<array>") else {
        return Vec::new();
    };
    let Some(close) = rest.find("</array>") else {
        return Vec::new();
    };
    let block = &rest[open + "<array>".len()..close];
    let mut strings: Vec<String> = Vec::new();
    for line in block.lines() {
        let line = line.trim();
        if let Some(v) = line
            .strip_prefix("<string>")
            .and_then(|s| s.strip_suffix("</string>"))
        {
            strings.push(
                v.replace("&amp;", "&")
                    .replace("&lt;", "<")
                    .replace("&gt;", ">"),
            );
        }
    }
    // strings[0] = executable, strings[1] = "observe", the rest are roots.
    strings.into_iter().skip(2).collect()
}

/// One completed `observe` run's outcome, appended to the log and (for
/// the most recent run) persisted alongside the growth store so the
/// report header can read it without parsing the log.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RunOutcome {
    pub observed_at: u64,
    pub wall_ms: u64,
    pub walked_total: u64,
    pub projects: usize,
    pub mode: String,
    pub outcome: String,
}

impl RunOutcome {
    fn to_log_line(&self) -> String {
        format!(
            "observed_at={} wall_ms={} walked_total={} projects={} mode={} outcome={}",
            self.observed_at,
            self.wall_ms,
            self.walked_total,
            self.projects,
            self.mode,
            self.outcome
        )
    }

    fn from_log_line(line: &str) -> Option<RunOutcome> {
        let mut observed_at = None;
        let mut wall_ms = None;
        let mut walked_total = None;
        let mut projects = None;
        let mut mode = None;
        let mut outcome = None;
        for field in line.split_whitespace() {
            let Some((key, value)) = field.split_once('=') else {
                continue;
            };
            match key {
                "observed_at" => observed_at = value.parse().ok(),
                "wall_ms" => wall_ms = value.parse().ok(),
                "walked_total" => walked_total = value.parse().ok(),
                "projects" => projects = value.parse().ok(),
                "mode" => mode = Some(value.to_string()),
                "outcome" => outcome = Some(value.to_string()),
                _ => {}
            }
        }
        Some(RunOutcome {
            observed_at: observed_at?,
            wall_ms: wall_ms?,
            walked_total: walked_total?,
            projects: projects?,
            mode: mode?,
            outcome: outcome?,
        })
    }
}

/// Appends one line to the observation log.
pub fn append_log(path: &Path, outcome: &RunOutcome) -> Result<()> {
    if let Some(parent) = path.parent() {
        store::create_dir_all(parent)?;
    }
    store::append_line(store::LogFile::Observations(path), &outcome.to_log_line())
        .with_context(|| format!("open {}", path.display()))?;
    Ok(())
}

/// Reads the last well-formed line of the observation log.
pub fn last_log_outcome(path: &Path) -> Option<RunOutcome> {
    let text = read_owned_string(path).ok()?;
    text.lines().rev().find_map(RunOutcome::from_log_line)
}

fn last_run_path(store_dir: &Path) -> PathBuf {
    store_dir.join("last_run.json")
}

/// Persists the most recent run's summary alongside the growth store, so
/// the report header can read it without parsing the log.
pub fn write_last_run(store_dir: &Path, outcome: &RunOutcome) -> Result<()> {
    let path = last_run_path(store_dir);
    store::write_json(store::JsonFile::LastRun { store: store_dir }, outcome)
        .with_context(|| format!("write {}", path.display()))
}

pub fn read_last_run(store_dir: &Path) -> Option<RunOutcome> {
    let text = read_owned_string(last_run_path(store_dir)).ok()?;
    serde_json::from_str(&text).ok()
}

fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else if secs < 86400 {
        format!("{}h", secs / 3600)
    } else {
        format!("{}d", secs / 86400)
    }
}

fn format_ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then);
    if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86400)
    }
}

/// The report header line describing scheduled-observation state, shown
/// right after `observed_at=...`. `suggested_root` is used only in the
/// "no schedule" suggestion text.
pub fn header_line(store_dir: &Path, suggested_root: &Path, now: u64) -> String {
    if !fs_gate::exists(plist_path()) {
        return format!(
            "no schedule (swamp schedule --every 30m {})",
            suggested_root.display()
        );
    }
    match read_last_run(store_dir) {
        Some(run) if run.outcome == "ok" => {
            format!(
                "last scheduled run {} ({}, {:.1} s)",
                format_ago(now, run.observed_at),
                run.mode,
                run.wall_ms as f64 / 1000.0
            )
        }
        Some(run) => {
            format!(
                "last scheduled run {} ({})",
                format_ago(now, run.observed_at),
                run.outcome
            )
        }
        None => "schedule installed; no run recorded yet".to_string(),
    }
}

/// `swamp schedule` with no arguments: report installed/loaded
/// state, interval, roots, last run, and the next expected run.
pub fn status(store_dir: &Path) -> Result<String> {
    let plist = plist_path();
    if !fs_gate::exists(&plist) {
        return Ok(
            "Scheduled observation: not installed\n  Enable it with: swamp schedule --every 30m <root>\n"
                .to_string(),
        );
    }
    let text = read_owned_string(&plist).with_context(|| format!("read {}", plist.display()))?;
    let seconds = installed_interval(&text);
    let roots = installed_roots(&text);
    let mut out = String::new();
    out.push_str("Scheduled observation: installed\n");
    out.push_str(&format!("  Label:    {LABEL}\n"));
    out.push_str(&format!("  Plist:    {}\n", plist.display()));
    match seconds {
        Some(s) => out.push_str(&format!("  Interval: {}\n", format_interval(s))),
        None => out.push_str("  Interval: unreadable (the plist was edited by hand)\n"),
    }
    if roots.is_empty() {
        // No frozen roots baked into the plist (#42/#50): each fire runs
        // `observe` with no positional roots, which re-resolves the
        // configured scope fresh every time -- so a config edit (a new
        // `include`, a new `exclude`, a detector toggle) takes effect on
        // the very next scheduled run, not only after `schedule --every`
        // is run again.
        out.push_str("  Roots:    (configured scope, resolved fresh on every run)\n");
    } else {
        out.push_str(&format!("  Roots:    {}\n", roots.join(" ")));
    }

    match read_last_run(store_dir).or_else(|| last_log_outcome(&log_file())) {
        Some(run) => {
            let now = crate::entities::now();
            out.push_str(&format!(
                "  Last run: {} ({}, {} projects, {:.1} s)\n",
                format_ago(now, run.observed_at),
                run.outcome,
                run.projects,
                run.wall_ms as f64 / 1000.0
            ));
            if let Some(s) = seconds {
                let next = run.observed_at + s;
                let now = crate::entities::now();
                if next > now {
                    out.push_str(&format!("  Next run: in {}\n", format_duration(next - now)));
                } else {
                    out.push_str("  Next run: due\n");
                }
            }
        }
        None => out.push_str("  Last run: none recorded yet\n"),
    }
    Ok(out)
}

/// A single-flight lock so a scheduled run and a manual `observe` cannot
/// interleave. Backed by a plain lock file under the store directory
/// holding `pid<TAB>started_at`; not `flock` because the loser needs to
/// print a friendly message rather than block.
pub struct LockGuard {
    store: PathBuf,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        let _ = store::ObserveLock { store: &self.store }.remove();
    }
}

fn lock_path(store_dir: &Path) -> PathBuf {
    store::ObserveLock { store: store_dir }.path()
}

fn pid_alive(pid: u32) -> bool {
    fs_gate::spawn::run(
        fs_gate::spawn::Program::Kill,
        ["-0", &pid.to_string()],
        std::time::Duration::from_secs(5),
    )
    .map(|o| o.success())
    .unwrap_or(false)
}

/// Result of trying to take the single-flight observation lock.
pub enum LockOutcome {
    Acquired(LockGuard),
    HeldBy { pid: u32, since: u64 },
}

/// Attempts to take the lock. A stale lock (owner pid no longer alive) is
/// reclaimed automatically.
pub fn acquire_lock(store_dir: &Path) -> Result<LockOutcome> {
    store::create_dir_all(store_dir)?;
    let path = lock_path(store_dir);

    loop {
        let pid = std::process::id();
        let since = crate::entities::now();
        match (store::ObserveLock { store: store_dir }).create(&format!("{pid}\t{since}\n")) {
            Ok(()) => {
                return Ok(LockOutcome::Acquired(LockGuard {
                    store: store_dir.to_path_buf(),
                }));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let contents = read_owned_string(&path).unwrap_or_default();
                let mut parts = contents.trim().splitn(2, '\t');
                let pid: Option<u32> = parts.next().and_then(|p| p.parse().ok());
                let since: u64 = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
                match pid {
                    Some(pid) if pid_alive(pid) => {
                        return Ok(LockOutcome::HeldBy { pid, since });
                    }
                    _ => {
                        // Stale lock: owner is gone or unparsable. Reclaim
                        // and retry once.
                        let _ = store::ObserveLock { store: store_dir }.remove(); // our own stale lock, owner pid confirmed dead
                        continue;
                    }
                }
            }
            Err(e) => return Err(e).context("create observe lock"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn parses_and_formats_intervals() {
        assert_eq!(parse_interval("30m").unwrap(), 1800);
        assert_eq!(parse_interval("1h").unwrap(), 3600);
        assert!(parse_interval("").is_err());
        assert!(parse_interval("bogus").is_err());
        assert!(parse_interval("0").is_err());
        assert_eq!(format_interval(1800), "30m");
        assert_eq!(format_interval(3600), "1h");
        assert_eq!(format_interval(86400), "1d");
        assert_eq!(format_interval(90), "90s");
    }

    #[test]
    fn plist_golden_content() {
        let exe = PathBuf::from("/usr/local/bin/swamp");
        let roots = vec![PathBuf::from("/Users/test/src")];
        let log = PathBuf::from("/Users/test/Library/Logs/swamp/observe.log");
        let body = render_plist(&exe, &roots, 1800, &log, Some("/tmp/store"));
        assert!(body.contains(&format!("<string>{LABEL}</string>")));
        assert!(body.contains("<string>/usr/local/bin/swamp</string>"));
        assert!(body.contains("<string>observe</string>"));
        assert!(body.contains("<string>/Users/test/src</string>"));
        assert!(body.contains("<integer>1800</integer>"));
        assert!(body.contains("<string>Background</string>"));
        assert!(body.contains("<key>LowPriorityIO</key>\n<true/>"));
        assert!(body.contains("<key>Nice</key>\n<integer>10</integer>"));
        assert!(body.contains("<key>RunAtLoad</key>\n<false/>"));
        assert!(body.contains("SWAMP_DIR"));
        assert!(body.contains("/tmp/store"));
        assert!(body.contains(&log.to_string_lossy().to_string()));

        let seconds = installed_interval(&body);
        assert_eq!(seconds, Some(1800));
        let roots_read = installed_roots(&body);
        assert_eq!(roots_read, vec!["/Users/test/src".to_string()]);
    }

    #[test]
    fn log_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("observe.log");
        let outcome = RunOutcome {
            observed_at: 1_000,
            wall_ms: 4200,
            walked_total: 12345,
            projects: 4,
            mode: "full".to_string(),
            outcome: "ok".to_string(),
        };
        append_log(&log, &outcome).unwrap();
        let read = last_log_outcome(&log).unwrap();
        assert_eq!(read.observed_at, 1_000);
        assert_eq!(read.wall_ms, 4200);
        assert_eq!(read.mode, "full");
        assert_eq!(read.outcome, "ok");
    }

    #[test]
    fn lock_prevents_second_observer() {
        let tmp = tempfile::tempdir().unwrap();
        let first = acquire_lock(tmp.path()).unwrap();
        assert!(matches!(first, LockOutcome::Acquired(_)));
        let second = acquire_lock(tmp.path()).unwrap();
        match second {
            LockOutcome::HeldBy { pid, .. } => assert_eq!(pid, std::process::id()),
            LockOutcome::Acquired(_) => panic!("second acquire should not succeed"),
        }
        drop(first);
        let third = acquire_lock(tmp.path()).unwrap();
        assert!(matches!(third, LockOutcome::Acquired(_)));
    }

    #[test]
    fn stale_lock_is_reclaimed() {
        let tmp = tempfile::tempdir().unwrap();
        let path = lock_path(tmp.path());
        fs::write(&path, "999999999\t1\n").unwrap();
        let outcome = acquire_lock(tmp.path()).unwrap();
        assert!(matches!(outcome, LockOutcome::Acquired(_)));
    }

    // Tests below mutate process-global env vars (SWAMP_* dirs and
    // test mode) to point launchd/plist/log paths at a temp dir instead
    // of the real machine. Serialized so parallel `cargo test` threads
    // never observe each other's env var.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn header_line_reports_no_schedule_when_plist_missing() {
        let _guard = ENV_LOCK.lock().unwrap();
        // SWAMP_LAUNCH_AGENTS_DIR points somewhere with no plist.
        let tmp = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", tmp.path());
        }
        let store = tempfile::tempdir().unwrap();
        let line = header_line(store.path(), Path::new("/Users/test/src"), 0);
        assert!(line.contains("no schedule"));
        assert!(line.contains("swamp schedule --every 30m"));
        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
        }
    }

    #[test]
    fn off_removes_plist_and_issues_bootout_in_test_mode() {
        let _guard = ENV_LOCK.lock().unwrap();
        let agents = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", agents.path());
            std::env::set_var("SWAMP_TEST_MODE", "1");
        }
        let plist = plist_path();
        fs::write(&plist, "placeholder").unwrap();
        assert!(plist.exists());

        let message = uninstall().unwrap();
        assert!(!plist.exists(), "uninstall must remove the plist file");
        assert!(message.contains("Removed the scheduled observation"));

        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
            std::env::remove_var("SWAMP_TEST_MODE");
        }
    }

    #[test]
    fn off_with_no_plist_still_reports_no_schedule() {
        let _guard = ENV_LOCK.lock().unwrap();
        let agents = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", agents.path());
            std::env::set_var("SWAMP_TEST_MODE", "1");
        }
        let message = uninstall().unwrap();
        assert!(message.contains("No scheduled observation is installed"));
        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
            std::env::remove_var("SWAMP_TEST_MODE");
        }
    }

    #[test]
    fn status_parses_a_fixture_log_and_installed_plist() {
        let _guard = ENV_LOCK.lock().unwrap();
        let agents = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", agents.path());
        }
        let plist = plist_path();
        let exe = PathBuf::from("/usr/local/bin/swamp");
        let body = render_plist(
            &exe,
            &[PathBuf::from("/Users/test/src")],
            1800,
            &PathBuf::from("/tmp/log"),
            None,
        );
        fs::write(&plist, body).unwrap();

        let store = tempfile::tempdir().unwrap();
        let outcome = RunOutcome {
            observed_at: 1_000,
            wall_ms: 4200,
            walked_total: 999,
            projects: 3,
            mode: "full".to_string(),
            outcome: "ok".to_string(),
        };
        write_last_run(store.path(), &outcome).unwrap();

        let text = status(store.path()).unwrap();
        assert!(text.contains("installed"));
        assert!(text.contains("30m"));
        assert!(text.contains("/Users/test/src"));
        assert!(text.contains("3 projects"));
        assert!(text.contains("4.2 s"));

        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
        }
    }

    /// #42/#50: `install` with no explicit roots must not bail (it used
    /// to require at least one), must write a plist whose
    /// `ProgramArguments` names no root at all beyond `observe` itself,
    /// and `status` must say so plainly rather than printing a blank
    /// "Roots:" line -- the tempting shortcut this guards against is
    /// resolving the scope once at install time and freezing the result
    /// into the plist, which would silently stop tracking a later config
    /// edit until `schedule --every` was run again.
    #[test]
    fn install_with_no_roots_freezes_nothing_and_status_says_so() {
        let _guard = ENV_LOCK.lock().unwrap();
        let agents = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", agents.path());
            std::env::set_var("SWAMP_LOG_DIR", logs.path());
            std::env::set_var("SWAMP_TEST_MODE", "1");
        }

        let message = install("30m", &[]).unwrap();
        assert!(message.contains("resolved fresh on every run"), "{message}");

        let plist_text = fs::read_to_string(plist_path()).unwrap();
        assert!(
            installed_roots(&plist_text).is_empty(),
            "no root should be frozen into the plist's argv: {plist_text}"
        );
        // The launched command is still exactly `<exe> observe` -- no
        // trailing empty-string argument sneaking in from an empty loop.
        let array_block = plist_text
            .split("<key>ProgramArguments</key>")
            .nth(1)
            .and_then(|s| s.split("</array>").next())
            .unwrap();
        assert_eq!(
            array_block.matches("<string>").count(),
            2,
            "exactly exe + \"observe\", no root strings: {array_block}"
        );

        let store = tempfile::tempdir().unwrap();
        let status_text = status(store.path()).unwrap();
        assert!(
            status_text.contains("(configured scope, resolved fresh on every run)"),
            "{status_text}"
        );

        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
            std::env::remove_var("SWAMP_LOG_DIR");
            std::env::remove_var("SWAMP_TEST_MODE");
        }
    }

    /// An explicit root list must still be frozen into the plist exactly
    /// as before -- only the *no-roots* case changed behavior.
    #[test]
    fn install_with_explicit_roots_still_freezes_them() {
        let _guard = ENV_LOCK.lock().unwrap();
        let agents = tempfile::tempdir().unwrap();
        let logs = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("SWAMP_LAUNCH_AGENTS_DIR", agents.path());
            std::env::set_var("SWAMP_LOG_DIR", logs.path());
            std::env::set_var("SWAMP_TEST_MODE", "1");
        }

        install("30m", &[PathBuf::from("/Users/test/src")]).unwrap();
        let plist_text = fs::read_to_string(plist_path()).unwrap();
        assert_eq!(
            installed_roots(&plist_text),
            vec!["/Users/test/src".to_string()]
        );

        unsafe {
            std::env::remove_var("SWAMP_LAUNCH_AGENTS_DIR");
            std::env::remove_var("SWAMP_LOG_DIR");
            std::env::remove_var("SWAMP_TEST_MODE");
        }
    }

    #[test]
    fn timeout_outcome_writes_a_log_line() {
        let tmp = tempfile::tempdir().unwrap();
        let log = tmp.path().join("observe.log");
        let outcome = RunOutcome {
            observed_at: 5_000,
            wall_ms: 1_800_000,
            walked_total: 0,
            projects: 0,
            mode: "full".to_string(),
            outcome: "timeout".to_string(),
        };
        append_log(&log, &outcome).unwrap();
        let read = last_log_outcome(&log).unwrap();
        assert_eq!(read.outcome, "timeout");
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("outcome=timeout"));
    }
}
