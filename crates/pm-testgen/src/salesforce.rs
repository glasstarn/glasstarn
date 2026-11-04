//! Made-up Salesforce extracts in the shape the Salesforce extract writes
//! them: one CSV per object, API field names as headers, times in UTC
//! (`2026-01-05T14:30:00.000Z`), booleans as `true` / `false`, and field
//! history rows the way Salesforce records them (a `created` row, owner
//! changes as an ID row plus a name row). Object and field names follow
//! public knowledge of the standard objects; every record is invented and the
//! counts in the truth structs are known by construction.

use crate::Rng;
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
            records: 400,
            seed: 3,
        }
    }
}

struct Table {
    header: &'static str,
    rows: Vec<String>,
}

impl Table {
    fn new(header: &'static str) -> Self {
        Self {
            header,
            rows: Vec::new(),
        }
    }
    fn push(&mut self, values: &[&str]) {
        assert_eq!(
            values.len(),
            self.header.split(',').count(),
            "{}",
            self.header
        );
        self.rows.push(
            values
                .iter()
                .map(|v| {
                    if v.contains([',', '"']) {
                        format!("\"{}\"", v.replace('"', "\"\""))
                    } else {
                        (*v).to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
}

fn save(dir: &Path, tables: &[(&str, Table)]) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for (name, table) in tables {
        let mut text = String::from(table.header);
        text.push('\n');
        for row in &table.rows {
            text.push_str(row);
            text.push('\n');
        }
        std::fs::write(dir.join(format!("{name}.csv")), text)?;
    }
    Ok(())
}

fn utc(at: NaiveDateTime) -> String {
    at.format("%Y-%m-%dT%H:%M:%S.000Z").to_string()
}

/// The org's time zone in the made-up data: UTC−5.
const ORG_OFFSET_HOURS: i64 = -5;

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2025, 11, 1)
        .unwrap()
        .and_hms_opt(0, 0, 0)
        .unwrap()
}

/// Field history rows, numbered.
struct History {
    table: Table,
    number: u64,
    prefix: &'static str,
}

impl History {
    fn new(header: &'static str, prefix: &'static str) -> Self {
        Self {
            table: Table::new(header),
            number: 0,
            prefix,
        }
    }
    fn row(
        &mut self,
        item: &str,
        field: &str,
        old: &str,
        new: &str,
        at: NaiveDateTime,
        kind: &str,
    ) {
        self.number += 1;
        let id = format!("{}{:010}", self.prefix, self.number);
        self.table
            .push(&[&id, item, field, old, new, &utc(at), kind]);
    }
}

/// What the lead-to-opportunity extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LeadTruth {
    pub leads: u64,
    /// Lead status changes in the field history.
    pub status_changes: u64,
    /// Owner changes (each recorded as an ID row and a name row).
    pub reassignments: u64,
    pub converted: u64,
    pub opportunities: u64,
    pub stage_changes: u64,
    pub won: u64,
    pub lost: u64,
    pub amount_changes: u64,
    pub close_date_moves: u64,
}

pub const LEAD_FILES: [&str; 4] = [
    "Lead",
    "LeadHistory",
    "Opportunity",
    "OpportunityFieldHistory",
];

/// Writes Lead, LeadHistory, Opportunity and OpportunityFieldHistory.
pub fn write_leads(dir: &Path, settings: &Settings) -> io::Result<LeadTruth> {
    let mut rng = Rng::new(settings.seed);
    let mut truth = LeadTruth::default();
    let mut leads = Table::new(
        "Id,CreatedDate,Status,LeadSource,OwnerId,Company,IsConverted,ConvertedDate,ConvertedOpportunityId,Region__c",
    );
    let mut lead_history = History::new(
        "Id,LeadId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g000",
    );
    let mut opportunities = Table::new(
        "Id,Name,AccountId,CreatedDate,StageName,IsClosed,IsWon,CloseDate,Amount,LeadSource",
    );
    let mut opportunity_history = History::new(
        "Id,OpportunityId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g100",
    );
    let sources = ["Web", "Partner Referral", "Trade Show", "Phone Inquiry"];
    let stages = [
        "Prospecting",
        "Qualification",
        "Proposal/Price Quote",
        "Negotiation/Review",
    ];
    let mut opportunity_number = 0u64;
    let mut new_opportunity = |rng: &mut Rng,
                               truth: &mut LeadTruth,
                               opportunities: &mut Table,
                               history: &mut History,
                               created: NaiveDateTime,
                               source: &str|
     -> String {
        opportunity_number += 1;
        truth.opportunities += 1;
        let id = format!("0065g00000{opportunity_number:08}");
        history.row(&id, "created", "", "", created, "Text");
        let mut stage = stages[0];
        let mut at = created;
        for next in stages.iter().take(1 + rng.below(3) as usize).skip(1) {
            at += Duration::hours(24 + rng.below(240) as i64);
            truth.stage_changes += 1;
            history.row(&id, "StageName", stage, next, at, "Picklist");
            stage = next;
        }
        if rng.chance(0.2) {
            at += Duration::hours(1 + rng.below(48) as i64);
            truth.amount_changes += 1;
            history.row(&id, "Amount", "20000", "25000", at, "Currency");
        }
        if rng.chance(0.3) {
            at += Duration::hours(1 + rng.below(48) as i64);
            truth.close_date_moves += 1;
            history.row(&id, "CloseDate", "2026-02-01", "2026-03-01", at, "Date");
        }
        let (stage, closed, won) = if rng.chance(0.6) {
            at += Duration::hours(24 + rng.below(480) as i64);
            truth.stage_changes += 1;
            let won = rng.chance(0.55);
            let end = if won { "Closed Won" } else { "Closed Lost" };
            if won {
                truth.won += 1;
            } else {
                truth.lost += 1;
            }
            history.row(&id, "StageName", stage, end, at, "Picklist");
            (end, true, won)
        } else {
            (stage, false, false)
        };
        opportunities.push(&[
            &id,
            "Equipment deal",
            "0015g00000AAAAA",
            &utc(created),
            stage,
            if closed { "true" } else { "false" },
            if won { "true" } else { "false" },
            &(at + Duration::hours(ORG_OFFSET_HOURS)).date().to_string(),
            "25000.00",
            source,
        ]);
        id
    };
    for n in 0..settings.records {
        truth.leads += 1;
        let id = format!("00Q5g00000{n:08}");
        let created = start()
            + Duration::days(rng.below(120) as i64)
            + Duration::minutes(13 * 60 + rng.below(9 * 60) as i64);
        let source = sources[rng.below(sources.len() as u64) as usize];
        lead_history.row(&id, "created", "", "", created, "Text");
        let mut status = "Open - Not Contacted";
        let mut at = created;
        let mut contacted = false;
        if rng.chance(0.85) {
            at += Duration::hours(2 + rng.below(70) as i64);
            truth.status_changes += 1;
            lead_history.row(&id, "Status", status, "Working - Contacted", at, "Picklist");
            status = "Working - Contacted";
            contacted = true;
        }
        if rng.chance(0.2) {
            at += Duration::hours(1 + rng.below(24) as i64);
            truth.reassignments += 1;
            lead_history.row(&id, "Owner", "0055g000001", "0055g000002", at, "EntityId");
            lead_history.row(&id, "Owner", "Ana Rossi", "Ben Okafor", at, "Text");
        }
        let (converted, opportunity) = if contacted && rng.chance(0.35) {
            at += Duration::hours(24 + rng.below(240) as i64);
            truth.status_changes += 1;
            lead_history.row(&id, "Status", status, "Qualified", at, "Picklist");
            status = "Qualified";
            // Converted the next working day, after the status change. As in
            // a real org, conversion writes the converted status and a
            // "leadConverted" row to the lead's history at that moment.
            let converted = at + Duration::days(1);
            truth.converted += 1;
            truth.status_changes += 1;
            lead_history.row(
                &id,
                "Status",
                status,
                "Closed - Converted",
                converted,
                "Picklist",
            );
            lead_history.row(&id, "leadConverted", "", "", converted, "Text");
            status = "Closed - Converted";
            let opportunity = new_opportunity(
                &mut rng,
                &mut truth,
                &mut opportunities,
                &mut opportunity_history,
                converted,
                source,
            );
            (Some(converted), opportunity)
        } else {
            if contacted && rng.chance(0.4) {
                at += Duration::hours(24 + rng.below(240) as i64);
                truth.status_changes += 1;
                lead_history.row(&id, "Status", status, "Unqualified", at, "Picklist");
                status = "Unqualified";
            }
            (None, String::new())
        };
        leads.push(&[
            &id,
            &utc(created),
            status,
            source,
            "0055g000001",
            "Acme Industrial",
            if converted.is_some() { "true" } else { "false" },
            &converted.map_or(String::new(), |c| {
                (c + Duration::hours(ORG_OFFSET_HOURS)).date().to_string()
            }),
            &opportunity,
            ["EMEA", "Americas", ""][n as usize % 3],
        ]);
    }
    // Opportunities that did not come from a lead.
    for _ in 0..settings.records / 5 {
        let created = start() + Duration::days(rng.below(120) as i64) + Duration::hours(15);
        new_opportunity(
            &mut rng,
            &mut truth,
            &mut opportunities,
            &mut opportunity_history,
            created,
            "Existing Customer",
        );
    }
    save(
        dir,
        &[
            ("Lead", leads),
            ("LeadHistory", lead_history.table),
            ("Opportunity", opportunities),
            ("OpportunityFieldHistory", opportunity_history.table),
        ],
    )?;
    Ok(truth)
}

/// What the case extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CaseTruth {
    pub cases: u64,
    pub status_changes: u64,
    pub reassignments: u64,
    pub priority_changes: u64,
    pub escalations: u64,
    pub closed: u64,
    pub emails_in: u64,
    pub emails_out: u64,
}

