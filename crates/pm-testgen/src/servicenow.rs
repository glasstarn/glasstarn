//! Made-up ServiceNow extracts in the shape the ServiceNow extract writes
//! them: one file per table with stored values, `<field>_label` columns with
//! display values, and one audit file with a row per changed field (stored
//! and display values), times in UTC. Table, field and state names follow
//! public knowledge of ServiceNow's Table API and `sys_audit`; every record
//! is invented, and the counts in each truth are known by construction.

use crate::Rng;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

pub const INCIDENT_FILES: [&str; 2] = ["ServiceNowIncident", "ServiceNowAudit"];
pub const CHANGE_FILES: [&str; 3] = [
    "ServiceNowChangeRequest",
    "ServiceNowChangeTask",
    "ServiceNowAudit",
];
pub const REQUEST_FILES: [&str; 3] = [
    "ServiceNowRequestItem",
    "ServiceNowCatalogTask",
    "ServiceNowAudit",
];

const AUDIT_HEADER: &str =
    "table,record,audit_id,created,user,field,old_value,old_label,new_value,new_label";

const GROUPS: [(&str, &str); 4] = [
    ("a1b2c3d4e5f60718293a4b5c6d7e8f01", "Service Desk"),
    ("a1b2c3d4e5f60718293a4b5c6d7e8f02", "Network"),
    ("a1b2c3d4e5f60718293a4b5c6d7e8f03", "Database"),
    ("a1b2c3d4e5f60718293a4b5c6d7e8f04", "Applications"),
];
const PEOPLE: [&str; 4] = [
    "b1b2c3d4e5f60718293a4b5c6d7e8f01",
    "b1b2c3d4e5f60718293a4b5c6d7e8f02",
    "b1b2c3d4e5f60718293a4b5c6d7e8f03",
    "b1b2c3d4e5f60718293a4b5c6d7e8f04",
];

#[derive(Debug, Clone)]
pub struct Settings {
    pub records: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            records: 300,
            seed: 11,
        }
    }
}

fn at(time: NaiveDateTime) -> String {
    time.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn csv(values: &[&str]) -> String {
    values
        .iter()
        .map(|v| {
            if v.contains([',', '"', '\n']) {
                format!("\"{}\"", v.replace('"', "\"\""))
            } else {
                (*v).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn sys_id(table: u64, n: u64) -> String {
    format!("{table:02x}{n:030x}")
}

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2025, 11, 1)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap()
}

/// The audit file being written.
struct Audit {
    rows: Vec<String>,
    next: u64,
}

impl Audit {
    fn new() -> Self {
        Self {
            rows: vec![AUDIT_HEADER.to_owned()],
            next: 0,
        }
    }

    /// One changed field: (field, old value, old label, new value, new label).
    fn change(
        &mut self,
        table: &str,
        record: &str,
        now: NaiveDateTime,
        change: (&str, &str, &str, &str, &str),
    ) {
        self.next += 1;
        let (field, old, old_label, new, new_label) = change;
        self.rows.push(csv(&[
            table,
            record,
            &format!("{:032x}", 0xa0d1_0000_0000u64 + self.next),
            &at(now),
            "agent.ana",
            field,
            old,
            old_label,
            new,
            new_label,
        ]));
    }
}

fn later(rng: &mut Rng, now: &mut NaiveDateTime, hours: u64) {
    *now += Duration::minutes(10 + rng.below(hours * 60) as i64);
}

fn write_files(dir: &Path, files: &[(&str, &[String])]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, rows) in files {
        std::fs::write(dir.join(format!("{name}.csv")), rows.join("\n") + "\n")?;
    }
    Ok(())
}

/// What the generated incident extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct IncidentTruth {
    pub incidents: u64,
    pub state_changes: u64,
    pub on_hold: u64,
    pub resolved: u64,
    pub reopened: u64,
    pub closed: u64,
    pub open: u64,
    pub group_assigned: u64,
    pub group_reassigned: u64,
    pub person_assigned: u64,
    pub person_reassigned: u64,
    pub priority_changes: u64,
    pub other_changes: u64,
    /// Work notes and the duplicate `incident_state` rows, which the pack
    /// leaves out.
    pub left_out: u64,
    /// Incidents with the company's own field "Business impact" filled.
    pub with_business_impact: u64,
}

const INCIDENT_STATES: [(&str, &str); 6] = [
    ("1", "New"),
    ("2", "In Progress"),
    ("3", "On Hold"),
    ("6", "Resolved"),
    ("7", "Closed"),
    ("8", "Canceled"),
];
const PRIORITIES: [(&str, &str); 4] = [
    ("1", "1 - Critical"),
    ("2", "2 - High"),
    ("3", "3 - Moderate"),
    ("4", "4 - Low"),
];

fn label(list: &[(&'static str, &'static str)], value: &str) -> &'static str {
    list.iter().find(|(v, _)| *v == value).unwrap().1
}

