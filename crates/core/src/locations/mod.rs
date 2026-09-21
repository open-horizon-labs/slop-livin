//! Source-aware registry for developer-storage location detection (#44).
//!
//! Detectors *propose* locations; they never authorize measurement,
//! attribution, or removal (see `.oh/guardrails/extractors-are-pluggable.md`
//! for the sibling discipline this mirrors). This registry is deliberately
//! **not** an `EventBus` consumer (`crate::bus`): the bus's consumers react
//! to walk events for an already-chosen root, while a detector's whole job
//! is figuring out which roots to walk in the first place. Reusing the bus
//! here would mean inventing walk events for a walk that has not started.
//!
//! What *is* reused from the bus's discipline: static registration
//! (`Registry::with_builtins`), no detector knows about another detector,
//! and adding a new detector is "write one small file, register it in
//! `with_builtins`" -- never a change to `scope.rs` or the walker. See
//! `docs/architecture.md`'s "Location detector registry" section for the
//! worked pattern to copy for a new detector.
//!
//! Detectors are read-only. They may inspect environment variables, the
//! home directory's conventional layout, and -- for a handful of detectors
//! -- run one bounded, allow-listed, read-only tool query (`brew --prefix`,
//! never a project script or shell startup file). A failed tool query is
//! reported on the location it would have resolved, never fatal to the
//! rest of detection.

pub mod aider;
pub mod android;
pub mod asdf;
pub mod builtin;
pub mod cargo_home;
pub mod claude_code;
pub mod cline;
pub mod codex;
pub mod codex_desktop;
pub mod conda;
pub mod continue_dev;
pub mod copilot_cli;
pub mod core_simulator;
pub mod cursor;
pub mod docker_desktop;
pub mod gemini_cli;
pub mod go;
pub mod gradle;
pub mod homebrew;
pub mod huggingface;
pub mod maven;
pub mod mise;
pub mod npm;
pub mod nvm;
pub mod oh_my_pi;
pub mod ollama;
pub mod opencode;
pub mod pi;
pub mod pip;
pub mod pnpm;
pub mod pyenv;
pub mod rbenv;
pub mod roo_code;
pub mod ruby_install;
pub mod rustup;
pub mod rvm;
pub mod uv;
pub mod vscode_hosts;
pub mod windsurf;
pub mod xcode;

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Bumped whenever the set of detectors or their resolution semantics
/// changes, so a persisted `EffectiveScope` (`crate::scope`) can show it
/// was resolved under an older catalog than the one now running.
pub const CATALOG_VERSION: &str = "2026-09-21.4";

/// Detection platform. Data, not a compile-time cfg: tests inject any
/// value so a Linux-configured `Environment` can be asserted to produce
/// zero macOS-specific paths without needing a second build target.
/// [`Platform::current`] is the only place this crate lets the *real*
/// process's OS pick a value, and it is `#[cfg]`-gated so a non-macOS
/// build of this crate never even compiles the macOS defaults path as
/// "current" -- Linux's own defaults are #84's job; this only guarantees
/// today's macOS table cannot silently leak into that later build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Platform {
    MacOS,
    Linux,
}

impl Platform {
    #[cfg(target_os = "macos")]
    pub fn current() -> Self {
        Platform::MacOS
    }

    #[cfg(target_os = "linux")]
    pub fn current() -> Self {
        Platform::Linux
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    pub fn current() -> Self {
        // No built-in table exists for this target; detectors that gate
        // on a specific platform simply propose nothing.
        Platform::Linux
    }
}

/// What kind of developer storage a proposed location holds. Deliberately
/// coarse -- fine-grained role identification (tests vs. deps vs. output)
/// is the build-artifact-identification epic's job (#74), not this one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StorageCategory {
    Installation,
    Downloads,
    Cache,
    LocalState,
    Environments,
    BuildOutput,
    Models,
    Unclassified,
}

/// How a detector arrived at a candidate path. Kept even after the path
/// is folded into scope, so a human can ask "why does swamp think this is
/// here" without re-running detection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Provenance {
    /// A hard-coded convention this detector always knows about
    /// (`~/.cargo`, `/opt/homebrew`), independent of any tool being
    /// installed -- this is what lets a leftover cache be found after
    /// its manager was uninstalled.
    BuiltinConvention,
    /// Resolved from this named environment variable's value.
    EnvVar(String),
    /// Resolved from a field in a config file the detector read.
    ConfigField(String),
    /// Resolved by running this bounded, read-only, allow-listed command.
    ToolQuery(String),
}

