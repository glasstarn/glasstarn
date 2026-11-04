//! The Jira issue-lifecycle pack on a made-up extract whose contents are
//! known by construction.
mod common;
use pm_ingest::pack::{BUILT_IN, Pack, build_recipe, check};
use pm_testgen::jira::{FILES, Settings, write};
use std::collections::BTreeMap;

#[test]
fn issue_lifecycle_yields_every_planted_step() {
    let pack = Pack::parse(
        BUILT_IN
            .iter()
            .find(|(id, _)| *id == "jira-issue-lifecycle")
            .unwrap()
            .1,
    )
    .unwrap();
    let dir = std::env::temp_dir().join(format!("pm-jira-pack-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write(&dir, &Settings::default()).unwrap();
    let files = FILES
        .iter()
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect::<BTreeMap<_, _>>();
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    // The company's own fields are found and asked about.
    let impact = report
        .questions
        .iter()
        .find(|q| q.id == "custom:JiraIssue.customfield_10200")
        .unwrap();
    assert_eq!(impact.recommended.as_deref(), Some("detail"));
    assert!(truth.with_customer_impact > 0);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(&pack, &hashes, &BTreeMap::new()).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    let report = &outcome.report;
    assert_eq!(report.issue_count, 0, "{:?}", report.issues);
    let emitted = |rule: &str| report.rules[rule].emitted;
    assert_eq!(emitted("issue_created"), truth.issues);
    assert_eq!(emitted("status_changed"), truth.status_changes);
    assert_eq!(emitted("issue_reassigned"), truth.reassignments);
    assert_eq!(emitted("priority_changed"), truth.priority_changes);
    assert_eq!(emitted("field_changed"), truth.story_point_changes);
    std::fs::remove_dir_all(dir).unwrap();
}
