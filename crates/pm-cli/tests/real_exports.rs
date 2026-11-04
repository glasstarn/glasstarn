//! Exports as admins really hand them over, found testing on a real
//! Business Central company: one workbook with a sheet per table (the
//! configuration-package export), an order archive beside the open orders,
//! quotes among the orders, no SystemCreatedAt, and text files in Windows
//! encodings with semicolons.
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

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "{}{}",
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

/// The CRC-32 a zip records for each file.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

/// A stored (uncompressed) zip of the given parts.
fn zip(path: &Path, parts: &[(String, Vec<u8>)]) {
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (name, data) in parts {
        let offset = out.len() as u32;
        let crc = crc32(data);
        let fields = |out: &mut Vec<u8>| {
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&[0, 0]);
        };
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        fields(&mut out);
        out.extend_from_slice(name.as_bytes());
        out.extend_from_slice(data);
        let mut c = 0x0201_4b50u32.to_le_bytes().to_vec();
        c.extend_from_slice(&[20, 0]);
        fields(&mut c);
        c.extend_from_slice(&[0; 10]);
        c.extend_from_slice(&offset.to_le_bytes());
        c.extend_from_slice(name.as_bytes());
        central.push(c);
    }
    let start = out.len() as u32;
    let size = central.iter().map(Vec::len).sum::<usize>() as u32;
    for c in &central {
        out.extend_from_slice(c);
    }
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&(parts.len() as u16).to_le_bytes());
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&start.to_le_bytes());
    out.extend_from_slice(&[0, 0]);
    fs::write(path, out).unwrap();
}