pub const CASE_FILES: [&str; 3] = ["Case", "CaseHistory", "EmailMessage"];

/// Writes Case, CaseHistory and EmailMessage.
pub fn write_cases(dir: &Path, settings: &Settings) -> io::Result<CaseTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xCA5E);
    let mut truth = CaseTruth::default();
    let mut cases = Table::new(
        "Id,CaseNumber,CreatedDate,Status,Priority,Origin,Type,IsClosed,ClosedDate,IsEscalated,OwnerId,Product__c",
    );
    let mut history = History::new(
        "Id,CaseId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g200",
    );
    let mut emails = Table::new("Id,ParentId,Incoming,MessageDate,Status");
    let mut email_number = 0u64;
    let mut email = |emails: &mut Table,
                     truth: &mut CaseTruth,
                     case: &str,
                     incoming: bool,
                     at: NaiveDateTime| {
        email_number += 1;
        if incoming {
            truth.emails_in += 1;
        } else {
            truth.emails_out += 1;
        }
        emails.push(&[
            &format!("02s5g00000{email_number:08}"),
            case,
            if incoming { "true" } else { "false" },
            &utc(at),
            if incoming { "0" } else { "3" },
        ]);
    };
    let origins = ["Email", "Phone", "Web"];
    for n in 0..settings.records {
        truth.cases += 1;
        let id = format!("5005g00000{n:08}");
        let created = start()
            + Duration::days(rng.below(120) as i64)
            + Duration::minutes(13 * 60 + rng.below(10 * 60) as i64);
        let origin = origins[rng.below(3) as usize];
        let mut priority = if rng.chance(0.2) { "High" } else { "Medium" };
        history.row(&id, "created", "", "", created, "Text");
        if origin == "Email" {
            email(&mut emails, &mut truth, &id, true, created);
        }
        let mut status = "New";
        let mut at = created;
        let step = |rng: &mut Rng,
                    history: &mut History,
                    truth: &mut CaseTruth,
                    status: &mut &str,
                    next: &'static str,
                    at: &mut NaiveDateTime,
                    hours: u64| {
            *at += Duration::minutes(30 + rng.below(hours * 60) as i64);
            truth.status_changes += 1;
            history.row(&id, "Status", status, next, *at, "Picklist");
            *status = next;
        };
        step(
            &mut rng,
            &mut history,
            &mut truth,
            &mut status,
            "Working",
            &mut at,
            8,
        );
        email(
            &mut emails,
            &mut truth,
            &id,
            false,
            at + Duration::minutes(20),
        );
        if rng.chance(0.25) {
            at += Duration::hours(1 + rng.below(12) as i64);
            truth.reassignments += 1;
            history.row(&id, "Owner", "0055g000010", "0055g000011", at, "EntityId");
            history.row(&id, "Owner", "Support Tier 1", "Support Tier 2", at, "Text");
        }
        if rng.chance(0.1) {
            at += Duration::hours(1 + rng.below(12) as i64);
            truth.priority_changes += 1;
            history.row(&id, "Priority", priority, "High", at, "Picklist");
            priority = "High";
        }
        // Waiting on the customer, sometimes more than once.
        for _ in 0..rng.below(3) {
            step(
                &mut rng,
                &mut history,
                &mut truth,
                &mut status,
                "Waiting on Customer",
                &mut at,
                24,
            );
            at += Duration::hours(4 + rng.below(72) as i64);
            email(&mut emails, &mut truth, &id, true, at);
            step(
                &mut rng,
                &mut history,
                &mut truth,
                &mut status,
                "Working",
                &mut at,
                4,
            );
        }
        let escalated = rng.chance(0.15);
        if escalated {
            at += Duration::hours(1 + rng.below(24) as i64);
            truth.escalations += 1;
            history.row(&id, "IsEscalated", "false", "true", at, "Boolean");
        }
        let closed = rng.chance(0.85);
        if closed {
            truth.closed += 1;
            step(
                &mut rng,
                &mut history,
                &mut truth,
                &mut status,
                "Closed",
                &mut at,
                48,
            );
        }
        cases.push(&[
            &id,
            &format!("{:08}", 1000 + n),
            &utc(created),
            status,
            priority,
            origin,
            "Problem",
            if closed { "true" } else { "false" },
            &if closed { utc(at) } else { String::new() },
            if escalated { "true" } else { "false" },
            "0055g000010",
            ["Pump", "Valve", ""][n as usize % 3],
        ]);
    }
    save(
        dir,
        &[
            ("Case", cases),
            ("CaseHistory", history.table),
            ("EmailMessage", emails),
        ],
    )?;
    Ok(truth)
}