/// Whether a detector actually produced a usable path for this candidate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "kebab-case")]
pub enum LocationStatus {
    /// The detector determined this path; scope resolution still checks
    /// whether it exists on disk (see `crate::scope::RootStatus`).
    Resolved,
    /// The detector positively determined this candidate does not apply
    /// here (e.g. no override and no known convention path for this
    /// platform).
    NotPresent,
    /// Excluded from scope by `disabled_detectors` (or `defaults = false`
    /// for the built-in-defaults detector); still reported for
    /// transparency, never silently dropped.
    Disabled,
    /// A tool query or other resolution step failed; the reason is
    /// surfaced rather than treated as "not present".
    UnresolvedWithReason { reason: String },
}

/// One candidate developer-storage location a detector proposes.
/// Discovery only: this never authorizes measurement, attribution, or
/// removal of anything at `path`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposedLocation {
    pub detector_id: String,
    /// Present exactly when `status == Resolved`.
    pub path: Option<PathBuf>,
    pub category: StorageCategory,
    pub provenance: Provenance,
    pub status: LocationStatus,
    /// Free-text note a detector can attach (e.g. which of two
    /// conventional prefixes this is), never used for control flow.
    pub note: Option<String>,
}

/// Outcome of a read-only command a detector wants to run.
pub struct CommandOutcome {
    pub stdout: String,
    pub success: bool,
}

/// Runs a detector's tool queries. The only implementation shipped for
/// real use is [`SystemCommandRunner`], which enforces a fixed
/// allow-list of `(program, args)` pairs *in addition to* each detector
/// only ever constructing an allow-listed call -- belt and suspenders,
/// so a future detector cannot introduce a side-effecting command by
/// mistake. Tests use [`FakeCommandRunner`], which records every call it
/// receives so a test can assert a detector never asked for anything
/// beyond the documented allow-list, even when the fixture's canned
/// answer would happily satisfy a broader request.
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutcome, String>;
}

/// Every command any detector in this catalog is allowed to run. A
/// detector that needs a new query adds one line here, in the open,
/// reviewable alongside the detector itself -- never a free-form string
/// built at call time.
pub const ALLOWED_COMMANDS: &[(&str, &[&str])] = &[
    ("brew", &["--prefix"]),
    (
        "defaults",
        &["read", "com.apple.dt.Xcode", "IDECustomDerivedDataLocation"],
    ),
];

/// Runs allow-listed commands for real, bounded by `timeout`. Refuses
/// (without spawning a process) anything not on [`ALLOWED_COMMANDS`].
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        timeout: Duration,
    ) -> Result<CommandOutcome, String> {
        if !ALLOWED_COMMANDS
            .iter()
            .any(|(p, a)| *p == program && *a == args)
        {
            return Err(format!(
                "refusing to run non-allow-listed command: {program} {args:?}"
            ));
        }
        let mut child = Command::new(program)
            .args(args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("{program} {args:?}: spawn failed: {e}"))?;
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    use std::io::Read;
                    let mut out = String::new();
                    if let Some(mut stdout) = child.stdout.take() {
                        let _ = stdout.read_to_string(&mut out);
                    }
                    return Ok(CommandOutcome {
                        stdout: out.trim().to_string(),
                        success: status.success(),
                    });
                }
                Ok(None) => {
                    if start.elapsed() >= timeout {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Err(format!("{program} {args:?}: timed out after {timeout:?}"));
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("{program} {args:?}: wait failed: {e}")),
            }
        }
    }
}

/// A command runner that never runs anything and reports every call as
/// unavailable -- the default for fixture environments that do not
/// explicitly opt into tool queries, so a test that forgets to inject a
/// [`FakeCommandRunner`] fails loudly (`UnresolvedWithReason`) instead of
/// quietly reaching the real `brew` on the machine running the test.
pub struct NullCommandRunner;

impl CommandRunner for NullCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _timeout: Duration,
    ) -> Result<CommandOutcome, String> {
        Err(format!(
            "no command runner configured for {program} {args:?}"
        ))
    }
}

