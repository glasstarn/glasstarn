//! Made-up extracts from build-your-own workflow platforms: Pega's work and
//! work history tables, Appian record events, and Power Automate's Dataverse
//! audit log and flow runs. Times in UTC (`2026-01-05T14:30:00Z`). Every
//! record is invented; the counts in the truth structs are known by
//! construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

/// Settings for one generated extract.
#[derive(Debug, Clone)]
pub struct Settings {
    pub records: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            records: 300,
            seed: 23,
        }
    }
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
}

/// What the Pega extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PegaTruth {
    pub cases: u64,
    pub history_rows: u64,
    pub resolved: u64,
}

pub const PEGA_FILES: [&str; 2] = ["pc_work", "pc_history_work"];

/// Writes pc_work and pc_history_work for claims cases.
pub fn write_pega(dir: &Path, settings: &Settings) -> io::Result<PegaTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x9E6A);
    let mut truth = PegaTruth::default();
    let mut work = Table::new(
        "pzInsKey,pyID,pxObjClass,pxCreateDateTime,pyStatusWork,pyResolvedTimestamp,pxUrgencyWork,pxCreateOperator,pyResolvedUserID",
    );
    let mut history = Table::new(
        "pzInsKey,pxHistoryForReference,pxTimeCreated,pyPerformer,pyMemo,pyHistoryType,pyMessageKey",
    );
    let steps = [
        "Intake completed",
        "Assigned to adjuster",
        "Documents requested",
        "Documents received",
        "Assessment completed",
        "Approved for payment",
    ];
    let mut row = 0u64;
    for n in 0..settings.records {
        truth.cases += 1;
        let id = format!("C-{}", 1000 + n);
        let key = format!("ACME-CLAIMS-WORK {id}");
        let created = start()
            + Duration::days(rng.below(90) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let mut at = created;
        let mut add =
            |history: &mut Table, truth: &mut PegaTruth, memo: &str, at: NaiveDateTime| {
                row += 1;
                truth.history_rows += 1;
                history.push(&[
                    &format!("HISTORY-ACME-CLAIMS-WORK {id} {row:08}"),
                    &key,
                    &utc(at),
                    "adjuster1",
                    memo,
                    "F",
                    "FlowAction",
                ]);
            };
        let done = if rng.chance(0.8) {
            steps.len()
        } else {
            1 + rng.below(5) as usize
        };
        for (index, step) in steps.iter().enumerate().take(done) {
            at += Duration::hours(2 + rng.below(72) as i64);
            add(&mut history, &mut truth, step, at);
            // Documents are sometimes requested twice.
            if index == 3 && rng.chance(0.2) {
                at += Duration::hours(24);
                add(&mut history, &mut truth, "Documents requested", at);
                at += Duration::hours(30);
                add(&mut history, &mut truth, "Documents received", at);
            }
        }
        let resolved = done == steps.len();
        if resolved {
            truth.resolved += 1;
            at += Duration::hours(1);
        }
        work.push(&[
            &key,
            &id,
            "ACME-Claims-Work-Claim",
            &utc(created),
            if resolved {
                "Resolved-Completed"
            } else {
                "Open"
            },
            &if resolved { utc(at) } else { String::new() },
            "10",
            "intake1",
            if resolved { "adjuster1" } else { "" },
        ]);
    }
    save(dir, &[("pc_work", &work), ("pc_history_work", &history)])?;
    Ok(truth)
}

/// What the Appian extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct AppianTruth {
    pub records: u64,
    pub events: u64,
}

pub const APPIAN_FILES: [&str; 3] = ["Record", "RecordEvent", "RecordEventType"];

