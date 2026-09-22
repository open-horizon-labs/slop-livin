//! target: crates/core/src/report.rs
//! why: the composed guardrail's third half -- the report path calling a stage instead of publishing an event
pub fn sweep_report_calls_stage_directly(dir: &std::path::Path) {
    let _ = crate::growth::observe_and_annotate_files(dir, &[], 1_000, 30);
}