/// Records every call it receives (even a refused one) so a test can
/// assert a detector only ever asked for allow-listed, read-only
/// commands -- the adversarial check #44 requires: a detector whose tool
/// query *would* have a side effect if it ran for real must never
/// actually run it.
pub struct FakeCommandRunner {
    pub calls: std::sync::Mutex<Vec<(String, Vec<String>)>>,
    pub answers: HashMap<(&'static str, &'static [&'static str]), Result<String, String>>,
}

impl FakeCommandRunner {
    pub fn new() -> Self {
        Self {
            calls: std::sync::Mutex::new(Vec::new()),
            answers: HashMap::new(),
        }
    }

    pub fn with_answer(
        mut self,
        program: &'static str,
        args: &'static [&'static str],
        out: &str,
    ) -> Self {
        self.answers.insert((program, args), Ok(out.to_string()));
        self
    }

    pub fn with_failure(
        mut self,
        program: &'static str,
        args: &'static [&'static str],
        reason: &str,
    ) -> Self {
        self.answers
            .insert((program, args), Err(reason.to_string()));
        self
    }

    pub fn calls(&self) -> Vec<(String, Vec<String>)> {
        self.calls.lock().unwrap().clone()
    }
}

impl Default for FakeCommandRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl CommandRunner for FakeCommandRunner {
    fn run(
        &self,
        program: &str,
        args: &[&str],
        _timeout: Duration,
    ) -> Result<CommandOutcome, String> {
        self.calls.lock().unwrap().push((
            program.to_string(),
            args.iter().map(|s| s.to_string()).collect(),
        ));
        if !ALLOWED_COMMANDS
            .iter()
            .any(|(p, a)| *p == program && *a == args)
        {
            return Err(format!(
                "refusing to run non-allow-listed command: {program} {args:?}"
            ));
        }
        match self
            .answers
            .iter()
            .find(|((p, a), _)| *p == program && *a == args)
        {
            Some((_, Ok(out))) => Ok(CommandOutcome {
                stdout: out.clone(),
                success: true,
            }),
            Some((_, Err(reason))) => Err(reason.clone()),
            None => Err(format!("no fixture answer for {program} {args:?}")),
        }
    }
}

/// Everything a detector is allowed to see, injected so tests never scan
/// the developer's real home (#44's fixture-injection requirement).
pub struct Environment {
    pub home: PathBuf,
    pub env: HashMap<String, String>,
    pub platform: Platform,
    pub runner: Arc<dyn CommandRunner>,
    /// Bound applied to every tool query a detector runs through this
    /// environment.
    pub command_timeout: Duration,
}

impl Environment {
    /// The real process's home/env/platform, with the real (allow-listed)
    /// command runner. Only ever constructed at the CLI's entry point.
    pub fn from_process() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            home,
            env: std::env::vars().collect(),
            platform: Platform::current(),
            runner: Arc::new(SystemCommandRunner),
            command_timeout: Duration::from_secs(2),
        }
    }

    /// A fixture environment: explicit home/env/platform, no live tool
    /// queries unless a [`FakeCommandRunner`] is supplied with
    /// [`Environment::with_runner`].
    pub fn fixture(home: PathBuf, env: HashMap<String, String>, platform: Platform) -> Self {
        Self {
            home,
            env,
            platform,
            runner: Arc::new(NullCommandRunner),
            command_timeout: Duration::from_millis(50),
        }
    }

    pub fn with_runner(mut self, runner: Arc<dyn CommandRunner>) -> Self {
        self.runner = runner;
        self
    }

    pub fn env_var(&self, key: &str) -> Option<&str> {
        self.env.get(key).map(|s| s.as_str())
    }

    pub fn run_command(&self, program: &str, args: &[&str]) -> Result<CommandOutcome, String> {
        self.runner.run(program, args, self.command_timeout)
    }
}