/// What the quote extract contains, beside the opportunities of
/// [`write_leads`], which it writes too.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct QuoteTruth {
    pub opportunities: LeadTruth,
    pub quotes: u64,
    pub submitted: u64,
    pub approved: u64,
    pub rejected: u64,
    pub recalled: u64,
    pub orders: u64,
}

pub const QUOTE_FILES: [&str; 6] = [
    "Opportunity",
    "OpportunityFieldHistory",
    "Quote",
    "ProcessInstance",
    "ProcessInstanceStep",
    "Order",
];

/// Writes opportunities as [`write_leads`] does, then Quote, ProcessInstance,
/// ProcessInstanceStep and Order for some of them.
pub fn write_quotes(dir: &Path, settings: &Settings) -> io::Result<QuoteTruth> {
    let opportunities = write_leads(dir, settings)?;
    let mut rng = Rng::new(settings.seed ^ 0x0007E);
    let mut truth = QuoteTruth {
        opportunities: opportunities.clone(),
        ..QuoteTruth::default()
    };
    let mut quotes = Table::new(
        "Id,QuoteNumber,OpportunityId,CreatedDate,Status,ExpirationDate,GrandTotal,Discount,IsSyncing,Margin__c",
    );
    let mut requests =
        Table::new("Id,TargetObjectId,Status,CreatedDate,CompletedDate,ProcessDefinitionId");
    let mut steps =
        Table::new("Id,ProcessInstanceId,StepStatus,CreatedDate,ActorId,OriginalActorId");
    let mut orders = Table::new("Id,QuoteId,OpportunityId,CreatedDate,Status,OrderNumber");
    let (mut quote_number, mut request_number, mut step_number, mut order_number) =
        (0u64, 0u64, 0u64, 0u64);
    for o in 1..=opportunities.opportunities {
        if !rng.chance(0.7) {
            continue;
        }
        let opportunity = format!("0065g00000{o:08}");
        let mut at = start() + Duration::days(rng.below(110) as i64) + Duration::hours(15);
        for version in 0..1 + rng.below(2) {
            quote_number += 1;
            truth.quotes += 1;
            let quote = format!("0Q05g00000{quote_number:08}");
            let created = at;
            let discount = if rng.chance(0.4) { 25 } else { 5 };
            let mut status = "Draft";
            // Large discounts need approval.
            if discount > 20 {
                request_number += 1;
                let request = format!("04g5g00000{request_number:08}");
                let mut step = |steps: &mut Table, kind: &str, at: NaiveDateTime| {
                    step_number += 1;
                    steps.push(&[
                        &format!("04h5g00000{step_number:08}"),
                        &request,
                        kind,
                        &utc(at),
                        "0055g000020",
                        "0055g000020",
                    ]);
                };
                let submitted = created + Duration::hours(1 + rng.below(24) as i64);
                truth.submitted += 1;
                step(&mut steps, "Started", submitted);
                let decided = submitted + Duration::hours(2 + rng.below(72) as i64);
                let outcome = match rng.below(10) {
                    0 => {
                        truth.recalled += 1;
                        step(&mut steps, "Removed", decided);
                        "Removed"
                    }
                    1..=3 => {
                        truth.rejected += 1;
                        step(&mut steps, "Rejected", decided);
                        status = "Rejected";
                        "Rejected"
                    }
                    _ => {
                        truth.approved += 1;
                        step(&mut steps, "Approved", decided);
                        status = "Approved";
                        "Approved"
                    }
                };
                requests.push(&[
                    &request,
                    &quote,
                    outcome,
                    &utc(submitted),
                    &utc(decided),
                    "04a5g000000001",
                ]);
                at = decided;
            } else {
                status = "Approved";
            }
            let accepted = status == "Approved" && (version == 1 || rng.chance(0.6));
            if accepted {
                at += Duration::hours(24 + rng.below(240) as i64);
                order_number += 1;
                truth.orders += 1;
                orders.push(&[
                    &format!("8015g00000{order_number:08}"),
                    &quote,
                    &opportunity,
                    &utc(at),
                    "Draft",
                    &format!("{:08}", 100 + order_number),
                ]);
                status = "Accepted";
            }
            quotes.push(&[
                &quote,
                &format!("{:08}", quote_number),
                &opportunity,
                &utc(created),
                status,
                &(created + Duration::days(30)).date().to_string(),
                "25000.00",
                &discount.to_string(),
                if accepted { "true" } else { "false" },
                "0.3",
            ]);
            if accepted {
                break;
            }
            at += Duration::hours(24);
        }
    }
    save(
        dir,
        &[
            ("Quote", quotes),
            ("ProcessInstance", requests),
            ("ProcessInstanceStep", steps),
            ("Order", orders),
        ],
    )?;
    Ok(truth)
}