/// A workbook with named sheets of text cells, laid out as Business
/// Central's configuration package writes them: the package and table on
/// row 1, field names on row 2, data below.
fn package(path: &Path, sheets: &[(&str, &str, &[&[&str]])]) {
    let mut parts = Vec::new();
    let mut listed = String::new();
    let mut rels = String::new();
    for (n, (sheet, table, rows)) in sheets.iter().enumerate() {
        let mut xml = String::from(
            r#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        let first = ["GLASSTARN", *table, "0"];
        let all = std::iter::once(&first[..]).chain(rows.iter().copied());
        for (r, row) in all.enumerate() {
            xml.push_str(&format!("<row r=\"{}\">", r + 1));
            for (c, cell) in row.iter().enumerate() {
                if !cell.is_empty() {
                    xml.push_str(&format!(
                        "<c r=\"{}{}\" t=\"inlineStr\"><is><t>{cell}</t></is></c>",
                        (b'A' + c as u8) as char,
                        r + 1
                    ));
                }
            }
            xml.push_str("</row>");
        }
        xml.push_str("</sheetData></worksheet>");
        parts.push((
            format!("xl/worksheets/sheet{}.xml", n + 1),
            xml.into_bytes(),
        ));
        listed.push_str(&format!(
            r#"<sheet name="{sheet}" sheetId="{}" r:id="rId{}"/>"#,
            n + 1,
            n + 1
        ));
        rels.push_str(&format!(
            r#"<Relationship Id="rId{}" Target="worksheets/sheet{}.xml"/>"#,
            n + 1,
            n + 1
        ));
    }
    parts.push((
        "xl/workbook.xml".into(),
        format!(r#"<workbook xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets>{listed}</sheets></workbook>"#).into_bytes(),
    ));
    parts.push((
        "xl/_rels/workbook.xml.rels".into(),
        format!("<Relationships>{rels}</Relationships>").into_bytes(),
    ));
    zip(path, &parts);
}

const PACK: &str = "d365-business-central-purchase-to-pay";

#[test]
fn a_business_central_package_workbook_builds_as_handed_over() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Buying"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let book = root.join("package.xlsx");
    // Order 1001 is open; 1000 was invoiced and archived (two versions);
    // 999 was invoiced before archiving was on, so only its receipt is left.
    // A quote shares the order table and an order's number.
    package(
        &book,
        &[
            (
                "38 Purchase Header",
                "Purchase Header",
                &[
                    &[
                        "Document Type",
                        "No.",
                        "Buy-from Vendor No.",
                        "Order Date",
                        "Status",
                    ],
                    &["Order", "1001", "V1", "2026-03-02", "Released"],
                    &["Quote", "1001", "V1", "2026-03-01", "Open"],
                ],
            ),
            (
                "39 Purchase Line",
                "Purchase Line",
                &[
                    &[
                        "Document Type",
                        "Document No.",
                        "Line No.",
                        "No.",
                        "Quantity",
                    ],
                    &["Order", "1001", "10000", "ITEM", "2"],
                    &["Quote", "1001", "10000", "ITEM", "2"],
                ],
            ),
            (
                "5109 Purchase Header Archive",
                "Purchase Header Archive",
                &[
                    &[
                        "Document Type",
                        "No.",
                        "Buy-from Vendor No.",
                        "Order Date",
                        "Status",
                        "Version No.",
                    ],
                    &["Order", "1000", "V1", "2026-02-01", "Released", "1"],
                    &["Order", "1000", "V1", "2026-02-01", "Released", "2"],
                ],
            ),
            (
                "5110 Purchase Line Archive",
                "Purchase Line Archive",
                &[
                    &[
                        "Document Type",
                        "Document No.",
                        "Line No.",
                        "No.",
                        "Quantity",
                        "Version No.",
                    ],
                    &["Order", "1000", "10000", "ITEM", "1", "1"],
                    &["Order", "1000", "10000", "ITEM", "1", "2"],
                ],
            ),
            (
                "121 Purch. Rcpt. Line",
                "Purch. Rcpt. Line",
                &[
                    &[
                        "Document No.",
                        "Line No.",
                        "Order No.",
                        "Order Line No.",
                        "Posting Date",
                        "Quantity",
                    ],
                    &["R1", "10000", "1000", "10000", "2026-02-03", "1"],
                    &["R2", "10000", "1001", "10000", "2026-03-04", "2"],
                    &["R3", "10000", "999", "10000", "2026-01-05", "1"],
                ],
            ),
            (
                "122 Purch. Inv. Header",
                "Purch. Inv. Header",
                &[
                    &["No.", "Order No.", "Posting Date"],
                    &["I1", "1000", "2026-02-05"],
                ],
            ),
            (
                "123 Purch. Inv. Line",
                "Purch. Inv. Line",
                &[
                    &[
                        "Document No.",
                        "Line No.",
                        "Order No.",
                        "Order Line No.",
                        "Quantity",
                    ],
                    &["I1", "10000", "1000", "10000", "1"],
                ],
            ),
        ],
    );
    let inventory = json(&cli(
        root,
        &["workspace", "inventory", &id, book.to_str().unwrap()],
    ));
    assert_eq!(inventory["status"], "complete");

    let check = cli(root, &["workspace", "pack", "check", &id, "--pack", PACK]);
    let check = json(&check);
    assert_eq!(check["status"], "ready", "{check}");
    let tables = check["tables"].to_string();
    assert!(
        tables.contains("SystemCreatedAt not exported so Order Date is used"),
        "{tables}"
    );
    let answer = |question: &str, choice: &str, value: Option<&str>| {
        let mut args = vec![
            "workspace",
            "pack",
            "answer",
            id.as_str(),
            "--pack",
            PACK,
            "--question",
            question,
            "--choice",
            choice,
            "--confirmed-by",
            "Buyer",
        ];
        if let Some(value) = value {
            args.extend(["--value", value]);
        }
        assert!(cli(root, &args).status.success());
    };
    answer("setting:offset", "value", Some("+01:00"));
    answer("link:posted-lines-have-order-lines", "keep", None);
    let built = json(&cli(
        root,
        &["workspace", "pack", "build", &id, "--pack", PACK],
    ));
    assert_eq!(built["status"], "complete", "{built}");
    let rules = &built["preparation"]["rules"];
    // The open order and the archived one (once, at its latest version) are
    // created; the quote is not an order; 999 is followed from its receipt.
    assert_eq!(rules["bc_p_order_created"]["emitted"], 2);
    assert_eq!(rules["bc_p_goods_moved"]["emitted"], 3);
    assert_eq!(rules["bc_p_line_from_posted"]["emitted"], 1);
    assert_eq!(built["preparation"]["issue_count"], 0);
}

#[test]
fn windows_text_exports_are_read_as_they_are() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Claims"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    // Windows-1252, semicolons and a decimal comma, as European Excel saves
    // "CSV".
    let path = root.join("claims.csv");
    fs::write(&path, b"Claim;Step;When;Amount\nC-1;Eingang M\xfcller;2026-03-05T09:00:00Z;1,50\nC-1;Bezahlt;2026-03-06T09:00:00Z;1,50\n").unwrap();
    let inventory = json(&cli(
        root,
        &["workspace", "inventory", &id, path.to_str().unwrap()],
    ));
    assert_eq!(
        inventory["inputs"][0]["converted"],
        "read as Windows-1252, semicolon-separated"
    );
    let imported = json(&cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            path.to_str().unwrap(),
            "--case",
            "Claim",
            "--activity",
            "Step",
            "--timestamp",
            "When",
        ],
    ));
    assert_eq!(imported["status"], "complete", "{imported}");
    assert!(imported.to_string().contains("Eingang Müller"));
}