/// Writes ServiceNowIncident.csv and ServiceNowAudit.csv into `dir`.
pub fn write_incidents(dir: &Path, settings: &Settings) -> io::Result<IncidentTruth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = IncidentTruth::default();
    let mut audit = Audit::new();
    let mut records = vec!["sys_id,number,created,state,priority,category,assignment_group,assigned_to,u_business_impact,state_label,priority_label,assignment_group_label".to_owned()];
    for n in 0..settings.records {
        truth.incidents += 1;
        let id = sys_id(0x10, n);
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(rng.below(10 * 60) as i64);
        let mut now = created;
        let mut state = "1";
        let mut priority = if rng.chance(0.2) { "2" } else { "3" };
        let mut category = "inquiry";
        let mut group = if rng.chance(0.3) { GROUPS[0].0 } else { "" };
        let mut person;
        let set_state = |audit: &mut Audit,
                         truth: &mut IncidentTruth,
                         now: NaiveDateTime,
                         state: &mut &'static str,
                         next: &'static str| {
            truth.state_changes += 1;
            truth.left_out += 1;
            let (from, to) = (
                label(&INCIDENT_STATES, state),
                label(&INCIDENT_STATES, next),
            );
            audit.change("incident", &id, now, ("state", state, from, next, to));
            audit.change(
                "incident",
                &id,
                now,
                ("incident_state", state, from, next, to),
            );
            *state = next;
        };
        if group.is_empty() {
            later(&mut rng, &mut now, 2);
            truth.group_assigned += 1;
            let (to, to_label) = GROUPS[1 + rng.below(3) as usize];
            audit.change(
                "incident",
                &id,
                now,
                ("assignment_group", "", "", to, to_label),
            );
            group = to;
        }
        later(&mut rng, &mut now, 8);
        truth.person_assigned += 1;
        let first = PEOPLE[rng.below(4) as usize];
        audit.change("incident", &id, now, ("assigned_to", "", "", first, first));
        person = first;
        set_state(&mut audit, &mut truth, now, &mut state, "2");
        if rng.chance(0.5) {
            later(&mut rng, &mut now, 4);
            truth.left_out += 1;
            audit.change(
                "incident",
                &id,
                now,
                ("work_notes", "", "", "", "Checked the logs"),
            );
        }
        if rng.chance(0.25) {
            later(&mut rng, &mut now, 24);
            truth.group_reassigned += 1;
            let from = GROUPS.iter().position(|(g, _)| *g == group).unwrap();
            let (to, to_label) = GROUPS[1 + from % 3];
            audit.change(
                "incident",
                &id,
                now,
                ("assignment_group", group, GROUPS[from].1, to, to_label),
            );
            group = to;
        }
        if rng.chance(0.1) {
            later(&mut rng, &mut now, 12);
            truth.person_reassigned += 1;
            let to = PEOPLE[(PEOPLE.iter().position(|p| *p == person).unwrap() + 1) % 4];
            audit.change(
                "incident",
                &id,
                now,
                ("assigned_to", person, person, to, to),
            );
            person = to;
        }
        if rng.chance(0.1) {
            later(&mut rng, &mut now, 12);
            truth.priority_changes += 1;
            audit.change(
                "incident",
                &id,
                now,
                (
                    "priority",
                    priority,
                    label(&PRIORITIES, priority),
                    "1",
                    "1 - Critical",
                ),
            );
            priority = "1";
        }
        if rng.chance(0.1) {
            later(&mut rng, &mut now, 12);
            truth.other_changes += 1;
            audit.change(
                "incident",
                &id,
                now,
                ("category", category, "Inquiry / Help", "network", "Network"),
            );
            category = "network";
        }
        if rng.chance(0.15) {
            later(&mut rng, &mut now, 24);
            truth.on_hold += 1;
            set_state(&mut audit, &mut truth, now, &mut state, "3");
            later(&mut rng, &mut now, 48);
            set_state(&mut audit, &mut truth, now, &mut state, "2");
        }
        if rng.chance(0.85) {
            later(&mut rng, &mut now, 48);
            truth.resolved += 1;
            set_state(&mut audit, &mut truth, now, &mut state, "6");
            if rng.chance(0.08) {
                later(&mut rng, &mut now, 72);
                truth.reopened += 1;
                set_state(&mut audit, &mut truth, now, &mut state, "2");
                later(&mut rng, &mut now, 24);
                truth.resolved += 1;
                set_state(&mut audit, &mut truth, now, &mut state, "6");
            }
            if rng.chance(0.8) {
                // Closed automatically some days after resolution.
                now += Duration::days(3);
                truth.closed += 1;
                set_state(&mut audit, &mut truth, now, &mut state, "7");
            }
        } else {
            truth.open += 1;
        }
        let impact = rng
            .chance(0.4)
            .then(|| ["Low", "Medium", "High"][rng.below(3) as usize]);
        if impact.is_some() {
            truth.with_business_impact += 1;
        }
        let group_label = GROUPS
            .iter()
            .find(|(g, _)| *g == group)
            .map_or("", |(_, l)| *l);
        records.push(csv(&[
            &id,
            &format!("INC{:07}", 10_000 + n),
            &at(created),
            state,
            priority,
            category,
            group,
            person,
            impact.unwrap_or(""),
            label(&INCIDENT_STATES, state),
            label(&PRIORITIES, priority),
            group_label,
        ]));
    }
    write_files(
        dir,
        &[
            ("ServiceNowIncident", &records),
            ("ServiceNowAudit", &audit.rows),
        ],
    )?;
    Ok(truth)
}

