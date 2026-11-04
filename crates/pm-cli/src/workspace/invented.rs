//! Invented demo data, generated on the spot: no file is shipped, nothing comes
//! from a real or trial system. Each demo is small and carries one or two
//! planted stories, so a first look has something real to find:
//!
//! - purchase to pay: orders over 25,000 wait on one approver; 14 invoices
//!   are paid twice; one vendor's invoices bounce on price. Its
//!   `order_status` column changes from step to step.
//! - order to cash: new customers are held on credit; the Reno warehouse
//!   ships days later than the others.
//! - connected: orders, deliveries, invoices and payments as linked records;
//!   a quarter of orders ship in two deliveries; retail customers pay late
//!   and get reminders.
//! - sales pipeline: webinar leads stall before qualification; deals over
//!   50,000 loop on discount approval.
//!
//! The same seed always gives the same data, so the stories and the tests
//! that check them never drift.
use chrono::{Duration, NaiveDate, NaiveDateTime};

/// A small, fixed random source (splitmix64): enough for plausible spread,
/// and identical on every machine.
pub(super) struct Rng(u64);

impl Rng {
    pub(super) fn new(seed: u64) -> Self {
        Self(seed)
    }
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }
    fn range(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.unit()
    }
    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

/// One item's history: its fields and its steps in order.
pub(super) struct Case {
    pub(super) id: String,
    pub(super) fields: Vec<String>,
    pub(super) steps: Vec<(String, NaiveDateTime, String)>,
    /// A column whose value changes from step to step (an order's status),
    /// one value per step, written after the fields. Empty when the demo has
    /// none.
    pub(super) step_values: Vec<String>,
}

/// Moves forward in time by a random number of hours.
struct Clock(NaiveDateTime);

impl Clock {
    fn after(&mut self, rng: &mut Rng, lo_hours: f64, hi_hours: f64) -> NaiveDateTime {
        let minutes = (rng.range(lo_hours, hi_hours) * 60.0).round() as i64;
        self.0 += Duration::minutes(minutes.max(1));
        self.0
    }
}

fn year_start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2025, 1, 6)
        .and_then(|day| day.and_hms_opt(8, 0, 0))
        .expect("a valid date")
}

/// A start time spread over the year, on a working hour.
fn start_time(rng: &mut Rng) -> NaiveDateTime {
    year_start() + Duration::minutes((rng.range(0.0, 340.0) * 24.0 * 60.0) as i64)
}

fn step(case: &mut Case, activity: &str, at: NaiveDateTime, who: &str) {
    case.steps.push((activity.to_owned(), at, who.to_owned()));
}

/// The CSV the import reads: one row per step, the item's fields repeated.
/// `case_column` names the item ("purchase_order"), so the app can call the
/// items by their business name.
pub(super) fn to_csv(case_column: &str, names: &[&str], cases: &[Case]) -> String {
    let mut out = format!("{case_column},activity,timestamp,resource");
    for name in names {
        out.push(',');
        out.push_str(name);
    }
    out.push('\n');
    for case in cases {
        for (index, (activity, at, who)) in case.steps.iter().enumerate() {
            out.push_str(&format!(
                "{},{},{},{}",
                case.id,
                activity,
                at.format("%Y-%m-%dT%H:%M:%S"),
                who
            ));
            for value in &case.fields {
                out.push(',');
                out.push_str(value);
            }
            if let Some(value) = case.step_values.get(index) {
                out.push(',');
                out.push_str(value);
            }
            out.push('\n');
        }
    }
    out
}

/// The last is `order_status`, which changes from step to step.
pub(super) const P2P_FIELDS: [&str; 5] =
    ["vendor", "buyer", "company", "order_amount", "order_status"];
/// Orders above this go to the slow approver.
pub(super) const P2P_APPROVAL_LIMIT: f64 = 25_000.0;
pub(super) const P2P_SLOW_APPROVER: &str = "Dana Whitfield";
pub(super) const P2P_DUPLICATE_PAYMENTS: usize = 14;
pub(super) const P2P_PRICE_VENDOR: &str = "Northwind Supplies";

