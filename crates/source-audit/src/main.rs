//! Source audits: every technical constraint in `.oh/guardrails/` as a
//! named check over the AST. `--list` prints the names (what a guardrail
//! or ADR may reference); a plain run executes them all and fails on the
//! first broken constraint per audit, naming the file and shape.

mod ast;
mod audits;
mod repair_audits;
mod tui_nonblocking;

use std::path::{Path, PathBuf};

fn main() {
    let default_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        for (name, _) in audits::AUDITS {
            println!("{name}");
        }
        return;
    }
    // `--root <dir>` audits another checkout of this workspace. That is
    // how the pre-repair baseline in
    // `.oh/sessions/2026-09-21-foundation-repairs.md` was produced: the
    // audits had to enumerate violations in the *reviewed* code, not in
    // a working tree that was already being repaired.
    let mut root_buf = default_root;
    let mut only: Vec<&str> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--root" && i + 1 < args.len() {
            root_buf = PathBuf::from(&args[i + 1]);
            i += 2;
            continue;
        }
        only.push(args[i].as_str());
        i += 1;
    }
    let root: &Path = &root_buf;
    let mut failed = 0;
    for (name, audit) in audits::AUDITS {
        if !only.is_empty() && !only.contains(name) {
            continue;
        }
        match audit(root) {
            Ok(()) => println!("ok    {name}"),
            Err(e) => {
                failed += 1;
                println!("FAIL  {name}: {e}");
            }
        }
    }
    if failed > 0 {
        eprintln!("{failed} audit(s) failed");
        std::process::exit(1);
    }
}
