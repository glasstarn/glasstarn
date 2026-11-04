//! `workspace demo <purchase-to-pay|order-to-cash|connected|sales-pipeline>`:
//! a ready project on invented data, generated on the spot (see `invented`).
//!
//! `workspace demo bpic2019 <file>`: the five-minute demo on public data. It
//! creates a project and imports the BPI Challenge 2019 purchasing log with
//! the mapping and answers a person would confirm, so the first look at the
//! product needs no setup. The guide is docs/demo-bpic2019.md.
use super::{import, root_option};
use pm_project::workspace::{InvestigationPurpose, WorkspaceFolder};
use std::process::ExitCode;

/// The published file's SHA-256 (4TU.ResearchData, BPI Challenge 2019).
const BPIC_2019_SHA256: &str = "af63bc687fc4152f2123b05c3af7772b37ef3fce2d3f67f812666c9e356baae7";

/// The order line's own fields, kept so cases can be compared by them.
const CASE_ATTRIBUTES: [&str; 15] = [
    "Company",
    "Document Type",
    "GR-Based Inv. Verif.",
    "Goods Receipt",
    "Item Category",
    "Item Type",
    "Spend area text",
    "Sub spend area text",
    "Spend classification text",
    "Vendor",
    "Name",
    "Purch. Doc. Category name",
    "Purchasing Document",
    "Source",
    "Cumulative net worth (EUR)",
];

