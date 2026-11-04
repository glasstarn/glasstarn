//! Two normal orders from one customer are two nine-day transactions,
//! not five years of process delay and rework.
use pm_ingest::{finding::Policy, validate::Mapping};
use pm_query::{Outcome, analyze_filtered};

#[test]
fn repeat_business_is_not_an_object_history_problem_finding() {
    let path = std::env::temp_dir().join(format!("pm-repeat-business-{}.csv", std::process::id()));
    std::fs::write(&path, "customer,order,activity,timestamp\nABC,O1,Receive Order,2020-01-01T00:00:00Z\nABC,O1,Approve Order,2020-01-02T00:00:00Z\nABC,O1,Receive Payment,2020-01-10T00:00:00Z\nABC,O2,Receive Order,2025-01-01T00:00:00Z\nABC,O2,Approve Order,2025-01-02T00:00:00Z\nABC,O2,Receive Payment,2025-01-10T00:00:00Z\n").unwrap();
    let mut history = Mapping::new("customer", "activity", "timestamp");
    history.object_history = true;
    let Outcome::Complete(customer) = analyze_filtered(
        &path,
        Some(history),
        &Policy::new(),
        Default::default(),
        Default::default(),
    )
    .unwrap() else {
        panic!("literal source must analyze")
    };
    assert_eq!(customer.summary.cases, 1);
    assert_eq!(customer.summary.events, 6);
    assert!(customer.summary.median_throughput_seconds > 1000 * 86400);
    assert_eq!(customer.rework.cases_affected, 1);
    assert!(
        customer.key_findings.is_empty(),
        "repeat business must not be ranked as a process problem"
    );
    assert!(
        customer.process.is_none(),
        "customer lifetime is not time to cash"
    );
    assert!(customer.targeting.is_none());
    let Outcome::Complete(orders) = analyze_filtered(
        &path,
        Some(Mapping::new("order", "activity", "timestamp")),
        &Policy::new(),
        Default::default(),
        Default::default(),
    )
    .unwrap() else {
        panic!("literal orders must analyze")
    };
    assert_eq!(orders.summary.cases, 2);
    assert_eq!(orders.summary.median_throughput_seconds, 9 * 86400);
    assert_eq!(orders.rework.cases_affected, 0);
    std::fs::remove_file(path).unwrap();
}