/// What the order extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct OrderTruth {
    pub orders: u64,
    pub reduction_orders: u64,
    /// Status changes other than activation.
    pub status_changes: u64,
    pub activated: u64,
    pub order_products: u64,
}

pub const ORDER_FILES: [&str; 3] = ["Order", "OrderHistory", "OrderItem"];

/// Writes Order, OrderHistory and OrderItem.
pub fn write_orders(dir: &Path, settings: &Settings) -> io::Result<OrderTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x0D0E);
    let mut truth = OrderTruth::default();
    let mut orders = Table::new(
        "Id,OrderNumber,AccountId,OpportunityId,QuoteId,CreatedDate,EffectiveDate,Status,ActivatedDate,TotalAmount,IsReductionOrder,OriginalOrderId,StatusCode,Channel__c",
    );
    let mut history = History::new(
        "Id,OrderId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g300",
    );
    let mut items =
        Table::new("Id,OrderId,Product2Id,Quantity,UnitPrice,TotalPrice,ServiceDate,CreatedDate");
    let mut item_number = 0u64;
    let mut number = 0u64;
    let mut activated_orders = Vec::new();
    let mut add_items = |rng: &mut Rng,
                         truth: &mut OrderTruth,
                         items: &mut Table,
                         order: &str,
                         at: NaiveDateTime,
                         sign: i64| {
        for _ in 0..1 + rng.below(3) {
            item_number += 1;
            truth.order_products += 1;
            let quantity = sign * (1 + rng.below(5) as i64);
            items.push(&[
                &format!("8025g00000{item_number:08}"),
                order,
                "01t5g000000001",
                &quantity.to_string(),
                "500.00",
                &format!("{}.00", 500 * quantity),
                &(at + Duration::days(7)).date().to_string(),
                &utc(at + Duration::minutes(2)),
            ]);
        }
    };
    for _ in 0..settings.records {
        number += 1;
        truth.orders += 1;
        let order = format!("8015g00000{number:08}");
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(14 * 60 + rng.below(8 * 60) as i64);
        history.row(&order, "created", "", "", created, "Text");
        add_items(&mut rng, &mut truth, &mut items, &order, created, 1);
        let mut at = created;
        let mut status = "Draft";
        if rng.chance(0.3) {
            at += Duration::hours(2 + rng.below(48) as i64);
            truth.status_changes += 1;
            history.row(&order, "Status", status, "In Review", at, "Picklist");
            status = "In Review";
        }
        let activated = rng.chance(0.8);
        if activated {
            at += Duration::hours(1 + rng.below(72) as i64);
            truth.activated += 1;
            history.row(&order, "Status", status, "Activated", at, "Picklist");
            status = "Activated";
            activated_orders.push((order.clone(), at));
        }
        orders.push(&[
            &order,
            &format!("{:08}", 100 + number),
            "0015g00000AAAAA",
            "",
            "",
            &utc(created),
            &(created + Duration::hours(ORG_OFFSET_HOURS))
                .date()
                .to_string(),
            status,
            &if activated { utc(at) } else { String::new() },
            "1500.00",
            "false",
            "",
            if activated { "Activated" } else { "Draft" },
            ["Web", "Direct", ""][number as usize % 3],
        ]);
    }
    // Some activated orders are reduced later: a reduction order of their own.
    for (original, at) in activated_orders {
        if !rng.chance(0.1) {
            continue;
        }
        number += 1;
        truth.reduction_orders += 1;
        let order = format!("8015g00000{number:08}");
        let created = at + Duration::days(3 + rng.below(20) as i64);
        history.row(&order, "created", "", "", created, "Text");
        add_items(&mut rng, &mut truth, &mut items, &order, created, -1);
        orders.push(&[
            &order,
            &format!("{:08}", 100 + number),
            "0015g00000AAAAA",
            "",
            "",
            &utc(created),
            &(created + Duration::hours(ORG_OFFSET_HOURS))
                .date()
                .to_string(),
            "Draft",
            "",
            "-500.00",
            "true",
            &original,
            "Draft",
            "",
        ]);
    }
    save(
        dir,
        &[
            ("Order", orders),
            ("OrderHistory", history.table),
            ("OrderItem", items),
        ],
    )?;
    Ok(truth)
}

/// What the field service extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldServiceTruth {
    pub work_orders: u64,
    pub work_order_status_changes: u64,
    pub appointments: u64,
    pub appointment_status_changes: u64,
    pub assignments: u64,
    pub started: u64,
    pub finished: u64,
}

pub const FIELD_SERVICE_FILES: [&str; 5] = [
    "WorkOrder",
    "WorkOrderHistory",
    "ServiceAppointment",
    "ServiceAppointmentHistory",
    "AssignedResource",
];

