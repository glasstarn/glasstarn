//! A column that changes between an item's steps (status, owner) is kept on
//! each step; one that never changes (vendor) stays once per item. Filters,
//! Breakdown, value pickers and the item timeline all read both.
use pm_ingest::{finding::Policy, validate::Mapping};

fn load() -> pm_core::EventLog {
    // Tests run in parallel: each load gets its own file.
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!("pm-step-level-{}-{n}.csv", std::process::id()));
    std::fs::write(
        &path,
        "case,activity,timestamp,vendor,status\n\
         1,Create,2020-01-01T00:00:00Z,Acme,Open\n\
         1,Block,2020-01-02T00:00:00Z,Acme,Blocked\n\
         1,Release,2020-01-03T00:00:00Z,Acme,Open\n\
         1,Pay,2020-01-04T00:00:00Z,Acme,Paid\n\
         2,Create,2020-01-01T00:00:00Z,Bolt,Open\n\
         2,Pay,2020-01-02T00:00:00Z,Bolt,Paid\n\
         3,Create,2020-01-01T00:00:00Z,Bolt,Open\n\
         3,Pay,2020-01-05T00:00:00Z,,\n",
    )
    .unwrap();
    let mapping = Mapping::new("case", "activity", "timestamp")
        .with_case_attributes(vec!["vendor".into(), "status".into()]);
    let loaded = pm_query::load_log(&path, Some(mapping), &Policy::new(), Default::default())
        .unwrap()
        .unwrap();
    std::fs::remove_file(&path).ok();
    loaded.log
}

