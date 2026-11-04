//! The Pega, Appian and Power Automate packs on made-up extracts whose
//! contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::workflow_platforms::{
    APPIAN_FILES, PEGA_FILES, POWER_AUTOMATE_FILES, Settings, write_appian, write_pega,
    write_power_automate,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn files(dir: &Path, names: &[&str]) -> BTreeMap<String, PathBuf> {
    names
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect()
}
fn dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}
fn run(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
    answers: BTreeMap<String, Answer>,
) -> BTreeMap<String, u64> {
    let ready = pack.with_settings(&answers).unwrap();
    let report = check(&ready, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect()
}

#[test]
fn pega_cases_yield_every_planted_step() {
    let dir = dir("pega");
    let truth = write_pega(&dir, &Settings::default()).unwrap();
    // The company says which history column names the step.
    let answers = BTreeMap::from([(
        "setting:step_field".to_owned(),
        Answer {
            choice: "value".into(),
            value: Some("pyMemo".into()),
            confirmed_by: "Claims operations lead".into(),
            at: "2026-10-14T00:00:00Z".into(),
            question: String::new(),
            carries: Vec::new(),
        },
    )]);
    let emitted = run(
        &pack("pega-case-management"),
        &files(&dir, &PEGA_FILES),
        answers,
    );
    assert_eq!(emitted["pega_case_created"], truth.cases);
    assert_eq!(emitted["pega_history"], truth.history_rows);
    assert_eq!(emitted["pega_case_resolved"], truth.resolved);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn appian_record_events_yield_every_planted_step() {
    let dir = dir("appian");
    let truth = write_appian(&dir, &Settings::default()).unwrap();
    let emitted = run(
        &pack("appian-record-events"),
        &files(&dir, &APPIAN_FILES),
        BTreeMap::new(),
    );
    assert_eq!(emitted["appian_event"], truth.events);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn power_automate_yields_every_planted_step() {
    let dir = dir("power-automate");
    let truth = write_power_automate(&dir, &Settings::default()).unwrap();
    let pack = pack("power-automate-dataverse");
    let emitted = run(&pack, &files(&dir, &POWER_AUTOMATE_FILES), BTreeMap::new());
    assert_eq!(emitted["pa_record_changed"], truth.audit_rows);
    assert_eq!(emitted["pa_run_started"], truth.runs);
    assert_eq!(emitted["pa_run_succeeded"], truth.succeeded);
    assert_eq!(emitted["pa_run_failed"], truth.failed);
    // The audit log alone is enough.
    let mut core = files(&dir, &POWER_AUTOMATE_FILES);
    core.remove("flowrun");
    core.remove("workflow");
    let emitted = run(&pack, &core, BTreeMap::new());
    assert_eq!(emitted["pa_record_changed"], truth.audit_rows);
    std::fs::remove_dir_all(dir).unwrap();
}
