//! Benchmark harness.
//!
//! Performance is this project's central claim, so it is measured rather than
//! asserted. No performance figure goes into public material unless it exists
//! in a run of this harness on a stated machine.
//!
//! Measures **the path the product runs** — `pm_query::analyze_filtered` — not
//! a parallel implementation. A benchmark of code users do not execute proves
//! nothing, and an earlier version of this harness made exactly that mistake.
//!
//! Reports peak heap alongside timings, because memory rather than CPU is the
//! binding constraint on a laptop.
//!
//! Usage:
//!   pm-bench                    # default sizes
//!   pm-bench --events 1000000
//!   pm-bench --ocel --ocel-shape lifecycle --events 1000000
//!   pm-bench --ocel --ocel-shape interaction --events 1000000
//!   pm-bench --json

mod ocel_bench;

use chrono::NaiveDate;
use peak_alloc::PeakAlloc;
use pm_ingest::Mapping;
use pm_ingest::finding::Policy;
use pm_testgen::{LogBuilder, Model, Wait};
use std::time::Instant;

#[global_allocator]
static PEAK: PeakAlloc = PeakAlloc;

/// One measured phase.
pub(crate) struct Phase {
    pub(crate) name: &'static str,
    pub(crate) millis: f64,
    pub(crate) peak_mb: f64,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let json = args.iter().any(|a| a == "--json");
    let object_centric = args.iter().any(|a| a == "--ocel");

    let complex = args.iter().any(|a| a == "--complex");
    let sizes: Vec<usize> = match args.iter().position(|a| a == "--events") {
        Some(i) => vec![
            args.get(i + 1)
                .and_then(|v| v.parse().ok())
                .unwrap_or(1_000_000),
        ],
        None => vec![100_000, 1_000_000],
    };

    if args.iter().any(|argument| argument == "--explore") {
        run_exploration(sizes[0]);
        return;
    }

    if object_centric {
        let shape = ocel_shape(&args);
        let all = sizes
            .into_iter()
            .map(|target| ocel_bench::run(target, shape))
            .collect::<Vec<_>>();
        if json {
            println!(
                "{}",
                serde_json::to_string_pretty(&all.iter().map(ocel_bench::json).collect::<Vec<_>>())
                    .unwrap()
            );
        } else {
            for result in &all {
                ocel_bench::report(result);
            }
        }
        return;
    }

    let mut all = Vec::new();
    for target in sizes {
        let result = run_shape(target, complex);
        if !json {
            report(&result);
        }
        all.push(result);
    }