/// What the generated change extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ChangeTruth {
    pub changes: u64,
    pub standard: u64,
    pub state_changes: u64,
    pub approval_changes: u64,
    pub approved: u64,
    pub rejected: u64,
    pub group_reassigned: u64,
    pub reschedules: u64,
    pub tasks: u64,
    pub task_state_changes: u64,
    pub closed: u64,
    pub left_out: u64,
}

const CHANGE_STATES: [(&str, &str); 8] = [
    ("-5", "New"),
    ("-4", "Assess"),
    ("-3", "Authorize"),
    ("-2", "Scheduled"),
    ("-1", "Implement"),
    ("0", "Review"),
    ("3", "Closed"),
    ("4", "Canceled"),
];
const APPROVALS: [(&str, &str); 4] = [
    ("not requested", "Not Yet Requested"),
    ("requested", "Requested"),
    ("approved", "Approved"),
    ("rejected", "Rejected"),
];
const TASK_STATES: [(&str, &str); 5] = [
    ("1", "Open"),
    ("2", "Work in Progress"),
    ("3", "Closed Complete"),
    ("4", "Closed Incomplete"),
    ("7", "Closed Skipped"),
];

/// A task record and its state changes, written alongside its parent.
fn run_task(
    rng: &mut Rng,
    audit: &mut Audit,
    table: &str,
    id: &str,
    now: &mut NaiveDateTime,
    reassign: bool,
) -> (u64, u64, &'static str, &'static str) {
    let mut changes = 0;
    let mut reassigned = 0;
    let mut group = GROUPS[1].0;
    later(rng, now, 4);
    audit.change(
        table,
        id,
        *now,
        ("state", "1", "Open", "2", "Work in Progress"),
    );
    changes += 1;
    if reassign {
        later(rng, now, 8);
        reassigned += 1;
        audit.change(
            table,
            id,
            *now,
            (
                "assignment_group",
                group,
                GROUPS[1].1,
                GROUPS[2].0,
                GROUPS[2].1,
            ),
        );
        group = GROUPS[2].0;
    }
    later(rng, now, 24);
    audit.change(
        table,
        id,
        *now,
        ("state", "2", "Work in Progress", "3", "Closed Complete"),
    );
    changes += 1;
    (changes, reassigned, "3", group)
}