/// Writes WorkOrder, WorkOrderHistory, ServiceAppointment,
/// ServiceAppointmentHistory and AssignedResource.
pub fn write_field_service(dir: &Path, settings: &Settings) -> io::Result<FieldServiceTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xF5F5);
    let mut truth = FieldServiceTruth::default();
    let mut work_orders = Table::new(
        "Id,WorkOrderNumber,CaseId,AccountId,AssetId,WorkTypeId,CreatedDate,Status,Priority,Region__c",
    );
    let mut work_order_history = History::new(
        "Id,WorkOrderId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g400",
    );
    let mut appointments = Table::new(
        "Id,AppointmentNumber,ParentRecordId,CreatedDate,Status,SchedStartTime,SchedEndTime,ActualStartTime,ActualEndTime,ArrivalWindowStartTime,ArrivalWindowEndTime",
    );
    let mut appointment_history = History::new(
        "Id,ServiceAppointmentId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g500",
    );
    let mut assignments = Table::new("Id,ServiceAppointmentId,ServiceResourceId,CreatedDate");
    let (mut appointment_number, mut assignment_number) = (0u64, 0u64);
    for n in 0..settings.records {
        truth.work_orders += 1;
        let work_order = format!("0WO5g00000{n:08}");
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(13 * 60 + rng.below(9 * 60) as i64);
        work_order_history.row(&work_order, "created", "", "", created, "Text");
        let mut status = "New";
        let mut at = created;
        let work_order_step = |history: &mut History,
                               truth: &mut FieldServiceTruth,
                               status: &mut &str,
                               next: &'static str,
                               at: NaiveDateTime| {
            truth.work_order_status_changes += 1;
            history.row(&work_order, "Status", status, next, at, "Picklist");
            *status = next;
        };
        // One visit, sometimes a second when the first could not finish.
        let visits = if rng.chance(0.2) { 2 } else { 1 };
        let mut done = false;
        for visit in 0..visits {
            appointment_number += 1;
            truth.appointments += 1;
            let appointment = format!("08p5g00000{appointment_number:08}");
            let booked = at + Duration::hours(1 + rng.below(24) as i64);
            appointment_history.row(&appointment, "created", "", "", booked, "Text");
            let mut appointment_status = "None";
            let step = |history: &mut History,
                        truth: &mut FieldServiceTruth,
                        status: &mut &str,
                        next: &'static str,
                        at: NaiveDateTime| {
                truth.appointment_status_changes += 1;
                history.row(&appointment, "Status", status, next, at, "Picklist");
                *status = next;
            };
            let scheduled = booked + Duration::hours(2);
            step(
                &mut appointment_history,
                &mut truth,
                &mut appointment_status,
                "Scheduled",
                scheduled,
            );
            assignment_number += 1;
            truth.assignments += 1;
            assignments.push(&[
                &format!("03r5g00000{assignment_number:08}"),
                &appointment,
                "0Hn5g000000001",
                &utc(scheduled),
            ]);
            if visit == 0 {
                work_order_step(
                    &mut work_order_history,
                    &mut truth,
                    &mut status,
                    "In Progress",
                    scheduled,
                );
            }
            let window = scheduled + Duration::days(1 + rng.below(5) as i64);
            let dispatched = window - Duration::hours(2);
            step(
                &mut appointment_history,
                &mut truth,
                &mut appointment_status,
                "Dispatched",
                dispatched,
            );
            // Arrives within the window or up to three hours late.
            let started = window + Duration::minutes(rng.below(5 * 60) as i64);
            step(
                &mut appointment_history,
                &mut truth,
                &mut appointment_status,
                "In Progress",
                started,
            );
            truth.started += 1;
            let finished = started + Duration::minutes(30 + rng.below(180) as i64);
            truth.finished += 1;
            let completed = visit + 1 == visits;
            step(
                &mut appointment_history,
                &mut truth,
                &mut appointment_status,
                if completed {
                    "Completed"
                } else {
                    "Cannot Complete"
                },
                finished,
            );
            appointments.push(&[
                &appointment,
                &format!("SA-{appointment_number:05}"),
                &work_order,
                &utc(booked),
                appointment_status,
                &utc(window),
                &utc(window + Duration::hours(2)),
                &utc(started),
                &utc(finished),
                &utc(window),
                &utc(window + Duration::hours(2)),
            ]);
            at = finished;
            done = completed;
        }
        if done {
            work_order_step(
                &mut work_order_history,
                &mut truth,
                &mut status,
                "Completed",
                at + Duration::hours(1),
            );
        }
        work_orders.push(&[
            &work_order,
            &format!("{:08}", n),
            "5005g00000AAAAA",
            "0015g00000AAAAA",
            "02i5g00000AAAAA",
            "08q5g000000001",
            &utc(created),
            status,
            if n % 4 == 0 { "High" } else { "Medium" },
            ["North", "South", ""][n as usize % 3],
        ]);
    }
    // Appointments still to come: booked, not yet visited.
    for n in 0..settings.records / 10 {
        appointment_number += 1;
        truth.appointments += 1;
        let appointment = format!("08p5g00000{appointment_number:08}");
        let booked = start() + Duration::days(115) + Duration::hours(n as i64);
        appointment_history.row(&appointment, "created", "", "", booked, "Text");
        appointments.push(&[
            &appointment,
            &format!("SA-{appointment_number:05}"),
            &format!("0WO5g00000{n:08}"),
            &utc(booked),
            "None",
            "",
            "",
            "",
            "",
            "",
            "",
        ]);
    }
    save(
        dir,
        &[
            ("WorkOrder", work_orders),
            ("WorkOrderHistory", work_order_history.table),
            ("ServiceAppointment", appointments),
            ("ServiceAppointmentHistory", appointment_history.table),
            ("AssignedResource", assignments),
        ],
    )?;
    Ok(truth)
}

/// What the contract extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ContractTruth {
    pub contracts: u64,
    /// Status changes other than activation.
    pub status_changes: u64,
    pub activated: u64,
    /// Opportunities in the export, renewals and others.
    pub opportunities: u64,
    pub renewals: u64,
    pub renewal_stage_changes: u64,
    pub renewals_won: u64,
    pub renewals_lost: u64,
    /// Every opportunity step that reaches a contract.
    pub renewal_steps: u64,
}

pub const CONTRACT_FILES: [&str; 4] = [
    "Contract",
    "ContractHistory",
    "Opportunity",
    "OpportunityFieldHistory",
];

