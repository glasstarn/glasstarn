use pm_ingest::{finding::Policy, validate::Mapping};
#[test]
fn actual_values_strict_bounds_and_unreached_share_the_filtered_log() {
    let path = std::env::temp_dir().join(format!("pm-choice-fixture-{}.csv", std::process::id()));
    std::fs::write(&path,"case,activity,timestamp,customer,amount\n1,Receive,2020-01-01T00:00:00Z,ABC,10\n1,Pay,2020-01-02T00:00:00Z,ABC,10\n2,Receive,2020-01-01T00:00:00Z,ABC,20\n3,Pay,2020-01-01T00:00:00Z,XYZ,30\n3,Receive,2020-01-02T00:00:00Z,XYZ,30\n4,Receive,2020-01-01T00:00:00Z,,\n4,Pay,2020-01-01T00:00:00Z,,\n").unwrap();
    let mapping = Mapping::new("case", "activity", "timestamp")
        .with_case_attributes(vec!["customer".into(), "amount".into()]);
    let loaded = pm_query::load_log(&path, Some(mapping), &Policy::new(), Default::default())
        .unwrap()
        .unwrap();
    let choices = pm_query::attribute_choices(&loaded.log, "customer", "", 1).unwrap();
    assert_eq!(choices.values[0].value, "ABC");
    assert_eq!(choices.values[0].cases, 2);
    assert!(choices.truncated);
    assert_eq!(choices.missing_cases, 1);
    assert_eq!(
        pm_query::attribute_choices(&loaded.log, "customer", "xy", 10)
            .unwrap()
            .values[0]
            .value,
        "XYZ"
    );
    let filter = pm_core::Filter {
        attribute_between: vec![pm_core::NumberRange {
            attribute: "amount".into(),
            greater_than: Some(10.0),
            less_than: Some(30.0),
            ..Default::default()
        }],
        ..Default::default()
    };
    let filtered = filter.apply(&loaded.log);
    assert_eq!(filtered.log.traces.len(), 1);
    assert_eq!(filtered.log.case_label(filtered.log.traces[0].case), "2");
    assert_eq!(
        pm_query::attribute_choices(&filtered.log, "customer", "", 100)
            .unwrap()
            .values[0]
            .cases,
        1
    );
    let filter = pm_core::Filter {
        not_reached: std::collections::BTreeSet::from([("Receive".into(), "Pay".into())]),
        ..Default::default()
    };
    let filtered = filter.apply(&loaded.log);
    assert_eq!(
        filtered
            .log
            .traces
            .iter()
            .map(|trace| filtered.log.case_label(trace.case))
            .collect::<Vec<_>>(),
        ["2", "3", "4"]
    );
    std::fs::remove_file(path).unwrap();
}