/// Writes ServiceNowChangeRequest.csv, ServiceNowChangeTask.csv and
/// ServiceNowAudit.csv into `dir`.
pub fn write_changes(dir: &Path, settings: &Settings) -> io::Result<ChangeTruth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = ChangeTruth::default();
    let mut audit = Audit::new();
    let mut changes = vec!["sys_id,number,created,state,type,risk,approval,assignment_group,start_date,end_date,state_label,type_label,approval_label,assignment_group_label".to_owned()];
    let mut tasks = vec![
        "sys_id,number,created,change_request,state,assignment_group,state_label,assignment_group_label"
            .to_owned(),
    ];
    let mut task_number = 0u64;
    for n in 0..settings.records {
        truth.changes += 1;
        let id = sys_id(0x20, n);
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(rng.below(10 * 60) as i64);
        let mut now = created;
        let mut state = "-5";
        let mut approval = "not requested";
        let standard = rng.chance(0.3);
        let group = GROUPS[3].0;
        let mut group_now = group;
        let mut planned = created + Duration::days(7);
        let step = |audit: &mut Audit,
                    truth: &mut ChangeTruth,
                    now: NaiveDateTime,
                    state: &mut &'static str,
                    next: &'static str| {
            truth.state_changes += 1;
            audit.change(
                "change_request",
                &id,
                now,
                (
                    "state",
                    state,
                    label(&CHANGE_STATES, state),
                    next,
                    label(&CHANGE_STATES, next),
                ),
            );
            *state = next;
        };
        let approve = |audit: &mut Audit,
                       truth: &mut ChangeTruth,
                       now: NaiveDateTime,
                       approval: &mut &'static str,
                       next: &'static str| {
            truth.approval_changes += 1;
            audit.change(
                "change_request",
                &id,
                now,
                (
                    "approval",
                    approval,
                    label(&APPROVALS, approval),
                    next,
                    label(&APPROVALS, next),
                ),
            );
            *approval = next;
        };
        let mut rejected = false;
        if standard {
            truth.standard += 1;
            later(&mut rng, &mut now, 8);
            step(&mut audit, &mut truth, now, &mut state, "-2");
        } else {
            later(&mut rng, &mut now, 24);
            step(&mut audit, &mut truth, now, &mut state, "-4");
            later(&mut rng, &mut now, 24);
            step(&mut audit, &mut truth, now, &mut state, "-3");
            approve(&mut audit, &mut truth, now, &mut approval, "requested");
            later(&mut rng, &mut now, 48);
            if rng.chance(0.1) {
                truth.rejected += 1;
                approve(&mut audit, &mut truth, now, &mut approval, "rejected");
                step(&mut audit, &mut truth, now, &mut state, "4");
                rejected = true;
            } else {
                truth.approved += 1;
                approve(&mut audit, &mut truth, now, &mut approval, "approved");
                step(&mut audit, &mut truth, now, &mut state, "-2");
            }
        }
        if !rejected {
            if rng.chance(0.15) {
                later(&mut rng, &mut now, 12);
                truth.group_reassigned += 1;
                audit.change(
                    "change_request",
                    &id,
                    now,
                    (
                        "assignment_group",
                        group,
                        GROUPS[3].1,
                        GROUPS[1].0,
                        GROUPS[1].1,
                    ),
                );
                group_now = GROUPS[1].0;
            }
            if rng.chance(0.1) {
                later(&mut rng, &mut now, 12);
                truth.reschedules += 1;
                let from = at(planned).replace('T', " ").replace('Z', "");
                planned += Duration::days(3);
                let to = at(planned).replace('T', " ").replace('Z', "");
                audit.change(
                    "change_request",
                    &id,
                    now,
                    ("start_date", &from, &from, &to, &to),
                );
            }
            now = now.max(planned - Duration::hours(1));
            later(&mut rng, &mut now, 2);
            step(&mut audit, &mut truth, now, &mut state, "-1");
            let mut done = now;
            for _ in 0..1 + rng.below(3) {
                truth.tasks += 1;
                task_number += 1;
                let task = sys_id(0x21, task_number);
                let mut at_task = now + Duration::minutes(5);
                let task_created = at_task;
                let (moves, reassigned, task_state, task_group) = run_task(
                    &mut rng,
                    &mut audit,
                    "change_task",
                    &task,
                    &mut at_task,
                    rng_chance_free(n, task_number),
                );
                truth.task_state_changes += moves;
                truth.group_reassigned += reassigned;
                done = done.max(at_task);
                tasks.push(csv(&[
                    &task,
                    &format!("CTASK{:07}", 10_000 + task_number),
                    &at(task_created),
                    &id,
                    task_state,
                    task_group,
                    label(&TASK_STATES, task_state),
                    GROUPS.iter().find(|(g, _)| *g == task_group).unwrap().1,
                ]));
            }
            now = done;
            later(&mut rng, &mut now, 4);
            step(&mut audit, &mut truth, now, &mut state, "0");
            if rng.chance(0.9) {
                later(&mut rng, &mut now, 24);
                truth.closed += 1;
                step(&mut audit, &mut truth, now, &mut state, "3");
            }
        }
        if rng.chance(0.5) {
            later(&mut rng, &mut now, 4);
            truth.left_out += 1;
            audit.change(
                "change_request",
                &id,
                now,
                ("work_notes", "", "", "", "Implemented as planned"),
            );
        }
        changes.push(csv(&[
            &id,
            &format!("CHG{:07}", 30_000 + n),
            &at(created),
            state,
            if standard { "standard" } else { "normal" },
            "3",
            approval,
            group_now,
            &at(planned),
            &at(planned + Duration::hours(4)),
            label(&CHANGE_STATES, state),
            if standard { "Standard" } else { "Normal" },
            label(&APPROVALS, approval),
            GROUPS.iter().find(|(g, _)| *g == group_now).unwrap().1,
        ]));
    }
    write_files(
        dir,
        &[
            ("ServiceNowChangeRequest", &changes),
            ("ServiceNowChangeTask", &tasks),
            ("ServiceNowAudit", &audit.rows),
        ],
    )?;
    Ok(truth)
}