pub(super) fn command(args: &[String]) -> Result<ExitCode, String> {
    let (root, args) = root_option(args)?;
    let mut rest = args.iter().filter(|a| *a != "--json");
    match rest.next().map(String::as_str) {
        Some("bpic2019") => {}
        Some(name) if INVENTED.iter().any(|(id, ..)| *id == name) => {
            if rest.next().is_some() {
                return Err(format!(
                    "workspace demo {name} takes no file: its data is generated"
                ));
            }
            return invented_demo(&root, name);
        }
        _ => {
            return Err(format!(
                "workspace demo expects one of: {} (invented data), or bpic2019 <path to BPI_Challenge_2019.xes>",
                INVENTED
                    .iter()
                    .map(|(id, ..)| *id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let file = rest
        .next()
        .ok_or("workspace demo bpic2019 needs the downloaded BPI_Challenge_2019.xes")?;
    if rest.next().is_some() {
        return Err("workspace demo bpic2019 accepts one file".into());
    }
    match sha256(file) {
        Ok(sha) if sha == BPIC_2019_SHA256 => {}
        Ok(_) => eprintln!(
            "note: this is not the published BPI Challenge 2019 file; the demo runs, but its numbers will differ from the guide"
        ),
        Err(error) => return Err(format!("cannot read {file}: {error}")),
    }
    let folder = WorkspaceFolder::create_named(
        &root,
        "BPIC 2019 purchasing (demo)",
        InvestigationPurpose::Improvement,
    )
    .map_err(|error| error.to_string())?;
    // The batch accounts are SAP background jobs; a person would confirm
    // that when the automation view asks. The demo confirms it up front.
    // The import opens the project itself; this handle holds its lock.
    let project = folder.root.clone();
    drop(folder);
    let rules = project.join("demo-rules.json");
    let kinds = (0..20)
        .map(|n| format!("\"batch_{n:02}\": \"system\""))
        .collect::<Vec<_>>()
        .join(", ");
    // "NONE" is the log's word for no recorded user.
    std::fs::write(
        &rules,
        format!("{{\"resource_kinds\": {{{kinds}, \"NONE\": \"unrecorded\"}}}}"),
    )
    .map_err(|error| error.to_string())?;
    let mut import_args = vec![
        "--workspace-dir".to_owned(),
        root.display().to_string(),
        project.display().to_string(),
        file.clone(),
        "--case".into(),
        "case_id".into(),
        "--activity".into(),
        "activity".into(),
        "--timestamp".into(),
        "timestamp".into(),
        "--resource".into(),
        "resource".into(),
        // The log repeats rows with the same line, step and minute; the
        // source records them as separate events, so they are kept.
        "--answer".into(),
        "duplicate_rows=keep_as_rework".into(),
        "--rules".into(),
        rules.display().to_string(),
    ];
    for attribute in CASE_ATTRIBUTES {
        import_args.push("--case-attribute".into());
        import_args.push(attribute.into());
    }
    let code = import(&import_args)?;
    eprintln!(
        "Demo project ready: {}\nSee it in your browser with `glasstarn`, or ask your agent the questions in docs/demo-bpic2019.md.",
        project.display()
    );
    Ok(code)
}

/// The invented demos: command name, project name, and what to look for.
const INVENTED: [(&str, &str, &str); 4] = [
    (
        "purchase-to-pay",
        "Purchase to pay (demo)",
        "Large orders wait on one approver; some invoices are paid twice; one vendor's invoices bounce on price.",
    ),
    (
        "order-to-cash",
        "Order to cash (demo)",
        "New customers wait on a credit hold; one warehouse ships days later than the rest.",
    ),
    (
        "connected",
        "Order to cash, connected (demo)",
        "Linked orders, deliveries, invoices and payments with real-life mess: credit blocks, changes, cancellations, split and failed deliveries, returns, invoice blocks, disputes, partial payments. Retail customers pay weeks late, many after reminders.",
    ),
    (
        "sales-pipeline",
        "Sales pipeline (demo)",
        "Leads from webinars stall before qualification; big deals loop on discount approval.",
    ),
];

/// The fixed seed: every demo is the same on every machine.
const SEED: u64 = 2025;

fn invented_demo(root: &std::path::Path, name: &str) -> Result<ExitCode, String> {
    use super::invented;
    let (_, title, story) = INVENTED
        .iter()
        .find(|(id, ..)| *id == name)
        .ok_or("unknown demo")?;
    let scratch = tempfile::tempdir().map_err(|error| error.to_string())?;
    let (file, (fields, case_column)): (std::path::PathBuf, (&[&str], &str)) = match name {
        "connected" => {
            let path = scratch.path().join("order-to-cash-connected.jsonocel");
            let text = serde_json::to_string(&invented::connected(SEED))
                .map_err(|error| error.to_string())?;
            std::fs::write(&path, text).map_err(|error| error.to_string())?;
            (path, (&[], ""))
        }
        _ => {
            // The item column is named for the business object, so the app
            // calls the items "purchase orders" or "leads".
            let (cases, fields, file_name, case_column): (_, &[&str], _, _) = match name {
                "purchase-to-pay" => (
                    invented::purchase_to_pay(SEED),
                    &invented::P2P_FIELDS,
                    "purchase-to-pay.csv",
                    "purchase_order",
                ),
                "order-to-cash" => (
                    invented::order_to_cash(SEED),
                    &invented::O2C_FIELDS,
                    "order-to-cash.csv",
                    "sales_order",
                ),
                _ => (
                    invented::sales_pipeline(SEED),
                    &invented::SALES_FIELDS,
                    "sales-pipeline.csv",
                    "lead_id",
                ),
            };
            let path = scratch.path().join(file_name);
            std::fs::write(&path, invented::to_csv(case_column, fields, &cases))
                .map_err(|error| error.to_string())?;
            (path, (fields, case_column))
        }
    };
    let folder = WorkspaceFolder::create_named(root, title, InvestigationPurpose::Improvement)
        .map_err(|error| error.to_string())?;
    let project = folder.root.clone();
    drop(folder);
    let mut import_args = vec![
        "--workspace-dir".to_owned(),
        root.display().to_string(),
        project.display().to_string(),
        file.display().to_string(),
    ];
    if !fields.is_empty() {
        for (flag, column) in [
            ("--case", case_column),
            ("--activity", "activity"),
            ("--timestamp", "timestamp"),
            ("--resource", "resource"),
        ] {
            import_args.push(flag.into());
            import_args.push(column.into());
        }
        for field in fields {
            import_args.push("--case-attribute".into());
            import_args.push((*field).into());
        }
    }
    let code = import(&import_args)?;
    eprintln!(
        "Demo project ready: {}\nInvented data, no real company. What to look for: {story}\nSee it in your browser with `glasstarn`, or ask your agent about it.",
        project.display()
    );
    Ok(code)
}

fn sha256(path: &str) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
