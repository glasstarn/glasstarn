//! What kind of process a log records, and what the business wants from it.
//!
//! A first read from the step names, so a page can say "this looks like
//! order-to-cash; the goal is getting paid sooner" before anyone asks. It is
//! a suggestion: the agent's read, saved with its answer, takes precedence,
//! and a log that matches no type clearly gets none.

use crate::findings::FindingKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessType {
    OrderToCash,
    PurchaseToPay,
    SalesPipeline,
    ItTickets,
    Claims,
    Onboarding,
    Permits,
    Manufacturing,
    Logistics,
}

/// How a page speaks about one kind of process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessProfile {
    pub kind: ProcessType,
    /// "Order-to-cash".
    pub label: String,
    /// What the business wants: "Getting paid sooner".
    pub goal: String,
    /// The name of the end-to-end time: "Time to cash".
    pub time_label: String,
    /// The findings that matter most for this goal, first first.
    pub emphasis: Vec<FindingKind>,
    /// "engine" for a guess from step names, "agent" for the agent's read.
    pub source: String,
}

impl ProcessType {
    const ALL: [ProcessType; 9] = [
        ProcessType::OrderToCash,
        ProcessType::PurchaseToPay,
        ProcessType::SalesPipeline,
        ProcessType::ItTickets,
        ProcessType::Claims,
        ProcessType::Onboarding,
        ProcessType::Permits,
        ProcessType::Manufacturing,
        ProcessType::Logistics,
    ];

    fn words(self) -> &'static [&'static str] {
        match self {
            ProcessType::OrderToCash => &[
                "sales order",
                "invoice issued",
                "invoice sent",
                "send invoice",
                "issue invoice",
                "generate invoice",
                "payment received",
                "receive payment",
                "payment recorded",
                "dunning",
                "payment reminder",
                "credit note",
                "dispute",
                "ship",
                "deliver",
            ],
            ProcessType::PurchaseToPay => &[
                "requisition",
                "purchase order",
                "goods receipt",
                "record goods",
                "vendor",
                "supplier",
                "invoice received",
                "receive invoice",
                "approve invoice",
                "clear invoice",
                "three-way",
                "3-way",
                "service entry",
            ],
            ProcessType::SalesPipeline => &[
                "lead",
                "opportunity",
                "deal",
                "qualification",
                "qualified",
                "proposal",
                "quote",
                "negotiation",
                "closed won",
                "closed lost",
                "demo",
            ],
            ProcessType::ItTickets => &[
                "ticket",
                "incident",
                "assigned",
                "reassign",
                "resolved",
                "resolve",
                "reopen",
                "escalat",
                "triage",
                "service request",
                "close ticket",
            ],
            ProcessType::Claims => &[
                "claim",
                "assess",
                "adjust",
                "settlement",
                "documents requested",
                "fraud",
                "payout",
                "coverage",
            ],
            ProcessType::Onboarding => &[
                "offer",
                "contract signed",
                "equipment",
                "access granted",
                "onboard",
                "start date",
                "hire",
                "background check",
                "orientation",
            ],
            ProcessType::Permits => &[
                "application",
                "permit",
                "completeness",
                "licen",
                "inspection scheduled",
                "decision",
                "approval granted",
                "public notice",
            ],
            ProcessType::Manufacturing => &[
                "production order",
                "set-up",
                "setup",
                "machine",
                "inspection",
                "scrap",
                "repair",
                "maintenance",
                "work order",
                "assembly",
            ],
            ProcessType::Logistics => &[
                "pick",
                "pack",
                "load",
                "transport",
                "dispatch",
                "delivered",
                "return",
                "shipment",
                "container",
                "customs",
            ],
        }
    }

    pub fn profile(self, source: &str) -> ProcessProfile {
        use FindingKind::*;
        let (label, goal, time_label, emphasis): (&str, &str, &str, Vec<FindingKind>) = match self {
            ProcessType::OrderToCash => (
                "Order-to-cash",
                "Getting paid sooner",
                "Time to cash",
                vec![DropOut, Open, SlowTail, Bottleneck, Rework],
            ),
            ProcessType::PurchaseToPay => (
                "Purchase-to-pay",
                "Paying on time, without surprises",
                "Time to pay",
                vec![Bottleneck, Rework, SlowTail, Variation, DropOut],
            ),
            ProcessType::SalesPipeline => (
                "Sales pipeline",
                "Winning more, faster",
                "Deal cycle time",
                vec![DropOut, Bottleneck, Rework, SlowTail, Variation],
            ),
            ProcessType::ItTickets => (
                "IT or service tickets",
                "Fast first resolution",
                "Time to resolve",
                vec![SlowTail, Rework, Bottleneck, Open, Variation],
            ),
            ProcessType::Claims => (
                "Claims",
                "Fast, fair decisions",
                "Time to decide",
                vec![Bottleneck, Rework, SlowTail, Variation, DropOut],
            ),
            ProcessType::Onboarding => (
                "Onboarding",
                "Ready on day one",
                "Time to ready",
                vec![Bottleneck, SlowTail, Rework, DropOut, Variation],
            ),
            ProcessType::Permits => (
                "Permits and public services",
                "Predictable turnaround",
                "Turnaround time",
                vec![SlowTail, Rework, Bottleneck, Open, Variation],
            ),
            ProcessType::Manufacturing => (
                "Manufacturing or maintenance",
                "Throughput and first-time quality",
                "Lead time",
                vec![Rework, Bottleneck, SlowTail, Variation, DropOut],
            ),
            ProcessType::Logistics => (
                "Logistics",
                "On-time, complete delivery",
                "Delivery time",
                vec![SlowTail, DropOut, Rework, Bottleneck, Variation],
            ),
        };
        ProcessProfile {
            kind: self,
            label: label.to_owned(),
            goal: goal.to_owned(),
            time_label: time_label.to_owned(),
            emphasis,
            source: source.to_owned(),
        }
    }
}