#[test]
fn only_the_changing_column_is_kept_on_steps() {
    let log = load();
    let vendor = log.attribute_index("vendor").unwrap();
    let status = log.attribute_index("status").unwrap();
    assert!(!log.is_step_level(vendor));
    assert!(log.is_step_level(status));
    // Item 1 kept its later statuses; items 2 and 3 only differ where they changed.
    let one = &log.traces[0];
    let labels = |values: Vec<u32>| {
        values
            .into_iter()
            .map(|v| log.attribute_values.label(v).unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        labels(log.case_values(one, status)),
        ["Open", "Blocked", "Paid"]
    );
    assert_eq!(labels(log.case_values(one, vendor)), ["Acme"]);
    // A blank cell on a step is not a change.
    assert_eq!(labels(log.case_values(&log.traces[2], status)), ["Open"]);
    assert_eq!(log.attribute_options("status"), ["Blocked", "Open", "Paid"]);
}

#[test]
fn a_filter_keeps_items_that_ever_had_the_value() {
    let log = load();
    let paid = pm_core::Filter::new()
        .attribute("status", "Paid")
        .apply(&log);
    assert_eq!(paid.log.case_count(), 2);
    let blocked = pm_core::Filter {
        attribute_in: [("status".to_owned(), ["Blocked".to_owned()].into())].into(),
        ..Default::default()
    }
    .apply(&log);
    assert_eq!(blocked.log.case_count(), 1);
    // The kept items keep their step values.
    let kept = &blocked.log.traces[0];
    let status = blocked.log.attribute_index("status").unwrap();
    assert_eq!(blocked.log.case_values(kept, status).len(), 3);
    // An item-level column is unchanged.
    let acme = pm_core::Filter::new()
        .attribute("vendor", "Acme")
        .apply(&log);
    assert_eq!(acme.log.case_count(), 1);
}

#[test]
fn breakdown_and_value_pickers_count_an_item_under_each_value() {
    let log = load();
    let request = pm_query::exploration::ExplorationRequest {
        segment: None,
        group_by: Some("status".into()),
        min_micros: None,
        max_micros: None,
    };
    let result = pm_query::exploration::explore(&log, &request).unwrap();
    assert!(result.groups_overlap);
    let count = |value: &str| {
        result
            .groups
            .iter()
            .find(|g| g.value.as_deref() == Some(value))
            .map(|g| g.population.items)
    };
    assert_eq!(count("Open"), Some(3));
    assert_eq!(count("Paid"), Some(2));
    assert_eq!(count("Blocked"), Some(1));
    let by_vendor = pm_query::exploration::explore(
        &log,
        &pm_query::exploration::ExplorationRequest {
            group_by: Some("vendor".into()),
            ..request
        },
    )
    .unwrap();
    assert!(!by_vendor.groups_overlap);
    let choices = pm_query::attribute_choices(&log, "status", "", 10).unwrap();
    let paid = choices.values.iter().find(|c| c.value == "Paid").unwrap();
    assert_eq!(paid.cases, 2);
}

#[test]
fn the_item_timeline_shows_where_a_value_changed() {
    let log = load();
    let detail = pm_query::drilldown::inspect_case(&log, "1").unwrap();
    // Vendor never changes: it stays with the item.
    assert_eq!(
        detail.attributes,
        vec![("vendor".to_owned(), "Acme".to_owned())]
    );
    let changes = detail
        .events
        .iter()
        .map(|e| e.changes.clone())
        .collect::<Vec<_>>();
    let status = |v: &str| vec![("status".to_owned(), v.to_owned())];
    assert_eq!(
        changes,
        vec![
            status("Open"),
            status("Blocked"),
            status("Open"),
            status("Paid")
        ]
    );
}

#[test]
fn drivers_find_a_status_some_items_passed_through() {
    let mut csv = String::from("case,activity,timestamp,status\n");
    for item in 0..120 {
        let blocked = item % 6 == 0;
        csv.push_str(&format!("{item},Create,2020-01-01T00:00:00Z,Open\n"));
        if blocked {
            csv.push_str(&format!("{item},Block,2020-01-02T00:00:00Z,Blocked\n"));
        }
        let day = if blocked { 20 } else { 3 + item % 5 };
        csv.push_str(&format!("{item},Pay,2020-01-{day:02}T00:00:00Z,Paid\n"));
    }
    let path = std::env::temp_dir().join(format!("pm-step-drivers-{}.csv", std::process::id()));
    std::fs::write(&path, csv).unwrap();
    let mapping =
        Mapping::new("case", "activity", "timestamp").with_case_attributes(vec!["status".into()]);
    let log = pm_query::load_log(&path, Some(mapping), &Policy::new(), Default::default())
        .unwrap()
        .unwrap()
        .log;
    std::fs::remove_file(&path).ok();
    let found = pm_query::drivers::item_drivers(&log, 3);
    let top = &found.drivers[0];
    assert_eq!(
        (top.field.as_str(), top.value.as_str()),
        ("status", "Blocked")
    );
    // Every item was Open and Paid at some point: those say nothing.
    assert!(
        found
            .drivers
            .iter()
            .all(|d| d.value != "Open" && d.value != "Paid")
    );
}

#[test]
fn a_person_is_kept_over_a_status_that_picks_the_same_items() {
    let mut csv = String::from("case,activity,timestamp,resource,status\n");
    for item in 0..120 {
        let held = item % 6 == 0;
        csv.push_str(&format!("{item},Create,2020-01-01T00:00:00Z,Ann,Open\n"));
        if held {
            csv.push_str(&format!(
                "{item},Approve,2020-01-02T00:00:00Z,Dana,Awaiting approval\n"
            ));
        }
        let day = if held { 20 } else { 3 + item % 5 };
        csv.push_str(&format!("{item},Pay,2020-01-{day:02}T00:00:00Z,Bob,Paid\n"));
    }
    let path = std::env::temp_dir().join(format!("pm-step-person-{}.csv", std::process::id()));
    std::fs::write(&path, csv).unwrap();
    let mapping = Mapping::new("case", "activity", "timestamp")
        .with_resource("resource")
        .with_case_attributes(vec!["status".into()]);
    let log = pm_query::load_log(&path, Some(mapping), &Policy::new(), Default::default())
        .unwrap()
        .unwrap()
        .log;
    std::fs::remove_file(&path).ok();
    let found = pm_query::drivers::item_drivers(&log, 3);
    assert_eq!(found.drivers[0].value, "Dana");
    assert!(found.drivers.iter().all(|d| d.field != "status"));
}

#[test]
fn an_item_whose_value_never_changed_keeps_it_in_its_header() {
    let log = load();
    let detail = pm_query::drilldown::inspect_case(&log, "3").unwrap();
    assert!(
        detail
            .attributes
            .contains(&("status".to_owned(), "Open".to_owned()))
    );
    assert!(detail.events.iter().all(|e| e.changes.is_empty()));
}

#[test]
fn a_filter_can_ask_for_the_latest_value() {
    let log = load();
    let latest = |value: &str| {
        pm_core::Filter {
            attribute_latest: [("status".to_owned(), [value.to_owned()].into())].into(),
            ..Default::default()
        }
        .apply(&log)
        .log
        .case_count()
    };
    // Item 1 was Blocked but ended Paid; item 3's last step is blank, so it
    // reads as its own value, Open.
    assert_eq!(latest("Blocked"), 0);
    assert_eq!(latest("Paid"), 2);
    assert_eq!(latest("Open"), 1);
}

#[test]
fn compare_counts_items_in_both_groups() {
    let log = load();
    let compared = pm_query::compare_cohorts(&log, "status", "Open", "Paid").unwrap();
    assert_eq!(compared.shared_items, 2);
    let vendors = pm_query::compare_cohorts(&log, "vendor", "Acme", "Bolt").unwrap();
    assert_eq!(vendors.shared_items, 0);
}

#[test]
fn a_revised_amount_counts_at_its_latest_value() {
    let path = std::env::temp_dir().join(format!("pm-step-amount-{}.csv", std::process::id()));
    std::fs::write(
        &path,
        "case,activity,timestamp,amount\n\
         1,Order,2020-01-01T00:00:00Z,100\n\
         1,Change,2020-01-02T00:00:00Z,150\n\
         1,Pay,2020-01-03T00:00:00Z,150\n\
         2,Order,2020-01-01T00:00:00Z,40\n",
    )
    .unwrap();
    let mapping =
        Mapping::new("case", "activity", "timestamp").with_case_attributes(vec!["amount".into()]);
    let log = pm_query::load_log(&path, Some(mapping), &Policy::new(), Default::default())
        .unwrap()
        .unwrap()
        .log;
    std::fs::remove_file(&path).ok();
    let index = log.attribute_index("amount").unwrap();
    assert_eq!(
        pm_core::case_number(&log, &log.traces[0], index),
        Some(150.0)
    );
    assert_eq!(
        pm_core::case_number(&log, &log.traces[1], index),
        Some(40.0)
    );
}

#[test]
fn spelling_differences_and_free_text_are_not_kept_per_step() {
    let mut csv = String::from("case,activity,timestamp,vendor,comment\n");
    for item in 0..1_200 {
        let vendor = if item % 2 == 0 { "Acme" } else { "Bolt" };
        csv.push_str(&format!(
            "{item},Order,2020-01-01T00:00:00Z,{vendor},note {item}a\n"
        ));
        csv.push_str(&format!(
            "{item},Pay,2020-01-02T00:00:00Z,{},note {item}b\n",
            vendor.to_uppercase()
        ));
    }
    let path = std::env::temp_dir().join(format!("pm-step-text-{}.csv", std::process::id()));
    std::fs::write(&path, csv).unwrap();
    let mapping = Mapping::new("case", "activity", "timestamp")
        .with_case_attributes(vec!["vendor".into(), "comment".into()]);
    let loaded = pm_query::load_log(
        &path,
        Some(mapping.clone()),
        &Policy::new(),
        Default::default(),
    )
    .unwrap()
    .unwrap();
    let log = &loaded.log;
    // "ACME" on a later step is Acme spelled differently, not a change.
    assert!(!log.is_step_level(log.attribute_index("vendor").unwrap()));
    // 1,200 different later comments: free text, kept once per item.
    assert!(!log.is_step_level(log.attribute_index("comment").unwrap()));
    assert!(log.step_values.is_empty());
    let table = pm_ingest::read::read_csv(&path).unwrap();
    std::fs::remove_file(&path).ok();
    let report = pm_ingest::validate::validate(&table.rows, &mapping);
    let messages: Vec<String> = report.findings.iter().map(|f| f.detail.clone()).collect();
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("vendor have values spelled differently")),
        "{messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("comment take more than 1000")),
        "{messages:?}"
    );
}
