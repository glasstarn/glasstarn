//! Made-up SAP tables for production, maintenance and journal entries, in the
//! standard layout: technical field names as headers, dates as `YYYYMMDD`,
//! times as `HHMMSS`, document numbers with leading zeros. Every record is
//! invented; the counts in the truth structs are known by construction.

use crate::Rng;
use crate::csv::{Table, save};
use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime};
use std::io;
use std::path::Path;

/// Settings for one generated set of tables.
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

fn start() -> NaiveDateTime {
    NaiveDate::from_ymd_opt(2026, 1, 5)
        .unwrap()
        .and_hms_opt(7, 0, 0)
        .unwrap()
}

fn date(at: NaiveDateTime) -> String {
    at.format("%Y%m%d").to_string()
}

fn time(at: NaiveDateTime) -> String {
    at.format("%H%M%S").to_string()
}

/// Status change documents (JCDS), numbered per object.
struct Statuses {
    table: Table,
}

impl Statuses {
    fn new() -> Self {
        Self {
            table: Table::new("OBJNR,STAT,CHGNR,UDATE,UTIME,INACT,USNAM,CHIND"),
        }
    }
    fn set(&mut self, object: &str, status: &str, number: u32, at: NaiveDateTime) {
        self.table.push(&[
            object,
            status,
            &format!("{number:03}"),
            &date(at),
            &time(at),
            "",
            "PLANNER1",
            if number == 1 { "I" } else { "U" },
        ]);
    }
}

/// Goods movements (MSEG), numbered as material documents.
struct Movements {
    table: Table,
    number: u64,
}

impl Movements {
    fn new() -> Self {
        Self {
            table: Table::new(
                "MBLNR,MJAHR,ZEILE,BWART,AUFNR,MATNR,MENGE,CPUDT_MKPF,CPUTM_MKPF,WERKS,LGORT,MEINS,USNAM_MKPF",
            ),
            number: 0,
        }
    }
    fn post(&mut self, kind: &str, order: &str, material: &str, quantity: u64, at: NaiveDateTime) {
        self.number += 1;
        self.table.push(&[
            &format!("49{:08}", self.number),
            "2026",
            "0001",
            kind,
            order,
            material,
            &quantity.to_string(),
            &date(at),
            &time(at),
            "1000",
            "0001",
            "ST",
            "WAREHOUSE1",
        ]);
    }
}

/// What the production tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ProductionTruth {
    pub orders: u64,
    pub released: u64,
    pub component_issues: u64,
    pub component_reversals: u64,
    pub partial_confirmations: u64,
    pub final_confirmations: u64,
    pub cancelled_confirmations: u64,
    pub goods_receipts: u64,
    pub goods_receipt_reversals: u64,
    pub completed: u64,
    pub closed: u64,
    pub deletion_flags: u64,
    /// Maintenance orders in AUFK, which the production pack must skip.
    pub other_orders: u64,
    /// Orders converted from a planned order of the planning run.
    pub from_planning: u64,
    /// Orders made for a sales order line.
    pub made_to_order: u64,
}

pub const PRODUCTION_TABLES: [&str; 6] = ["AUFK", "AFKO", "AFPO", "AFRU", "MSEG", "JCDS"];