#[test]
fn an_event_log_workbook_is_read_from_the_tab_that_holds_it() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Tickets"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let book = root.join("report.xlsx");
    // A summary tab first, the event log on the second tab.
    package(
        &book,
        &[
            (
                "Summary",
                "Summary",
                &[&["Team", "Open tickets"], &["Support", "12"]],
            ),
            (
                "Events",
                "Events",
                &[
                    &["Ticket", "Step", "When"],
                    &["T-1", "Opened", "2026-03-05T09:00:00Z"],
                    &["T-1", "Closed", "2026-03-06T09:00:00Z"],
                    &["T-2", "Opened", "2026-03-05T10:00:00Z"],
                ],
            ),
        ],
    );
    let inspected = json(
        &Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["workspace", "inspect", book.to_str().unwrap(), "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(inspected["workbook"]["chosen"], "Events", "{inspected}");
    assert_eq!(
        inspected["workbook"]["sheets"][1]["looks_like_event_log"],
        true
    );
    let imported = json(&cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            book.to_str().unwrap(),
            "--case",
            "Ticket",
            "--activity",
            "Step",
            "--timestamp",
            "When",
        ],
    ));
    assert_eq!(imported["status"], "complete", "{imported}");
}

#[test]
fn a_zipped_export_is_its_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Zipped"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let archive = root.join("export.zip");
    zip(
        &archive,
        &[
            (
                "export/EKKO.csv".into(),
                b"EBELN,BUKRS\n4500000001,1000\n".to_vec(),
            ),
            (
                "export/EKPO.txt".into(),
                b"EBELN|EBELP\n4500000001|00010\n".to_vec(),
            ),
            ("__MACOSX/export/._EKKO.csv".into(), b"junk".to_vec()),
        ],
    );
    let inventory = json(&cli(
        root,
        &["workspace", "inventory", &id, archive.to_str().unwrap()],
    ));
    let names = inventory["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["asset"]["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["EKKO.csv", "EKPO.csv"]);
    assert_eq!(inventory["inputs"][1]["converted"], "pipe-separated");
}

