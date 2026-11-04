use chrono::NaiveDate;
use pm_core::{
    ActivityId, CaseId, CaseTrace, Dictionary, Event, EventLog, InductiveMinerConfig, ProcessTree,
    discover_inductive,
};
use pm_testgen::{LogBuilder, Model, Wait};

fn generated_log(model: Model, cases: usize, seed: u64) -> EventLog {
    let start = NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap();
    let generated = LogBuilder::new(model, start)
        .seed(seed)
        .case_interval(Wait::fixed(60))
        .default_wait(Wait::fixed(1))
        .build(cases);

    let mut case_dictionary = Dictionary::new();
    let mut activity_dictionary = Dictionary::new();
    let mut traces = Vec::new();
    let mut current_case = None;
    let mut current_events = Vec::new();

    for (index, source) in generated.into_iter().enumerate() {
        let case = CaseId(case_dictionary.intern(&source.case_id));
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
            ActivityId(activity_dictionary.intern(&source.activity)),
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
        cases: case_dictionary,
        activities: activity_dictionary,
        step_values: Default::default(),
        traces,
        ..EventLog::default()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum LabelledTree {
    Activity(String),
    Tau,
    Sequence(Vec<LabelledTree>),
    Exclusive(Vec<LabelledTree>),
    Parallel(Vec<LabelledTree>),
    Loop(Box<LabelledTree>, Box<LabelledTree>),
}

fn labelled(tree: &ProcessTree, log: &EventLog) -> LabelledTree {
    match tree {
        ProcessTree::Activity { activity } => {
            LabelledTree::Activity(log.activity_label(*activity).to_owned())
        }
        ProcessTree::Tau => LabelledTree::Tau,
        ProcessTree::Sequence { children } => {
            LabelledTree::Sequence(children.iter().map(|child| labelled(child, log)).collect())
        }
        ProcessTree::Exclusive { children } => {
            let mut children: Vec<_> = children.iter().map(|child| labelled(child, log)).collect();
            children.sort_by_key(|child| format!("{child:?}"));
            LabelledTree::Exclusive(children)
        }
        ProcessTree::Parallel { children } => {
            let mut children: Vec<_> = children.iter().map(|child| labelled(child, log)).collect();
            children.sort_by_key(|child| format!("{child:?}"));
            LabelledTree::Parallel(children)
        }
        ProcessTree::Loop { body, redo } => {
            LabelledTree::Loop(Box::new(labelled(body, log)), Box::new(labelled(redo, log)))
        }
    }
}

fn discover(model: Model, cases: usize, seed: u64) -> LabelledTree {
    let log = generated_log(model, cases, seed);
    let result = discover_inductive(&log, InductiveMinerConfig::default()).unwrap();
    assert!(!result.used_flower_fallback);
    labelled(&result.tree, &log)
}

#[test]
fn recovers_generated_sequence() {
    assert_eq!(
        discover(Model::seq(&["Receive", "Approve", "Ship"]), 100, 1),
        LabelledTree::Sequence(vec![
            LabelledTree::Activity("Receive".to_owned()),
            LabelledTree::Activity("Approve".to_owned()),
            LabelledTree::Activity("Ship".to_owned()),
        ])
    );
}

#[test]
fn recovers_generated_exclusive_choice() {
    let model = Model::Sequence(vec![
        Model::act("Start"),
        Model::Choice(vec![
            (1.0, Model::act("Approve")),
            (1.0, Model::act("Reject")),
        ]),
        Model::act("Close"),
    ]);
    assert_eq!(
        discover(model, 500, 7),
        LabelledTree::Sequence(vec![
            LabelledTree::Activity("Start".to_owned()),
            LabelledTree::Exclusive(vec![
                LabelledTree::Activity("Approve".to_owned()),
                LabelledTree::Activity("Reject".to_owned()),
            ]),
            LabelledTree::Activity("Close".to_owned()),
        ])
    );
}

#[test]
fn recovers_generated_parallelism() {
    let model = Model::Sequence(vec![
        Model::act("Start"),
        Model::Parallel(vec![Model::act("Credit"), Model::act("Inventory")]),
        Model::act("Close"),
    ]);
    assert_eq!(
        discover(model, 500, 11),
        LabelledTree::Sequence(vec![
            LabelledTree::Activity("Start".to_owned()),
            LabelledTree::Parallel(vec![
                LabelledTree::Activity("Credit".to_owned()),
                LabelledTree::Activity("Inventory".to_owned()),
            ]),
            LabelledTree::Activity("Close".to_owned()),
        ])
    );
}

#[test]
fn recovers_generated_redo_loop() {
    let model = Model::Sequence(vec![
        Model::act("Submit"),
        Model::Loop {
            body: Box::new(Model::act("Review")),
            redo: Box::new(Model::act("Rework")),
            repeat: 0.5,
            max_repeats: 3,
        },
        Model::act("Close"),
    ]);
    assert_eq!(
        discover(model, 500, 23),
        LabelledTree::Sequence(vec![
            LabelledTree::Activity("Submit".to_owned()),
            LabelledTree::Loop(
                Box::new(LabelledTree::Activity("Review".to_owned())),
                Box::new(LabelledTree::Activity("Rework".to_owned())),
            ),
            LabelledTree::Activity("Close".to_owned()),
        ])
    );
}