/// Every seventh task is passed to another group, independent of the random
/// stream so the parent's draws stay the same.
fn rng_chance_free(_parent: u64, task: u64) -> bool {
    task.is_multiple_of(7)
}

/// What the generated catalog request extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RequestTruth {
    pub items: u64,
    pub state_changes: u64,
    pub stage_changes: u64,
    pub approval_changes: u64,
    pub rejected: u64,
    pub tasks: u64,
    pub task_state_changes: u64,
    pub task_reassignments: u64,
    pub completed: u64,
}

const ITEM_STATES: [(&str, &str); 4] = [
    ("1", "Open"),
    ("2", "Work in Progress"),
    ("3", "Closed Complete"),
    ("4", "Closed Incomplete"),
];
const STAGES: [(&str, &str); 4] = [
    ("waiting_for_approval", "Waiting for Approval"),
    ("fulfillment", "Fulfillment"),
    ("complete", "Completed"),
    ("Request Cancelled", "Request Cancelled"),
];
const ITEMS: [(&str, &str); 3] = [
    ("c1b2c3d4e5f60718293a4b5c6d7e8f01", "Standard Laptop"),
    ("c1b2c3d4e5f60718293a4b5c6d7e8f02", "Software Access"),
    ("c1b2c3d4e5f60718293a4b5c6d7e8f03", "New Mobile Phone"),
];