/// Purchase to pay: 1,500 orders.
pub(super) fn purchase_to_pay(seed: u64) -> Vec<Case> {
    let mut rng = Rng::new(seed);
    let vendors = [
        "Northwind Supplies",
        "Bluepeak Industrial",
        "Harbor Office Co",
        "Kestrel Components",
        "Meridian Packaging",
        "Oakline Logistics",
    ];
    let buyers = ["Priya Raman", "Tom Becker", "Lena Ortiz", "Sam Okafor"];
    let approvers = ["Chris Hale", "Mara Lindqvist"];
    let companies = ["US01", "DE02"];
    let total = 1_500;
    // Fixed, evenly spread orders get a second payment.
    let duplicates: Vec<usize> = (0..P2P_DUPLICATE_PAYMENTS)
        .map(|n| 37 + n * (total - 74) / P2P_DUPLICATE_PAYMENTS)
        .collect();
    let mut cases = Vec::with_capacity(total);
    for n in 0..total {
        let vendor = *rng.pick(&vendors);
        let buyer = *rng.pick(&buyers);
        let amount = if rng.chance(0.22) {
            rng.range(26_000.0, 140_000.0)
        } else {
            rng.range(300.0, 24_000.0)
        };
        let mut case = Case {
            id: format!("PO-{:05}", 40_001 + n),
            fields: vec![
                vendor.into(),
                buyer.into(),
                (*rng.pick(&companies)).into(),
                format!("{amount:.2}"),
            ],
            steps: Vec::new(),
            step_values: Vec::new(),
        };
        let mut clock = Clock(start_time(&mut rng));
        step(&mut case, "Create Purchase Requisition", clock.0, buyer);
        let at = clock.after(&mut rng, 2.0, 30.0);
        // Spelled out: Rust 1.88 cannot infer this inside the call.
        let approver = *rng.pick(&approvers);
        step(&mut case, "Approve Requisition", at, approver);
        let at = clock.after(&mut rng, 1.0, 20.0);
        step(&mut case, "Create Purchase Order", at, buyer);
        if amount > P2P_APPROVAL_LIMIT {
            // The story: one approver, and the large orders wait for them.
            let at = clock.after(&mut rng, 6.0 * 24.0, 14.0 * 24.0);
            step(&mut case, "Approve Purchase Order", at, P2P_SLOW_APPROVER);
        }
        let at = clock.after(&mut rng, 1.0, 12.0);
        step(&mut case, "Send Order to Vendor", at, buyer);
        if rng.chance(0.03) && !duplicates.contains(&n) {
            let at = clock.after(&mut rng, 24.0, 96.0);
            step(&mut case, "Cancel Order", at, buyer);
            cases.push(case);
            continue;
        }
        let at = clock.after(&mut rng, 3.0 * 24.0, 12.0 * 24.0);
        step(&mut case, "Receive Goods", at, "Warehouse");
        let at = clock.after(&mut rng, 1.0 * 24.0, 6.0 * 24.0);
        step(&mut case, "Receive Invoice", at, "Accounts Payable");
        let at = clock.after(&mut rng, 2.0, 30.0);
        step(&mut case, "Match Invoice", at, "Accounts Payable");
        if vendor == P2P_PRICE_VENDOR && rng.chance(0.35) {
            // The vendor's invoices often disagree with the order price.
            let at = clock.after(&mut rng, 24.0, 4.0 * 24.0);
            step(&mut case, "Change Price", at, buyer);
            let at = clock.after(&mut rng, 4.0, 48.0);
            step(&mut case, "Match Invoice", at, "Accounts Payable");
        }
        let at = clock.after(&mut rng, 3.0 * 24.0, 10.0 * 24.0);
        step(&mut case, "Pay Invoice", at, "Payment Run");
        if duplicates.contains(&n) {
            let at = clock.after(&mut rng, 3.0 * 24.0, 20.0 * 24.0);
            step(&mut case, "Pay Invoice", at, "Payment Run");
        }
        cases.push(case);
    }
    for case in &mut cases {
        case.step_values = p2p_status(case);
    }
    cases
}

/// The order's status after each step, as a purchasing system would show it:
/// large orders sit in "Awaiting approval"; invoices that disagree on price
/// sit in "Blocked: price" until the price is changed.
fn p2p_status(case: &Case) -> Vec<String> {
    let large = case.fields[3].parse::<f64>().unwrap_or(0.0) > P2P_APPROVAL_LIMIT;
    let steps = &case.steps;
    steps
        .iter()
        .enumerate()
        .map(|(index, (activity, ..))| {
            let next = steps.get(index + 1).map(|(next, ..)| next.as_str());
            match activity.as_str() {
                "Create Purchase Requisition" | "Approve Requisition" => "Requested",
                "Create Purchase Order" if large => "Awaiting approval",
                "Create Purchase Order" | "Approve Purchase Order" | "Send Order to Vendor" => {
                    "Ordered"
                }
                "Cancel Order" => "Cancelled",
                "Receive Goods" => "Received",
                "Receive Invoice" => "Invoiced",
                "Match Invoice" if next == Some("Change Price") => "Blocked: price",
                "Change Price" => "Blocked: price",
                "Match Invoice" => "Ready to pay",
                _ => "Paid",
            }
            .to_owned()
        })
        .collect()
}

pub(super) const O2C_FIELDS: [&str; 4] = ["customer_type", "warehouse", "region", "order_value"];
pub(super) const O2C_SLOW_WAREHOUSE: &str = "Reno";

