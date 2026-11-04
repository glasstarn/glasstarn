//! Made-up Jira extracts in the shape the Jira extract writes them: one row
//! per issue with the selected fields, and one row per changed field in each
//! change-history entry, with times in RFC 3339 and people as account IDs.
//! Field names follow public knowledge of Jira Cloud's REST API; every record
//! is invented, and the counts in [`Truth`] are known by construction.

use crate::Rng;
use chrono::{Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

/// What the generated extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Truth {
    pub issues: u64,
    pub bugs: u64,
    pub status_changes: u64,
    /// Returns from review to work.
    pub review_returns: u64,
    pub reopened: u64,
    pub resolved: u64,
    pub resolutions_cleared: u64,
    pub reassignments: u64,
    pub priority_changes: u64,
    pub story_point_changes: u64,
    pub open_issues: u64,
    /// Issues with the company's own field "Customer impact" filled.
    pub with_customer_impact: u64,
}

#[derive(Debug, Clone)]
pub struct Settings {
    pub issues: u64,
    pub seed: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            issues: 300,
            seed: 9,
        }
    }
}

pub const FILES: [&str; 2] = ["JiraIssue", "JiraIssueHistory"];

fn at(time: NaiveDateTime) -> String {
    // The site's time zone in the made-up data is UTC+1.
    format!("{}+01:00", time.format("%Y-%m-%dT%H:%M:%S%.3f"))
}

/// One issue's change history being written.
struct History<'a> {
    rows: &'a mut Vec<String>,
    next_id: &'a mut u64,
    key: &'a str,
}

impl History<'_> {
    /// One change entry, with a row per changed field.
    fn push(&mut self, now: NaiveDateTime, items: &[(&str, &str, &str, &str, &str, &str)]) {
        *self.next_id += 1;
        for (item, (field, field_id, from, from_string, to, to_string)) in items.iter().enumerate()
        {
            self.rows.push(csv(&[
                self.key,
                &self.next_id.to_string(),
                &item.to_string(),
                &at(now),
                "acc-ana",
                field,
                field_id,
                from,
                from_string,
                to,
                to_string,
            ]));
        }
    }

    /// A status change some time after `now`.
    fn step(
        &mut self,
        rng: &mut Rng,
        truth: &mut Truth,
        now: &mut NaiveDateTime,
        status: &mut &'static str,
        next: &'static str,
        hours: u64,
    ) {
        *now += Duration::minutes(30 + rng.below(hours * 60) as i64);
        truth.status_changes += 1;
        self.push(*now, &[("status", "status", "1", status, "3", next)]);
        *status = next;
    }
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