/// Writes Contract, ContractHistory, Opportunity (with ContractId) and
/// OpportunityFieldHistory.
pub fn write_contracts(dir: &Path, settings: &Settings) -> io::Result<ContractTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xC047);
    let mut truth = ContractTruth::default();
    let mut contracts = Table::new(
        "Id,ContractNumber,AccountId,Status,StartDate,EndDate,ContractTerm,ActivatedDate,CreatedDate,StatusCode,Tier__c",
    );
    let mut contract_history = History::new(
        "Id,ContractId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g600",
    );
    let mut opportunities = Table::new(
        "Id,Name,AccountId,CreatedDate,StageName,IsClosed,IsWon,CloseDate,Amount,Type,ContractId",
    );
    let mut opportunity_history = History::new(
        "Id,OpportunityId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g700",
    );
    let mut opportunity_number = 0u64;
    let mut new_opportunity = |rng: &mut Rng,
                               truth: &mut ContractTruth,
                               created: NaiveDateTime,
                               contract: &str| {
        opportunity_number += 1;
        truth.opportunities += 1;
        let renewal = !contract.is_empty();
        let id = format!("0065g10000{opportunity_number:08}");
        opportunity_history.row(&id, "created", "", "", created, "Text");
        let mut at = created + Duration::days(3 + rng.below(20) as i64);
        let mut steps = 1;
        opportunity_history.row(
            &id,
            "StageName",
            "Prospecting",
            "Negotiation/Review",
            at,
            "Picklist",
        );
        if renewal {
            truth.renewal_stage_changes += 1;
        }
        steps += 1;
        let (stage, closed, won) = if rng.chance(0.8) {
            at += Duration::days(5 + rng.below(40) as i64);
            let won = rng.chance(0.85);
            let end = if won { "Closed Won" } else { "Closed Lost" };
            opportunity_history.row(&id, "StageName", "Negotiation/Review", end, at, "Picklist");
            steps += 1;
            if renewal {
                if won {
                    truth.renewals_won += 1;
                } else {
                    truth.renewals_lost += 1;
                }
            }
            (end, true, won)
        } else {
            ("Negotiation/Review", false, false)
        };
        if renewal {
            truth.renewals += 1;
            truth.renewal_steps += steps;
        }
        opportunities.push(&[
            &id,
            if renewal { "Renewal" } else { "New deal" },
            "0015g00000AAAAA",
            &utc(created),
            stage,
            if closed { "true" } else { "false" },
            if won { "true" } else { "false" },
            &(at + Duration::hours(ORG_OFFSET_HOURS)).date().to_string(),
            "48000.00",
            if renewal { "Renewal" } else { "New Business" },
            contract,
        ]);
    };
    for n in 0..settings.records {
        truth.contracts += 1;
        let contract = format!("8005g00000{n:08}");
        let created = start()
            + Duration::days(rng.below(110) as i64)
            + Duration::minutes(14 * 60 + rng.below(8 * 60) as i64);
        contract_history.row(&contract, "created", "", "", created, "Text");
        let mut at = created;
        let mut status = "Draft";
        if rng.chance(0.4) {
            at += Duration::hours(2 + rng.below(72) as i64);
            truth.status_changes += 1;
            contract_history.row(
                &contract,
                "Status",
                status,
                "In Approval Process",
                at,
                "Picklist",
            );
            status = "In Approval Process";
        }
        let activated = rng.chance(0.85);
        if activated {
            at += Duration::hours(4 + rng.below(96) as i64);
            truth.activated += 1;
            contract_history.row(&contract, "Status", status, "Activated", at, "Picklist");
            status = "Activated";
        }
        let term = [12, 24, 36][rng.below(3) as usize];
        let start_date = (created + Duration::days(7)).date();
        let end_date = start_date + Duration::days(30 * term as i64 - 1);
        contracts.push(&[
            &contract,
            &format!("{:08}", n),
            "0015g00000AAAAA",
            status,
            &start_date.to_string(),
            &end_date.to_string(),
            &term.to_string(),
            &if activated { utc(at) } else { String::new() },
            &utc(created),
            if activated { "Activated" } else { "Draft" },
            ["Gold", "Silver", ""][n as usize % 3],
        ]);
        // Some activated contracts already have their renewal under way.
        if activated && rng.chance(0.5) {
            let created = at + Duration::days(10 + rng.below(40) as i64);
            new_opportunity(&mut rng, &mut truth, created, &contract);
        }
    }
    // Deals that are not renewals.
    for _ in 0..settings.records / 4 {
        let created = start() + Duration::days(rng.below(100) as i64) + Duration::hours(16);
        new_opportunity(&mut rng, &mut truth, created, "");
    }
    save(
        dir,
        &[
            ("Contract", contracts),
            ("ContractHistory", contract_history.table),
            ("Opportunity", opportunities),
            ("OpportunityFieldHistory", opportunity_history.table),
        ],
    )?;
    Ok(truth)
}

/// What the CPQ extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct CpqTruth {
    pub opportunities: u64,
    pub quotes: u64,
    pub status_changes: u64,
    pub orders: u64,
    pub orders_activated: u64,
    pub contracts: u64,
    pub contracts_activated: u64,
    pub renewals: u64,
    /// Steps of renewal opportunities (created, stage changes, won or lost).
    pub renewal_steps: u64,
}

pub const CPQ_FILES: [&str; 6] = [
    "Opportunity",
    "OpportunityFieldHistory",
    "SBQQ__Quote__c",
    "SBQQ__Quote__History",
    "Order",
    "Contract",
];

