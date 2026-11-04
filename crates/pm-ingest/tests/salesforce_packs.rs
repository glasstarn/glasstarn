//! The Salesforce packs (sales, service, quotes, orders, field service,
//! contracts, CPQ and nCino) on made-up extracts whose contents are known by
//! construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::salesforce::{
    CASE_FILES, CONTRACT_FILES, CPQ_FILES, FIELD_SERVICE_FILES, LEAD_FILES, LOAN_FILES,
    ORDER_FILES, QUOTE_FILES, Settings, write_cases, write_contracts, write_cpq,
    write_field_service, write_leads, write_ncino, write_orders, write_quotes,
};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn pack(id: &str) -> Pack {
    Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
}
fn answer(choice: &str, value: &str) -> Answer {
    Answer {
        choice: choice.into(),
        value: Some(value.into()),
        confirmed_by: "Sales operations analyst".into(),
        at: "2026-10-12T00:00:00Z".into(),
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
fn run(
    pack: &Pack,
    files: &BTreeMap<String, PathBuf>,
    answers: BTreeMap<String, Answer>,
) -> (pm_ingest::pack::CheckReport, BTreeMap<String, u64>) {
    let report = check(pack, files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(pack, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    let counts = outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect();
    (report, counts)
}

#[test]
fn lead_to_opportunity_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-leads-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_leads(&dir, &Settings::default()).unwrap();
    let answers = BTreeMap::from([(
        "custom:Lead.Region__c".to_owned(),
        answer("detail", "region"),
    )]);
    let (report, emitted) = run(
        &pack("salesforce-lead-to-opportunity"),
        &files(&dir, &LEAD_FILES),
        answers,
    );
    // The company's own field on leads is found and asked about.
    let region = report
        .questions
        .iter()
        .find(|q| q.id == "custom:Lead.Region__c")
        .unwrap();
    assert_eq!(region.recommended.as_deref(), Some("detail"));
    assert_eq!(emitted["sf_lead_created"], truth.leads);
    assert_eq!(emitted["sf_lead_status_changed"], truth.status_changes);
    // Owner changes come as an ID row and a name row; each counts once.
    assert_eq!(emitted["sf_lead_reassigned"], truth.reassignments);
    assert_eq!(emitted["sf_lead_converted"], truth.converted);
    assert_eq!(emitted["sf_opportunity_created"], truth.opportunities);
    assert_eq!(
        emitted["sf_stage_reached"]
            + emitted["sf_opportunity_won"]
            + emitted["sf_opportunity_lost"],
        truth.stage_changes
    );
    assert_eq!(emitted["sf_opportunity_won"], truth.won);
    assert_eq!(emitted["sf_opportunity_lost"], truth.lost);
    assert_eq!(emitted["sf_amount_changed"], truth.amount_changes);
    assert_eq!(emitted["sf_close_date_moved"], truth.close_date_moves);
    // Opportunity steps reach the lead they came from.
    assert_eq!(emitted["sf_opportunity_created_lead"], truth.converted);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn case_handling_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-cases-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_cases(&dir, &Settings::default()).unwrap();
    let (_, emitted) = run(
        &pack("salesforce-case-handling"),
        &files(&dir, &CASE_FILES),
        BTreeMap::new(),
    );
    assert_eq!(emitted["sf_case_created"], truth.cases);
    assert_eq!(emitted["sf_case_status_changed"], truth.status_changes);
    assert_eq!(emitted["sf_case_reassigned"], truth.reassignments);
    assert_eq!(emitted["sf_case_priority_changed"], truth.priority_changes);
    assert_eq!(emitted["sf_case_escalated"], truth.escalations);
    assert_eq!(emitted["sf_email_received"], truth.emails_in);
    assert_eq!(emitted["sf_email_sent"], truth.emails_out);
    // Without the optional email export the case steps still build.
    let mut no_email = files(&dir, &CASE_FILES);
    no_email.remove("EmailMessage");
    let (_, emitted) = run(
        &pack("salesforce-case-handling"),
        &no_email,
        BTreeMap::new(),
    );
    assert!(!emitted.contains_key("sf_email_received"));
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn quote_approval_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-quotes-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_quotes(&dir, &Settings::default()).unwrap();
    assert!(truth.rejected > 0 && truth.recalled > 0 && truth.orders > 0);
    let pack = pack("salesforce-quote-approval");
    let (_, emitted) = run(&pack, &files(&dir, &QUOTE_FILES), BTreeMap::new());
    assert_eq!(
        emitted["sf_opportunity_created"],
        truth.opportunities.opportunities
    );
    assert_eq!(emitted["sf_quote_created"], truth.quotes);
    assert_eq!(emitted["sf_quote_submitted"], truth.submitted);
    assert_eq!(emitted["sf_quote_approved"], truth.approved);
    assert_eq!(emitted["sf_quote_rejected"], truth.rejected);
    assert_eq!(emitted["sf_quote_recalled"], truth.recalled);
    assert_eq!(emitted["sf_order_from_quote"], truth.orders);
    assert_eq!(emitted["sf_quote_for_opportunity"], truth.quotes);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn order_management_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-orders-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_orders(&dir, &Settings::default()).unwrap();
    assert!(truth.reduction_orders > 0);
    let pack = pack("salesforce-order-management");
    let (_, emitted) = run(&pack, &files(&dir, &ORDER_FILES), BTreeMap::new());
    assert_eq!(emitted["sf_order_created"], truth.orders);
    assert_eq!(
        emitted["sf_reduction_order_created"],
        truth.reduction_orders
    );
    // Activation comes from the activation date, not the history row too.
    assert_eq!(emitted["sf_order_status_changed"], truth.status_changes);
    assert_eq!(emitted["sf_order_activated"], truth.activated);
    assert_eq!(emitted["sf_order_product_added"], truth.order_products);
    assert_eq!(emitted["sf_order_item_on_order"], truth.order_products);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn field_service_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-fsl-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_field_service(&dir, &Settings::default()).unwrap();
    let pack = pack("salesforce-field-service");
    let (_, emitted) = run(&pack, &files(&dir, &FIELD_SERVICE_FILES), BTreeMap::new());
    assert_eq!(emitted["sf_work_order_created"], truth.work_orders);
    assert_eq!(
        emitted["sf_work_order_status_changed"],
        truth.work_order_status_changes
    );
    assert_eq!(emitted["sf_appointment_created"], truth.appointments);
    assert_eq!(
        emitted["sf_appointment_status_changed"],
        truth.appointment_status_changes
    );
    assert_eq!(emitted["sf_technician_assigned"], truth.assignments);
    assert_eq!(emitted["sf_work_started"], truth.started);
    assert_eq!(emitted["sf_work_finished"], truth.finished);
    assert_eq!(emitted["sf_appointment_for_work_order"], truth.appointments);
    let mut core = files(&dir, &FIELD_SERVICE_FILES);
    core.remove("ServiceAppointmentHistory");
    core.remove("AssignedResource");
    let (_, emitted) = run(&pack, &core, BTreeMap::new());
    assert_eq!(emitted["sf_work_finished"], truth.finished);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn contract_renewal_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-contracts-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_contracts(&dir, &Settings::default()).unwrap();
    assert!(truth.renewals > 0 && truth.renewals < truth.opportunities);
    let pack = pack("salesforce-contract-renewal");
    let (_, emitted) = run(&pack, &files(&dir, &CONTRACT_FILES), BTreeMap::new());
    assert_eq!(emitted["sf_contract_created"], truth.contracts);
    assert_eq!(emitted["sf_contract_status_changed"], truth.status_changes);
    assert_eq!(emitted["sf_contract_activated"], truth.activated);
    assert_eq!(emitted["sf_opportunity_created"], truth.opportunities);
    // Renewals reach their contract; other deals do not.
    assert_eq!(emitted["sf_renewal_of_contract"], truth.renewals);
    assert_eq!(emitted["sf_opportunity_created_contract"], truth.renewals);
    assert_eq!(
        emitted["sf_stage_reached_contract"],
        truth.renewal_stage_changes
    );
    assert_eq!(emitted["sf_opportunity_won_contract"], truth.renewals_won);
    assert_eq!(emitted["sf_opportunity_lost_contract"], truth.renewals_lost);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn cpq_quote_to_renewal_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-cpq-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_cpq(&dir, &Settings::default()).unwrap();
    assert!(truth.renewals > 0);
    let pack = pack("salesforce-cpq-quote-to-renewal");
    let (_, emitted) = run(&pack, &files(&dir, &CPQ_FILES), BTreeMap::new());
    assert_eq!(emitted["sf_opportunity_created"], truth.opportunities);
    assert_eq!(emitted["sf_cpq_quote_created"], truth.quotes);
    assert_eq!(emitted["sf_cpq_quote_status_changed"], truth.status_changes);
    assert_eq!(emitted["sf_cpq_order_created"], truth.orders);
    assert_eq!(emitted["sf_cpq_order_activated"], truth.orders_activated);
    assert_eq!(emitted["sf_cpq_contract_created"], truth.contracts);
    assert_eq!(
        emitted["sf_cpq_contract_activated"],
        truth.contracts_activated
    );
    assert_eq!(emitted["sf_cpq_renewal_of_contract"], truth.renewals);
    // Renewal steps reach the contract and, through it, the original quote.
    assert_eq!(
        emitted["sf_opportunity_created_contract"] + emitted["sf_stage_reached_contract"],
        truth.renewal_steps
    );
    assert_eq!(
        emitted["sf_opportunity_created_quote"] + emitted["sf_stage_reached_quote"],
        truth.renewal_steps
    );
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn ncino_loan_origination_yields_every_planted_step() {
    let dir = std::env::temp_dir().join(format!("pm-sf-ncino-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let truth = write_ncino(&dir, &Settings::default()).unwrap();
    assert!(truth.declined > 0 && truth.booked > 0);
    let pack = pack("salesforce-ncino-loan-origination");
    let (_, emitted) = run(&pack, &files(&dir, &LOAN_FILES), BTreeMap::new());
    assert_eq!(emitted["ncino_loan_created"], truth.loans);
    assert_eq!(emitted["ncino_stage_reached"], truth.stage_changes);
    assert_eq!(emitted["ncino_status_changed"], truth.status_changes);
    assert_eq!(emitted["ncino_amount_changed"], truth.amount_changes);
    assert_eq!(emitted["ncino_submitted"], truth.submitted);
    assert_eq!(emitted["ncino_approved"], truth.approved);
    assert_eq!(emitted["ncino_rejected"], truth.declined);
    assert_eq!(emitted["ncino_recalled"], truth.recalled);
    let mut core = files(&dir, &LOAN_FILES);
    core.remove("ProcessInstance");
    core.remove("ProcessInstanceStep");
    let (_, emitted) = run(&pack, &core, BTreeMap::new());
    assert_eq!(emitted["ncino_stage_reached"], truth.stage_changes);
    std::fs::remove_dir_all(dir).unwrap();
}
