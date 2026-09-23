//! target: crates/tui/src/app.rs
//! by: audit:gate_paths_only_inside_gates
//! source: corpus, 2026-09-22
//! ported: 2026-09-23 -- the CLI confirmation names the plan it was shown (`cli_approve`), pinned to `cmd_approve`
//! why: a key handler approving a plan with a confirmation it minted itself, as if the CLI had asked
impl App {
    fn sweep_approve_on_key(&mut self, dir: &std::path::Path, plan: &swamp_core::actions::Plan) {
        let confirmed = swamp_core::authority::HumanConfirmed::cli_approve("agent", plan);
        let _ = swamp_core::actions::approve_confirmed(dir, &plan.id, confirmed);
    }
}