/// Writes opportunities, CPQ quotes and their history, orders, contracts and
/// renewal opportunities.
pub fn write_cpq(dir: &Path, settings: &Settings) -> io::Result<CpqTruth> {
    let mut rng = Rng::new(settings.seed ^ 0xC90);
    let mut truth = CpqTruth::default();
    let mut opportunities = Table::new(
        "Id,Name,AccountId,CreatedDate,StageName,IsClosed,IsWon,CloseDate,Amount,Type,SBQQ__RenewedContract__c",
    );
    let mut opportunity_history = History::new(
        "Id,OpportunityId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g800",
    );
    let mut quotes = Table::new(
        "Id,Name,SBQQ__Opportunity2__c,CreatedDate,SBQQ__Status__c,SBQQ__Primary__c,SBQQ__NetAmount__c,SBQQ__Ordered__c,SBQQ__Type__c,Region__c",
    );
    let mut quote_history = History::new(
        "Id,ParentId,Field,OldValue,NewValue,CreatedDate,DataType",
        "0175g900",
    );
    let mut orders =
        Table::new("Id,SBQQ__Quote__c,CreatedDate,ActivatedDate,Status,OrderNumber,TotalAmount");
    let mut contracts = Table::new(
        "Id,SBQQ__Quote__c,CreatedDate,StartDate,EndDate,ActivatedDate,SBQQ__Order__c,Status,ContractNumber,SBQQ__RenewalOpportunity__c",
    );
    let (mut opportunity_number, mut quote_number) = (0u64, 0u64);
    // An opportunity with a stage path; renewals name their contract.
    let mut new_opportunity = |rng: &mut Rng,
                               truth: &mut CpqTruth,
                               created: NaiveDateTime,
                               contract: &str|
     -> (String, NaiveDateTime) {
        opportunity_number += 1;
        truth.opportunities += 1;
        let id = format!("0065g20000{opportunity_number:08}");
        opportunity_history.row(&id, "created", "", "", created, "Text");
        let at = created + Duration::days(2 + rng.below(10) as i64);
        opportunity_history.row(
            &id,
            "StageName",
            "Prospecting",
            "Proposal/Price Quote",
            at,
            "Picklist",
        );
        let renewal = !contract.is_empty();
        if renewal {
            truth.renewals += 1;
            truth.renewal_steps += 2;
        }
        opportunities.push(&[
            &id,
            if renewal { "Renewal" } else { "New deal" },
            "0015g00000AAAAA",
            &utc(created),
            "Proposal/Price Quote",
            "false",
            "false",
            &(at + Duration::days(30)).date().to_string(),
            "60000.00",
            if renewal { "Renewal" } else { "New Business" },
            contract,
        ]);
        (id, at)
    };
    let mut contract_number = 0u64;
    let mut order_number = 0u64;
    for _ in 0..settings.records {
        let created = start() + Duration::days(rng.below(100) as i64) + Duration::hours(15);
        let (opportunity, mut at) = new_opportunity(&mut rng, &mut truth, created, "");
        quote_number += 1;
        truth.quotes += 1;
        let quote = format!("a0x5g00000{quote_number:08}");
        at += Duration::hours(4 + rng.below(48) as i64);
        let quoted = at;
        let mut status = "Draft";
        let step = |history: &mut History,
                    truth: &mut CpqTruth,
                    status: &mut &str,
                    next: &'static str,
                    at: NaiveDateTime| {
            truth.status_changes += 1;
            history.row(&quote, "SBQQ__Status__c", status, next, at, "Picklist");
            *status = next;
        };
        quote_history.row(&quote, "created", "", "", quoted, "Text");
        at += Duration::hours(1 + rng.below(24) as i64);
        step(&mut quote_history, &mut truth, &mut status, "In Review", at);
        at += Duration::hours(2 + rng.below(72) as i64);
        let approved = rng.chance(0.8);
        step(
            &mut quote_history,
            &mut truth,
            &mut status,
            if approved { "Approved" } else { "Denied" },
            at,
        );
        let mut ordered = false;
        if approved {
            at += Duration::hours(4 + rng.below(48) as i64);
            step(&mut quote_history, &mut truth, &mut status, "Presented", at);
            if rng.chance(0.7) {
                at += Duration::days(1 + rng.below(14) as i64);
                step(&mut quote_history, &mut truth, &mut status, "Accepted", at);
                ordered = true;
            }
        }
        if ordered {
            order_number += 1;
            truth.orders += 1;
            at += Duration::hours(2);
            let order = format!("8015g10000{order_number:08}");
            let activated = rng.chance(0.9);
            let activated_at = at + Duration::hours(3 + rng.below(48) as i64);
            truth.orders_activated += u64::from(activated);
            orders.push(&[
                &order,
                &quote,
                &utc(at),
                &if activated {
                    utc(activated_at)
                } else {
                    String::new()
                },
                if activated { "Activated" } else { "Draft" },
                &format!("{:08}", 500 + order_number),
                "60000.00",
            ]);
            if activated {
                contract_number += 1;
                truth.contracts += 1;
                let contract = format!("8005g10000{contract_number:08}");
                let contracted = activated_at + Duration::hours(1);
                let contract_live = rng.chance(0.9);
                truth.contracts_activated += u64::from(contract_live);
                let start_date = (contracted + Duration::days(1)).date();
                contracts.push(&[
                    &contract,
                    &quote,
                    &utc(contracted),
                    &start_date.to_string(),
                    &(start_date + Duration::days(364)).to_string(),
                    &if contract_live {
                        utc(contracted + Duration::hours(2))
                    } else {
                        String::new()
                    },
                    &order,
                    if contract_live { "Activated" } else { "Draft" },
                    &format!("{:08}", contract_number),
                    "",
                ]);
                if contract_live && rng.chance(0.5) {
                    let renewal_created = contracted + Duration::days(20 + rng.below(30) as i64);
                    new_opportunity(&mut rng, &mut truth, renewal_created, &contract);
                }
            }
        }
        quotes.push(&[
            &quote,
            &format!("Q-{quote_number:05}"),
            &opportunity,
            &utc(quoted),
            status,
            "true",
            "60000.00",
            if ordered { "true" } else { "false" },
            "Quote",
            ["EMEA", "Americas", ""][quote_number as usize % 3],
        ]);
    }
    save(
        dir,
        &[
            ("Opportunity", opportunities),
            ("OpportunityFieldHistory", opportunity_history.table),
            ("SBQQ__Quote__c", quotes),
            ("SBQQ__Quote__History", quote_history.table),
            ("Order", orders),
            ("Contract", contracts),
        ],
    )?;
    Ok(truth)
}

