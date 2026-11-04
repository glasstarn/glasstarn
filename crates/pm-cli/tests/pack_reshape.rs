//! An export as a business user makes it, from Excel, in the shape of the
//! Appian record list export found on a real workspace: title rows, column
//! labels instead of the pack's names, local times with the user's name under
//! them. The inventory reads the workbook, the check proposes column matches,
//! and the reshape command rewrites it into the file the pack reads.
use serde_json::Value;
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};

fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_glasstarn"))
        .args(args)
        .arg("--workspace-dir")
        .arg(root.join("projects"))
        .output()
        .unwrap()
}

fn success(output: Output) -> Value {
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// A workbook of one sheet with inline strings, stored without compression.
fn workbook(path: &Path, rows: &[&[&str]]) {
    let escape = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut sheet = String::from(
        r#"<?xml version="1.0" encoding="UTF-8"?><worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
    );
    for (r, row) in rows.iter().enumerate() {
        sheet.push_str(&format!("<row r=\"{}\">", r + 1));
        for (c, cell) in row.iter().enumerate() {
            if cell.is_empty() {
                continue;
            }
            let reference = format!("{}{}", (b'A' + c as u8) as char, r + 1);
            if !cell.is_empty() && cell.parse::<f64>().is_ok() {
                sheet.push_str(&format!("<c r=\"{reference}\"><v>{cell}</v></c>"));
            } else {
                sheet.push_str(&format!(
                    "<c r=\"{reference}\" t=\"inlineStr\"><is><t>{}</t></is></c>",
                    escape(cell)
                ));
            }
        }
        sheet.push_str("</row>");
    }
    sheet.push_str("</sheetData></worksheet>");
    workbook_parts(path, &[("xl/worksheets/sheet1.xml", sheet.into_bytes())]);
}

fn workbook_parts(path: &Path, parts: &[(&str, Vec<u8>)]) {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (name, data) in parts {
        let offset = zip.len() as u32;
        let header = |signature: u32| {
            let mut h = signature.to_le_bytes().to_vec();
            h.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name.len() as u16).to_le_bytes());
            h.extend_from_slice(&[0, 0]);
            h
        };
        zip.extend(header(0x0403_4b50));
        zip.extend_from_slice(name.as_bytes());
        zip.extend_from_slice(data);
        // Central entries carry "version made by" ahead of the local fields.
        let mut c = 0x0201_4b50u32.to_le_bytes().to_vec();
        c.extend_from_slice(&[20, 0]);
        c.extend(header(0)[4..].iter());
        c.extend_from_slice(&[0; 10]);
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.push(c);
    }
    let start = zip.len() as u32;
    let size = central.iter().map(Vec::len).sum::<usize>() as u32;
    for c in &central {
        zip.extend_from_slice(c);
    }
    zip.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    zip.extend_from_slice(&[0, 0, 0, 0]);
    zip.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    zip.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    zip.extend_from_slice(&size.to_le_bytes());
    zip.extend_from_slice(&start.to_le_bytes());
    zip.extend_from_slice(&[0, 0]);
    fs::write(path, zip).unwrap();
}