/// A first read from step and item names: the type whose words the names
/// contain most often, if it clearly leads. Ties and weak matches give none.
pub fn recognise<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<ProcessProfile> {
    let names = names.into_iter().map(str::to_lowercase).collect::<Vec<_>>();
    let mut scores = ProcessType::ALL
        .iter()
        .map(|kind| {
            let score = kind
                .words()
                .iter()
                .filter(|word| names.iter().any(|name| name.contains(*word)))
                .count();
            (score, *kind)
        })
        .collect::<Vec<_>>();
    scores.sort_by_key(|(score, _)| std::cmp::Reverse(*score));
    let (best, kind) = scores[0];
    let runner_up = scores[1].0;
    (best >= 2 && best > runner_up).then(|| kind.profile("engine"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn step_names_suggest_the_process_and_its_goal() {
        let o2c = recognise([
            "Receive Customer Order",
            "Generate Invoice",
            "Send Invoice to Customer",
            "Receive Payment",
            "Issue Payment Reminder",
        ])
        .unwrap();
        assert_eq!(o2c.kind, ProcessType::OrderToCash);
        assert_eq!(o2c.time_label, "Time to cash");
        assert_eq!(o2c.source, "engine");
        let tickets = recognise([
            "Ticket created",
            "Assigned to team",
            "Reassigned",
            "Resolved",
            "Reopened",
        ])
        .unwrap();
        assert_eq!(tickets.kind, ProcessType::ItTickets);
        assert_eq!(tickets.emphasis[0], FindingKind::SlowTail);
        let deals = recognise([
            "Deal created",
            "Qualification",
            "Proposal/Price Quote",
            "Negotiation/Review",
            "Closed Won",
            "Order created",
        ])
        .unwrap();
        assert_eq!(deals.kind, ProcessType::SalesPipeline);
    }

    #[test]
    fn unclear_logs_get_no_guess() {
        assert!(recognise(["A", "B", "C"]).is_none());
        assert!(recognise(["Approve"]).is_none());
    }
}