/// Order to cash: 1,200 orders.
pub(super) fn order_to_cash(seed: u64) -> Vec<Case> {
    let mut rng = Rng::new(seed);
    let warehouses = ["Reno", "Columbus", "Atlanta", "Dallas"];
    let regions = ["West", "Midwest", "South", "Northeast"];
    let reps = ["Ana Silva", "Ben Carter", "Ivy Chen"];
    let mut cases = Vec::with_capacity(1_200);
    for n in 0..1_200 {
        let new_customer = rng.chance(0.3);
        let warehouse = *rng.pick(&warehouses);
        let value = rng.range(150.0, 18_000.0);
        let rep = *rng.pick(&reps);
        let mut case = Case {
            id: format!("SO-{:05}", 70_001 + n),
            fields: vec![
                if new_customer { "New" } else { "Existing" }.into(),
                warehouse.into(),
                (*rng.pick(&regions)).into(),
                format!("{value:.2}"),
            ],
            steps: Vec::new(),
            step_values: Vec::new(),
        };
        let mut clock = Clock(start_time(&mut rng));
        step(&mut case, "Receive Order", clock.0, rep);
        let at = clock.after(&mut rng, 0.5, 6.0);
        step(&mut case, "Check Credit", at, "Credit Control");
        if new_customer {
            // The story: new customers wait on a credit hold.
            let at = clock.after(&mut rng, 0.2, 2.0);
            step(&mut case, "Credit Hold", at, "Credit Control");
            if rng.chance(0.12) {
                let at = clock.after(&mut rng, 2.0 * 24.0, 6.0 * 24.0);
                step(&mut case, "Cancel Order", at, rep);
                cases.push(case);
                continue;
            }
            let at = clock.after(&mut rng, 3.0 * 24.0, 8.0 * 24.0);
            step(&mut case, "Release Credit Hold", at, "Credit Control");
        }
        let at = clock.after(&mut rng, 1.0, 10.0);
        step(&mut case, "Confirm Order", at, rep);
        if rng.chance(0.07) {
            let at = clock.after(&mut rng, 12.0, 48.0);
            step(&mut case, "Change Delivery Date", at, rep);
        }
        let at = clock.after(&mut rng, 4.0, 30.0);
        step(&mut case, "Pick Items", at, warehouse);
        let at = clock.after(&mut rng, 1.0, 8.0);
        step(&mut case, "Pack Items", at, warehouse);
        // The story: Reno ships days after packing; the others the same day or next.
        let at = if warehouse == O2C_SLOW_WAREHOUSE {
            clock.after(&mut rng, 4.0 * 24.0, 7.0 * 24.0)
        } else {
            clock.after(&mut rng, 3.0, 26.0)
        };
        step(&mut case, "Ship Order", at, warehouse);
        let at = clock.after(&mut rng, 2.0, 24.0);
        step(&mut case, "Create Invoice", at, "Billing");
        let at = clock.after(&mut rng, 6.0 * 24.0, 18.0 * 24.0);
        step(&mut case, "Receive Payment", at, "Cash Application");
        cases.push(case);
    }
    cases
}

pub(super) const SALES_FIELDS: [&str; 4] = ["lead_source", "region", "owner", "deal_size"];
pub(super) const SALES_DISCOUNT_LIMIT: f64 = 50_000.0;

/// Sales pipeline, lead to closed opportunity: 1,600 leads.
pub(super) fn sales_pipeline(seed: u64) -> Vec<Case> {
    let mut rng = Rng::new(seed);
    let sources = ["Webinar", "Website", "Referral", "Trade Show"];
    let regions = ["AMER", "EMEA", "APAC"];
    let owners = ["Jordan Price", "Aisha Khan", "Mateo Rossi", "Nora Blake"];
    let mut cases = Vec::with_capacity(1_600);
    for n in 0..1_600 {
        let source = *rng.pick(&sources);
        let owner = *rng.pick(&owners);
        let size = if rng.chance(0.25) {
            rng.range(55_000.0, 260_000.0)
        } else {
            rng.range(4_000.0, 48_000.0)
        };
        let mut case = Case {
            id: format!("LEAD-{:05}", 10_001 + n),
            fields: vec![
                source.into(),
                (*rng.pick(&regions)).into(),
                owner.into(),
                format!("{size:.0}"),
            ],
            steps: Vec::new(),
            step_values: Vec::new(),
        };
        let mut clock = Clock(start_time(&mut rng));
        step(&mut case, "Lead Created", clock.0, "Marketing");
        // The story: webinar leads need several attempts and stall before qualification.
        let attempts = if source == "Webinar" {
            2 + (rng.next() % 3) as usize
        } else {
            1
        };
        for _ in 0..attempts {
            let at = if source == "Webinar" {
                clock.after(&mut rng, 3.0 * 24.0, 9.0 * 24.0)
            } else {
                clock.after(&mut rng, 2.0, 36.0)
            };
            step(&mut case, "Contact Attempt", at, owner);
        }
        let disqualify = if source == "Webinar" { 0.45 } else { 0.25 };
        if rng.chance(disqualify) {
            let at = clock.after(&mut rng, 24.0, 5.0 * 24.0);
            step(&mut case, "Lead Disqualified", at, owner);
            cases.push(case);
            continue;
        }
        let at = clock.after(&mut rng, 4.0, 3.0 * 24.0);
        step(&mut case, "Lead Qualified", at, owner);
        let at = clock.after(&mut rng, 1.0, 24.0);
        step(&mut case, "Opportunity Created", at, owner);
        let at = clock.after(&mut rng, 2.0 * 24.0, 8.0 * 24.0);
        step(&mut case, "Discovery Call", at, owner);
        let at = clock.after(&mut rng, 3.0 * 24.0, 12.0 * 24.0);
        step(&mut case, "Proposal Sent", at, owner);
        if size > SALES_DISCOUNT_LIMIT {
            // The story: big deals go round the discount approval more than once.
            let rounds = 1 + (rng.next() % 3) as usize;
            for round in 0..rounds {
                let at = clock.after(&mut rng, 1.0 * 24.0, 4.0 * 24.0);
                step(&mut case, "Discount Requested", at, owner);
                let at = clock.after(&mut rng, 2.0 * 24.0, 7.0 * 24.0);
                let outcome = if round + 1 == rounds {
                    "Discount Approved"
                } else {
                    "Discount Rejected"
                };
                step(&mut case, outcome, at, "Deal Desk");
            }
        }
        let at = clock.after(&mut rng, 3.0 * 24.0, 15.0 * 24.0);
        step(&mut case, "Negotiation", at, owner);
        let at = clock.after(&mut rng, 2.0 * 24.0, 20.0 * 24.0);
        let won = rng.chance(if size > SALES_DISCOUNT_LIMIT {
            0.45
        } else {
            0.55
        });
        step(
            &mut case,
            if won { "Closed Won" } else { "Closed Lost" },
            at,
            owner,
        );
        cases.push(case);
    }
    cases
}

