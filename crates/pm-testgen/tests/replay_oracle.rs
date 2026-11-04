use chrono::NaiveDate;
use pm_core::{
    ActivityId, CaseId, CaseTrace, Dictionary, Event, EventLog, ProcessTree, replay_tokens,
};
use pm_testgen::{LogBuilder, Model, Wait};

fn generated_log(model: Model, cases: usize) -> EventLog {
    let start = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let generated = LogBuilder::new(model, start)
        .seed(77)
        .case_interval(Wait::fixed(60))
        .default_wait(Wait::fixed(1))
        .build(cases);
    let mut cases_dictionary = Dictionary::new();
    let mut activities = Dictionary::new();
    let mut traces = Vec::new();
    let mut current_case = None;
    let mut current_events = Vec::new();
    for (index, source) in generated.into_iter().enumerate() {
        let case = CaseId(cases_dictionary.intern(&source.case_id));
        if current_case.is_some_and(|current| current != case) {
            traces.push(CaseTrace {
                case: current_case.unwrap(),
                events: std::mem::take(&mut current_events),
                incomplete: false,
                attributes: Vec::new(),
            });
        }
        current_case = Some(case);
        current_events.push(Event::new(
            ActivityId(activities.intern(&source.activity)),
            source.timestamp.and_utc().timestamp_micros(),
            u32::try_from(index + 2).unwrap(),
        ));
    }
    if let Some(case) = current_case {
        traces.push(CaseTrace {
            case,
            events: current_events,
            incomplete: false,
            attributes: Vec::new(),
        });
    }
    EventLog {
        cases: cases_dictionary,
        activities,
        step_values: Default::default(),
        traces,
        ..EventLog::default()
    }
}

fn activity(log: &EventLog, label: &str) -> ProcessTree {
    ProcessTree::activity(ActivityId(log.activities.get(label).unwrap()))
}

#[test]
fn every_trace_generated_from_a_composite_model_replays_perfectly() {
    let source_model = Model::Sequence(vec![
        Model::act("Start"),
        Model::Parallel(vec![Model::act("Credit"), Model::act("Inventory")]),
        Model::Choice(vec![
            (1.0, Model::act("Approve")),
            (1.0, Model::act("Escalate")),
        ]),
        Model::Loop {
            body: Box::new(Model::act("Review")),
            redo: Box::new(Model::act("Rework")),
            repeat: 0.45,
            max_repeats: 3,
        },
        Model::act("Close"),
    ]);
    let log = generated_log(source_model, 1_000);
    let expected = ProcessTree::sequence(vec![
        activity(&log, "Start"),
        ProcessTree::parallel(vec![activity(&log, "Credit"), activity(&log, "Inventory")]),
        ProcessTree::exclusive(vec![activity(&log, "Approve"), activity(&log, "Escalate")]),
        ProcessTree::loop_of(activity(&log, "Review"), activity(&log, "Rework")),
        activity(&log, "Close"),
    ]);

    let replay = replay_tokens(&log, &expected.to_petri_net().unwrap()).unwrap();
    assert_eq!(replay.cases.len(), 1_000);
    assert_eq!(replay.fit_cases, 1_000);
    assert_eq!(replay.fitness_millionths, 1_000_000);
    assert_eq!(replay.missing_tokens, 0);
    assert_eq!(replay.remaining_tokens, 0);
    assert_eq!(replay.unmapped_events, 0);
    assert!(replay.cases.iter().all(|case| case.deviations.is_empty()));
}