/// Writes AUFK, AFKO, AFPO, AFRU, MSEG and JCDS for production orders.
pub fn write_production(dir: &Path, settings: &Settings) -> io::Result<ProductionTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x9E0D);
    let mut truth = ProductionTruth::default();
    let mut orders = Table::new("AUFNR,AUTYP,AUART,ERDAT,ERFZEIT,BUKRS,WERKS,ERNAM,OBJNR");
    let mut headers = Table::new("AUFNR,PLNBEZ,GAMNG,GSTRP,GLTRP,DISPO,FEVOR,GMEIN,GSTRI,GETRI");
    let mut items = Table::new("AUFNR,POSNR,MATNR,PSMNG,DGLTP,PLNUM,KDAUF,KDPOS,WEMNG,LTRMI,DWERK");
    let mut confirmations = Table::new(
        "RUECK,RMZHL,AUFNR,VORNR,ERSDA,ERZET,AUERU,STZHL,ERNAM,LMNGA,XMNGA,STOKZ,BUDAT,ARBID",
    );
    let mut movements = Movements::new();
    let mut statuses = Statuses::new();
    let materials = ["FG-PUMP-100", "FG-VALVE-200", "FG-MOTOR-300"];
    for n in 0..settings.records {
        truth.orders += 1;
        let order = format!("{:012}", 1_000_000 + n);
        let object = format!("OR{order}");
        let created = start()
            + Duration::days(rng.below(90) as i64)
            + Duration::minutes(rng.below(9 * 60) as i64);
        let material = materials[rng.below(3) as usize];
        let quantity = 10 * (1 + rng.below(10));
        let planned_start = created + Duration::days(2 + rng.below(5) as i64);
        let planned_finish = planned_start + Duration::days(3 + rng.below(4) as i64);
        orders.push(&[
            &order,
            "10",
            "PP01",
            &date(created),
            &time(created),
            "1000",
            "1000",
            "PLANNER1",
            &object,
        ]);
        let mut at = created;
        let mut status_number = 0u32;
        let mut set_status = |statuses: &mut Statuses, status: &str, at: NaiveDateTime| {
            status_number += 1;
            statuses.set(&object, status, status_number, at);
        };
        // Created status, not a step the pack reads.
        set_status(&mut statuses, "I0001", created);
        let released = rng.chance(0.92);
        let mut finished = String::from("00000000");
        let mut actual_start = String::from("00000000");
        if released {
            at = planned_start - Duration::hours(rng.below(30) as i64);
            truth.released += 1;
            set_status(&mut statuses, "I0002", at);
            actual_start = date(at);
            // Components, sometimes one taken back.
            for line in 0..1 + rng.below(3) {
                at += Duration::minutes(20 + rng.below(240) as i64);
                truth.component_issues += 1;
                movements.post("261", &order, &format!("RM-{line:03}"), quantity, at);
            }
            if rng.chance(0.08) {
                at += Duration::hours(1);
                truth.component_reversals += 1;
                movements.post("262", &order, "RM-000", quantity / 2, at);
            }
            let mut counter = 0u64;
            let mut confirm = |confirmations: &mut Table,
                               operation: &str,
                               last: bool,
                               cancels: Option<u64>,
                               at: NaiveDateTime| {
                counter += 1;
                confirmations.push(&[
                    &format!("{:010}", 5_000_000 + n),
                    &format!("{counter:08}"),
                    &order,
                    operation,
                    &date(at),
                    &time(at),
                    if last { "X" } else { "" },
                    &cancels.map_or("00000000".to_owned(), |c| format!("{c:08}")),
                    "OPERATOR1",
                    &quantity.to_string(),
                    "0",
                    "",
                    &date(at),
                    "10000001",
                ]);
                counter
            };
            let operations = ["0010", "0020", "0030"];
            let done = if rng.chance(0.85) {
                3
            } else {
                rng.below(3) as usize
            };
            for (index, operation) in operations.iter().enumerate().take(done) {
                at += Duration::hours(2 + rng.below(20) as i64);
                if rng.chance(0.3) {
                    truth.partial_confirmations += 1;
                    confirm(&mut confirmations, operation, false, None, at);
                    at += Duration::hours(1 + rng.below(8) as i64);
                }
                truth.final_confirmations += 1;
                let number = confirm(&mut confirmations, operation, true, None, at);
                if index == 0 && rng.chance(0.05) {
                    // A wrong confirmation, cancelled and confirmed again.
                    at += Duration::minutes(30);
                    truth.cancelled_confirmations += 1;
                    confirm(&mut confirmations, operation, false, Some(number), at);
                    at += Duration::minutes(10);
                    truth.final_confirmations += 1;
                    confirm(&mut confirmations, operation, true, None, at);
                }
            }
            if done == 3 {
                at += Duration::hours(1 + rng.below(12) as i64);
                truth.goods_receipts += 1;
                movements.post("101", &order, material, quantity, at);
                finished = date(at);
                if rng.chance(0.04) {
                    at += Duration::hours(2);
                    truth.goods_receipt_reversals += 1;
                    movements.post("102", &order, material, quantity, at);
                    at += Duration::hours(1);
                    truth.goods_receipts += 1;
                    movements.post("101", &order, material, quantity, at);
                }
                if rng.chance(0.8) {
                    at += Duration::days(1 + rng.below(10) as i64);
                    truth.completed += 1;
                    set_status(&mut statuses, "I0045", at);
                    if rng.chance(0.6) {
                        at += Duration::days(20 + rng.below(15) as i64);
                        truth.closed += 1;
                        set_status(&mut statuses, "I0046", at);
                    }
                }
            }
        } else if rng.chance(0.5) {
            at += Duration::days(3);
            truth.deletion_flags += 1;
            set_status(&mut statuses, "I0076", at);
        }
        // Most orders come from the planning run; some are made for a sales
        // order line; the rest were created by hand.
        let planned = rng.chance(0.7);
        let for_sale = !planned && rng.chance(0.5);
        truth.from_planning += u64::from(planned);
        truth.made_to_order += u64::from(for_sale);
        items.push(&[
            &order,
            "0001",
            material,
            &quantity.to_string(),
            &date(planned_finish),
            &if planned {
                format!("{:010}", 30_000 + n)
            } else {
                String::new()
            },
            &if for_sale {
                format!("{:010}", 1_200_000 + n)
            } else {
                String::new()
            },
            if for_sale { "000010" } else { "" },
            &quantity.to_string(),
            &finished,
            "1000",
        ]);
        headers.push(&[
            &order,
            material,
            &quantity.to_string(),
            &date(planned_start),
            &date(planned_finish),
            "001",
            "PRD",
            "ST",
            &actual_start,
            &finished,
        ]);
    }
    // Maintenance orders share AUFK; the production pack must leave them out.
    for n in 0..settings.records / 10 {
        truth.other_orders += 1;
        let order = format!("{:012}", 4_000_000 + n);
        let at = start() + Duration::days(n as i64 % 60);
        orders.push(&[
            &order,
            "30",
            "PM01",
            &date(at),
            &time(at),
            "1000",
            "1000",
            "PLANNER2",
            &format!("OR{order}"),
        ]);
    }
    save(
        dir,
        &[
            ("AUFK", &orders),
            ("AFKO", &headers),
            ("AFPO", &items),
            ("AFRU", &confirmations),
            ("MSEG", &movements.table),
            ("JCDS", &statuses.table),
        ],
    )?;
    Ok(truth)
}