pub(super) const CONNECTED_LATE_GROUP: &str = "Retail";
pub(super) const CONNECTED_ORDERS: usize = 1_500;

/// Connected order to cash as linked records (OCEL 2.0 JSON), made to look
/// like a real extract rather than a tidy example: 1,500 orders from 300
/// customers, credit checks and blocks, changed and cancelled orders, orders
/// shipped in one to three deliveries with blocks, partial picks, failed
/// deliveries and returns, invoices per order or per delivery (some raised
/// before the goods arrive), invoice blocks, disputes, credit memos,
/// reminders, partial payments and invoices never paid.
pub(super) fn connected(seed: u64) -> serde_json::Value {
    use serde_json::json;
    let mut rng = Rng::new(seed);
    let groups = ["Retail", "Wholesale", "Distributor", "Key Account"];
    let mut objects = Vec::new();
    let mut events: Vec<serde_json::Value> = Vec::new();
    let mut event_id = 0usize;
    let mut add = |events: &mut Vec<serde_json::Value>,
                   kind: &str,
                   at: NaiveDateTime,
                   links: &[(&str, &str)]| {
        event_id += 1;
        events.push(json!({
            "id": format!("e{event_id}"),
            "type": kind,
            "time": format!("{}Z", at.format("%Y-%m-%dT%H:%M:%S")),
            "attributes": [],
            "relationships": links.iter().map(|(id, role)| json!({"objectId": id, "qualifier": role})).collect::<Vec<_>>(),
        }));
    };
    let attribute = |name: &str, value: serde_json::Value| json!({"name": name, "time": "1970-01-01T00:00:00Z", "value": value});
    let money = |value: f64| json!((value * 100.0).round() / 100.0);
    let customers: Vec<(String, &str)> = (0..300)
        .map(|n| (format!("customer_{:03}", n + 1), *rng.pick(&groups)))
        .collect();
    for (id, group) in &customers {
        objects.push(json!({"id": id, "type": "Customer", "attributes": [attribute("customer_group", json!(group))]}));
    }
    let (mut deliveries_made, mut invoices_made, mut payments_made) = (0usize, 0usize, 0usize);
    for n in 0..CONNECTED_ORDERS {
        let (customer, group) = rng.pick(&customers).clone();
        let order = format!("order_{:04}", n + 1);
        let value = rng.range(200.0, 25_000.0);
        objects.push(json!({"id": order, "type": "Order", "attributes": [attribute("order_value", money(value))]}));
        let mut clock = Clock(start_time(&mut rng));
        add(
            &mut events,
            "Create Order",
            clock.0,
            &[(&order, "order"), (&customer, "customer")],
        );
        // Credit: a third are checked; some of those are blocked, and a few
        // blocked orders are rejected outright.
        if rng.chance(0.35) {
            let at = clock.after(&mut rng, 0.5, 8.0);
            add(
                &mut events,
                "Check Credit",
                at,
                &[(&order, "order"), (&customer, "customer")],
            );
            if rng.chance(0.3) {
                let at = clock.after(&mut rng, 0.2, 4.0);
                add(&mut events, "Set Credit Block", at, &[(&order, "order")]);
                if rng.chance(0.15) {
                    let at = clock.after(&mut rng, 24.0, 5.0 * 24.0);
                    add(
                        &mut events,
                        "Reject Order",
                        at,
                        &[(&order, "order"), (&customer, "customer")],
                    );
                    continue;
                }
                let at = clock.after(&mut rng, 1.0 * 24.0, 6.0 * 24.0);
                add(
                    &mut events,
                    "Release Credit Block",
                    at,
                    &[(&order, "order")],
                );
            }
        }
        // Rush orders ship before they are formally approved.
        let rush = rng.chance(0.03);
        if !rush {
            let at = clock.after(&mut rng, 1.0, 20.0);
            add(&mut events, "Approve Order", at, &[(&order, "order")]);
        }
        // Customers change their minds: once for some, twice for a few.
        let changes = if rng.chance(0.07) {
            if rng.chance(0.25) { 2 } else { 1 }
        } else {
            0
        };
        for _ in 0..changes {
            let at = clock.after(&mut rng, 2.0, 48.0);
            add(&mut events, "Change Order", at, &[(&order, "order")]);
        }
        if rng.chance(0.03) {
            let at = clock.after(&mut rng, 4.0, 72.0);
            add(
                &mut events,
                "Cancel Order",
                at,
                &[(&order, "order"), (&customer, "customer")],
            );
            continue;
        }
        let parts = if rng.chance(0.1) {
            3
        } else if rng.chance(0.28) {
            2
        } else {
            1
        };
        let per_delivery_invoices = parts > 1 && rng.chance(0.3);
        let advance_invoice = rng.chance(0.06);
        let mut deliveries = Vec::new();
        let mut delivered_at = Vec::new();
        let mut returned = false;
        for part in 0..parts {
            deliveries_made += 1;
            let delivery = format!("delivery_{deliveries_made:05}");
            objects.push(json!({"id": delivery, "type": "Delivery", "attributes": [attribute("part", json!(part + 1))]}));
            let mut track = Clock(clock.0);
            // Later parts follow days later: the back-ordered goods.
            let at = track.after(
                &mut rng,
                2.0 + part as f64 * 60.0,
                20.0 + part as f64 * 150.0,
            );
            add(
                &mut events,
                "Create Delivery",
                at,
                &[(&delivery, "delivery"), (&order, "order")],
            );
            if rush && part == 0 {
                let at = track.after(&mut rng, 0.5, 6.0);
                add(&mut events, "Approve Order", at, &[(&order, "order")]);
            }
            if rng.chance(0.04) {
                let at = track.after(&mut rng, 0.5, 6.0);
                add(
                    &mut events,
                    "Set Delivery Block",
                    at,
                    &[(&delivery, "delivery")],
                );
                let at = track.after(&mut rng, 24.0, 4.0 * 24.0);
                add(
                    &mut events,
                    "Remove Delivery Block",
                    at,
                    &[(&delivery, "delivery")],
                );
            }
            if changes > 0 && part == 0 && rng.chance(0.25) {
                // A change that lands after the delivery is already planned.
                let at = track.after(&mut rng, 1.0, 12.0);
                add(
                    &mut events,
                    "Change Order",
                    at,
                    &[(&order, "order"), (&delivery, "delivery")],
                );
            }
            let at = track.after(&mut rng, 2.0, 20.0);
            add(&mut events, "Pick Goods", at, &[(&delivery, "delivery")]);
            if rng.chance(0.06) {
                // Not everything was on the shelf: a second pick.
                let at = track.after(&mut rng, 12.0, 72.0);
                add(&mut events, "Pick Goods", at, &[(&delivery, "delivery")]);
            }
            let at = track.after(&mut rng, 1.0, 8.0);
            add(&mut events, "Pack Goods", at, &[(&delivery, "delivery")]);
            if rng.chance(0.035) {
                let at = track.after(&mut rng, 24.0, 72.0);
                add(
                    &mut events,
                    "Reschedule Delivery",
                    at,
                    &[(&delivery, "delivery")],
                );
            }
            let at = track.after(&mut rng, 2.0, 24.0);
            add(
                &mut events,
                "Post Goods Issue",
                at,
                &[(&delivery, "delivery")],
            );
            if rng.chance(0.025) {
                // Nobody was there to receive it: back and out again.
                let at = track.after(&mut rng, 24.0, 72.0);
                add(
                    &mut events,
                    "Delivery Failed",
                    at,
                    &[(&delivery, "delivery")],
                );
                let at = track.after(&mut rng, 24.0, 96.0);
                add(
                    &mut events,
                    "Post Goods Issue",
                    at,
                    &[(&delivery, "delivery")],
                );
            }
            let at = track.after(&mut rng, 24.0, 4.0 * 24.0);
            add(
                &mut events,
                "Confirm Delivery",
                at,
                &[(&delivery, "delivery"), (&order, "order")],
            );
            if rng.chance(0.02) {
                let at = track.after(&mut rng, 2.0 * 24.0, 9.0 * 24.0);
                add(
                    &mut events,
                    "Return Goods",
                    at,
                    &[(&delivery, "delivery"), (&customer, "customer")],
                );
                returned = true;
            }
            delivered_at.push(track.0);
            deliveries.push(delivery);
        }
        // Invoices: one per order, or one per delivery; a few are raised in
        // advance, before the goods arrive.
        let bills: Vec<(Vec<String>, NaiveDateTime, f64)> = if per_delivery_invoices {
            deliveries
                .iter()
                .zip(&delivered_at)
                .map(|(d, at)| (vec![d.clone()], *at, value / parts as f64))
                .collect()
        } else {
            let at = if advance_invoice {
                clock.0 + Duration::hours(6)
            } else {
                *delivered_at.iter().max().unwrap_or(&clock.0)
            };
            vec![(deliveries.clone(), at, value)]
        };
        let mut all_paid_at = clock.0;
        let mut all_paid = true;
        for (linked, from, amount) in bills {
            invoices_made += 1;
            let invoice = format!("invoice_{invoices_made:05}");
            objects.push(json!({"id": invoice, "type": "Invoice", "attributes": [attribute("invoice_amount", money(amount))]}));
            let mut bill = Clock(from);
            let at = bill.after(&mut rng, 2.0, 30.0);
            let mut links = vec![(invoice.as_str(), "invoice"), (order.as_str(), "order")];
            for delivery in &linked {
                links.push((delivery.as_str(), "delivery"));
            }
            add(&mut events, "Create Invoice", at, &links);
            if rng.chance(0.05) {
                // Price on the invoice does not match the order.
                let at = bill.after(&mut rng, 1.0, 12.0);
                add(&mut events, "Block Invoice", at, &[(&invoice, "invoice")]);
                let at = bill.after(&mut rng, 24.0, 6.0 * 24.0);
                add(
                    &mut events,
                    "Release Invoice Block",
                    at,
                    &[(&invoice, "invoice")],
                );
            }
            let at = bill.after(&mut rng, 1.0, 12.0);
            add(
                &mut events,
                "Send Invoice",
                at,
                &[(&invoice, "invoice"), (&customer, "customer")],
            );
            if returned {
                let at = bill.after(&mut rng, 24.0, 5.0 * 24.0);
                add(
                    &mut events,
                    "Create Credit Memo",
                    at,
                    &[(&invoice, "invoice"), (&customer, "customer")],
                );
            }
            if rng.chance(0.03) {
                let at = bill.after(&mut rng, 3.0 * 24.0, 15.0 * 24.0);
                add(
                    &mut events,
                    "Dispute Invoice",
                    at,
                    &[(&invoice, "invoice"), (&customer, "customer")],
                );
                let at = bill.after(&mut rng, 4.0 * 24.0, 20.0 * 24.0);
                add(&mut events, "Resolve Dispute", at, &[(&invoice, "invoice")]);
                if rng.chance(0.4) {
                    let at = bill.after(&mut rng, 4.0, 72.0);
                    add(
                        &mut events,
                        "Create Credit Memo",
                        at,
                        &[(&invoice, "invoice"), (&customer, "customer")],
                    );
                }
            }
            if rng.chance(0.04) {
                // Never paid inside the extract.
                all_paid = false;
                continue;
            }
            // The story: retail customers pay weeks later, most after a
            // reminder, some after two.
            let late = group == CONNECTED_LATE_GROUP;
            let reminders = if late {
                if rng.chance(0.7) {
                    if rng.chance(0.35) { 2 } else { 1 }
                } else {
                    0
                }
            } else if rng.chance(0.08) {
                1
            } else {
                0
            };
            for _ in 0..reminders {
                let at = bill.after(&mut rng, 25.0 * 24.0, 35.0 * 24.0);
                add(
                    &mut events,
                    "Send Reminder",
                    at,
                    &[(&invoice, "invoice"), (&customer, "customer")],
                );
            }
            // Some pay in two instalments.
            let instalments = if rng.chance(0.08) { 2 } else { 1 };
            for instalment in 0..instalments {
                payments_made += 1;
                let payment = format!("payment_{payments_made:05}");
                objects.push(json!({"id": payment, "type": "Payment", "attributes": [attribute("payment_amount", money(amount / instalments as f64))]}));
                let at = if instalment == 0 {
                    if late {
                        bill.after(&mut rng, 10.0 * 24.0, 25.0 * 24.0)
                    } else {
                        bill.after(&mut rng, 6.0 * 24.0, 22.0 * 24.0)
                    }
                } else {
                    bill.after(&mut rng, 7.0 * 24.0, 30.0 * 24.0)
                };
                add(
                    &mut events,
                    "Receive Payment",
                    at,
                    &[
                        (&payment, "payment"),
                        (&invoice, "invoice"),
                        (&customer, "customer"),
                    ],
                );
                let at = bill.after(&mut rng, 4.0, 72.0);
                add(
                    &mut events,
                    "Clear Payment",
                    at,
                    &[(&payment, "payment"), (&invoice, "invoice")],
                );
            }
            all_paid_at = all_paid_at.max(bill.0);
        }
        if all_paid {
            let at = Clock(all_paid_at).after(&mut rng, 1.0, 48.0);
            add(&mut events, "Close Order", at, &[(&order, "order")]);
        }
    }
    let mut kinds: Vec<String> = events
        .iter()
        .filter_map(|event| event["type"].as_str().map(str::to_owned))
        .collect();
    kinds.sort();
    kinds.dedup();
    let object_types = [
        ("Customer", "customer_group", "string"),
        ("Order", "order_value", "float"),
        ("Delivery", "part", "integer"),
        ("Invoice", "invoice_amount", "float"),
        ("Payment", "payment_amount", "float"),
    ];
    json!({
        "objectTypes": object_types.iter().map(|(name, field, kind)| json!({"name": name, "attributes": [{"name": field, "type": kind}]})).collect::<Vec<_>>(),
        "eventTypes": kinds.iter().map(|name| json!({"name": name, "attributes": []})).collect::<Vec<_>>(),
        "objects": objects,
        "events": events,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn days(a: NaiveDateTime, b: NaiveDateTime) -> f64 {
        (b - a).num_minutes() as f64 / 1440.0
    }

    fn gap(case: &Case, from: &str, to: &str) -> Option<f64> {
        let a = case.steps.iter().position(|s| s.0 == from)?;
        let b = case.steps.iter().position(|s| s.0 == to)?;
        Some(days(case.steps[a].1, case.steps[b].1))
    }

    fn mean(values: &[f64]) -> f64 {
        values.iter().sum::<f64>() / values.len().max(1) as f64
    }

    #[test]
    fn the_same_seed_gives_the_same_data() {
        assert_eq!(
            to_csv("purchase_order", &P2P_FIELDS, &purchase_to_pay(7)),
            to_csv("purchase_order", &P2P_FIELDS, &purchase_to_pay(7))
        );
        assert_eq!(connected(7), connected(7));
    }

    #[test]
    fn purchase_to_pay_plants_a_slow_approver_and_duplicate_payments() {
        let cases = purchase_to_pay(7);
        assert_eq!(cases.len(), 1_500);
        let paid_twice = cases
            .iter()
            .filter(|c| c.steps.iter().filter(|s| s.0 == "Pay Invoice").count() == 2)
            .count();
        assert_eq!(paid_twice, P2P_DUPLICATE_PAYMENTS);
        let approvals: Vec<f64> = cases
            .iter()
            .filter_map(|c| gap(c, "Create Purchase Order", "Approve Purchase Order"))
            .collect();
        assert!(approvals.len() > 200, "{}", approvals.len());
        assert!(mean(&approvals) > 6.0);
        assert!(cases.iter().all(|c| {
            let amount: f64 = c.fields[3].parse().unwrap();
            c.steps.iter().any(|s| s.0 == "Approve Purchase Order") == (amount > P2P_APPROVAL_LIMIT)
        }));
        assert!(
            cases
                .iter()
                .flat_map(|c| &c.steps)
                .filter(|s| s.0 == "Approve Purchase Order")
                .all(|s| s.2 == P2P_SLOW_APPROVER)
        );
        let price_changes: Vec<&Case> = cases
            .iter()
            .filter(|c| c.steps.iter().any(|s| s.0 == "Change Price"))
            .collect();
        assert!(
            !price_changes.is_empty()
                && price_changes
                    .iter()
                    .all(|c| c.fields[0] == P2P_PRICE_VENDOR)
        );
    }

    /// The order status changes from step to step, so the import keeps it
    /// on each step: large orders pass through "Awaiting approval", the
    /// price-mismatch invoices through "Blocked: price".
    #[test]
    fn purchase_to_pay_status_changes_per_step() {
        let cases = purchase_to_pay(7);
        assert!(cases.iter().all(|c| c.step_values.len() == c.steps.len()));
        let passed = |c: &Case, status: &str| c.step_values.iter().any(|s| s == status);
        assert!(cases.iter().all(|c| {
            passed(c, "Awaiting approval")
                == c.steps.iter().any(|s| s.0 == "Approve Purchase Order")
        }));
        assert!(cases.iter().all(|c| {
            passed(c, "Blocked: price") == c.steps.iter().any(|s| s.0 == "Change Price")
        }));
        let csv = to_csv("purchase_order", &P2P_FIELDS, &cases[..1]);
        assert!(csv.lines().next().unwrap().ends_with(",order_status"));
        assert!(csv.lines().nth(1).unwrap().ends_with(",Requested"));
    }

    #[test]
    fn order_to_cash_holds_new_customers_and_reno_ships_late() {
        let cases = order_to_cash(7);
        assert!(
            cases
                .iter()
                .all(|c| c.steps.iter().any(|s| s.0 == "Credit Hold") == (c.fields[0] == "New"))
        );
        let ship = |slow: bool| -> f64 {
            mean(
                &cases
                    .iter()
                    .filter(|c| (c.fields[1] == O2C_SLOW_WAREHOUSE) == slow)
                    .filter_map(|c| gap(c, "Pack Items", "Ship Order"))
                    .collect::<Vec<_>>(),
            )
        };
        assert!(
            ship(true) > 4.0 && ship(false) < 1.2,
            "{} {}",
            ship(true),
            ship(false)
        );
    }

    #[test]
    fn sales_pipeline_stalls_webinar_leads_and_loops_big_discounts() {
        let cases = sales_pipeline(7);
        let to_qualify = |webinar: bool| -> f64 {
            mean(
                &cases
                    .iter()
                    .filter(|c| (c.fields[0] == "Webinar") == webinar)
                    .filter_map(|c| gap(c, "Lead Created", "Lead Qualified"))
                    .collect::<Vec<_>>(),
            )
        };
        assert!(to_qualify(true) > 3.0 * to_qualify(false));
        for case in &cases {
            let size: f64 = case.fields[3].parse().unwrap();
            let rounds = case
                .steps
                .iter()
                .filter(|s| s.0 == "Discount Requested")
                .count();
            if size <= SALES_DISCOUNT_LIMIT {
                assert_eq!(rounds, 0);
            }
        }
        assert!(cases.iter().any(|c| {
            c.steps
                .iter()
                .filter(|s| s.0 == "Discount Requested")
                .count()
                > 1
        }));
    }

    #[test]
    fn connected_links_records_and_retail_pays_late() {
        let log = connected(7);
        let objects = log["objects"].as_array().unwrap();
        let count = |kind: &str| objects.iter().filter(|o| o["type"] == kind).count();
        assert_eq!(count("Order"), CONNECTED_ORDERS);
        // One to three deliveries an order: well over one each on average.
        assert!(
            count("Delivery") > 1_700 && count("Delivery") < 2_300,
            "{}",
            count("Delivery")
        );
        assert!(count("Invoice") > count("Order") * 9 / 10);
        assert!(count("Payment") > count("Invoice") * 9 / 10);
        let events = log["events"].as_array().unwrap();
        // Every relationship points at a real object.
        let ids: std::collections::HashSet<&str> =
            objects.iter().map(|o| o["id"].as_str().unwrap()).collect();
        assert!(
            events
                .iter()
                .flat_map(|e| e["relationships"].as_array().unwrap())
                .all(|r| ids.contains(r["objectId"].as_str().unwrap()))
        );
        // The real-life mess is all there.
        for kind in [
            "Check Credit",
            "Set Credit Block",
            "Release Credit Block",
            "Reject Order",
            "Change Order",
            "Cancel Order",
            "Set Delivery Block",
            "Remove Delivery Block",
            "Pick Goods",
            "Pack Goods",
            "Reschedule Delivery",
            "Post Goods Issue",
            "Delivery Failed",
            "Return Goods",
            "Block Invoice",
            "Release Invoice Block",
            "Dispute Invoice",
            "Resolve Dispute",
            "Create Credit Memo",
            "Send Reminder",
            "Receive Payment",
            "Clear Payment",
            "Close Order",
        ] {
            assert!(events.iter().any(|e| e["type"] == kind), "{kind}");
        }
        // Retail invoices are paid later than the others.
        let group: std::collections::HashMap<&str, &str> = objects
            .iter()
            .filter(|o| o["type"] == "Customer")
            .map(|o| {
                (
                    o["id"].as_str().unwrap(),
                    o["attributes"][0]["value"].as_str().unwrap(),
                )
            })
            .collect();
        let time = |e: &serde_json::Value| {
            NaiveDateTime::parse_from_str(e["time"].as_str().unwrap(), "%Y-%m-%dT%H:%M:%SZ")
                .unwrap()
        };
        let mut sent = std::collections::HashMap::new();
        let (mut late, mut other) = (Vec::new(), Vec::new());
        for e in events {
            let links = e["relationships"].as_array().unwrap();
            let invoice = links
                .iter()
                .find(|r| r["qualifier"] == "invoice")
                .and_then(|r| r["objectId"].as_str());
            let customer = links
                .iter()
                .find(|r| r["qualifier"] == "customer")
                .and_then(|r| r["objectId"].as_str());
            match (e["type"].as_str(), invoice, customer) {
                (Some("Send Invoice"), Some(i), _) => {
                    sent.insert(i, time(e));
                }
                (Some("Receive Payment"), Some(i), Some(c)) => {
                    if let Some(at) = sent.remove(i) {
                        let days = days(at, time(e));
                        if group[c] == CONNECTED_LATE_GROUP {
                            late.push(days)
                        } else {
                            other.push(days)
                        }
                    }
                }
                _ => {}
            }
        }
        assert!(
            mean(&late) > mean(&other) + 15.0,
            "{} {}",
            mean(&late),
            mean(&other)
        );
    }

    #[test]
    fn csv_rows_carry_the_item_fields() {
        let csv = to_csv("lead_id", &SALES_FIELDS, &sales_pipeline(7)[..1]);
        let mut lines = csv.lines();
        assert_eq!(
            lines.next().unwrap(),
            "lead_id,activity,timestamp,resource,lead_source,region,owner,deal_size"
        );
        assert!(
            lines
                .next()
                .unwrap()
                .starts_with("LEAD-10001,Lead Created,2025-")
        );
    }
}
