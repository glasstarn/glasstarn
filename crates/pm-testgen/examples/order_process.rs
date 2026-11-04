//! Generates a realistic order-to-cash log and prints it as CSV.
//!
//! Run with: `cargo run -p pm-testgen --example order_process`

use chrono::NaiveDate;
use pm_testgen::{LogBuilder, Model, Wait};

fn main() {
    // Receive → Check Credit → (Approve | Reject) → pick & pack in parallel
    // → Ship, with an occasional rework loop on approval.
    let model = Model::Sequence(vec![
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
                    Model::Parallel(vec![Model::act("Pick Items"), Model::act("Print Invoice")]),
                    Model::act("Ship Order"),
                ]),
            ),
            (2.0, Model::act("Reject Order")),
        ]),
    ]);

    let start = NaiveDate::from_ymd_opt(2026, 3, 2)
        .unwrap()
        .and_hms_opt(8, 15, 0)
        .unwrap();

    let log = LogBuilder::new(model, start)
        .seed(2026)
        .case_prefix("ORDER-")
        .case_interval(Wait::range(900, 10_800))
        .default_wait(Wait::range(600, 5_400))
        // The planted bottleneck: credit checks sit for one to three days.
        .wait_for("Check Credit", Wait::range(86_400, 259_200))
        .wait_for("Request Information", Wait::range(43_200, 172_800));

    let cases: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(200);
    print!("{}", log.build_csv(cases));
}