/// What the maintenance tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct MaintenanceTruth {
    pub notifications: u64,
    pub breakdowns: u64,
    pub equipment: u64,
    pub orders: u64,
    /// Orders raised from a notification.
    pub orders_from_notifications: u64,
    pub released: u64,
    pub spare_part_issues: u64,
    pub spare_part_returns: u64,
    pub partial_confirmations: u64,
    pub final_confirmations: u64,
    pub cancelled_confirmations: u64,
    pub completed: u64,
    pub closed: u64,
    pub notifications_in_process: u64,
    pub notifications_completed: u64,
    /// Production orders in AUFK, which the maintenance pack must skip.
    pub other_orders: u64,
}

pub const MAINTENANCE_TABLES: [&str; 8] = [
    "QMEL", "QMIH", "EQUI", "AUFK", "AFIH", "AFRU", "MSEG", "JCDS",
];

/// Writes QMEL, QMIH, EQUI, AUFK, AFIH, AFRU, MSEG and JCDS for maintenance.
pub fn write_maintenance(dir: &Path, settings: &Settings) -> io::Result<MaintenanceTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x3A1E);
    let mut truth = MaintenanceTruth::default();
    let mut notifications = Table::new("QMNUM,QMART,QMDAT,MZEIT,PRIOK,AUFNR,ERNAM,OBJNR");
    let mut details = Table::new("QMNUM,EQUNR,MSAUS,AUSVN,AUZTV,AUSBS,AUZTB");
    let mut equipment = Table::new("EQUNR,EQTYP,ERDAT,HERST,TYPBZ");
    let mut orders = Table::new("AUFNR,AUTYP,AUART,ERDAT,ERFZEIT,BUKRS,WERKS,ERNAM,OBJNR");
    let mut headers = Table::new("AUFNR,EQUNR,QMNUM,ILART,PRIOK,IWERK");
    let mut confirmations =
        Table::new("RUECK,RMZHL,AUFNR,VORNR,ERSDA,ERZET,AUERU,STZHL,ERNAM,ISMNW,STOKZ,BUDAT");
    let mut movements = Movements::new();
    let mut statuses = Statuses::new();
    let machines = 1 + settings.records / 8;
    for m in 0..machines {
        truth.equipment += 1;
        equipment.push(&[
            &format!("{:018}", 10_000_000 + m),
            "M",
            "20200101",
            "Pumpenwerk",
            ["P-200", "C-40", "K-9"][m as usize % 3],
        ]);
    }
    let mut order_number = 0u64;
    let mut new_order = |rng: &mut Rng,
                         truth: &mut MaintenanceTruth,
                         orders: &mut Table,
                         headers: &mut Table,
                         confirmations: &mut Table,
                         movements: &mut Movements,
                         statuses: &mut Statuses,
                         machine: &str,
                         notification: &str,
                         created: NaiveDateTime|
     -> (String, NaiveDateTime) {
        order_number += 1;
        truth.orders += 1;
        let order = format!("{:012}", 4_000_000 + order_number);
        let object = format!("OR{order}");
        orders.push(&[
            &order,
            "30",
            if notification.is_empty() {
                "PM02"
            } else {
                "PM01"
            },
            &date(created),
            &time(created),
            "1000",
            "1000",
            "PLANNER2",
            &object,
        ]);
        headers.push(&[
            &order,
            machine,
            notification,
            if notification.is_empty() {
                "PRV"
            } else {
                "REP"
            },
            if notification.is_empty() { "3" } else { "1" },
            "1000",
        ]);
        let mut number = 0u32;
        statuses.set(&object, "I0001", 1, created);
        number += 1;
        let mut at = created;
        if !rng.chance(0.93) {
            return (order, at);
        }
        at += Duration::hours(1 + rng.below(24) as i64);
        truth.released += 1;
        number += 1;
        statuses.set(&object, "I0002", number, at);
        if rng.chance(0.6) {
            at += Duration::hours(1 + rng.below(6) as i64);
            truth.spare_part_issues += 1;
            movements.post("261", &order, "SP-SEAL-01", 2, at);
            if rng.chance(0.1) {
                at += Duration::hours(3);
                truth.spare_part_returns += 1;
                movements.post("262", &order, "SP-SEAL-01", 1, at);
            }
        }
        let mut counter = 0u64;
        let mut confirm =
            |table: &mut Table, last: bool, cancels: Option<u64>, at: NaiveDateTime| {
                counter += 1;
                table.push(&[
                    &format!("{:010}", 7_000_000 + order_number),
                    &format!("{counter:08}"),
                    &order,
                    "0010",
                    &date(at),
                    &time(at),
                    if last { "X" } else { "" },
                    &cancels.map_or("00000000".to_owned(), |c| format!("{c:08}")),
                    "TECH1",
                    "2.5",
                    "",
                    &date(at),
                ]);
                counter
            };
        if rng.chance(0.4) {
            at += Duration::hours(2 + rng.below(10) as i64);
            truth.partial_confirmations += 1;
            confirm(confirmations, false, None, at);
        }
        if !rng.chance(0.9) {
            return (order, at);
        }
        at += Duration::hours(2 + rng.below(30) as i64);
        truth.final_confirmations += 1;
        let first = confirm(confirmations, true, None, at);
        if rng.chance(0.05) {
            at += Duration::minutes(40);
            truth.cancelled_confirmations += 1;
            confirm(confirmations, false, Some(first), at);
            at += Duration::minutes(15);
            truth.final_confirmations += 1;
            confirm(confirmations, true, None, at);
        }
        if rng.chance(0.85) {
            at += Duration::hours(4 + rng.below(72) as i64);
            truth.completed += 1;
            number += 1;
            statuses.set(&object, "I0045", number, at);
            if rng.chance(0.5) {
                at += Duration::days(10 + rng.below(20) as i64);
                truth.closed += 1;
                number += 1;
                statuses.set(&object, "I0046", number, at);
            }
        }
        (order, at)
    };
    for n in 0..settings.records {
        truth.notifications += 1;
        let notification = format!("{:012}", 10_000_000 + n);
        let object = format!("QM{notification}");
        let machine = format!("{:018}", 10_000_000 + rng.below(machines));
        let reported = start()
            + Duration::days(rng.below(90) as i64)
            + Duration::minutes(rng.below(14 * 60) as i64);
        let breakdown = rng.chance(0.3);
        let (down, up) = if breakdown {
            truth.breakdowns += 1;
            let down = reported - Duration::minutes(5 + rng.below(90) as i64);
            (
                (date(down), time(down)),
                ("00000000".to_owned(), "000000".to_owned()),
            )
        } else {
            (
                ("00000000".to_owned(), "000000".to_owned()),
                ("00000000".to_owned(), "000000".to_owned()),
            )
        };
        details.push(&[
            &notification,
            &machine,
            if breakdown { "X" } else { "" },
            &down.0,
            &down.1,
            &up.0,
            &up.1,
        ]);
        statuses.set(&object, "I0068", 1, reported);
        let mut order = String::new();
        if rng.chance(0.8) {
            let created = reported + Duration::minutes(15 + rng.below(240) as i64);
            truth.orders_from_notifications += 1;
            let (number, done) = new_order(
                &mut rng,
                &mut truth,
                &mut orders,
                &mut headers,
                &mut confirmations,
                &mut movements,
                &mut statuses,
                &machine,
                &notification,
                created,
            );
            order = number;
            truth.notifications_in_process += 1;
            statuses.set(&object, "I0070", 2, created + Duration::minutes(1));
            if rng.chance(0.7) {
                truth.notifications_completed += 1;
                statuses.set(&object, "I0072", 3, done + Duration::hours(1));
            }
        }
        notifications.push(&[
            &notification,
            "M2",
            &date(reported),
            &time(reported),
            if breakdown { "1" } else { "3" },
            &order,
            "OPERATOR2",
            &object,
        ]);
    }
    // Preventive orders without a notification.
    for _ in 0..settings.records / 5 {
        let machine = format!("{:018}", 10_000_000 + rng.below(machines));
        let created = start() + Duration::days(rng.below(90) as i64) + Duration::hours(1);
        new_order(
            &mut rng,
            &mut truth,
            &mut orders,
            &mut headers,
            &mut confirmations,
            &mut movements,
            &mut statuses,
            &machine,
            "",
            created,
        );
    }
    // Production orders share AUFK; the maintenance pack must leave them out.
    for n in 0..settings.records / 10 {
        truth.other_orders += 1;
        let order = format!("{:012}", 1_000_000 + n);
        let at = start() + Duration::days(n as i64 % 60);
        orders.push(&[
            &order,
            "10",
            "PP01",
            &date(at),
            &time(at),
            "1000",
            "1000",
            "PLANNER1",
            &format!("OR{order}"),
        ]);
    }
    save(
        dir,
        &[
            ("QMEL", &notifications),
            ("QMIH", &details),
            ("EQUI", &equipment),
            ("AUFK", &orders),
            ("AFIH", &headers),
            ("AFRU", &confirmations),
            ("MSEG", &movements.table),
            ("JCDS", &statuses.table),
        ],
    )?;
    Ok(truth)
}

