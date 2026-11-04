//! Made-up HubSpot extracts in the shape the HubSpot extract writes them: one
//! file per object with current values and stage labels, and one history
//! file with every value each listed property took, its label, its time and
//! whether it was the record's first value. Property names and stage values
//! follow public knowledge of HubSpot's CRM API; every record is invented,
//! and the counts in each truth are known by construction.

use crate::Rng;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

const HISTORY_HEADER: &str = "object,record,property,value,label,timestamp,source,user,first";
const OWNERS: [&str; 3] = ["101", "102", "103"];

#[derive(Debug, Clone)]
pub struct Settings {
    pub records: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            records: 300,
            seed: 7,
        }
    }
}

/// One HubSpot object as a pack reads it.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    pub object: &'static str,
    pub file: &'static str,
    /// The property moved through `stages`.
    pub stage: &'static str,
    /// Stored value and label, in the usual order; the last two end the
    /// record (for deals, won and lost).
    pub stages: &'static [(&'static str, &'static str)],
    /// A second property changed now and then, with its two values.
    pub other: (&'static str, &'static str, &'static str),
    /// Further columns the extract writes, with a value.
    pub extra: &'static [(&'static str, &'static str)],
}

pub const CONTACTS: Kind = Kind {
    object: "contacts",
    file: "HubSpotContact",
    stage: "lifecyclestage",
    stages: &[
        ("subscriber", "Subscriber"),
        ("lead", "Lead"),
        ("marketingqualifiedlead", "Marketing Qualified Lead"),
        ("salesqualifiedlead", "Sales Qualified Lead"),
        ("opportunity", "Opportunity"),
        ("customer", "Customer"),
        ("other", "Other"),
    ],
    other: ("hs_lead_status", "NEW", "IN_PROGRESS"),
    extra: &[],
};

pub const DEALS: Kind = Kind {
    object: "deals",
    file: "HubSpotDeal",
    stage: "dealstage",
    stages: &[
        ("appointmentscheduled", "Appointment scheduled"),
        ("qualifiedtobuy", "Qualified to buy"),
        ("presentationscheduled", "Presentation scheduled"),
        ("decisionmakerboughtin", "Decision maker bought-in"),
        ("contractsent", "Contract sent"),
        ("closedwon", "Closed won"),
        ("closedlost", "Closed lost"),
    ],
    other: ("amount", "5000", "6500"),
    extra: &[
        ("pipeline", "default"),
        ("closedate", "2026-03-31T00:00:00.000Z"),
    ],
};

pub const TICKETS: Kind = Kind {
    object: "tickets",
    file: "HubSpotTicket",
    stage: "hs_pipeline_stage",
    stages: &[
        ("1", "New"),
        ("2", "Waiting on contact"),
        ("3", "Waiting on us"),
        ("4", "Closed"),
        ("4", "Closed"),
    ],
    other: ("hs_ticket_priority", "MEDIUM", "HIGH"),
    extra: &[("hs_pipeline", "0")],
};

pub const QUOTES: Kind = Kind {
    object: "quotes",
    file: "HubSpotQuote",
    stage: "hs_status",
    stages: &[
        ("DRAFT", "Draft"),
        ("PENDING_APPROVAL", "Pending approval"),
        ("APPROVED", "Approved"),
        ("REJECTED", "Rejected"),
    ],
    other: ("hs_expiration_date", "2026-03-01", "2026-03-15"),
    extra: &[("hs_title", "Annual licence")],
};

/// What the generated extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub records: u64,
    /// Stage changes after the first value.
    pub stage_changes: u64,
    /// Stages entered again after moving back.
    pub sent_back: u64,
    pub other_changes: u64,
    pub owner_changes: u64,
    /// First values, which the packs leave out.
    pub first_values: u64,
}

fn at(time: NaiveDateTime) -> String {
    time.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 1, 1)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
}