/// A LibreOffice spreadsheet of one sheet.
fn ods(path: &Path, sheet: &str, rows: &[&[&str]]) {
    let cells = rows
        .iter()
        .map(|row| {
            let cells = row
                .iter()
                .map(|c| format!(r#"<table:table-cell office:value-type="string"><text:p>{c}</text:p></table:table-cell>"#))
                .collect::<String>();
            format!("<table:table-row>{cells}</table:table-row>")
        })
        .collect::<String>();
    let content = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.2"><office:body><office:spreadsheet><table:table table:name="{sheet}">{cells}</table:table></office:spreadsheet></office:body></office:document-content>"#
    );
    zip(
        path,
        &[
            ("mimetype".into(), b"application/vnd.oasis.opendocument.spreadsheet".to_vec()),
            ("content.xml".into(), content.into_bytes()),
            (
                "META-INF/manifest.xml".into(),
                br#"<?xml version="1.0" encoding="UTF-8"?><manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0"><manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/></manifest:manifest>"#.to_vec(),
            ),
        ],
    );
}

#[test]
fn libreoffice_misnamed_and_unreadable_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Mixed"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    // A LibreOffice spreadsheet.
    let sheet = root.join("tickets.ods");
    ods(
        &sheet,
        "Events",
        &[
            &["Ticket", "Step", "When"],
            &["T-1", "Opened", "2026-03-05T09:00:00Z"],
            &["T-1", "Closed", "2026-03-06T09:00:00Z"],
        ],
    );
    let imported = json(&cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            sheet.to_str().unwrap(),
            "--case",
            "Ticket",
            "--activity",
            "Step",
            "--timestamp",
            "When",
        ],
    ));
    assert_eq!(imported["status"], "complete", "{imported}");
    // A workbook saved with a .csv name is read as the workbook it is; a PDF
    // saved as .csv is described for the agent instead of read as text.
    let misnamed = root.join("orders.csv");
    package(
        &misnamed,
        &[(
            "Orders",
            "Orders",
            &[&["Order", "Date"], &["1", "2026-01-05"]],
        )],
    );
    let pdf = root.join("report.csv");
    fs::write(&pdf, b"%PDF-1.7\n\x00\x01\x02 binary report").unwrap();
    let output = cli(
        root,
        &[
            "workspace",
            "inventory",
            &id,
            misnamed.to_str().unwrap(),
            pdf.to_str().unwrap(),
        ],
    );
    let inventory = json(&output);
    assert_eq!(inventory["status"], "complete");
    assert_eq!(inventory["inputs"][0]["asset"]["name"], "orders.csv");
    assert_eq!(inventory["unreadable"][0]["file"], "report.csv");
    assert_eq!(
        inventory["unreadable"][0]["description"]["kind"],
        "a PDF document"
    );
    assert!(
        inventory["next"]
            .as_str()
            .unwrap()
            .contains("could not be read")
    );
    // Only unreadable files: the agent is asked to advise, with exit 3.
    let output = cli(
        root,
        &["workspace", "inventory", &id, pdf.to_str().unwrap()],
    );
    assert_eq!(output.status.code(), Some(3));
    assert_eq!(json(&output)["status"], "unreadable");
}

#[test]
fn an_old_excel_workbook_is_read() {
    // Written by xlwt as "Excel 97-2004": a notes sheet, then the events with
    // real date-time cells. Invented data.
    let book =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../testdata/exports/claims-excel97.xls");
    let inspected = json(
        &Command::new(env!("CARGO_BIN_EXE_glasstarn"))
            .args(["workspace", "inspect", book.to_str().unwrap(), "--json"])
            .output()
            .unwrap(),
    );
    assert_eq!(inspected["workbook"]["chosen"], "Events", "{inspected}");
    assert!(inspected.to_string().contains("2026-03-05T09:41:00"));
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let created = json(&cli(root, &["workspace", "create", "--name", "Claims"]));
    let id = created["project"]["id"].as_str().unwrap().to_owned();
    let imported = json(&cli(
        root,
        &[
            "workspace",
            "import",
            &id,
            book.to_str().unwrap(),
            "--case",
            "Claim",
            "--activity",
            "Step",
            "--timestamp",
            "When",
        ],
    ));
    assert_eq!(imported["status"], "complete", "{imported}");
    assert!(imported["headline"]["routes"].to_string().contains("Paid"));
}
