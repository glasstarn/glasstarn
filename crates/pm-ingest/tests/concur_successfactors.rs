//! The SAP Concur and SuccessFactors packs on made-up extracts whose contents
//! are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn answer(choice: &str, value: &str) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Finance analyst".into(),
        at: "2026-10-14T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
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
    let report = check(pack, files).unwrap();
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
fn concur_expenses_yield_every_planted_step() {
    use pm_testgen::concur::{EXPENSE_FILES, Settings, write_expenses};
    let dir = dir("concur");
    let truth = write_expenses(&dir, &Settings::default()).unwrap();
    assert!(truth.sent_back > 0);
    let answers = BTreeMap::from([(
        "setting:concur_offset".to_owned(),
        answer("value", "+01:00"),
    )]);
    let emitted = run(
        &pack("concur-expense"),
        &files(&dir, &EXPENSE_FILES),
        answers,
    );
    assert_eq!(emitted["report_created"], truth.reports);
    assert_eq!(emitted["expense_incurred"], truth.entries);
    assert_eq!(emitted["workflow_action"], truth.actions);
    assert_eq!(emitted["employee_paid"], truth.paid);
    assert_eq!(emitted["entry_on_report"], truth.entries);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn successfactors_hiring_yields_every_planted_step() {
    use pm_testgen::successfactors::{RECRUITING_FILES, Settings, write_recruiting};
    let dir = dir("successfactors");
    let truth = write_recruiting(&dir, &Settings::default()).unwrap();
    assert!(truth.hired > 0);
    let pack = pack("successfactors-recruiting");
    let emitted = run(&pack, &files(&dir, &RECRUITING_FILES), BTreeMap::new());
    assert_eq!(emitted["requisition_opened"], truth.requisitions);
    assert_eq!(emitted["requisition_closed"], truth.closed_requisitions);
    assert_eq!(emitted["application_received"], truth.applications);
    assert_eq!(emitted["application_status_changed"], truth.status_changes);
    assert_eq!(emitted["interview_held"], truth.interviews);
    assert_eq!(emitted["onboarding_started"], truth.onboarding_started);
    assert_eq!(emitted["onboarding_completed"], truth.onboarding_completed);
    assert_eq!(emitted["application_for_requisition"], truth.applications);
    let mut core = files(&dir, &RECRUITING_FILES);
    core.remove("JobApplicationInterview");
    core.remove("ONB2Process");
    let emitted = run(&pack, &core, BTreeMap::new());
    assert!(!emitted.contains_key("interview_held"));
    std::fs::remove_dir_all(dir).unwrap();
}