/// Writes JiraIssue.csv and JiraIssueHistory.csv into `dir`.
pub fn write(dir: &Path, settings: &Settings) -> io::Result<Truth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = Truth::default();
    let mut issues = vec![
        "key,id,project,issuetype,status,priority,resolution,created,updated,resolutiondate,assignee,customfield_10016,customfield_10200".to_owned(),
    ];
    let mut history = vec![
        "issue_key,history_id,item,created,author,field,field_id,from,from_string,to,to_string"
            .to_owned(),
    ];
    let start = NaiveDate::from_ymd_opt(2025, 11, 1)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    let people = ["acc-ana", "acc-ben", "acc-chen", "acc-dora"];
    let mut history_id = 10_000u64;
    for n in 0..settings.issues {
        truth.issues += 1;
        let key = format!("OPS-{}", 100 + n);
        let bug = rng.chance(0.4);
        if bug {
            truth.bugs += 1;
        }
        let created = start
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(rng.below(8 * 60) as i64);
        let mut now = created;
        let mut status = "To Do";
        let mut priority = if rng.chance(0.2) { "High" } else { "Medium" };
        let mut assignee = people[rng.below(people.len() as u64) as usize];
        let mut points = 3 + rng.below(5);
        let mut resolution = "";
        let mut resolved_at = String::new();
        let mut log = History {
            rows: &mut history,
            next_id: &mut history_id,
            key: &key,
        };
        let finished = rng.chance(0.85);
        log.step(
            &mut rng,
            &mut truth,
            &mut now,
            &mut status,
            "In Progress",
            48,
        );
        if rng.chance(0.3) {
            now += Duration::hours(2 + rng.below(24) as i64);
            truth.reassignments += 1;
            let next =
                people[(people.iter().position(|p| *p == assignee).unwrap() + 1) % people.len()];
            log.push(
                now,
                &[("assignee", "assignee", assignee, assignee, next, next)],
            );
            assignee = next;
        }
        if rng.chance(0.1) {
            now += Duration::hours(1 + rng.below(12) as i64);
            truth.priority_changes += 1;
            log.push(now, &[("priority", "priority", "3", priority, "2", "High")]);
            priority = "High";
        }
        if rng.chance(0.1) {
            now += Duration::hours(1 + rng.below(12) as i64);
            truth.story_point_changes += 1;
            let from = points.to_string();
            points += 2;
            log.push(
                now,
                &[(
                    "Story Points",
                    "customfield_10016",
                    "",
                    &from,
                    "",
                    &points.to_string(),
                )],
            );
        }
        if finished {
            log.step(&mut rng, &mut truth, &mut now, &mut status, "In Review", 72);
            if rng.chance(0.2) {
                truth.review_returns += 1;
                log.step(
                    &mut rng,
                    &mut truth,
                    &mut now,
                    &mut status,
                    "In Progress",
                    24,
                );
                log.step(&mut rng, &mut truth, &mut now, &mut status, "In Review", 48);
            }
            // Done and the resolution are set in one change entry.
            now += Duration::minutes(30 + rng.below(24 * 60) as i64);
            truth.status_changes += 1;
            truth.resolved += 1;
            resolution = if bug { "Fixed" } else { "Done" };
            log.push(
                now,
                &[
                    ("status", "status", "3", status, "10001", "Done"),
                    ("resolution", "resolution", "", "", "10000", resolution),
                ],
            );
            status = "Done";
            resolved_at = at(now);
            if bug && rng.chance(0.1) {
                truth.reopened += 1;
                truth.resolutions_cleared += 1;
                now += Duration::hours(24 + rng.below(240) as i64);
                truth.status_changes += 1;
                log.push(
                    now,
                    &[
                        ("status", "status", "10001", "Done", "4", "Reopened"),
                        ("resolution", "resolution", "10000", resolution, "", ""),
                    ],
                );
                status = "Reopened";
                resolved_at.clear();
                log.step(
                    &mut rng,
                    &mut truth,
                    &mut now,
                    &mut status,
                    "In Progress",
                    24,
                );
                now += Duration::minutes(30 + rng.below(24 * 60) as i64);
                truth.status_changes += 1;
                truth.resolved += 1;
                resolution = "Fixed";
                log.push(
                    now,
                    &[
                        ("status", "status", "3", status, "10001", "Done"),
                        ("resolution", "resolution", "", "", "10000", resolution),
                    ],
                );
                status = "Done";
                resolved_at = at(now);
            }
        } else {
            truth.open_issues += 1;
        }
        let impact =
            (bug && rng.chance(0.5)).then(|| ["Low", "Medium", "High"][rng.below(3) as usize]);
        if impact.is_some() {
            truth.with_customer_impact += 1;
        }
        issues.push(csv(&[
            &key,
            &(10_000 + n).to_string(),
            "OPS",
            if bug { "Bug" } else { "Story" },
            status,
            priority,
            resolution,
            &at(created),
            &at(now),
            &resolved_at,
            assignee,
            &points.to_string(),
            impact.unwrap_or(""),
        ]));
    }
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join("JiraIssue.csv"), issues.join("\n") + "\n")?;
    std::fs::write(dir.join("JiraIssueHistory.csv"), history.join("\n") + "\n")?;
    Ok(truth)
}
