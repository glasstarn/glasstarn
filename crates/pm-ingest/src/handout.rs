//! The export request a person forwards to the system administrator: every
//! table of a pack with only the columns it needs, which rows to take, how to
//! export it from that kind of system, and the file name that drops straight
//! into the project. Written in plain words for the person asking and precise
//! enough for the administrator. Built from the pack alone, so it works for
//! every pack.

use crate::pack::{Pack, Table};
use std::fmt::Write;

/// What the person settled with the agent before asking for the export.
#[derive(Debug, Clone, Default)]
pub struct Request {
    /// The project or question the export is for.
    pub purpose: String,
    /// First and last day of the period, `YYYY-MM-DD`.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Values of the pack's scope field, such as company codes.
    pub scope: Vec<String>,
    /// The day the request was written, `YYYY-MM-DD`.
    pub today: String,
    /// What the agent learned in the conversation that the administrator
    /// should know: the goal, a deadline, where to send the files, the
    /// company's own document types or fields, a warehouse copy, a table to
    /// split by month, what the administrator said last time.
    pub notes: Vec<String>,
}

/// How a table is exported, by the kind of system it lives in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Source {
    /// A table of SAP ECC or S/4HANA on-premise or private cloud.
    SapTable,
    SapBusinessOne,
    SapByDesign,
    S4Cloud,
    Ariba,
    Other,
}

/// SAP tables that usually hold millions of rows: export them in the
/// background rather than on screen.
const LARGE_SAP_TABLES: [&str; 22] = [
    "ACDOCA", "BSEG", "BKPF", "BSAK", "BSAD", "BSAS", "BSIK", "BSID", "BSIS", "CDHDR", "CDPOS",
    "EKBE", "MSEG", "MKPF", "MATDOC", "VBFA", "COEP", "JEST", "JCDS", "AFRU", "RSEG", "VBAP",
];

fn sap_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == b'_' || c == b'/')
}

fn source(pack: &Pack, table: &Table) -> Source {
    let system = pack.system.to_lowercase();
    if table.name.starts_with("Ariba") {
        Source::Ariba
    } else if system.contains("business one") {
        Source::SapBusinessOne
    } else if system.contains("bydesign") {
        Source::SapByDesign
    } else if sap_name(&table.name) && (system.contains("sap ecc") || system.contains("s/4hana")) {
        if system.contains("cloud, public") && !system.contains("ecc") {
            Source::S4Cloud
        } else {
            Source::SapTable
        }
    } else if system.contains("s/4hana cloud") {
        Source::S4Cloud
    } else {
        Source::Other
    }
}

fn large(table: &Table) -> bool {
    LARGE_SAP_TABLES.contains(&table.name.as_str())
}

/// How to export one table, in a few words; "How to export" explains each.
fn method(source: Source, table: &Table) -> &'static str {
    match source {
        Source::SapTable if large(table) => "background job (large table)",
        Source::SapTable => "SE16N",
        Source::SapBusinessOne => "Query Manager or SQL query",
        Source::SapByDesign => "Business Analytics report",
        Source::S4Cloud => "Custom Analytical Query or OData extract",
        Source::Ariba => "Ariba reporting report",
        Source::Other => "the system's own export",
    }
}