/// Writes Record, RecordEvent and RecordEventType for requests.
pub fn write_appian(dir: &Path, settings: &Settings) -> io::Result<AppianTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xA991);
    let mut truth = AppianTruth::default();
    let mut records = Table::new("id,createdOn,status,createdBy,modifiedOn");
    let mut events = Table::new("id,recordId,eventTypeId,timestamp,user,details");
    let mut types = Table::new("id,value");
    let names = [
        "Created",
        "Submitted",
        "Reviewed",
        "Returned",
        "Approved",
        "Closed",
    ];
    for (index, name) in names.iter().enumerate() {
        types.push(&[&(index + 1).to_string(), name]);
    }
    let mut event = 0u64;
    for n in 0..settings.records {
        truth.records += 1;
        let id = (5000 + n).to_string();
        let created = start()
            + Duration::days(rng.below(90) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let mut at = created;
        let mut add =
            |events: &mut Table, truth: &mut AppianTruth, kind: usize, at: NaiveDateTime| {
                event += 1;
                truth.events += 1;
                events.push(&[
                    &event.to_string(),
                    &id,
                    &(kind + 1).to_string(),
                    &utc(at),
                    "jane.doe",
                    "",
                ]);
            };
        add(&mut events, &mut truth, 0, at);
        at += Duration::hours(1 + rng.below(48) as i64);
        add(&mut events, &mut truth, 1, at);
        // Returned for rework, then reviewed again.
        while rng.chance(0.25) {
            at += Duration::hours(4 + rng.below(48) as i64);
            add(&mut events, &mut truth, 3, at);
            at += Duration::hours(4 + rng.below(48) as i64);
            add(&mut events, &mut truth, 1, at);
        }
        at += Duration::hours(4 + rng.below(72) as i64);
        add(&mut events, &mut truth, 2, at);
        let closed = rng.chance(0.8);
        if closed {
            at += Duration::hours(1 + rng.below(24) as i64);
            add(&mut events, &mut truth, 4, at);
            at += Duration::hours(1);
            add(&mut events, &mut truth, 5, at);
        }
        records.push(&[
            &id,
            &utc(created),
            if closed { "Closed" } else { "In Review" },
            "jane.doe",
            &utc(at),
        ]);
    }
    save(
        dir,
        &[
            ("Record", &records),
            ("RecordEvent", &events),
            ("RecordEventType", &types),
        ],
    )?;
    Ok(truth)
}

/// What the Power Automate extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PowerAutomateTruth {
    pub records: u64,
    pub audit_rows: u64,
    pub runs: u64,
    pub succeeded: u64,
    pub failed: u64,
}

pub const POWER_AUTOMATE_FILES: [&str; 3] = ["audit", "flowrun", "workflow"];

/// Writes audit, flowrun and workflow.
pub fn write_power_automate(dir: &Path, settings: &Settings) -> io::Result<PowerAutomateTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x9A0E);
    let mut truth = PowerAutomateTruth::default();
    let mut audit = Table::new("auditid,objectid,objecttypecode,action,createdon,userid,operation");
    let mut runs =
        Table::new("flowrunid,workflowid,name,starttime,endtime,status,triggertype,duration");
    let mut flows = Table::new("workflowid,name,category,statecode");
    let flow_ids = [
        "6f1e0000-0000-0000-0000-000000000001",
        "6f1e0000-0000-0000-0000-000000000002",
    ];
    for (id, name) in flow_ids
        .iter()
        .zip(["Route new request", "Notify approver"])
    {
        flows.push(&[id, name, "5", "1"]);
    }
    let mut row = 0u64;
    let mut run = 0u64;
    for n in 0..settings.records {
        truth.records += 1;
        let record = format!("a1b2c3d4-0000-0000-0000-{n:012}");
        let created = start()
            + Duration::days(rng.below(90) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let mut at = created;
        let mut add =
            |audit: &mut Table, truth: &mut PowerAutomateTruth, action: &str, at: NaiveDateTime| {
                row += 1;
                truth.audit_rows += 1;
                audit.push(&[
                    &format!("00000000-0000-0000-aaaa-{row:012}"),
                    &record,
                    "incident",
                    action,
                    &utc(at),
                    "user-1",
                    "Create",
                ]);
            };
        add(&mut audit, &mut truth, "Create", at);
        // A flow runs on every new record.
        run += 1;
        truth.runs += 1;
        let failed = rng.chance(0.08);
        let ended = at + Duration::seconds(5 + rng.below(120) as i64);
        if failed {
            truth.failed += 1;
        } else {
            truth.succeeded += 1;
        }
        runs.push(&[
            &format!("08585{run:015}"),
            flow_ids[0],
            &format!("08585{run:015}"),
            &utc(at),
            &utc(ended),
            if failed { "Failed" } else { "Succeeded" },
            "Automated",
            "30",
        ]);
        for _ in 0..1 + rng.below(4) {
            at += Duration::hours(1 + rng.below(48) as i64);
            add(&mut audit, &mut truth, "Update", at);
        }
        if rng.chance(0.3) {
            at += Duration::hours(2);
            add(&mut audit, &mut truth, "Assign", at);
        }
    }
    // A run still going: no end yet.
    run += 1;
    truth.runs += 1;
    runs.push(&[
        &format!("08585{run:015}"),
        flow_ids[1],
        &format!("08585{run:015}"),
        &utc(start() + Duration::days(95)),
        "",
        "Running",
        "Manual",
        "",
    ]);
    save(
        dir,
        &[("audit", &audit), ("flowrun", &runs), ("workflow", &flows)],
    )?;
    Ok(truth)
}