    if json {
        let out: Vec<_> = all
            .iter()
            .map(|r| {
                serde_json::json!({
                    "events": r.events,
                    "cases": r.cases,
                    "activities": r.activities,
                    "variants": r.variants,
                    "csv_bytes": r.csv_bytes,
                    "phases": r.phases.iter().map(|p| serde_json::json!({
                        "phase": p.name,
                        "millis": p.millis,
                        "peak_mb": p.peak_mb,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
    }
}

fn ocel_shape(args: &[String]) -> ocel_bench::Shape {
    match args.iter().position(|argument| argument == "--ocel-shape") {
        None => ocel_bench::Shape::Lifecycle,
        Some(index) => match args.get(index + 1).map(String::as_str) {
            Some("lifecycle") => ocel_bench::Shape::Lifecycle,
            Some("interaction") => ocel_bench::Shape::Interaction,
            Some(other) => {
                panic!("unknown object-centric shape '{other}'; use lifecycle or interaction")
            }
            None => panic!("--ocel-shape needs lifecycle or interaction"),
        },
    }
}

struct Run {
    events: usize,
    cases: usize,
    activities: usize,
    variants: usize,
    csv_bytes: usize,
    phases: Vec<Phase>,
}

/// A hostile shape: many activities and enormous variant diversity.
///
/// The tidy model below has 10 activities and 19 variants. Real logs can have
/// hundreds of activities and tens of thousands of distinct paths, and variant
/// computation holds every distinct sequence — so this is the shape most
/// likely to break a memory estimate derived from the tidy one.
fn complex_model(activities: usize) -> Model {
    // A long chain of independent choices. Each doubles the number of possible
    // paths, so variant diversity explodes rather than staying tidy.
    let mut steps: Vec<Model> = vec![Model::act("Start")];
    for i in 0..activities {
        steps.push(Model::Choice(vec![
            (3.0, Model::act(&format!("Step {i:03} A"))),
            (2.0, Model::act(&format!("Step {i:03} B"))),
            (1.0, Model::act(&format!("Step {i:03} C"))),
        ]));
    }
    steps.push(Model::act("End"));
    Model::Sequence(steps)
}

/// A deliberately unflattering model: a choice, a parallel block, and a rework
/// loop, so variant counts and graph structure are realistic rather than a
/// single trivial path.
fn model() -> Model {
    Model::Sequence(vec![
        Model::act("Receive Order"),
        Model::act("Check Credit"),
        Model::Choice(vec![
            (
                8.0,
                Model::Sequence(vec![
                    Model::Loop {
                        body: Box::new(Model::act("Approve Order")),
                        redo: Box::new(Model::act("Request Information")),
                        repeat: 0.25,
                        max_repeats: 2,
                    },
                    Model::Parallel(vec![
                        Model::act("Pick Items"),
                        Model::act("Print Invoice"),
                        Model::act("Reserve Stock"),
                    ]),
                    Model::act("Ship Order"),
                    Model::act("Invoice Customer"),
                ]),
            ),
            (2.0, Model::act("Reject Order")),
        ]),
    ])
}

fn run_shape(target_events: usize, complex: bool) -> Run {
    // The complex model emits one event per step plus start and end.
    let (events_per_case, process) = if complex {
        (62, complex_model(60))
    } else {
        (9, model())
    };
    let cases = (target_events / events_per_case).max(1);

    let builder = LogBuilder::new(
        process,
        NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(8, 0, 0)
            .unwrap(),
    )
    .seed(2026)
    .case_prefix("ORDER-")
    .case_interval(Wait::range(60, 600))
    .default_wait(Wait::range(600, 5_400))
    .wait_for("Check Credit", Wait::range(86_400, 259_200));

    let mut phases = Vec::new();

    // Generation is not part of the product; it is measured only so the
    // pipeline timings can be read without wondering what the setup cost.
    let (csv, generate) = time(|| builder.build_csv(cases));
    phases.push(generate("generate log"));

    let path = std::env::temp_dir().join(format!("pm_bench_{cases}.csv"));
    std::fs::write(&path, &csv).expect("write benchmark log");
    let csv_bytes = csv.len();
    drop(csv);

    let mapping = Mapping::new("case_id", "activity", "timestamp");

    // The product path, end to end. Anything measured here is what a user
    // running `glasstarn analyze` actually experiences.
    let (outcome, analyze_time) = time(|| {
        pm_query::analyze_filtered(
            &path,
            Some(mapping.clone()),
            &Policy::new(),
            pm_ingest::Rules::new(),
            pm_core::filter::Filter::new(),
        )
        .expect("analysis")
    });
    phases.push(analyze_time("analyze (product path)"));

    let analysis = match outcome {
        pm_query::Outcome::Complete(a) => a,
        pm_query::Outcome::Blocked(b) => panic!("benchmark log blocked: {:?}", b.report),
    };

    let result = Run {
        events: analysis.summary.events,
        cases: analysis.summary.cases,
        activities: analysis.summary.activities,
        variants: analysis.summary.variants,
        csv_bytes,
        phases,
    };

    let _ = std::fs::remove_file(&path);
    result
}

/// Times a closure and captures peak heap use during it.
pub(crate) fn time<T>(f: impl FnOnce() -> T) -> (T, impl FnOnce(&'static str) -> Phase) {
    PEAK.reset_peak_usage();
    let start = Instant::now();
    let value = f();
    let millis = start.elapsed().as_secs_f64() * 1000.0;
    let peak_mb = PEAK.peak_usage_as_mb() as f64;
    (value, move |name| Phase {
        name,
        millis,
        peak_mb,
    })
}

fn report(r: &Run) {
    println!(
        "\n{} events · {} cases · {} activities · {} variants · {:.1} MB CSV",
        r.events,
        r.cases,
        r.activities,
        r.variants,
        r.csv_bytes as f64 / 1_048_576.0
    );
    println!("{:<26} {:>12} {:>12}", "phase", "time", "peak heap");
    println!("{}", "-".repeat(52));

    let mut pipeline_ms = 0.0;
    for p in &r.phases {
        if p.name != "generate log" {
            pipeline_ms += p.millis;
        }
        println!(
            "{:<26} {:>12} {:>10.0} MB",
            p.name,
            format_ms(p.millis),
            p.peak_mb
        );
    }
    println!("{}", "-".repeat(52));
    println!("{:<26} {:>12}", "pipeline total", format_ms(pipeline_ms));
    println!(
        "{:<26} {:>12}",
        "per million events",
        format_ms(pipeline_ms / r.events as f64 * 1_000_000.0)
    );
}

fn format_ms(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else {
        format!("{ms:.0} ms")
    }
}

// Separate query measurement: source loading is outside the timed query phase.
fn run_exploration(events: usize) {
    let builder = LogBuilder::new(
        model(),
        NaiveDate::from_ymd_opt(2026, 1, 1)
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap(),
    )
    .seed(2026)
    .default_wait(Wait::range(600, 5400));
    // The generated model varies around 7.5 events per case. Overshoot the
    // requested size so a million-event run actually exercises at least a million.
    let csv = builder.build_csv((events / 7).max(1));
    let path = std::env::temp_dir().join(format!(
        "glasstarn-exploration-bench-{}.csv",
        std::process::id()
    ));
    std::fs::write(&path, &csv).unwrap();
    drop(csv);
    let loaded = pm_query::load_log(
        &path,
        Some(Mapping::new("case_id", "activity", "timestamp")),
        &Policy::new(),
        Default::default(),
    )
    .unwrap()
    .unwrap();
    let first = &loaded.log.traces[0];
    let from = loaded
        .log
        .activities
        .label(first.events[0].activity.0)
        .unwrap()
        .to_owned();
    let to = loaded
        .log
        .activities
        .label(first.events.last().unwrap().activity.0)
        .unwrap()
        .to_owned();
    let query = pm_query::exploration::ExplorationRequest {
        segment: Some(pm_core::segment::Segment {
            from,
            to,
            to_occurrence: pm_core::segment::Occurrence::Last,
            ..Default::default()
        }),
        ..Default::default()
    };
    let (report, phase) = time(|| pm_query::exploration::explore(&loaded.log, &query).unwrap());
    let phase = phase("endpoint exploration");
    println!(
        "{}",
        serde_json::json!({"events":loaded.log.event_count(),"items":report.overall.items,"measurable":report.overall.measurable,"query_millis":phase.millis,"peak_heap_mb":phase.peak_mb,"source_loading_included":false})
    );
    std::fs::remove_file(path).unwrap();
}