/// Writes ServiceNowRequestItem.csv, ServiceNowCatalogTask.csv and
/// ServiceNowAudit.csv into `dir`.
pub fn write_requests(dir: &Path, settings: &Settings) -> io::Result<RequestTruth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = RequestTruth::default();
    let mut audit = Audit::new();
    let mut items = vec!["sys_id,number,created,request,cat_item,state,stage,approval,state_label,stage_label,approval_label,cat_item_label".to_owned()];
    let mut tasks = vec![
        "sys_id,number,created,request_item,state,assignment_group,state_label,assignment_group_label"
            .to_owned(),
    ];
    let mut task_number = 0u64;
    for n in 0..settings.records {
        truth.items += 1;
        let id = sys_id(0x30, n);
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(rng.below(10 * 60) as i64);
        let mut now = created;
        let (item, item_label) = ITEMS[rng.below(3) as usize];
        let mut state = "1";
        let mut stage = "waiting_for_approval";
        let mut approval = "requested";
        let set = |audit: &mut Audit,
                   truth: &mut RequestTruth,
                   now: NaiveDateTime,
                   field: &str,
                   list: &[(&'static str, &'static str)],
                   old: &mut &'static str,
                   new: &'static str| {
            match field {
                "state" => truth.state_changes += 1,
                "stage" => truth.stage_changes += 1,
                _ => truth.approval_changes += 1,
            }
            audit.change(
                "sc_req_item",
                &id,
                now,
                (field, old, label(list, old), new, label(list, new)),
            );
            *old = new;
        };
        later(&mut rng, &mut now, 24);
        if rng.chance(0.08) {
            truth.rejected += 1;
            set(
                &mut audit,
                &mut truth,
                now,
                "approval",
                &APPROVALS,
                &mut approval,
                "rejected",
            );
            set(
                &mut audit,
                &mut truth,
                now,
                "stage",
                &STAGES,
                &mut stage,
                "Request Cancelled",
            );
            set(
                &mut audit,
                &mut truth,
                now,
                "state",
                &ITEM_STATES,
                &mut state,
                "4",
            );
        } else {
            set(
                &mut audit,
                &mut truth,
                now,
                "approval",
                &APPROVALS,
                &mut approval,
                "approved",
            );
            set(
                &mut audit,
                &mut truth,
                now,
                "stage",
                &STAGES,
                &mut stage,
                "fulfillment",
            );
            set(
                &mut audit,
                &mut truth,
                now,
                "state",
                &ITEM_STATES,
                &mut state,
                "2",
            );
            let finished = rng.chance(0.9);
            let mut done = now;
            for _ in 0..1 + rng.below(2) {
                truth.tasks += 1;
                task_number += 1;
                let task = sys_id(0x31, task_number);
                let mut at_task = now + Duration::minutes(1);
                let task_created = at_task;
                let (task_state, task_group) = if finished {
                    let (moves, reassigned, task_state, task_group) = run_task(
                        &mut rng,
                        &mut audit,
                        "sc_task",
                        &task,
                        &mut at_task,
                        rng_chance_free(n, task_number),
                    );
                    truth.task_state_changes += moves;
                    truth.task_reassignments += reassigned;
                    (task_state, task_group)
                } else {
                    ("1", GROUPS[1].0)
                };
                done = done.max(at_task);
                tasks.push(csv(&[
                    &task,
                    &format!("SCTASK{:07}", 10_000 + task_number),
                    &at(task_created),
                    &id,
                    task_state,
                    task_group,
                    label(&TASK_STATES, task_state),
                    GROUPS.iter().find(|(g, _)| *g == task_group).unwrap().1,
                ]));
            }
            if finished {
                now = done + Duration::minutes(1);
                truth.completed += 1;
                set(
                    &mut audit, &mut truth, now, "stage", &STAGES, &mut stage, "complete",
                );
                set(
                    &mut audit,
                    &mut truth,
                    now,
                    "state",
                    &ITEM_STATES,
                    &mut state,
                    "3",
                );
            }
        }
        items.push(csv(&[
            &id,
            &format!("RITM{:07}", 10_000 + n),
            &at(created),
            &sys_id(0x32, n),
            item,
            state,
            stage,
            approval,
            label(&ITEM_STATES, state),
            label(&STAGES, stage),
            label(&APPROVALS, approval),
            item_label,
        ]));
    }
    write_files(
        dir,
        &[
            ("ServiceNowRequestItem", &items),
            ("ServiceNowCatalogTask", &tasks),
            ("ServiceNowAudit", &audit.rows),
        ],
    )?;
    Ok(truth)
}