/// What the nCino extract contains.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct LoanTruth {
    pub loans: u64,
    pub stage_changes: u64,
    pub status_changes: u64,
    pub amount_changes: u64,
    pub submitted: u64,
    pub approved: u64,
    pub declined: u64,
    pub recalled: u64,
    pub booked: u64,
}

pub const LOAN_FILES: [&str; 4] = [
    "LLC_BI__Loan__c",
    "LLC_BI__Loan__History",
    "ProcessInstance",
    "ProcessInstanceStep",
];

/// Writes nCino loans, their field history and their credit approvals.
pub fn write_ncino(dir: &Path, settings: &Settings) -> io::Result<LoanTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x10A4);
    let mut truth = LoanTruth::default();
    let mut loans = Table::new(
        "Id,Name,CreatedDate,LLC_BI__Stage__c,LLC_BI__Status__c,LLC_BI__Amount__c,LLC_BI__Product_Line__c,LLC_BI__Product__c,LLC_BI__Branch__c,Risk_Rating__c",
    );
    let mut history = History::new(
        "Id,ParentId,Field,OldValue,NewValue,CreatedDate,DataType",
        "017aa000",
    );
    let mut requests =
        Table::new("Id,TargetObjectId,Status,CreatedDate,CompletedDate,ProcessDefinitionId");
    let mut steps =
        Table::new("Id,ProcessInstanceId,StepStatus,CreatedDate,ActorId,OriginalActorId");
    let (mut request_number, mut step_number) = (0u64, 0u64);
    let before_approval = ["Proposal", "Underwriting", "Credit Approval"];
    let after_approval = ["Processing", "Doc Prep", "Closing", "Booked"];
    for n in 0..settings.records {
        truth.loans += 1;
        let loan = format!("a0L5g00000{n:08}");
        let created = start()
            + Duration::days(rng.below(100) as i64)
            + Duration::minutes(14 * 60 + rng.below(8 * 60) as i64);
        history.row(&loan, "created", "", "", created, "Text");
        let mut stage = "Qualification";
        let mut status = "Open";
        let mut at = created;
        let move_to = |history: &mut History,
                       truth: &mut LoanTruth,
                       stage: &mut &str,
                       next: &'static str,
                       at: NaiveDateTime| {
            truth.stage_changes += 1;
            history.row(&loan, "LLC_BI__Stage__c", stage, next, at, "Picklist");
            *stage = next;
        };
        let set_status = |history: &mut History,
                          truth: &mut LoanTruth,
                          status: &mut &str,
                          next: &'static str,
                          at: NaiveDateTime| {
            truth.status_changes += 1;
            history.row(&loan, "LLC_BI__Status__c", status, next, at, "Picklist");
            *status = next;
        };
        let withdrawn = rng.chance(0.1);
        let reached = if withdrawn {
            1 + rng.below(2) as usize
        } else {
            3
        };
        for next in before_approval.iter().take(reached) {
            at += Duration::hours(12 + rng.below(120) as i64);
            move_to(&mut history, &mut truth, &mut stage, next, at);
        }
        if rng.chance(0.15) {
            at += Duration::hours(3);
            truth.amount_changes += 1;
            history.row(
                &loan,
                "LLC_BI__Amount__c",
                "250000",
                "300000",
                at,
                "Currency",
            );
        }
        if withdrawn {
            at += Duration::days(2 + rng.below(10) as i64);
            set_status(&mut history, &mut truth, &mut status, "Withdrawn", at);
        } else {
            request_number += 1;
            let request = format!("04g5g10000{request_number:08}");
            let mut step = |steps: &mut Table, kind: &str, at: NaiveDateTime| {
                step_number += 1;
                steps.push(&[
                    &format!("04h5g10000{step_number:08}"),
                    &request,
                    kind,
                    &utc(at),
                    "0055g000030",
                    "0055g000030",
                ]);
            };
            let submitted = at + Duration::hours(1);
            truth.submitted += 1;
            step(&mut steps, "Started", submitted);
            let decided = submitted + Duration::hours(4 + rng.below(96) as i64);
            let outcome = match rng.below(10) {
                0 => {
                    truth.recalled += 1;
                    step(&mut steps, "Removed", decided);
                    "Removed"
                }
                1 | 2 => {
                    truth.declined += 1;
                    step(&mut steps, "Rejected", decided);
                    "Rejected"
                }
                _ => {
                    truth.approved += 1;
                    step(&mut steps, "Approved", decided);
                    "Approved"
                }
            };
            requests.push(&[
                &request,
                &loan,
                outcome,
                &utc(submitted),
                &utc(decided),
                "04a5g000000002",
            ]);
            at = decided;
            if outcome == "Approved" {
                let done = if rng.chance(0.8) {
                    4
                } else {
                    1 + rng.below(3) as usize
                };
                for next in after_approval.iter().take(done) {
                    at += Duration::hours(24 + rng.below(96) as i64);
                    move_to(&mut history, &mut truth, &mut stage, next, at);
                }
                if done == 4 {
                    truth.booked += 1;
                    set_status(
                        &mut history,
                        &mut truth,
                        &mut status,
                        "Closed",
                        at + Duration::minutes(5),
                    );
                }
            } else if outcome == "Rejected" {
                set_status(
                    &mut history,
                    &mut truth,
                    &mut status,
                    "Declined",
                    at + Duration::minutes(30),
                );
            }
        }
        loans.push(&[
            &loan,
            &format!("LN-{n:06}"),
            &utc(created),
            stage,
            status,
            "250000.00",
            [
                "Commercial Real Estate",
                "Commercial & Industrial",
                "Consumer",
            ][n as usize % 3],
            "Term Loan",
            "Main Street",
            ["3", "5", ""][n as usize % 3],
        ]);
    }
    save(
        dir,
        &[
            ("LLC_BI__Loan__c", loans),
            ("LLC_BI__Loan__History", history.table),
            ("ProcessInstance", requests),
            ("ProcessInstanceStep", steps),
        ],
    )?;
    Ok(truth)
}
