//! Made-up SAP SuccessFactors Recruiting extracts: requisitions, applications,
//! their status history and interviews, one CSV each, times in UTC
//! (`2026-01-05T14:30:00Z`). Every record is invented; the counts in the
//! truth struct are known by construction.

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
            records: 40,
            seed: 13,
        }
    }
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// What the recruiting extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RecruitingTruth {
    pub requisitions: u64,
    pub closed_requisitions: u64,
    pub applications: u64,
    pub status_changes: u64,
    pub interviews: u64,
    pub hired: u64,
    pub onboarding_started: u64,
    pub onboarding_completed: u64,
}

pub const RECRUITING_FILES: [&str; 5] = [
    "JobRequisition",
    "JobApplication",
    "JobApplicationStatusAuditTrail",
    "JobApplicationInterview",
    "ONB2Process",
];

/// Writes the four Recruiting files and the onboarding processes of the
/// hires; `records` is the number of requisitions.
pub fn write_recruiting(dir: &Path, settings: &Settings) -> io::Result<RecruitingTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x51F0);
    let mut truth = RecruitingTruth::default();
    let mut requisitions = Table::new(
        "jobReqId,createdDateTime,closedDateTime,department,location,numberOpenings,internalStatus,cust_band",
    );
    let mut applications =
        Table::new("applicationId,jobReqId,candidateId,applicationDate,source,appStatusName");
    let mut history = Table::new("revNumber,applicationId,statusName,createdDateTime,createdBy");
    let mut interviews =
        Table::new("applicationInterviewId,applicationId,startDate,endDate,status");
    let mut onboarding = Table::new(
        "processId,jobApplicationId,createdDateTime,onboardingHireStatus,completedDateTime",
    );
    let start = NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(9, 0, 0)
        .unwrap();
    let mut application_number = 0u64;
    let mut revision = 0u64;
    let mut interview_number = 0u64;
    for n in 0..settings.records {
        truth.requisitions += 1;
        let requisition = (1000 + n).to_string();
        let opened =
            start + Duration::days(rng.below(60) as i64) + Duration::hours(rng.below(8) as i64);
        let mut filled = false;
        let mut last = opened;
        for _ in 0..3 + rng.below(12) {
            application_number += 1;
            truth.applications += 1;
            let application = (50_000 + application_number).to_string();
            let mut at = opened + Duration::hours(6 + rng.below(30 * 24) as i64);
            let applied = at;
            let mut status = "Default";
            let mut change = |history: &mut Table,
                              truth: &mut RecruitingTruth,
                              name: &'static str,
                              at: NaiveDateTime,
                              status: &mut &str| {
                revision += 1;
                truth.status_changes += 1;
                history.push(&[
                    &revision.to_string(),
                    &application,
                    name,
                    &utc(at),
                    "recruiter1",
                ]);
                *status = name;
            };
            at += Duration::hours(12 + rng.below(96) as i64);
            if filled || rng.chance(0.45) {
                change(&mut history, &mut truth, "Rejected", at, &mut status);
            } else {
                change(&mut history, &mut truth, "Phone Screen", at, &mut status);
                at += Duration::hours(24 + rng.below(120) as i64);
                if rng.chance(0.5) {
                    change(&mut history, &mut truth, "Rejected", at, &mut status);
                } else {
                    change(&mut history, &mut truth, "Interview", at, &mut status);
                    for _ in 0..1 + rng.below(2) {
                        at += Duration::hours(24 + rng.below(96) as i64);
                        interview_number += 1;
                        truth.interviews += 1;
                        interviews.push(&[
                            &(900 + interview_number).to_string(),
                            &application,
                            &utc(at),
                            &utc(at + Duration::hours(1)),
                            "Completed",
                        ]);
                    }
                    at += Duration::hours(24 + rng.below(72) as i64);
                    if rng.chance(0.5) {
                        change(&mut history, &mut truth, "Offer", at, &mut status);
                        at += Duration::hours(24 + rng.below(120) as i64);
                        if rng.chance(0.8) {
                            change(&mut history, &mut truth, "Hired", at, &mut status);
                            truth.hired += 1;
                            truth.onboarding_started += 1;
                            let started = at + Duration::hours(2);
                            let completed = rng.chance(0.7);
                            truth.onboarding_completed += u64::from(completed);
                            onboarding.push(&[
                                &format!("ONB{application}"),
                                &application,
                                &utc(started),
                                if completed { "Hired" } else { "Onboarding" },
                                &if completed {
                                    utc(started + Duration::days(10 + rng.below(20) as i64))
                                } else {
                                    String::new()
                                },
                            ]);
                            filled = true;
                            last = last.max(at);
                        } else {
                            change(&mut history, &mut truth, "Offer Declined", at, &mut status);
                        }
                    } else {
                        change(&mut history, &mut truth, "Rejected", at, &mut status);
                    }
                }
            }
            applications.push(&[
                &application,
                &requisition,
                &(70_000 + application_number).to_string(),
                &utc(applied),
                ["Career Site", "Referral", "Agency"][rng.below(3) as usize],
                status,
            ]);
        }
        let closed = if filled {
            truth.closed_requisitions += 1;
            utc(last + Duration::days(1))
        } else {
            String::new()
        };
        requisitions.push(&[
            &requisition,
            &utc(opened),
            &closed,
            ["Engineering", "Sales", "Finance"][n as usize % 3],
            ["Berlin", "Austin"][n as usize % 2],
            "1",
            if filled { "Closed" } else { "Open" },
            ["B1", "B2", ""][n as usize % 3],
        ]);
    }
    save(
        dir,
        &[
            ("JobRequisition", &requisitions),
            ("JobApplication", &applications),
            ("JobApplicationStatusAuditTrail", &history),
            ("JobApplicationInterview", &interviews),
            ("ONB2Process", &onboarding),
        ],
    )?;
    Ok(truth)
}