/// What the journal-entry tables contain.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct JournalTruth {
    /// Documents entered, not counting reversal documents.
    pub entries: u64,
    pub reversals: u64,
    /// Changed fields in the change log.
    pub changes: u64,
    pub cleared_items: u64,
    /// Entries entered after the period they post to had ended.
    pub entered_after_period: u64,
    /// Entries parked before posting, including those still parked.
    pub parked: u64,
    /// Parked entries not yet posted: parked, never entered.
    pub still_parked: u64,
    /// Completed approval work items.
    pub approvals: u64,
}

pub const JOURNAL_TABLES: [&str; 7] = [
    "BKPF",
    "VBKPF",
    "SWWWIHEAD",
    "SWW_CONTOB",
    "BSAS",
    "CDHDR",
    "CDPOS",
];

/// Writes BKPF, VBKPF, SWWWIHEAD, SWW_CONTOB, BSAS, CDHDR and CDPOS for
/// accounting documents.
pub fn write_journal_entries(dir: &Path, settings: &Settings) -> io::Result<JournalTruth> {
    let mut rng = Rng::new(settings.seed ^ 0x2E20);
    let mut truth = JournalTruth::default();
    let mut headers = Table::new(
        "BUKRS,BELNR,GJAHR,BLART,BUDAT,CPUDT,CPUTM,USNAM,MONAT,XREVERSAL,STBLG,STJAH,BLDAT,TCODE,AWTYP,BSTAT,WAERS",
    );
    let mut cleared = Table::new("BUKRS,HKONT,AUGDT,AUGBL,ZUONR,GJAHR,BELNR,BUZEI,DMBTR,SHKZG");
    let mut change_headers = Table::new("OBJECTCLAS,OBJECTID,CHANGENR,USERNAME,UDATE,UTIME");
    let mut change_items =
        Table::new("OBJECTCLAS,OBJECTID,CHANGENR,TABNAME,TABKEY,FNAME,VALUE_NEW,VALUE_OLD");
    let mut parked = Table::new("BUKRS,BELNR,GJAHR,CPUDT,CPUTM,USNAM,BLART,BUDAT,BSTAT");
    let mut work_items = Table::new(
        "WI_ID,WI_TYPE,WI_STAT,WI_AED,WI_AED_TIMESTAMP,WI_RH_TASK,WI_AAGENT,WI_CD,WI_CT",
    );
    let mut work_item_objects = Table::new("WI_ID,OBJTYPE,OBJKEY");
    let mut work_item_number = 0u64;
    let mut number = 0u64;
    let mut change_number = 0u64;
    let kinds = [
        ("SA", "FB50"),
        ("SA", "FB01"),
        ("KR", "MIRO"),
        ("RV", "VF01"),
    ];
    let mut reversals = Vec::new();
    for _ in 0..settings.records {
        number += 1;
        truth.entries += 1;
        let document = format!("{:010}", 100_000_000 + number);
        let (kind, transaction) = kinds[rng.below(4) as usize];
        let entered = start()
            + Duration::days(rng.below(85) as i64)
            + Duration::minutes(60 + rng.below(9 * 60) as i64);
        // Manual entries in the first days of a month often post to the
        // month before: the close.
        let mut posting = entered.date();
        if kind == "SA" && entered.day() <= 5 {
            posting = entered.date().with_day(1).unwrap() - Duration::days(1);
            truth.entered_after_period += 1;
        }
        // Manual entries are often parked and approved before posting; a few
        // are still waiting.
        let mut still_parked = false;
        if kind == "SA" && rng.chance(0.3) {
            truth.parked += 1;
            let parked_at = entered - Duration::hours(4 + rng.below(72) as i64);
            parked.push(&[
                "1000",
                &document,
                "2026",
                &date(parked_at),
                &time(parked_at),
                "ACCOUNTANT3",
                kind,
                &posting.format("%Y%m%d").to_string(),
                "V",
            ]);
            still_parked = rng.chance(0.1);
            work_item_number += 1;
            let work_item = format!("{:012}", 900_000 + work_item_number);
            let decided = parked_at + Duration::hours(1 + rng.below(3) as i64);
            work_items.push(&[
                &work_item,
                "W",
                if still_parked { "READY" } else { "COMPLETED" },
                &if still_parked {
                    "00000000".to_owned()
                } else {
                    date(decided)
                },
                &if still_parked {
                    "0.0000000".to_owned()
                } else {
                    decided.format("%Y%m%d%H%M%S.0000000").to_string()
                },
                "TS00007947",
                if still_parked { "" } else { "CONTROLLER1" },
                &date(parked_at),
                &time(parked_at),
            ]);
            work_item_objects.push(&[&work_item, "FIPP", &format!("1000{document}2026")]);
            if still_parked {
                truth.still_parked += 1;
                truth.entries -= 1;
            } else {
                truth.approvals += 1;
            }
        }
        let reversal =
            (!still_parked && rng.chance(0.06)).then(|| format!("{:010}", 200_000_000 + number));
        if let Some(reversal) = &reversal {
            reversals.push((reversal.clone(), document.clone(), entered));
        }
        headers.push(&[
            "1000",
            &document,
            "2026",
            kind,
            &posting.format("%Y%m%d").to_string(),
            &date(entered),
            &time(entered),
            if kind == "SA" {
                "ACCOUNTANT1"
            } else {
                "BATCHJOB"
            },
            &posting.format("%m").to_string(),
            if reversal.is_some() { "1" } else { "" },
            reversal.as_deref().unwrap_or(""),
            if reversal.is_some() { "2026" } else { "" },
            &posting.format("%Y%m%d").to_string(),
            transaction,
            if kind == "SA" { "BKPF" } else { "RMRP" },
            if still_parked { "V" } else { "" },
            "EUR",
        ]);
        if still_parked {
            continue;
        }
        if rng.chance(0.1) {
            change_number += 1;
            truth.changes += 1;
            let at = entered + Duration::hours(2 + rng.below(48) as i64);
            let object = format!("100{}{}2026", "1000", document);
            let change = format!("{change_number:010}");
            change_headers.push(&[
                "BELEG",
                &object,
                &change,
                "ACCOUNTANT2",
                &date(at),
                &time(at),
            ]);
            change_items.push(&[
                "BELEG",
                &object,
                &change,
                "BKPF",
                &format!("1001000{document}2026"),
                "BKTXT",
                "Accrual Q1",
                "Accrual",
            ]);
        }
        if rng.chance(0.4) {
            let at = entered + Duration::days(3 + rng.below(30) as i64);
            for item in ["001", "002"] {
                truth.cleared_items += 1;
                cleared.push(&[
                    "1000",
                    "191100",
                    &date(at),
                    &format!("{:010}", 1_500_000_000 + number),
                    "",
                    "2026",
                    &document,
                    item,
                    "1250.00",
                    if item == "001" { "S" } else { "H" },
                ]);
            }
        }
    }
    // Reversals: a new document pointing back at the original, which is
    // flagged as reversed.
    for (reversal, document, entered) in reversals {
        truth.reversals += 1;
        let at = entered + Duration::hours(4 + rng.below(100) as i64);
        headers.push(&[
            "1000",
            &reversal,
            "2026",
            "AB",
            &date(at),
            &date(at),
            &time(at),
            "ACCOUNTANT1",
            &at.format("%m").to_string(),
            "2",
            &document,
            "2026",
            &date(at),
            "FB08",
            "BKPF",
            "",
            "EUR",
        ]);
    }
    save(
        dir,
        &[
            ("BKPF", &headers),
            ("VBKPF", &parked),
            ("SWWWIHEAD", &work_items),
            ("SWW_CONTOB", &work_item_objects),
            ("BSAS", &cleared),
            ("CDHDR", &change_headers),
            ("CDPOS", &change_items),
        ],
    )?;
    Ok(truth)
}