/// Which task-based table the made-up tickets come from: the generic ticket
/// files of an HR case table, or ServiceNow's own problem tables.
#[derive(Debug, Clone, Copy)]
pub struct TicketKind {
    pub table: &'static str,
    pub task_table: &'static str,
    pub file: &'static str,
    pub task_file: &'static str,
    /// The task file's column naming its ticket.
    pub parent: &'static str,
    pub prefix: &'static str,
    pub states: &'static [(&'static str, &'static str)],
}

pub const HR_CASES: TicketKind = TicketKind {
    table: "sn_hr_core_case",
    task_table: "sn_hr_core_task",
    file: "ServiceNowTicket",
    task_file: "ServiceNowTicketTask",
    parent: "ticket",
    prefix: "HRC",
    states: &[
        ("10", "Ready"),
        ("18", "Awaiting Acceptance"),
        ("20", "Awaiting Approval"),
        ("24", "Suspended"),
        ("3", "Closed Complete"),
        ("7", "Cancelled"),
    ],
};

pub const PROBLEMS: TicketKind = TicketKind {
    table: "problem",
    task_table: "problem_task",
    file: "ServiceNowProblem",
    task_file: "ServiceNowProblemTask",
    parent: "problem",
    prefix: "PRB",
    states: &[
        ("101", "New"),
        ("102", "Assess"),
        ("103", "Root Cause Analysis"),
        ("104", "Fix in Progress"),
        ("106", "Resolved"),
        ("107", "Closed"),
    ],
};

/// What the generated ticket extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TicketTruth {
    pub tickets: u64,
    pub state_changes: u64,
    pub group_assigned: u64,
    pub group_reassigned: u64,
    pub person_assigned: u64,
    pub person_reassigned: u64,
    pub priority_changes: u64,
    pub other_changes: u64,
    pub tasks: u64,
    pub task_state_changes: u64,
    pub task_reassigned: u64,
    /// Work notes, which the packs leave out.
    pub left_out: u64,
}

