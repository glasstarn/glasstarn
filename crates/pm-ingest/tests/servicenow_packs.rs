//! The ServiceNow packs on made-up extracts whose contents are known by
//! construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::servicenow::{
    CHANGE_FILES, HR_CASES, INCIDENT_FILES, PROBLEMS, REQUEST_FILES, Settings, TicketKind,
    write_changes, write_incidents, write_requests, write_tickets,
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

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("pm-sn-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Checks and builds a pack; returns the per-rule emitted counts and the
/// step names.
fn build(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
) -> (BTreeMap<String, u64>, std::collections::BTreeSet<String>) {
    build_with(pack, files, &BTreeMap::new())
}

fn build_with(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
    answers: &BTreeMap<String, Answer>,
) -> (BTreeMap<String, u64>, std::collections::BTreeSet<String>) {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let emitted = outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect();
    let log = serde_json::to_value(outcome.log.as_ref().unwrap()).unwrap();
    let names = log["eventTypes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_owned())
        .collect();
    (emitted, names)
}

#[test]
fn incident_management_yields_every_planted_step() {
    let dir = scratch("incident");
    let truth = write_incidents(&dir, &Settings::default()).unwrap();
    let files = files(&dir, &INCIDENT_FILES);
    let pack = pack("servicenow-incident-management");
    // The company's own field is found and asked about.
    let report = check(&pack, &files).unwrap();
    assert!(
        report
            .questions
            .iter()
            .any(|q| q.id == "custom:ServiceNowIncident.u_business_impact")
    );
    assert!(truth.with_business_impact > 0);
    let (emitted, names) = build(&pack, &files);
    assert_eq!(emitted["incident_opened"], truth.incidents);
    assert_eq!(emitted["state_changed"], truth.state_changes);
    assert_eq!(emitted["group_assigned"], truth.group_assigned);
    assert_eq!(emitted["group_reassigned"], truth.group_reassigned);
    assert_eq!(emitted["person_assigned"], truth.person_assigned);
    assert_eq!(emitted["person_reassigned"], truth.person_reassigned);
    assert_eq!(emitted["priority_changed"], truth.priority_changes);
    // Work notes and the duplicate incident_state rows are left out.
    assert_eq!(emitted["field_changed"], truth.other_changes);
    assert!(truth.left_out > truth.state_changes);
    for step in [
        "Open incident",
        "In Progress",
        "Resolved",
        "Closed",
        "Change category",
    ] {
        assert!(names.contains(step), "{step} missing from {names:?}");
    }
    assert!(truth.reopened > 0 && truth.on_hold > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn change_management_yields_every_planted_step() {
    let dir = scratch("change");
    let truth = write_changes(&dir, &Settings::default()).unwrap();
    let (emitted, names) = build(
        &pack("servicenow-change-management"),
        &files(&dir, &CHANGE_FILES),
    );
    assert_eq!(emitted["change_raised"], truth.changes);
    assert_eq!(emitted["change_state_changed"], truth.state_changes);
    assert_eq!(emitted["change_approval"], truth.approval_changes);
    assert_eq!(emitted["change_field_changed"], truth.reschedules);
    assert_eq!(emitted["task_created"], truth.tasks);
    assert_eq!(emitted["task_state_changed"], truth.task_state_changes);
    assert_eq!(
        emitted["change_reassigned"] + emitted["task_reassigned"],
        truth.group_reassigned
    );
    for step in [
        "Raise change",
        "Authorize",
        "Approval Approved",
        "Approval Rejected",
        "Implement",
        "Change task Closed Complete",
        "Change start_date",
    ] {
        assert!(names.contains(step), "{step} missing from {names:?}");
    }
    assert!(truth.standard > 0 && truth.rejected > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn request_fulfillment_yields_every_planted_step() {
    let dir = scratch("request");
    let truth = write_requests(&dir, &Settings::default()).unwrap();
    let (emitted, names) = build(
        &pack("servicenow-request-fulfillment"),
        &files(&dir, &REQUEST_FILES),
    );
    assert_eq!(emitted["item_requested"], truth.items);
    assert_eq!(emitted["item_state_changed"], truth.state_changes);
    assert_eq!(emitted["item_stage_changed"], truth.stage_changes);
    assert_eq!(emitted["item_approval"], truth.approval_changes);
    assert_eq!(emitted["item_field_changed"], 0);
    assert_eq!(emitted["task_created"], truth.tasks);
    assert_eq!(emitted["task_state_changed"], truth.task_state_changes);
    assert_eq!(emitted["task_reassigned"], truth.task_reassignments);
    for step in [
        "Request item",
        "Approval Approved",
        "Stage Fulfillment",
        "Create catalog task",
        "Catalog task Work in Progress",
        "Closed Complete",
    ] {
        assert!(names.contains(step), "{step} missing from {names:?}");
    }
    assert!(truth.rejected > 0 && truth.task_reassignments > 0);
    std::fs::remove_dir_all(dir).unwrap();
}

fn answer(value: &str) -> Answer {
    Answer {
        choice: "value".into(),
        value: Some(value.into()),
        confirmed_by: "Service desk lead".into(),
        at: "2026-10-15T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
}

fn ticket_steps(id: &str, kind: TicketKind, answers: BTreeMap<String, Answer>) {
    let dir = scratch(id);
    let truth = write_tickets(&dir, &Settings::default(), kind).unwrap();
    let (emitted, names) = build_with(
        &pack(id),
        &files(&dir, &[kind.file, kind.task_file, "ServiceNowAudit"]),
        &answers,
    );
    let opened = emitted
        .iter()
        .find(|(rule, _)| rule.ends_with("_opened"))
        .unwrap()
        .1;
    assert_eq!(*opened, truth.tickets);
    assert_eq!(emitted["state_changed"], truth.state_changes);
    assert_eq!(emitted["group_assigned"], truth.group_assigned);
    assert_eq!(emitted["group_reassigned"], truth.group_reassigned);
    assert_eq!(emitted["person_assigned"], truth.person_assigned);
    assert_eq!(emitted["person_reassigned"], truth.person_reassigned);
    assert_eq!(emitted["priority_changed"], truth.priority_changes);
    // Work notes are left out; other kept fields are steps.
    assert_eq!(emitted["field_changed"], truth.other_changes);
    assert_eq!(emitted["task_created"], truth.tasks);
    assert_eq!(emitted["task_state_changed"], truth.task_state_changes);
    assert_eq!(emitted["task_reassigned"], truth.task_reassigned);
    assert!(truth.left_out > 0 && truth.task_reassigned > 0 && truth.person_reassigned > 0);
    for (_, state) in &kind.states[1..] {
        assert!(names.contains(*state), "{state} missing from {names:?}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ticket_handling_works_on_any_task_table_named_in_the_settings() {
    ticket_steps(
        "servicenow-ticket-handling",
        HR_CASES,
        BTreeMap::from([
            ("setting:ticket_table".to_owned(), answer("sn_hr_core_case")),
            ("setting:task_table".to_owned(), answer("sn_hr_core_task")),
        ]),
    );
}

#[test]
fn ticket_handling_without_tasks_still_builds() {
    let dir = scratch("tickets-no-tasks");
    let truth = write_tickets(&dir, &Settings::default(), HR_CASES).unwrap();
    let answers = BTreeMap::from([
        ("setting:ticket_table".to_owned(), answer("sn_hr_core_case")),
        ("setting:task_table".to_owned(), answer("none")),
    ]);
    let (emitted, _) = build_with(
        &pack("servicenow-ticket-handling"),
        &files(&dir, &["ServiceNowTicket", "ServiceNowAudit"]),
        &answers,
    );
    assert_eq!(emitted["ticket_opened"], truth.tickets);
    assert_eq!(emitted["state_changed"], truth.state_changes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn problem_management_yields_every_planted_step() {
    ticket_steps("servicenow-problem-management", PROBLEMS, BTreeMap::new());
}
