//! Source audits: every technical constraint in `.oh/guardrails/` as a
//! named check over the AST. `--list` prints the names (what a guardrail
//! or ADR may reference); a plain run executes them all and fails on the
//! first broken constraint per audit, naming the file and shape.

mod ast;
mod audits;

use std::path::Path;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--list") {
        for (name, _) in audits::AUDITS {
            println!("{name}");
        }
        return;
    }
    let only: Vec<&str> = args.iter().map(String::as_str).collect();
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