/// Writes the object's file and HubSpotHistory.csv into `dir`. Every record
/// starts in the first stage and moves forward; some step back once, some
/// stop early, and the rest end in one of the last two stages.
pub fn write(dir: &Path, settings: &Settings, kind: Kind) -> io::Result<Truth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = Truth::default();
    let mut history = vec![HISTORY_HEADER.to_owned()];
    let mut records = vec![format!(
        "id,created,{},{},hubspot_owner_id,{}_label{}",
        kind.stage,
        kind.other.0,
        kind.stage,
        kind.extra
            .iter()
            .map(|(c, _)| format!(",{c}"))
            .collect::<String>()
    )];
    let ends = kind.stages.len() - 2;
    let row = |history: &mut Vec<String>,
               id: &str,
               property: &str,
               (value, label): (&str, &str),
               time: NaiveDateTime,
               first: bool| {
        history.push(format!(
            "{},{id},{property},{value},{label},{},CRM_UI,{},{first}",
            kind.object,
            at(time),
            OWNERS[0]
        ));
    };
    for n in 0..settings.records {
        truth.records += 1;
        let id = format!("{}", 50_000 + n);
        let created = start()
            + Duration::days(rng.below(80) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let mut now = created;
        let mut stage = 0usize;
        truth.first_values += 3;
        row(&mut history, &id, kind.stage, kind.stages[0], now, true);
        row(
            &mut history,
            &id,
            kind.other.0,
            (kind.other.1, kind.other.1),
            now,
            true,
        );
        row(
            &mut history,
            &id,
            "hubspot_owner_id",
            (OWNERS[0], OWNERS[0]),
            now,
            true,
        );
        let goal = if rng.chance(0.2) {
            1 + rng.below(ends as u64 - 1) as usize
        } else {
            ends
        };
        while stage < goal {
            now += Duration::hours(4 + rng.below(72) as i64);
            stage += 1;
            truth.stage_changes += 1;
            row(
                &mut history,
                &id,
                kind.stage,
                kind.stages[stage],
                now,
                false,
            );
            if stage > 1 && stage < goal && rng.chance(0.1) {
                now += Duration::hours(6);
                truth.stage_changes += 2;
                truth.sent_back += 1;
                row(
                    &mut history,
                    &id,
                    kind.stage,
                    kind.stages[stage - 1],
                    now,
                    false,
                );
                now += Duration::hours(6);
                row(
                    &mut history,
                    &id,
                    kind.stage,
                    kind.stages[stage],
                    now,
                    false,
                );
            }
        }
        if goal == ends && kind.stages[ends] != kind.stages[ends + 1] && rng.chance(0.3) {
            // Lost (or rejected) instead of the usual end.
            now += Duration::hours(24);
            stage = ends + 1;
            truth.stage_changes += 1;
            row(
                &mut history,
                &id,
                kind.stage,
                kind.stages[stage],
                now,
                false,
            );
        }
        let mut other = kind.other.1;
        if rng.chance(0.2) {
            now += Duration::hours(2);
            truth.other_changes += 1;
            other = kind.other.2;
            row(&mut history, &id, kind.other.0, (other, other), now, false);
        }
        let mut owner = OWNERS[0];
        if rng.chance(0.15) {
            now += Duration::hours(1);
            truth.owner_changes += 1;
            owner = OWNERS[1 + rng.below(2) as usize];
            row(
                &mut history,
                &id,
                "hubspot_owner_id",
                (owner, owner),
                now,
                false,
            );
        }
        records.push(format!(
            "{id},{},{},{other},{owner},{}{}",
            at(created),
            kind.stages[stage].0,
            kind.stages[stage].1,
            kind.extra
                .iter()
                .map(|(_, v)| format!(",{v}"))
                .collect::<String>()
        ));
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        dir.join(format!("{}.csv", kind.file)),
        records.join("\n") + "\n",
    )?;
    std::fs::write(dir.join("HubSpotHistory.csv"), history.join("\n") + "\n")?;
    Ok(truth)
}
