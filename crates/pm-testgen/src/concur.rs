//! Made-up SAP Concur Expense extracts: reports, their entries and their
//! approval workflow actions, one CSV each, times in UTC
//! (`2026-01-05T14:30:00Z`) and transaction dates as dates. Every record is
//! invented; the counts in the truth struct are known by construction.

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
            seed: 11,
        }
    }
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// What the expense extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ExpenseTruth {
    pub reports: u64,
    pub entries: u64,
    /// Workflow actions of every kind.
    pub actions: u64,
    pub submitted: u64,
    pub sent_back: u64,
    pub manager_approvals: u64,
    pub processor_approvals: u64,
    pub paid: u64,
}

pub const EXPENSE_FILES: [&str; 3] = ["Report", "Entry", "WorkflowAction"];

/// Writes Report, Entry and WorkflowAction.
pub fn write_expenses(dir: &Path, settings: &Settings) -> io::Result<ExpenseTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xC0C0);
    let mut truth = ExpenseTruth::default();
    let mut reports = Table::new(
        "ReportId,CreateDate,SubmitDate,ApprovalStatusCode,PaymentStatusCode,Total,CurrencyCode,PaidDate,PolicyName,OwnerId,Custom1",
    );
    let mut entries = Table::new(
        "EntryId,ReportId,ExpenseTypeName,TransactionDate,TransactionAmount,TransactionCurrencyCode,HasExceptions,IsPersonal",
    );
    let mut actions = Table::new("ActionId,ReportId,Action,ActionDate,ActorRole,ActorId");
    let types = [
        ("Hotel", 180),
        ("Airfare", 420),
        ("Meals", 45),
        ("Taxi", 30),
    ];
    let mut entry_number = 0u64;
    let mut action_number = 0u64;
    let start = NaiveDate::from_ymd_opt(2026, 1, 2)
        .unwrap()
        .and_hms_opt(8, 0, 0)
        .unwrap();
    for n in 0..settings.records {
        truth.reports += 1;
        let report = format!("RPT{n:07}");
        let trip = start + Duration::days(rng.below(80) as i64);
        let mut total = 0u64;
        for _ in 0..1 + rng.below(5) {
            entry_number += 1;
            truth.entries += 1;
            let (kind, amount) = types[rng.below(4) as usize];
            let amount = amount + rng.below(50);
            total += amount;
            let spent = trip + Duration::days(rng.below(4) as i64);
            entries.push(&[
                &format!("ENT{entry_number:08}"),
                &report,
                kind,
                &spent.date().to_string(),
                &format!("{amount}.00"),
                "EUR",
                if amount > 400 { "Y" } else { "N" },
                "N",
            ]);
        }
        let created = trip + Duration::days(4 + rng.below(10) as i64) + Duration::hours(1);
        let mut at = created;
        let mut action = |actions: &mut Table, name: &str, role: &str, at: NaiveDateTime| {
            action_number += 1;
            actions.push(&[
                &format!("ACT{action_number:08}"),
                &report,
                name,
                &utc(at),
                role,
                "USER0001",
            ]);
        };
        let submitted = rng.chance(0.9);
        let mut paid = String::new();
        let (approval, payment) = if submitted {
            at += Duration::hours(2 + rng.below(72) as i64);
            truth.submitted += 1;
            action(&mut actions, "Submit report", "Employee", at);
            if rng.chance(0.12) {
                at += Duration::hours(4 + rng.below(48) as i64);
                truth.sent_back += 1;
                action(&mut actions, "Send back to employee", "Manager", at);
                at += Duration::hours(2 + rng.below(48) as i64);
                truth.submitted += 1;
                action(&mut actions, "Submit report", "Employee", at);
            }
            if rng.chance(0.9) {
                at += Duration::hours(4 + rng.below(96) as i64);
                truth.manager_approvals += 1;
                action(&mut actions, "Approve (manager)", "Manager", at);
                at += Duration::hours(4 + rng.below(72) as i64);
                truth.processor_approvals += 1;
                action(&mut actions, "Approve (expense processor)", "Processor", at);
                if rng.chance(0.85) {
                    at += Duration::days(3 + rng.below(10) as i64);
                    truth.paid += 1;
                    paid = utc(at);
                    ("A_APPR", "P_PAID")
                } else {
                    ("A_APPR", "P_PROC")
                }
            } else {
                ("A_PEND", "P_NOTP")
            }
        } else {
            ("A_NOTF", "P_NOTP")
        };
        reports.push(&[
            &report,
            &utc(created),
            &if submitted {
                utc(created + Duration::hours(1))
            } else {
                String::new()
            },
            approval,
            payment,
            &format!("{total}.00"),
            "EUR",
            &paid,
            "Travel policy",
            "USER0001",
            ["Sales", "Service", ""][n as usize % 3],
        ]);
    }
    truth.actions =
        truth.submitted + truth.sent_back + truth.manager_approvals + truth.processor_approvals;
    save(
        dir,
        &[
            ("Report", &reports),
            ("Entry", &entries),
            ("WorkflowAction", &actions),
        ],
    )?;
    Ok(truth)
}