/// One fact source under the registry. Copy `builtin.rs` or
/// `cargo_home.rs` for the pattern; see `docs/architecture.md`.
pub trait Detector: Send + Sync {
    /// Stable, never-reused identifier (used in `disabled_detectors`
    /// config and in `EffectiveScope` provenance).
    fn id(&self) -> &'static str;
    /// Human-readable name for `swamp scope` output.
    fn name(&self) -> &'static str;
    /// Platforms this detector applies to. Checked by the registry, not
    /// by `#[cfg]`, so a fixture can exercise a Linux-only detector on a
    /// macOS test run.
    fn platforms(&self) -> &'static [Platform];
    /// One or more supported-version/format notes for `swamp scope`
    /// output and docs generation; not machine-checked.
    fn version_note(&self) -> &'static str {
        "current documented layout"
    }
    fn detect(&self, env: &Environment) -> Vec<ProposedLocation>;
}

/// Static registration, mirroring `EventBus::with_builtins`: every
/// detector is registered here before the first call to
/// [`Registry::resolve`], and nothing registers a detector at runtime.
pub struct Registry {
    detectors: Vec<Box<dyn Detector>>,
}

impl Registry {
    pub fn with_builtins() -> Self {
        Self {
            detectors: vec![
                Box::new(builtin::BuiltinDefaultsDetector),
                Box::new(cargo_home::CargoHomeDetector),
                Box::new(rustup::RustupDetector),
                Box::new(homebrew::HomebrewDetector),
                Box::new(claude_code::ClaudeCodeDetector),
                Box::new(codex::CodexDetector),
                Box::new(codex_desktop::CodexDesktopDetector),
                Box::new(oh_my_pi::OhMyPiDetector),
                Box::new(opencode::OpenCodeDetector),
                Box::new(gemini_cli::GeminiCliDetector),
                Box::new(pi::PiDetector),
                Box::new(aider::AiderDetector),
                Box::new(copilot_cli::CopilotCliDetector),
                Box::new(cursor::CursorDetector),
                Box::new(windsurf::WindsurfDetector),
                Box::new(cline::ClineDetector),
                Box::new(roo_code::RooCodeDetector),
                Box::new(continue_dev::ContinueDetector),
                Box::new(mise::MiseDetector),
                Box::new(asdf::AsdfDetector),
                Box::new(pyenv::PyenvDetector),
                Box::new(uv::UvDetector),
                Box::new(conda::CondaDetector),
                Box::new(rbenv::RbenvDetector),
                Box::new(rvm::RvmDetector),
                Box::new(ruby_install::RubyInstallDetector),
                Box::new(nvm::NvmDetector),
                Box::new(npm::NpmDetector),
                Box::new(pnpm::PnpmDetector),
                Box::new(gradle::GradleDetector),
                Box::new(maven::MavenDetector),
                Box::new(go::GoDetector),
                Box::new(pip::PipDetector),
                Box::new(xcode::XcodeDetector),
                Box::new(core_simulator::CoreSimulatorDetector),
                Box::new(android::AndroidDetector),
                Box::new(huggingface::HuggingFaceDetector),
                Box::new(ollama::OllamaDetector),
                Box::new(docker_desktop::DockerDesktopDetector),
            ],
        }
    }

    pub fn detectors(&self) -> &[Box<dyn Detector>] {
        &self.detectors
    }

    /// Runs every detector applicable to `env.platform`, marking any
    /// whose `id()` is in `disabled` as `Disabled` without calling
    /// `detect` at all (a disabled detector never even touches the
    /// environment or runs a tool query). Results are grouped by
    /// detector in registration order; callers needing a flat,
    /// deduplicated candidate list should go through
    /// `crate::scope::resolve_effective_scope`, which also handles
    /// cross-detector deduplication (two detectors resolving the same
    /// canonical path).
    pub fn resolve(
        &self,
        env: &Environment,
        disabled: &[String],
    ) -> Vec<(String, Vec<ProposedLocation>)> {
        self.detectors
            .iter()
            .filter(|d| d.platforms().contains(&env.platform))
            .map(|d| {
                let id = d.id().to_string();
                if disabled.iter().any(|x| x == d.id()) {
                    (
                        id.clone(),
                        vec![ProposedLocation {
                            detector_id: id,
                            path: None,
                            category: StorageCategory::Unclassified,
                            provenance: Provenance::BuiltinConvention,
                            status: LocationStatus::Disabled,
                            note: Some(format!("{} disabled by configuration", d.name())),
                        }],
                    )
                } else {
                    (id, d.detect(env))
                }
            })
            .collect()
    }
}

impl Default for Registry {
    fn default() -> Self {
        Self::with_builtins()
    }
}