fn source_help(source: Source) -> &'static str {
    match source {
        Source::SapTable => {
            "**SAP tables.** Smaller tables come straight from transaction SE16N: enter the table name, set the filters shown, use the layout to keep only the listed columns, and save the result as a comma-separated file. Tables marked *large* usually hold millions of rows; run those in the background (an SQ01/SQVI query or your usual table download program as a background job) and collect the file from the application server. If your SAP data is already copied into a data warehouse or lake (SAP Datasphere or BW, Snowflake, Databricks, Azure Synapse, BigQuery), a query there works just as well, as long as the columns keep their SAP names and the same filters are applied."
        }
        Source::SapBusinessOne => {
            "**SAP Business One.** Use Query Manager, or an SQL query on the company database, one query per table, and save each result as a comma-separated file."
        }
        Source::SapByDesign => {
            "**SAP Business ByDesign.** Use a report on the matching data source in Business Analytics, with the filters shown, and export it to CSV."
        }
        Source::S4Cloud => {
            "**SAP S/4HANA Cloud.** These are SAP's released data views. Export them with a Custom Analytical Query (to CSV) or through the integration team's OData extract, with the columns named as shown."
        }
        Source::Ariba => {
            "**SAP Ariba.** Build a reporting (analytical or operational) report with the columns shown and export it as CSV. Column names may differ by report; keep the ones listed."
        }
        Source::Other => {
            "**Other tables.** Use the system's own report or data export and save each as CSV."
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl Pack {
    /// The export request for the system administrator, as Markdown.
    pub fn handout(&self, request: &Request) -> String {
        let mut out = String::new();
        let scope_fields: Vec<&str> = self
            .tables
            .iter()
            .filter_map(|t| t.scope_field.as_deref())
            .collect();
        let period = match (&request.from, &request.to) {
            (Some(from), Some(to)) => format!("{from} to {to}"),
            (Some(from), None) => format!("from {from} to the day of the export"),
            _ => "the months agreed with the person asking (6 to 12 months is typical)".into(),
        };
        let scope = if request.scope.is_empty() {
            "the ones agreed with the person asking".to_owned()
        } else {
            request.scope.join(", ")
        };
        let required = self.tables.iter().filter(|t| !t.optional).count();
        let optional = self.tables.len() - required;

        let _ = writeln!(out, "# Data export request: {}", self.name);
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "For the {} administrator. Prepared on {} with Glasstarn{}.",
            self.system,
            request.today,
            if request.purpose.is_empty() {
                String::new()
            } else {
                format!(" for \"{}\"", request.purpose)
            }
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "We want to review how {} runs, step by step, from the records the system already keeps. Please export the {} below as files. Only the listed columns are needed: no passwords, no system access and no screenshots.",
            self.process,
            if self.tables.len() == 1 {
                "table"
            } else {
                "tables"
            }
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "## In short");
        let _ = writeln!(out);
        let _ = writeln!(out, "- **Period:** {period}.");
        if !scope_fields.is_empty() {
            let mut names = scope_fields.clone();
            names.sort_unstable();
            names.dedup();
            let _ = writeln!(
                out,
                "- **Company or unit ({}):** {scope}.",
                names.join(", ")
            );
        }
        let _ = writeln!(
            out,
            "- **Files:** {}, one per table, named exactly as below{}.",
            plural(required, "required file", "required files"),
            if optional > 0 {
                format!(
                    ", and {} if available",
                    plural(optional, "optional file", "optional files")
                )
            } else {
                String::new()
            }
        );
        if !self.custom_prefixes.is_empty() {
            let _ = writeln!(
                out,
                "- **Company's own fields:** on every table, also include the company's own fields (names starting with {}), if there are any.",
                self.custom_prefixes.join(" or ")
            );
        }
        let _ = writeln!(
            out,
            "- **Format:** comma-separated CSV in UTF-8, with the technical field names as the header row. Keep leading zeros and export dates as they are stored."
        );
        let _ = writeln!(
            out,
            "- **Before you start:** please tell us roughly: {}",
            self.size_check.ask
        );
        let _ = writeln!(out);

        if !request.notes.is_empty() {
            let _ = writeln!(out, "## From our conversation");
            let _ = writeln!(out);
            for note in &request.notes {
                let _ = writeln!(out, "- {note}");
            }
            let _ = writeln!(out);
        }
        let mut sources: Vec<Source> = self.tables.iter().map(|t| source(self, t)).collect();
        sources.sort_unstable();
        sources.dedup();
        let _ = writeln!(out, "## How to export");
        let _ = writeln!(out);
        for s in &sources {
            let _ = writeln!(out, "{}", source_help(*s));
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "## The files");
        let _ = writeln!(out);
        for (n, table) in self.tables.iter().enumerate() {
            let src = source(self, table);
            let _ = writeln!(
                out,
                "### {}. {}.csv: {}{}",
                n + 1,
                table.name,
                table.label,
                match (table.optional, src == Source::SapTable && large(table)) {
                    (true, true) => " (optional, large)",
                    (true, false) => " (optional)",
                    (false, true) => " (large)",
                    (false, false) => "",
                }
            );
            let _ = writeln!(out);
            let _ = writeln!(out, "- **Which rows:** {}", table.export);
            let mut filters = Vec::new();
            if let Some(field) = &table.scope_field {
                filters.push(format!("{field} = {scope}"));
            }
            if let Some(field) = table.date_field.as_deref().filter(|_| {
                table.export.contains("in the period")
                    || table.export.contains("in the same period")
            }) {
                // "In the period or later" runs to the day of the export.
                let open_ended = table.export.contains("or later");
                filters.push(match (&request.from, open_ended) {
                    (Some(from), true) => format!("{field} from {from} to the day of the export"),
                    _ => format!("{field}: {period}"),
                });
            }
            if !filters.is_empty() {
                let _ = writeln!(out, "- **Filters:** {}.", filters.join("; "));
            }
            let columns = table
                .required
                .iter()
                .chain(&table.fields)
                .map(|c| format!("`{c}`"))
                .collect::<Vec<_>>();
            let _ = writeln!(
                out,
                "- **Columns ({}):** {}",
                columns.len(),
                columns.join(", ")
            );
            let _ = writeln!(out, "- **How:** {}.", method(src, table));
            let _ = writeln!(out);
        }

        let _ = writeln!(out, "## Before sending");
        let _ = writeln!(out);
        for table in &self.tables {
            let _ = writeln!(
                out,
                "- [ ] {}.csv{}",
                table.name,
                if table.optional { " (optional)" } else { "" }
            );
        }
        let _ = writeln!(
            out,
            "- [ ] Each file has a header row with the technical field names"
        );
        let _ = writeln!(
            out,
            "- [ ] Leading zeros kept (document numbers such as 0000012345)"
        );
        let _ = writeln!(
            out,
            "- [ ] No totals, page headers or empty lines in the files"
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "Send the files back to the person who asked; nothing needs to be uploaded anywhere. If a table or column does not exist in your system, say which, and we will adjust."
        );
        let _ = writeln!(out);
        let _ = writeln!(
            out,
            "_From Glasstarn's \"{}\" pack ({}). Companies often add their own fields or change standard ones; the data check that follows will say if anything is missing._",
            self.name,
            if self.status == "tested" {
                "tested on a real system by the Glasstarn project, not certified or endorsed by the system's maker"
            } else {
                "written from public knowledge of the standard tables, not yet checked on a real system"
            }
        );
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pack::BUILT_IN;

    fn pack(id: &str) -> Pack {
        Pack::parse(BUILT_IN.iter().find(|(p, _)| *p == id).unwrap().1).unwrap()
    }

    fn request() -> Request {
        Request {
            purpose: "Why are invoices paid late?".into(),
            from: Some("2026-01-01".into()),
            to: Some("2026-06-30".into()),
            scope: vec!["1000".into(), "2000".into()],
            today: "2026-10-15".into(),
            notes: vec!["Invoices also use the company's own document type ZR.".into()],
        }
    }

    #[test]
    fn the_purchase_to_pay_request_names_every_file_column_filter_and_method() {
        let p2p = pack("sap-purchase-to-pay");
        let text = p2p.handout(&request());
        for table in &p2p.tables {
            assert!(
                text.contains(&format!("{}.csv", table.name)),
                "{} missing",
                table.name
            );
            for column in table.required.iter().chain(&table.fields) {
                assert!(text.contains(&format!("`{column}`")), "{column} missing");
            }
        }
        // Concrete filters where the pack knows the scope and date fields.
        assert!(text.contains("BUKRS = 1000, 2000"), "{text}");
        assert!(text.contains("AEDAT: 2026-01-01 to 2026-06-30"), "{text}");
        // Methods: SE16N for small tables, the background for large ones,
        // and a warehouse query as an alternative.
        assert!(text.contains("### 3. EKKO.csv: Purchase order headers\n"));
        assert!(text.contains("CDPOS.csv: Change log items (optional, large)"));
        assert!(
            text.contains("SE16N") && text.contains("background") && text.contains("Snowflake")
        );
        assert!(text.contains("names starting with ZZ or YY"));
        assert!(
            text.contains("CPUDT from 2026-01-01 to the day of the export"),
            "{text}"
        );
        assert!(
            text.contains("- **How:** SE16N.")
                && text.contains("- **How:** background job (large table).")
        );
        assert!(text.contains("- [ ] EKPO.csv\n"));
        assert!(text.contains(
            "## From our conversation\n\n- Invoices also use the company's own document type ZR."
        ));
    }

    #[test]
    fn every_pack_gets_a_request_with_its_own_way_to_export() {
        for (id, _) in BUILT_IN {
            let p = pack(id);
            let text = p.handout(&Request {
                today: "2026-10-15".into(),
                ..Request::default()
            });
            assert!(text.starts_with("# Data export request: "), "{id}");
            assert!(text.contains("6 to 12 months"), "{id}");
            for table in &p.tables {
                assert!(
                    text.contains(&format!("{}.csv", table.name)),
                    "{id}: {}",
                    table.name
                );
            }
        }
        let b1 = pack("sap-business-one-purchase-to-pay").handout(&request());
        assert!(b1.contains("Query Manager") && !b1.contains("SE16N"));
        let cloud = pack("sap-s4-cloud-purchase-to-pay").handout(&request());
        assert!(cloud.contains("Custom Analytical Query") && !cloud.contains("SE16N"));
        let ariba = pack("sap-ariba-purchase-to-pay").handout(&request());
        assert!(ariba.contains("Ariba reporting") && ariba.contains("SE16N"));
    }
}
