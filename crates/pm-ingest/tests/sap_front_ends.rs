//! SAP Sales Cloud and Salesforce in front of SAP order to cash, on-premise
//! and Public Cloud, on made-up data whose contents are known by construction.
mod common;
use pm_ingest::pack::{Answer, BUILT_IN, Pack, build_recipe, check};
use pm_testgen::sap_o2c::{
    CLOUD_TABLES, Front, SALES_CLOUD_FILES, SALESFORCE_FILES, Settings, TABLES, write_front,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn answer(value: &str) -> Answer {
    Answer {
        choice: "value".into(),
        value: Some(value.into()),
        confirmed_by: "Sales operations analyst".into(),
        at: "2026-10-12T00:00:00Z".into(),
        question: String::new(),
        carries: Vec::new(),
    }
}

/// Generates, checks with the answered settings, builds and returns counts.
fn run(
    id: &str,
    front: Front,
    cloud: bool,
) -> (BTreeMap<String, u64>, pm_testgen::sap_o2c::FrontTruth) {
    let pack = Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap();
    let dir = std::env::temp_dir().join(format!("pm-front-{id}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let (_, truth) = write_front(&dir, &Settings::default(), front, cloud).unwrap();
    let base: &[&str] = if cloud { &CLOUD_TABLES } else { &TABLES };
    let extra: &[&str] = match front {
        Front::SalesCloud => &SALES_CLOUD_FILES,
        Front::Salesforce => &SALESFORCE_FILES,
    };
    let files = base
        .iter()
        .chain(extra)
        .map(|t| ((*t).to_owned(), dir.join(format!("{t}.csv"))))
        .collect::<BTreeMap<_, PathBuf>>();
    let answers = BTreeMap::from([
        ("setting:offset".to_owned(), answer("+01:00")),
        ("setting:sales_cloud_offset".to_owned(), answer("+01:00")),
        (
            "setting:salesforce_sap_order_field".to_owned(),
            answer("SAP_Order_Number__c"),
        ),
    ]);
    let pack = pack.with_settings(&answers).unwrap();
    let report = check(&pack, &files).unwrap();
    assert_eq!(report.status, "ready", "{:?}", report.gaps);
    // Some SAP orders were entered directly; the analyst is asked about them.
    assert!(truth.orders_without_front > 0);
    let question = report
        .questions
        .iter()
        .find(|q| q.id == "link:sap-orders-came-from-front")
        .unwrap();
    assert!(
        question
            .text
            .starts_with(&format!("{} of ", truth.orders_without_front))
    );
    let hashes = files.keys().map(|t| (t.clone(), "0".repeat(64))).collect();
    let built = build_recipe(&pack, &hashes, &answers).unwrap();
    let outcome = common::prepare_both(&built.recipe, &files);
    assert_eq!(outcome.report.issue_count, 0, "{:?}", outcome.report.issues);
    std::fs::remove_dir_all(dir).unwrap();
    let counts = outcome
        .report
        .rules
        .iter()
        .map(|(rule, counts)| (rule.clone(), counts.emitted))
        .collect();
    (counts, truth)
}

#[test]
fn sap_sales_cloud_opportunities_and_quotes_reach_the_sap_order_lines() {
    for (id, cloud) in [
        ("sap-sales-cloud-order-to-cash", false),
        ("sap-sales-cloud-s4-cloud-order-to-cash", true),
    ] {
        let (emitted, truth) = run(id, Front::SalesCloud, cloud);
        assert_eq!(emitted["opportunity_created"], truth.opportunities);
        assert_eq!(emitted["opportunity_won"], truth.won);
        assert_eq!(emitted["opportunity_lost"], truth.lost);
        assert_eq!(emitted["quote_created"], truth.quotes);
        assert_eq!(emitted["quote_submitted"], truth.quotes_approved);
        assert_eq!(emitted["quote_approved"], truth.quotes_approved);
        assert_eq!(emitted["quote_sent"], truth.quotes_sent);
        assert_eq!(emitted["quote_accepted"], truth.quotes_accepted);
        // Accepted quotes reach their SAP order and its lines; won
        // opportunities reach them through their quote.
        assert_eq!(emitted["quote_accepted_order"], truth.quotes_accepted);
        assert_eq!(emitted["opportunity_won_order"], truth.won);
        assert!(emitted["opportunity_won_lines"] >= truth.won);
    }
}

#[test]
fn salesforce_opportunities_stages_and_quotes_reach_the_sap_order_lines() {
    for (id, cloud) in [
        ("salesforce-sap-order-to-cash", false),
        ("salesforce-s4-cloud-order-to-cash", true),
    ] {
        let (emitted, truth) = run(id, Front::Salesforce, cloud);
        assert_eq!(emitted["sf_opportunity_created"], truth.opportunities);
        assert_eq!(
            emitted["sf_stage_reached"]
                + emitted["sf_opportunity_won"]
                + emitted["sf_opportunity_lost"],
            truth.stage_changes
        );
        assert_eq!(emitted["sf_opportunity_won"], truth.won);
        assert_eq!(emitted["sf_opportunity_lost"], truth.lost);
        assert_eq!(emitted["sf_quote_created"], truth.quotes);
        assert_eq!(emitted["sf_opportunity_won_order"], truth.won);
        assert!(emitted["sf_quote_created_lines"] >= truth.won);
    }
}