/// Writes the ticket file, its task file and the audit file into `dir`:
/// each ticket moves through the kind's states in order, some skip back a
/// state, some are reassigned, and some have tasks.
pub fn write_tickets(dir: &Path, settings: &Settings, kind: TicketKind) -> io::Result<TicketTruth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = TicketTruth::default();
    let mut audit = Audit::new();
    let mut tickets = vec!["sys_id,number,created,state,priority,assignment_group,assigned_to,state_label,priority_label,assignment_group_label".to_owned()];
    let mut tasks = vec![format!(
        "sys_id,number,created,{},state,assignment_group,state_label,assignment_group_label",
        kind.parent
    )];
    let last = kind.states.len() - 2;
    for n in 0..settings.records {
        truth.tickets += 1;
        let id = sys_id(0x30, n);
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(rng.below(10 * 60) as i64);
        let mut now = created;
        let mut state = 0usize;
        let mut priority = "3";
        let set = |audit: &mut Audit, truth: &mut TicketTruth, now, from: usize, to: usize| {
            truth.state_changes += 1;
            audit.change(
                kind.table,
                &id,
                now,
                (
                    "state",
                    kind.states[from].0,
                    kind.states[from].1,
                    kind.states[to].0,
                    kind.states[to].1,
                ),
            );
        };
        later(&mut rng, &mut now, 2);
        truth.group_assigned += 1;
        let mut group = GROUPS[rng.below(4) as usize];
        audit.change(
            kind.table,
            &id,
            now,
            ("assignment_group", "", "", group.0, group.1),
        );
        later(&mut rng, &mut now, 4);
        truth.person_assigned += 1;
        let mut person = PEOPLE[rng.below(4) as usize];
        audit.change(
            kind.table,
            &id,
            now,
            ("assigned_to", "", "", person, person),
        );
        let cancelled = rng.chance(0.1);
        let goal = if cancelled { 1 } else { last };
        while state < goal {
            later(&mut rng, &mut now, 24);
            set(&mut audit, &mut truth, now, state, state + 1);
            state += 1;
            // Sent back a step now and then.
            if state > 1 && state < goal && rng.chance(0.1) {
                later(&mut rng, &mut now, 12);
                set(&mut audit, &mut truth, now, state, state - 1);
                later(&mut rng, &mut now, 12);
                set(&mut audit, &mut truth, now, state - 1, state);
            }
            if rng.chance(0.1) {
                later(&mut rng, &mut now, 2);
                truth.left_out += 1;
                audit.change(
                    kind.table,
                    &id,
                    now,
                    ("work_notes", "", "", "", "Called the requester"),
                );
            }
        }
        if cancelled {
            later(&mut rng, &mut now, 4);
            set(&mut audit, &mut truth, now, state, kind.states.len() - 1);
            state = kind.states.len() - 1;
        }
        if rng.chance(0.2) {
            later(&mut rng, &mut now, 6);
            truth.group_reassigned += 1;
            let to = GROUPS[(GROUPS.iter().position(|g| *g == group).unwrap() + 1) % 4];
            audit.change(
                kind.table,
                &id,
                now,
                ("assignment_group", group.0, group.1, to.0, to.1),
            );
            group = to;
        }
        if rng.chance(0.1) {
            later(&mut rng, &mut now, 6);
            truth.person_reassigned += 1;
            let to = PEOPLE[(PEOPLE.iter().position(|p| *p == person).unwrap() + 1) % 4];
            audit.change(
                kind.table,
                &id,
                now,
                ("assigned_to", person, person, to, to),
            );
            person = to;
        }
        if rng.chance(0.1) {
            later(&mut rng, &mut now, 6);
            truth.priority_changes += 1;
            audit.change(
                kind.table,
                &id,
                now,
                (
                    "priority",
                    priority,
                    label(&PRIORITIES, priority),
                    "2",
                    "2 - High",
                ),
            );
            priority = "2";
        }
        if rng.chance(0.05) {
            later(&mut rng, &mut now, 6);
            truth.other_changes += 1;
            audit.change(
                kind.table,
                &id,
                now,
                ("short_description", "", "", "", "Updated"),
            );
        }
        if rng.chance(0.4) {
            truth.tasks += 1;
            let task = sys_id(0x31, n);
            let mut at_task = created + Duration::hours(1);
            later(&mut rng, &mut at_task, 4);
            let mut task_group = GROUPS[rng.below(4) as usize];
            later(&mut rng, &mut at_task, 8);
            truth.task_state_changes += 1;
            audit.change(
                kind.task_table,
                &task,
                at_task,
                ("state", "1", "Open", "2", "Work in Progress"),
            );
            if rng.chance(0.25) {
                later(&mut rng, &mut at_task, 4);
                truth.task_reassigned += 1;
                let to = GROUPS[(GROUPS.iter().position(|g| *g == task_group).unwrap() + 1) % 4];
                audit.change(
                    kind.task_table,
                    &task,
                    at_task,
                    ("assignment_group", task_group.0, task_group.1, to.0, to.1),
                );
                task_group = to;
            }
            later(&mut rng, &mut at_task, 8);
            truth.task_state_changes += 1;
            audit.change(
                kind.task_table,
                &task,
                at_task,
                ("state", "2", "Work in Progress", "3", "Closed Complete"),
            );
            tasks.push(csv(&[
                &task,
                &format!("TASK{:07}", 20_000 + n),
                &at(created + Duration::hours(1)),
                &id,
                "3",
                task_group.0,
                label(&TASK_STATES, "3"),
                task_group.1,
            ]));
        }
        tickets.push(csv(&[
            &id,
            &format!("{}{:07}", kind.prefix, 10_000 + n),
            &at(created),
            kind.states[state].0,
            priority,
            group.0,
            person,
            kind.states[state].1,
            label(&PRIORITIES, priority),
            group.1,
        ]));
    }
    write_files(
        dir,
        &[
            (kind.file, &tickets),
            (kind.task_file, &tasks),
            ("ServiceNowAudit", &audit.rows),
        ],
    )?;
    Ok(truth)
}