#[test]
fn native_excel_dates_respect_styles_epochs_and_keep_identifiers_and_amounts() {
    // Calendar oracle: Microsoft documents serial 39448 as 2008-01-01.
    // The 1904 epoch is 1462 days later. All values below are hand chosen.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("dates.xlsx");
    let styles = r#"<styleSheet><numFmts count="3"><numFmt numFmtId="164" formatCode="yyyy-mm-dd hh:mm:ss"/><numFmt numFmtId="165" formatCode="0.00 &quot;days&quot;"/><numFmt numFmtId="166" formatCode="[h]:mm:ss"/></numFmts><cellXfs count="6"><xf numFmtId="0"/><xf numFmtId="14"/><xf numFmtId="164"/><xf numFmtId="21"/><xf numFmtId="165"/><xf numFmtId="166"/></cellXfs></styleSheet>"#;
    for (epoch, serial) in [(false, 39448), (true, 37986)] {
        let sheet = format!(
            r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>0000123</t></is></c><c r="B1" s="1"><v>{serial}</v></c><c r="C1" s="2"><v>{serial}.5</v></c><c r="D1" s="3"><v>0.25</v></c><c r="E1"><v>39448.0</v></c><c r="F1" s="1" t="inlineStr"><is><t>00000000</t></is></c><c r="G1" s="4"><v>12.5</v></c><c r="H1" s="5"><v>2.5</v></c><c r="I1" s="1"><f>DATE(2008,1,1)</f><v>{serial}</v></c><c r="J1" t="d" s="1"><v>2008-01-01T00:00:00.000Z</v></c><c r="K1" t="d" s="2"><v>2008-01-01T12:00:00.000Z</v></c></row></sheetData></worksheet>"#
        );
        workbook_parts(
            &path,
            &[
                ("xl/styles.xml", styles.as_bytes().to_vec()),
                (
                    "xl/workbook.xml",
                    format!(r#"<workbook><workbookPr date1904="{epoch}"/></workbook>"#)
                        .into_bytes(),
                ),
                ("xl/worksheets/sheet1.xml", sheet.into_bytes()),
            ],
        );
        assert_eq!(
            pm_ingest::xlsx::read(&path).unwrap()[0],
            [
                "0000123",
                "2008-01-01",
                "2008-01-01T12:00:00",
                "06:00:00",
                "39448",
                "00000000",
                "12.5",
                "2.5",
                "2008-01-01",
                "2008-01-01",
                "2008-01-01T12:00:00.000Z"
            ]
        );
    }
}

#[test]
fn an_excel_export_with_labels_and_local_times_becomes_the_packs_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = success(cli(root, &["workspace", "create", "--name", "Rentals"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let exports = root.join("exports");
    fs::create_dir_all(&exports).unwrap();
    fs::write(
        exports.join("Record.csv"),
        "id,createdOn,status\n26,2026-10-15T17:40:00Z,Pending\n27,2026-10-15T17:45:00Z,Approved\n",
    )
    .unwrap();
    fs::write(
        exports.join("RecordEventType.csv"),
        "id,value\n1,Created\n2,Approved\n",
    )
    .unwrap();
    let events = exports.join("RecordEvent.xlsx");
    workbook(
        &events,
        &[
            &["Record Name:", "Rental Request Events", "", "", ""],
            &["Exported On:", "Oct 15, 2026 2:10 PM", "", "", ""],
            &["", "", "", "", ""],
            &[
                "Id",
                "Rental Request",
                "Event Type",
                "Event User",
                "Timestamp",
            ],
            &["1.0", "26.0", "1.0", "Pat Lee", "10/15/2026 1:47 PM"],
            &[
                "2.0",
                "26.0",
                "2.0",
                "Sam Roe",
                "10/15/2026 1:50 PM\nSam Roe",
            ],
            &["3.0", "27.0", "1.0", "Pat Lee", "10/15/2026 1:52 PM"],
        ],
    );

    // The workbook is read directly, from its header row, with whole
    // numbers as written.
    let inventory = success(cli(
        root,
        &[
            "workspace",
            "inventory",
            &id,
            exports.join("Record.csv").to_str().unwrap(),
            exports.join("RecordEventType.csv").to_str().unwrap(),
            events.to_str().unwrap(),
        ],
    ));
    assert_eq!(inventory["inputs"][2]["asset"]["name"], "RecordEvent.csv");

    // The check finds the export's names differ and proposes matches by name;
    // which column holds the record it cannot tell from names alone.
    let check: Value = serde_json::from_slice(
        &cli(
            root,
            &[
                "workspace",
                "pack",
                "check",
                &id,
                "--pack",
                "appian-record-events",
            ],
        )
        .stdout,
    )
    .unwrap();
    assert_eq!(check["status"], "export_incomplete");
    let matches = check["column_matches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["table"] == "RecordEvent")
        .unwrap_or_else(|| panic!("{check}"));
    assert_eq!(matches["suggested"]["eventTypeId"], "Event Type");
    assert_eq!(matches["suggested"]["timestamp"], "Timestamp");
    assert_eq!(matches["suggested"]["user"], "Event User");
    assert!(matches["columns"].to_string().contains("Rental Request"));

    // The person confirms; the times were US Eastern summer time.
    let reshaped = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "reshape",
            &id,
            "--pack",
            "appian-record-events",
            "--table",
            "RecordEvent",
            "--file",
            events.to_str().unwrap(),
            "--column",
            "recordId=Rental Request",
            "--column",
            "eventTypeId=Event Type",
            "--column",
            "user=Event User",
            "--column",
            "timestamp=Timestamp",
            "--column",
            "id=Id",
            "--time-zone",
            "-04:00",
        ],
    ));
    assert_eq!(reshaped["rows"], 3);
    assert_eq!(reshaped["times_rewritten"], 3);
    assert_eq!(reshaped["still_missing"].as_array().unwrap().len(), 0);
    let stored = fs::read_to_string(
        Path::new(created["project"]["path"].as_str().unwrap())
            .join(reshaped["asset"]["relative_path"].as_str().unwrap()),
    )
    .unwrap();
    assert_eq!(
        stored,
        "id,recordId,eventTypeId,user,timestamp\n\
         1,26,1,Pat Lee,2026-10-15T17:47:00Z\n\
         2,26,2,Sam Roe,2026-10-15T17:50:00Z\n\
         3,27,1,Pat Lee,2026-10-15T17:52:00Z\n"
    );

    // The rewritten copy is the newest and the one the check reads.
    let check = success(cli(
        root,
        &[
            "workspace",
            "pack",
            "check",
            &id,
            "--pack",
            "appian-record-events",
        ],
    ));
    assert!(
        check["column_matches"].as_array().unwrap().is_empty(),
        "{check}"
    );
    let audit = fs::read_to_string(
        Path::new(created["project"]["path"].as_str().unwrap())
            .join("audit")
            .join("log.jsonl"),
    )
    .unwrap_or_default();
    assert!(audit.contains("pack_reshape"), "{audit}");

    // A column that is not the pack's is refused.
    let wrong = cli(
        root,
        &[
            "workspace",
            "pack",
            "reshape",
            &id,
            "--pack",
            "appian-record-events",
            "--table",
            "RecordEvent",
            "--file",
            events.to_str().unwrap(),
            "--column",
            "caseNumber=Id",
        ],
    );
    assert!(!wrong.status.success());
}

/// Any file with a case, an event and a time is an event log: no pack, no
/// rewriting. An Excel export with title rows and twelve-hour times goes
/// straight into a project.
#[test]
fn a_plain_excel_event_log_imports_without_a_pack() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = success(cli(root, &["workspace", "create", "--name", "Claims"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let log = root.join("claims.xlsx");
    workbook(
        &log,
        &[
            &["Report:", "Claim steps", ""],
            &["", "", ""],
            &["Claim", "Step", "When"],
            &["C-1", "Received", "3/5/2026 9:41 AM"],
            &["C-1", "Assessed", "3/6/2026 2:15 PM"],
            &["C-1", "Paid", "3/20/2026 11:02 AM"],
            &["C-2", "Received", "3/7/2026 8:00 AM"],
            &["C-2", "Rejected", "3/9/2026 4:30 PM"],
        ],
    );
    let inspected = success(
        Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["workspace", "inspect", log.to_str().unwrap(), "--json"])
            .output()
            .unwrap(),
    );
    assert!(inspected["profile"]["roles"].to_string().contains("When"));
    let imported = success(cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            log.to_str().unwrap(),
            "--case",
            "Claim",
            "--activity",
            "Step",
            "--timestamp",
            "When",
        ],
    ));
    assert_eq!(imported["status"], "complete", "{imported}");
    let routes = imported["headline"]["routes"].to_string();
    assert!(
        routes.contains("Received") && routes.contains("Paid"),
        "{routes}"
    );
}
