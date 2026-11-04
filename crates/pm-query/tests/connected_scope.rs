//! Literal cohorts ensure shared customers/products cannot pull other orders in.
use pm_core::{OcelBuilder, OcelCohort, OcelFilter, OcelView};
use std::collections::BTreeSet;

fn cohort(kind: &str, roots: &[&str], paths: &[&[&str]]) -> OcelFilter {
    OcelFilter {
        cohort: Some(OcelCohort {
            root_type: kind.into(),
            roots: roots.iter().map(|s| s.to_string()).collect(),
            paths: paths
                .iter()
                .map(|path| path.iter().map(|s| s.to_string()).collect())
                .collect(),
            ..Default::default()
        }),
        ..Default::default()
    }
}
#[test]
fn typed_paths_keep_transaction_boundaries_and_parent_scope() {
    let mut b = OcelBuilder::new();
    for kind in ["Customer", "Order", "Product", "Invoice"] {
        b.declare_object_type(kind, vec![]).unwrap();
    }
    for kind in ["Receive", "Invoice"] {
        b.declare_event_type(kind, vec![]).unwrap();
    }
    for (id, kind) in [
        ("C1", "Customer"),
        ("C2", "Customer"),
        ("P", "Product"),
        ("O1", "Order"),
        ("O2", "Order"),
        ("O3", "Order"),
        ("I1", "Invoice"),
        ("I2", "Invoice"),
        ("I3", "Invoice"),
    ] {
        b.add_object(id, kind).unwrap();
    }
    for (index, (order, customer, invoice)) in
        [("O1", "C1", "I1"), ("O2", "C1", "I2"), ("O3", "C2", "I3")]
            .iter()
            .enumerate()
    {
        let receive = format!("r{index}");
        let issue = format!("i{index}");
        b.add_event(&receive, "Receive", index as i64 * 100)
            .unwrap();
        for id in [*order, *customer, "P"] {
            b.relate_event_object(&receive, id, "").unwrap();
        }
        b.add_event(&issue, "Invoice", index as i64 * 100 + 10)
            .unwrap();
        for id in [*order, *invoice] {
            b.relate_event_object(&issue, id, "").unwrap();
        }
    }
    let log = b.build();
    let order = cohort("Order", &["O1"], &[&["Order", "Invoice"]]);
    let view = OcelView::apply(&log, &order).unwrap();
    let labels = view
        .objects
        .iter()
        .map(|id| log.object_label(*id).unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(labels, BTreeSet::from(["O1", "I1"]));
    assert_eq!(view.events.len(), 2);
    let customers = cohort("Customer", &["C1"], &[&["Customer", "Order", "Invoice"]]);
    let view = OcelView::apply(&log, &customers).unwrap();
    let labels = view
        .objects
        .iter()
        .map(|id| log.object_label(*id).unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(labels, BTreeSet::from(["C1", "O1", "O2", "I1", "I2"]));
    assert_eq!(view.events.len(), 4);
    let mut child = cohort("Order", &["O2"], &[&["Order", "Invoice"]]);
    child.cohort.as_mut().unwrap().parent = order.cohort.map(Box::new);
    assert!(OcelView::apply(&log, &child).unwrap().objects.is_empty());
    assert!(
        OcelView::apply(&log, &cohort("Order", &[], &[&["Order", "Invoice"]]))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(OcelView::apply(&log, &cohort("Order", &["C1"], &[])).is_err());
    assert!(OcelView::apply(&log, &cohort("Order", &["O1"], &[&["Invoice", "Order"]])).is_err());
}
#[test]
fn disk_and_loaded_cohorts_agree_including_empty_parent_intersection() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../testdata/ocel/order-management.sqlite");
    let log = pm_ingest::read_ocel(&path).unwrap();
    let disk = pm_ingest::DiskLog::open(&path, &std::env::temp_dir()).unwrap();
    let one = cohort("item", &["item-1"], &[&["item", "package"]]);
    let mut child = cohort("item", &["item-2"], &[&["item", "package"]]);
    child.cohort.as_mut().unwrap().parent = one.cohort.clone().map(Box::new);
    for filter in [one, child, cohort("item", &[], &[])] {
        let loaded = OcelView::apply(&log, &filter).unwrap();
        let view = disk.view(&filter).unwrap();
        assert_eq!(
            disk.view_counts(&view).unwrap(),
            [
                loaded.events.len(),
                loaded.objects.len(),
                loaded.event_object_relations.len(),
                loaded.object_object_relations.len()
            ]
        );
        let expected =
            pm_query::analyze_ocel_log(&path.display().to_string(), &log, &filter).unwrap();
        let actual = pm_query::analyze_ocel_disk(&path, &filter).unwrap();
        assert_eq!(actual.scope, expected.scope);
        assert_eq!(actual.graphs, expected.graphs);
        assert_eq!(actual.variants, expected.variants);
    }
}

#[test]
fn filtering_to_only_unpaid_orders_keeps_the_zero_reached_payment_stage() {
    use serde_json::json;
    let path =
        std::env::temp_dir().join(format!("pm-fixed-journey-{}.jsonocel", std::process::id()));
    let facts = [
        ("Create", vec!["O1"]),
        ("Issue", vec!["O1", "I1"]),
        ("Pay", vec!["I1", "P1"]),
        ("Create", vec!["O2"]),
        ("Issue", vec!["O2", "I2"]),
    ];
    let events=facts.iter().enumerate().map(|(index,(activity,objects))|json!({"id":format!("E{index}"),"type":activity,"time":format!("2026-01-0{}T00:00:00Z",index+1),"attributes":[],"relationships":objects.iter().map(|id|json!({"objectId":id,"qualifier":""})).collect::<Vec<_>>()})).collect::<Vec<_>>();
    let objects = [
        ("O1", "Order"),
        ("O2", "Order"),
        ("I1", "Invoice"),
        ("I2", "Invoice"),
        ("P1", "Payment"),
    ]
    .iter()
    .map(|(id, kind)| json!({"id":id,"type":kind,"attributes":[],"relationships":[]}))
    .collect::<Vec<_>>();
    std::fs::write(&path,json!({"eventTypes":[{"name":"Create","attributes":[]},{"name":"Issue","attributes":[]},{"name":"Pay","attributes":[]}],"objectTypes":[{"name":"Order","attributes":[]},{"name":"Invoice","attributes":[]},{"name":"Payment","attributes":[]}],"events":events,"objects":objects}).to_string()).unwrap();
    let plan = pm_query::ocel_journey(&path, &OcelFilter::default(), "Order").unwrap();
    assert_eq!(plan.stages.len(), 3, "{plan:?}");
    let filter = cohort("Order", &["O2"], &[&["Order", "Invoice", "Payment"]]);
    let actual = pm_query::measure_journey_plan(&path, &filter, &plan).unwrap();
    assert_eq!(actual.stages[0].items, 1);
    assert_eq!(actual.stages[2].object_type, "Payment");
    assert_eq!(actual.stages[2].items, 0);
    let payment = actual
        .handoffs
        .iter()
        .find(|step| step.to_activity == "Pay")
        .unwrap();
    assert_eq!(
        (payment.started, payment.reached, payment.observations),
        (1, 0, 0)
    );
    assert!(
        actual
            .findings
            .iter()
            .any(|finding| finding.title.contains("have no “Pay” yet"))
    );
    std::fs::remove_file(path).unwrap();
}
