//! Process packs: a reviewed description of one process in one kind of
//! system, written as data. A pack lists the tables and fields to export,
//! the checks that tell whether an export is complete and linked, and a
//! connected-preparation recipe with placeholders for the answers only the
//! company can give (such as the system clock's time zone).
//!
//! The functions here are vendor-independent. [`check`] profiles exported
//! files against a pack and turns what it finds into gaps (re-export needed)
//! and questions (a person must decide). [`build_recipe`] turns a pack, the
//! exported files' identities and the saved answers into an ordinary recipe
//! for [`crate::prepare_ocel::prepare`]. Nothing is inferred silently: every
//! change an answer makes to the recipe is named in [`Built::changes`].
use crate::prepare_ocel::{Condition, KeyPart, Recipe, blank_date, erp_date};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
#[error("process pack: {0}")]
pub struct Error(String);
type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    pub version: u32,
    pub name: String,
    pub system: String,
    pub process: String,
    /// `untested` until the Glasstarn project tests the pack on a real
    /// system, then `tested`. Never a claim of the vendor's certification.
    pub status: String,
    pub note: String,
    pub case_object_type: String,
    pub size_check: SizeCheck,
    #[serde(default)]
    pub custom_prefixes: Vec<String>,
    pub tables: Vec<Table>,
    #[serde(default)]
    pub settings: Vec<Setting>,
    #[serde(default)]
    pub unique: Vec<Unique>,
    #[serde(default)]
    pub checks: Vec<LinkCheck>,
    /// A connected-preparation recipe without `inputs`; inputs are table names
    /// and `{setting:<id>}` strings are replaced by the answers.
    pub recipe: Value,
    /// Narrower runs of the same pack, e.g. accounts payable only.
    #[serde(default)]
    pub variants: Vec<Variant>,
    /// Items another system can point at by their number, so this pack can
    /// be combined with that system's pack (see [`Pack::combine`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub link_targets: Vec<LinkTarget>,
}

/// An item other systems can point at by its number, such as an SAP purchase
/// order that a workflow tool's request names.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinkTarget {
    pub id: String,
    /// What the number is, in words, e.g. "SAP purchase order number".
    pub label: String,
    /// The recipe object rule of that item.
    pub object: String,
    /// The table holding the number, and its key columns.
    pub table: String,
    pub key: Vec<String>,
    /// What a number looks like, e.g. "4500001234".
    pub example: String,
    /// Relationship rules from that item to the items the pack follows, so
    /// linked steps reach them too (an order to its lines).
    #[serde(default)]
    pub carry: Vec<String>,
}

/// A run over a subset of the pack's tables. Steps and links that need the
/// other tables are left out.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variant {
    pub id: String,
    pub name: String,
    pub case_object_type: String,
    pub tables: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SizeCheck {
    pub ask: String,
    pub unit: String,
    pub steps_per_unit: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Table {
    pub name: String,
    pub label: String,
    /// What to export, in words for the person running the export.
    pub export: String,
    pub required: Vec<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    /// The table's own date, used to time extra date fields against it.
    #[serde(default)]
    pub date_field: Option<String>,
    /// That date in words, e.g. "the order was created".
    #[serde(default)]
    pub date_label: Option<String>,
    /// What one row is, in the plural, e.g. "purchase orders".
    #[serde(default)]
    pub noun: Option<String>,
    /// The recipe object rule this table's rows become, if any.
    #[serde(default)]
    pub object: Option<String>,
    #[serde(default)]
    pub key: Vec<String>,
    /// A field whose values show which company or unit was exported.
    #[serde(default)]
    pub scope_field: Option<String>,
    /// Event links that carry a step on this table's object down to its
    /// lines, e.g. an order-level approval onto every order line.
    #[serde(default)]
    pub fan_out: Vec<Value>,
    /// The process still runs without this table (its rules are left out).
    #[serde(default)]
    pub optional: bool,
    /// For a field-history table: the object whose history it holds, and the
    /// fields whose history tracking the pack needs (checked by discovery).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_of: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub needs_tracked: Vec<String>,
    /// Columns that stand in for a required field the export lacks, such as
    /// a posting date for a creation time that a standard export leaves out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallbacks: Vec<Fallback>,
    /// Where the system moves finished rows of this table (Business
    /// Central's order archive): exported beside it, it is merged in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archive: Option<Archive>,
}

/// An archive table merged into its open table: for each key not among the
/// open rows, the archived row with the highest version.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Archive {
    pub table: String,
    pub key: Vec<String>,
    pub version: String,
}

/// A column used in place of a required field when the export lacks it.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Fallback {
    pub field: String,
    pub column: String,
    /// The column holds a date without a time; times then become that day
    /// in the company's time zone.
    #[serde(default)]
    pub date_only: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Setting {
    pub id: String,
    pub question: String,
    pub example: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Unique {
    pub id: String,
    pub table: String,
    pub columns: Vec<String>,
    #[serde(default, rename = "where")]
    pub filter: Vec<Condition>,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Side {
    pub table: String,
    pub columns: Vec<KeyPart>,
    #[serde(default, rename = "where")]
    pub filter: Vec<Condition>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LinkCheck {
    pub id: String,
    pub label: String,
    pub from: Side,
    pub to: Side,
    /// `all`: every row must match, or the export is incomplete. `most`:
    /// below `threshold` the pack asks its question.
    pub expect: String,
    /// Include blank source references as missing when the business link is required.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub require_nonblank: bool,
    /// The check guards the export's scope (only purchase orders, not
    /// contracts): a row that fails it was exported by mistake, so it is an
    /// export gap and never set aside.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub scope: bool,
    #[serde(default)]
    pub threshold: Option<f64>,
    #[serde(default)]
    pub question: Option<QuestionTemplate>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuestionTemplate {
    /// May use `{missing}`, `{total}` and `{missing_share}`.
    pub text: String,
    pub options: Vec<Choice>,
    pub recommended: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Choice {
    pub id: String,
    pub label: String,
}

impl Pack {
    /// Parses a pack, first merging the shared parts it names in `include`.
    pub fn parse(text: &str) -> Result<Self> {
        let value: Value =
            serde_json::from_str(text).map_err(|error| Error(format!("invalid pack: {error}")))?;
        let value = merge_includes(value)?;
        let pack: Self = serde_json::from_value(value)
            .map_err(|error| Error(format!("invalid pack: {error}")))?;
        let names = pack.tables.iter().map(|t| &t.name).collect::<BTreeSet<_>>();
        if names.len() != pack.tables.len() {
            return Err(Error("a table is listed twice".into()));
        }
        let known = |table: &str| names.contains(&table.to_owned());
        for unique in &pack.unique {
            if !known(&unique.table) {
                return Err(Error(format!(
                    "unique check names unknown table {}",
                    unique.table
                )));
            }
        }
        for variant in &pack.variants {
            if let Some(table) = variant.tables.iter().find(|t| !known(t)) {
                return Err(Error(format!(
                    "variant {} names unknown table {table}",
                    variant.id
                )));
            }
        }
        for check in &pack.checks {
            if !known(&check.from.table) || !known(&check.to.table) {
                return Err(Error(format!("check {} names an unknown table", check.id)));
            }
            if check.from.columns.len() != check.to.columns.len() {
                return Err(Error(format!(
                    "check {} compares keys of different widths",
                    check.id
                )));
            }
            if let Some(question) = &check.question
                && !question
                    .options
                    .iter()
                    .any(|o| o.id == question.recommended)
            {
                return Err(Error(format!(
                    "check {} recommends an unknown option",
                    check.id
                )));
            }
        }
        let rule_ids = |category: &str| {
            pack.recipe[category]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|r| Some((r["id"].as_str()?.to_owned(), r.clone())))
                .collect::<BTreeMap<_, _>>()
        };
        let objects = rule_ids("objects");
        let relationships = rule_ids("relationships");
        for target in &pack.link_targets {
            if !known(&target.table) || !objects.contains_key(&target.object) {
                return Err(Error(format!(
                    "link target {} names an unknown table or item",
                    target.id
                )));
            }
            if let Some(carry) = target.carry.iter().find(|r| {
                relationships
                    .get(*r)
                    .is_none_or(|rule| rule["source"]["object"] != *target.object)
            }) {
                return Err(Error(format!(
                    "link target {} carries through {carry}, which does not start at its item",
                    target.id
                )));
            }
        }
        Ok(pack)
    }

    /// This pack and `other` as one, joined where `table` of this pack holds
    /// the number of `other`'s link target `target`. Which field holds the
    /// number is a question for the company (the setting `link_field`); a
    /// data check reports how many rows find their item. Steps on this
    /// pack's item reach the linked item and the items `other` follows.
    pub fn combine(&self, other: &Pack, table: &str, target: &str) -> Result<Self> {
        let link = other
            .link_targets
            .iter()
            .find(|t| t.id == target)
            .ok_or_else(|| {
                Error(format!(
                    "pack {} offers no link target '{target}'; it offers {}",
                    other.id,
                    if other.link_targets.is_empty() {
                        "none".to_owned()
                    } else {
                        other
                            .link_targets
                            .iter()
                            .map(|t| t.id.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ))
            })?;
        let source = self.recipe["objects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["input"] == table)
            .cloned()
            .ok_or_else(|| {
                Error(format!(
                    "{table} is not a table whose rows are items of pack {}",
                    self.id
                ))
            })?;
        let source_id = source["id"].as_str().unwrap_or_default().to_owned();
        let clash = |what: &str, mine: Vec<String>, theirs: Vec<String>| -> Result<()> {
            match mine.iter().find(|m| theirs.contains(m)) {
                Some(name) => Err(Error(format!(
                    "both packs have the {what} '{name}', so they cannot be combined yet"
                ))),
                None => Ok(()),
            }
        };
        clash(
            "table",
            self.tables.iter().map(|t| t.name.clone()).collect(),
            other.tables.iter().map(|t| t.name.clone()).collect(),
        )?;
        clash(
            "data check",
            self.unique
                .iter()
                .map(|u| u.id.clone())
                .chain(self.checks.iter().map(|c| c.id.clone()))
                .collect(),
            other
                .unique
                .iter()
                .map(|u| u.id.clone())
                .chain(other.checks.iter().map(|c| c.id.clone()))
                .collect(),
        )?;
        let ids = |pack: &Pack| {
            RULES
                .iter()
                .flat_map(|category| {
                    pack.recipe[*category]
                        .as_array()
                        .cloned()
                        .unwrap_or_default()
                })
                .filter_map(|r| r["id"].as_str().map(str::to_owned))
                .collect::<Vec<_>>()
        };
        clash("rule", ids(self), ids(other))?;
        let mut settings = self.settings.clone();
        for setting in &other.settings {
            match settings.iter().find(|s| s.id == setting.id) {
                Some(mine) if mine.question != setting.question => {
                    return Err(Error(format!(
                        "both packs ask a different question '{}', so they cannot be combined yet",
                        setting.id
                    )));
                }
                Some(_) => {}
                None => settings.push(setting.clone()),
            }
        }
        settings.push(Setting {
            id: "link_field".into(),
            question: format!(
                "Which field of {table} ({}) holds the {}? Give its technical name; its values look like {}.",
                self.system, link.label, link.example
            ),
            example: "the field's technical name".into(),
        });
        let field = "{setting:link_field}".to_owned();
        let mut recipe = json!({"version": 1, "format": "ocel"});
        for category in RULES {
            let mut rules = self.recipe[category]
                .as_array()
                .cloned()
                .unwrap_or_default();
            rules.extend(
                other.recipe[category]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            );
            recipe[category] = Value::Array(rules);
        }
        let relationship = format!("link_{}", link.id);
        let target_key = other.recipe["objects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|o| o["id"] == *link.object)
            .map(|o| o["key"].clone())
            .unwrap_or(Value::Null);
        // People write the number they see (an order number), which may not
        // be the key the system links its own items by (an internal ID). Then
        // the number becomes an item of its own, linked to the system's item.
        let (linked_object, hops) = if target_key == json!(link.key) {
            (link.object.clone(), Vec::new())
        } else {
            let number = format!("link_number_{}", link.id);
            let number_of = format!("link_number_of_{}", link.id);
            recipe["objects"].as_array_mut().unwrap().push(json!({
                "id": number, "input": link.table, "namespace": "link",
                "object_type": link.label, "key": link.key,
            }));
            recipe["relationships"].as_array_mut().unwrap().push(json!({
                "id": number_of, "input": link.table,
                "source": {"object": number, "columns": link.key, "qualifier": link.label},
                "target": {"object": link.object, "columns": target_key, "qualifier": "numbered item"},
            }));
            (number.clone(), vec![(number, number_of)])
        };
        recipe["relationships"].as_array_mut().unwrap().push(json!({
            "id": relationship, "input": table,
            "where": [{"column": field, "not_in": [""]}],
            "source": {"object": source_id, "columns": source["key"], "qualifier": source["object_type"]},
            "target": {"object": linked_object, "columns": [field], "qualifier": link.label,
                       "optional": true, "allow_unmatched": true},
        }));
        let linked_events = self.recipe["events"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|e| {
                e["objects"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|r| r["object"] == *source_id)
            })
            .filter_map(|e| e["id"].as_str().map(str::to_owned))
            .collect::<Vec<_>>();
        let carried = recipe["carried_links"].as_array_mut().unwrap();
        for event in &linked_events {
            carried.push(
                json!({"id": format!("{event}_to_{}", link.id), "event": event,
                "through": source_id, "relationship": relationship, "qualifier": link.label}),
            );
            for (through, hop) in &hops {
                carried.push(
                    json!({"id": format!("{event}_to_{}_item", link.id), "event": event,
                    "through": through, "relationship": hop, "qualifier": link.label}),
                );
            }
            for further in &link.carry {
                carried.push(json!({"id": format!("{event}_to_{}_{further}", link.id), "event": event,
                    "through": link.object, "relationship": further, "qualifier": format!("via {}", link.label)}));
            }
        }
        let mut checks = self.checks.clone();
        checks.extend(other.checks.iter().cloned());
        checks.push(LinkCheck {
            id: "link-matches".into(),
            label: format!("{table} rows whose {} is in the {} export", link.label, other.system),
            from: Side {
                table: table.to_owned(),
                columns: vec![KeyPart::Column(field.clone())],
                filter: vec![Condition { column: field.clone(), any_of: Vec::new(), not_in: vec![String::new()] }],
            },
            to: Side {
                table: link.table.clone(),
                columns: link.key.iter().cloned().map(KeyPart::Column).collect(),
                filter: Vec::new(),
            },
            expect: "most".into(),
            require_nonblank: false,
            scope: false,
            threshold: Some(0.9),
            question: Some(QuestionTemplate {
                text: format!(
                    "{{missing}} of {{total}} {table} rows name a {} that is not in the {} export. Common causes: numbers saved without their leading zeros, or items outside the exported period. Keep them unlinked, or fix the data and re-export?",
                    link.label, other.system
                ),
                options: vec![
                    Choice { id: "keep".into(), label: "Keep them unlinked".into() },
                    Choice { id: "re-export".into(), label: "Fix the data and re-export".into() },
                ],
                recommended: "keep".into(),
            }),
        });
        let mut custom_prefixes = self.custom_prefixes.clone();
        for prefix in &other.custom_prefixes {
            if !custom_prefixes.contains(prefix) {
                custom_prefixes.push(prefix.clone());
            }
        }
        let pack = Pack {
            id: format!("{}+{}", self.id, other.id),
            version: 1,
            name: format!("{} with {}", self.name, other.name),
            system: format!("{} and {}", self.system, other.system),
            process: format!("{} and {}", self.process, other.process),
            status: if self.status == "tested" && other.status == "tested" {
                "tested".into()
            } else {
                "untested".into()
            },
            note: format!(
                "Combined from two packs, joined where {table} holds the {}. {} {}",
                link.label, self.note, other.note
            ),
            case_object_type: other.case_object_type.clone(),
            size_check: other.size_check.clone(),
            custom_prefixes,
            tables: self.tables.iter().chain(&other.tables).cloned().collect(),
            settings,
            unique: self.unique.iter().chain(&other.unique).cloned().collect(),
            checks,
            recipe,
            variants: Vec::new(),
            link_targets: other.link_targets.clone(),
        };
        // The combined pack must pass the same checks as any pack.
        Pack::parse(&serde_json::to_string(&pack).map_err(|e| Error(e.to_string()))?)
    }

    /// The pack narrowed to one of its variants.
    pub fn with_variant(&self, id: &str) -> Result<Self> {
        let variant = self.variants.iter().find(|v| v.id == id).ok_or_else(|| {
            Error(format!(
                "pack {} has no variant '{id}'; it has {}",
                self.id,
                self.variants
                    .iter()
                    .map(|v| v.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        let keep = |table: &str| variant.tables.iter().any(|t| t == table);
        let removed = self
            .tables
            .iter()
            .filter(|t| !keep(&t.name))
            .map(|t| t.name.clone())
            .collect::<Vec<_>>();
        let mut pack = self.clone();
        pack.name = format!("{} – {}", self.name, variant.name);
        pack.case_object_type = variant.case_object_type.clone();
        pack.tables.retain(|t| keep(&t.name));
        pack.unique.retain(|u| keep(&u.table));
        pack.checks
            .retain(|c| keep(&c.from.table) && keep(&c.to.table));
        pack.variants.clear();
        prune(&mut pack.recipe, &removed);
        Ok(pack)
    }

    fn table(&self, name: &str) -> Option<&Table> {
        self.tables.iter().find(|table| table.name == name)
    }

    /// The export request for the company's system administrator.
    pub fn checklist(&self) -> Value {
        json!({
            "pack": self.id, "name": self.name, "system": self.system, "status": self.status,
            "note": self.note,
            "size_check": self.size_check,
            "format": "One CSV file per table, named after the table (for example EKKO.csv), with technical field names as the header row. Keep leading zeros and export dates as they are stored.",
            "tables": self.tables.iter().map(|table| json!({
                "table": table.name, "label": table.label, "export": table.export,
                "fields": table.required.iter().chain(&table.fields).collect::<Vec<_>>(),
                "optional": table.optional,
            })).collect::<Vec<_>>(),
            "custom_fields": if self.custom_prefixes.is_empty() { Value::Null } else {
                json!(format!("Also include the company's own fields on these tables (names starting with {}).", self.custom_prefixes.join(" or ")))
            },
        })
    }
}

/// The packs that ship with the engine, by id.
pub const BUILT_IN: [(&str, &str); 59] = [
    (
        "sap-purchase-to-pay",
        include_str!("../../../packs/sap-purchase-to-pay/pack.json"),
    ),
    (
        "sap-s4-cloud-purchase-to-pay",
        include_str!("../../../packs/sap-s4-cloud-purchase-to-pay/pack.json"),
    ),
    (
        "sap-ariba-purchase-to-pay",
        include_str!("../../../packs/sap-ariba-purchase-to-pay/pack.json"),
    ),
    (
        "sap-ariba-s4-cloud-purchase-to-pay",
        include_str!("../../../packs/sap-ariba-s4-cloud-purchase-to-pay/pack.json"),
    ),
    (
        "sap-order-to-cash",
        include_str!("../../../packs/sap-order-to-cash/pack.json"),
    ),
    (
        "sap-s4-cloud-order-to-cash",
        include_str!("../../../packs/sap-s4-cloud-order-to-cash/pack.json"),
    ),
    (
        "sap-sales-cloud-order-to-cash",
        include_str!("../../../packs/sap-sales-cloud-order-to-cash/pack.json"),
    ),
    (
        "sap-sales-cloud-s4-cloud-order-to-cash",
        include_str!("../../../packs/sap-sales-cloud-s4-cloud-order-to-cash/pack.json"),
    ),
    (
        "salesforce-sap-order-to-cash",
        include_str!("../../../packs/salesforce-sap-order-to-cash/pack.json"),
    ),
    (
        "salesforce-s4-cloud-order-to-cash",
        include_str!("../../../packs/salesforce-s4-cloud-order-to-cash/pack.json"),
    ),
    (
        "salesforce-lead-to-opportunity",
        include_str!("../../../packs/salesforce-lead-to-opportunity/pack.json"),
    ),
    (
        "salesforce-case-handling",
        include_str!("../../../packs/salesforce-case-handling/pack.json"),
    ),
    (
        "jira-issue-lifecycle",
        include_str!("../../../packs/jira-issue-lifecycle/pack.json"),
    ),
    (
        "servicenow-incident-management",
        include_str!("../../../packs/servicenow-incident-management/pack.json"),
    ),
    (
        "servicenow-change-management",
        include_str!("../../../packs/servicenow-change-management/pack.json"),
    ),
    (
        "servicenow-request-fulfillment",
        include_str!("../../../packs/servicenow-request-fulfillment/pack.json"),
    ),
    (
        "sap-plan-to-produce",
        include_str!("../../../packs/sap-plan-to-produce/pack.json"),
    ),
    (
        "sap-plant-maintenance",
        include_str!("../../../packs/sap-plant-maintenance/pack.json"),
    ),
    (
        "sap-record-to-report",
        include_str!("../../../packs/sap-record-to-report/pack.json"),
    ),
    (
        "concur-expense",
        include_str!("../../../packs/concur-expense/pack.json"),
    ),
    (
        "successfactors-recruiting",
        include_str!("../../../packs/successfactors-recruiting/pack.json"),
    ),
    (
        "salesforce-quote-approval",
        include_str!("../../../packs/salesforce-quote-approval/pack.json"),
    ),
    (
        "salesforce-order-management",
        include_str!("../../../packs/salesforce-order-management/pack.json"),
    ),
    (
        "salesforce-field-service",
        include_str!("../../../packs/salesforce-field-service/pack.json"),
    ),
    (
        "salesforce-contract-renewal",
        include_str!("../../../packs/salesforce-contract-renewal/pack.json"),
    ),
    (
        "sap-business-one-purchase-to-pay",
        include_str!("../../../packs/sap-business-one-purchase-to-pay/pack.json"),
    ),
    (
        "sap-business-one-order-to-cash",
        include_str!("../../../packs/sap-business-one-order-to-cash/pack.json"),
    ),
    (
        "sap-bydesign-purchase-to-pay",
        include_str!("../../../packs/sap-bydesign-purchase-to-pay/pack.json"),
    ),
    (
        "sap-bydesign-order-to-cash",
        include_str!("../../../packs/sap-bydesign-order-to-cash/pack.json"),
    ),
    (
        "salesforce-cpq-quote-to-renewal",
        include_str!("../../../packs/salesforce-cpq-quote-to-renewal/pack.json"),
    ),
    (
        "salesforce-ncino-loan-origination",
        include_str!("../../../packs/salesforce-ncino-loan-origination/pack.json"),
    ),
    (
        "pega-case-management",
        include_str!("../../../packs/pega-case-management/pack.json"),
    ),
    (
        "appian-record-events",
        include_str!("../../../packs/appian-record-events/pack.json"),
    ),
    (
        "power-automate-dataverse",
        include_str!("../../../packs/power-automate-dataverse/pack.json"),
    ),
    (
        "oracle-ebs-purchase-to-pay",
        include_str!("../../../packs/oracle-ebs-purchase-to-pay/pack.json"),
    ),
    (
        "oracle-fusion-purchase-to-pay",
        include_str!("../../../packs/oracle-fusion-purchase-to-pay/pack.json"),
    ),
    (
        "oracle-ebs-order-to-cash",
        include_str!("../../../packs/oracle-ebs-order-to-cash/pack.json"),
    ),
    (
        "oracle-fusion-order-to-cash",
        include_str!("../../../packs/oracle-fusion-order-to-cash/pack.json"),
    ),
    (
        "jde-purchase-to-pay",
        include_str!("../../../packs/jde-purchase-to-pay/pack.json"),
    ),
    (
        "jde-order-to-cash",
        include_str!("../../../packs/jde-order-to-cash/pack.json"),
    ),
    (
        "peoplesoft-purchase-to-pay",
        include_str!("../../../packs/peoplesoft-purchase-to-pay/pack.json"),
    ),
    (
        "netsuite-purchase-to-pay",
        include_str!("../../../packs/netsuite-purchase-to-pay/pack.json"),
    ),
    (
        "netsuite-order-to-cash",
        include_str!("../../../packs/netsuite-order-to-cash/pack.json"),
    ),
    (
        "d365-finance-operations-purchase-to-pay",
        include_str!("../../../packs/d365-finance-operations-purchase-to-pay/pack.json"),
    ),
    (
        "d365-finance-operations-order-to-cash",
        include_str!("../../../packs/d365-finance-operations-order-to-cash/pack.json"),
    ),
    (
        "d365-business-central-purchase-to-pay",
        include_str!("../../../packs/d365-business-central-purchase-to-pay/pack.json"),
    ),
    (
        "d365-business-central-order-to-cash",
        include_str!("../../../packs/d365-business-central-order-to-cash/pack.json"),
    ),
    (
        "d365-sales",
        include_str!("../../../packs/d365-sales/pack.json"),
    ),
    (
        "d365-customer-service",
        include_str!("../../../packs/d365-customer-service/pack.json"),
    ),
    (
        "d365-field-service",
        include_str!("../../../packs/d365-field-service/pack.json"),
    ),
    (
        "d365-human-resources-leave",
        include_str!("../../../packs/d365-human-resources-leave/pack.json"),
    ),
    (
        "d365-project-operations-time",
        include_str!("../../../packs/d365-project-operations-time/pack.json"),
    ),
    (
        "d365-project-operations-expenses",
        include_str!("../../../packs/d365-project-operations-expenses/pack.json"),
    ),
    (
        "servicenow-ticket-handling",
        include_str!("../../../packs/servicenow-ticket-handling/pack.json"),
    ),
    (
        "servicenow-problem-management",
        include_str!("../../../packs/servicenow-problem-management/pack.json"),
    ),
    (
        "hubspot-lead-lifecycle",
        include_str!("../../../packs/hubspot-lead-lifecycle/pack.json"),
    ),
    (
        "hubspot-deal-pipeline",
        include_str!("../../../packs/hubspot-deal-pipeline/pack.json"),
    ),
    (
        "hubspot-ticket-pipeline",
        include_str!("../../../packs/hubspot-ticket-pipeline/pack.json"),
    ),
    (
        "hubspot-quote-approval",
        include_str!("../../../packs/hubspot-quote-approval/pack.json"),
    ),
];

/// Shared parts packs can include: tables, checks and recipe rules written
/// once, such as SAP vendors or supplier invoices and their payment.
pub const SHARED: [(&str, &str); 27] = [
    (
        "sap-clock",
        include_str!("../../../packs/shared/sap-clock.json"),
    ),
    (
        "sap-ecc-vendors",
        include_str!("../../../packs/shared/sap-ecc-vendors.json"),
    ),
    (
        "sap-ecc-purchasing",
        include_str!("../../../packs/shared/sap-ecc-purchasing.json"),
    ),
    (
        "sap-ecc-requisitions",
        include_str!("../../../packs/shared/sap-ecc-requisitions.json"),
    ),
    (
        "sap-ecc-payables",
        include_str!("../../../packs/shared/sap-ecc-payables.json"),
    ),
    (
        "sap-cloud-suppliers",
        include_str!("../../../packs/shared/sap-cloud-suppliers.json"),
    ),
    (
        "sap-cloud-purchasing",
        include_str!("../../../packs/shared/sap-cloud-purchasing.json"),
    ),
    (
        "sap-cloud-requisitions",
        include_str!("../../../packs/shared/sap-cloud-requisitions.json"),
    ),
    (
        "sap-cloud-payables",
        include_str!("../../../packs/shared/sap-cloud-payables.json"),
    ),
    (
        "sap-ariba-buying",
        include_str!("../../../packs/shared/sap-ariba-buying.json"),
    ),
    (
        "sap-ariba-invoicing",
        include_str!("../../../packs/shared/sap-ariba-invoicing.json"),
    ),
    (
        "sap-ecc-customers",
        include_str!("../../../packs/shared/sap-ecc-customers.json"),
    ),
    (
        "sap-ecc-sales",
        include_str!("../../../packs/shared/sap-ecc-sales.json"),
    ),
    (
        "sap-ecc-billing",
        include_str!("../../../packs/shared/sap-ecc-billing.json"),
    ),
    (
        "sap-ecc-receivables",
        include_str!("../../../packs/shared/sap-ecc-receivables.json"),
    ),
    (
        "sap-cloud-customers",
        include_str!("../../../packs/shared/sap-cloud-customers.json"),
    ),
    (
        "sap-cloud-sales",
        include_str!("../../../packs/shared/sap-cloud-sales.json"),
    ),
    (
        "sap-cloud-billing",
        include_str!("../../../packs/shared/sap-cloud-billing.json"),
    ),
    (
        "sap-cloud-receivables",
        include_str!("../../../packs/shared/sap-cloud-receivables.json"),
    ),
    (
        "sap-ecc-purchasing-changes",
        include_str!("../../../packs/shared/sap-ecc-purchasing-changes.json"),
    ),
    (
        "sap-cloud-purchasing-changes",
        include_str!("../../../packs/shared/sap-cloud-purchasing-changes.json"),
    ),
    (
        "sap-ecc-sales-changes",
        include_str!("../../../packs/shared/sap-ecc-sales-changes.json"),
    ),
    (
        "sap-cloud-sales-changes",
        include_str!("../../../packs/shared/sap-cloud-sales-changes.json"),
    ),
    (
        "sap-sales-cloud-front",
        include_str!("../../../packs/shared/sap-sales-cloud-front.json"),
    ),
    (
        "salesforce-front",
        include_str!("../../../packs/shared/salesforce-front.json"),
    ),
    (
        "salesforce-opportunities",
        include_str!("../../../packs/shared/salesforce-opportunities.json"),
    ),
    (
        "salesforce-clock",
        include_str!("../../../packs/shared/salesforce-clock.json"),
    ),
];

const MERGED: [&str; 5] = ["tables", "settings", "unique", "checks", "link_targets"];
const RULES: [&str; 5] = [
    "objects",
    "events",
    "relationships",
    "event_links",
    "carried_links",
];

/// Appends `own` to `list`; an entry with the same name or id replaces the
/// included one in place, so a pack can adjust a shared table or rule.
fn merge_list(list: &mut Vec<Value>, own: Vec<Value>) {
    let key = |v: &Value| v.get("name").or_else(|| v.get("id")).cloned();
    for item in own {
        match list.iter_mut().find(|existing| key(existing) == key(&item)) {
            Some(existing) => *existing = item,
            None => list.push(item),
        }
    }
}

fn merge_includes(mut pack: Value) -> Result<Value> {
    let Some(includes) = pack.as_object_mut().and_then(|p| p.remove("include")) else {
        return Ok(pack);
    };
    let ids = serde_json::from_value::<Vec<String>>(includes)
        .map_err(|_| Error("include must list shared part ids".into()))?;
    let mut lists = BTreeMap::<&str, Vec<Value>>::new();
    let mut rules = BTreeMap::<&str, Vec<Value>>::new();
    let mut prefixes = Vec::<Value>::new();
    for id in &ids {
        let text = SHARED
            .iter()
            .find(|(shared, _)| shared == id)
            .map(|(_, text)| *text)
            .ok_or_else(|| Error(format!("unknown shared part '{id}'")))?;
        let part: Value = serde_json::from_str(text)
            .map_err(|error| Error(format!("invalid shared part {id}: {error}")))?;
        for key in MERGED {
            let items = part[key].as_array().cloned().unwrap_or_default();
            merge_list(lists.entry(key).or_default(), items);
        }
        for key in RULES {
            let items = part["recipe"][key].as_array().cloned().unwrap_or_default();
            merge_list(rules.entry(key).or_default(), items);
        }
        for prefix in part["custom_prefixes"].as_array().into_iter().flatten() {
            if !prefixes.contains(prefix) {
                prefixes.push(prefix.clone());
            }
        }
    }
    let object = pack.as_object_mut().unwrap();
    for key in MERGED {
        let own = object
            .remove(key)
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        let mut list = lists.remove(key).unwrap_or_default();
        merge_list(&mut list, own);
        object.insert(key.into(), Value::Array(list));
    }
    for prefix in object
        .remove("custom_prefixes")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
    {
        if !prefixes.contains(&prefix) {
            prefixes.push(prefix);
        }
    }
    object.insert("custom_prefixes".into(), Value::Array(prefixes));
    let mut recipe = object
        .remove("recipe")
        .unwrap_or_else(|| json!({"version": 1, "format": "ocel"}));
    for (key, value) in [("version", json!(1)), ("format", json!("ocel"))] {
        if recipe.get(key).is_none() {
            recipe[key] = value;
        }
    }
    for key in RULES {
        let own = recipe
            .get(key)
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut list = rules.remove(key).unwrap_or_default();
        merge_list(&mut list, own);
        recipe[key] = Value::Array(list);
    }
    object.insert("recipe".into(), recipe);
    Ok(pack)
}

/// One question for the company's analyst, in business words.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Question {
    pub id: String,
    /// `setting`, `link` or `custom_field`.
    pub kind: String,
    pub text: String,
    pub options: Vec<Choice>,
    /// The recommended option, when the data supports one.
    pub recommended: Option<String>,
    /// The answer also needs a value, such as an offset or a step name.
    pub needs_value: Option<String>,
    /// Fields filled together with this one that a step would carry along,
    /// such as the approver's name beside an approval date.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carries: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Timing {
    /// Days from the table's own date, for rows where both are filled.
    pub median_days: i64,
    pub p10_days: i64,
    pub p90_days: i64,
    pub before_share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExtraField {
    pub column: String,
    pub custom: bool,
    pub filled: u64,
    pub filled_share: f64,
    pub date_like: bool,
    pub timing: Option<Timing>,
    /// Other custom fields filled on nearly every row this one is filled.
    pub filled_with: Vec<String>,
    /// Up to three most common values (custom text fields only).
    pub examples: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct TableReport {
    pub table: String,
    pub label: String,
    pub rows: u64,
    pub missing_fields: Vec<String>,
    pub first_date: Option<String>,
    pub last_date: Option<String>,
    pub scope: Vec<(String, u64)>,
    pub custom_fields: Vec<ExtraField>,
    /// Exported fields the pack does not use and that are not custom.
    pub other_fields: usize,
    /// When required fields are missing: the file's own column names, and
    /// for each missing field the column that most likely holds it, for the
    /// agent to propose and the person to confirm (`workspace pack reshape`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub suggested: BTreeMap<String, String>,
    /// Required fields the export lacks, each with the column used instead.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub fallbacks_used: BTreeMap<String, String>,
    /// Optional fields the export lacks; the build leaves out what they
    /// would have filled (an amount on a step, for example).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub absent_fields: Vec<String>,
    /// Fields the pack reads whose column name appears more than once.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repeated_fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UniqueReport {
    pub id: String,
    pub label: String,
    pub rows: u64,
    pub repeated: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LinkReport {
    pub id: String,
    pub label: String,
    pub rows: u64,
    pub matched: u64,
    pub missing: u64,
    pub share: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckReport {
    pub pack: String,
    pub status: &'static str,
    pub tables: Vec<TableReport>,
    pub missing_tables: Vec<String>,
    pub unique: Vec<UniqueReport>,
    pub links: Vec<LinkReport>,
    /// Problems only a new or corrected export can fix, in business words.
    pub gaps: Vec<String>,
    /// Records that do not fit the pack's rules but may be real: repeated
    /// keys and references that find nothing. The person decides, through
    /// the [`SET_ASIDE`] question, whether to set them aside or re-export.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub data_problems: Vec<String>,
    /// Missing tables or fields the build can do without, each with the
    /// steps and links it goes ahead without.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub left_out: Vec<String>,
    pub questions: Vec<Question>,
}

/// The question that decides whether records that do not fit are set aside
/// (built without them and listed as a data-quality finding) or sent back
/// for a corrected export.
pub const SET_ASIDE: &str = "data:set-aside";

/// The set-aside question, worded from what was found.
pub fn set_aside_question(found: &[String]) -> Question {
    let text = if found.is_empty() {
        "Some records do not fit the process: a repeated number, a reference that finds nothing, or a missing date. Leave them out, build with the rest and list them as a data-quality finding, or ask for a corrected export?".to_owned()
    } else {
        format!(
            "Some records do not fit the process: {} Leave them out, build with the rest and list them as a data-quality finding, or ask for a corrected export?",
            found.join(" ")
        )
    };
    Question {
        id: SET_ASIDE.into(),
        kind: "data".into(),
        text,
        options: vec![
            Choice {
                id: "set_aside".into(),
                label:
                    "Leave them out, build with the rest, and list them as a data-quality finding"
                        .into(),
            },
            Choice {
                id: "new_export".into(),
                label: "Stop and ask for a corrected export".into(),
            },
        ],
        recommended: Some("set_aside".into()),
        needs_value: None,
        carries: Vec::new(),
    }
}

struct Reader {
    headers: Vec<String>,
    reader: csv::Reader<std::fs::File>,
}
fn open(path: &Path) -> Result<Reader> {
    let mut reader = csv::Reader::from_path(path)
        .map_err(|_| Error(format!("cannot read {}", path.display())))?;
    let headers = reader
        .headers()
        .map_err(|_| Error("invalid CSV headers".into()))?
        .iter()
        .map(|h| h.trim_start_matches('\u{feff}').to_owned())
        .collect();
    Ok(Reader { headers, reader })
}
fn position(headers: &[String], column: &str) -> Option<usize> {
    headers.iter().position(|h| h == column)
}
fn filled(value: &str) -> bool {
    !value.trim().is_empty() && !blank_date(value)
}
fn percent(share: f64) -> String {
    let value = share * 100.0;
    if value > 0.0 && value < 1.0 {
        "under 1%".into()
    } else {
        format!("{}%", value.round() as i64)
    }
}

/// Keys of a side's rows; `None` for a row with a blank key part.
/// A set of keys held in memory until it grows large, then in a scratch
/// database on disk; the same answers either way.
struct KeySet {
    memory: HashSet<Vec<String>>,
    disk: Option<(rusqlite::Connection, PathBuf)>,
}

/// Keys held in memory before the set moves to disk.
const KEYS_IN_MEMORY: usize = 2_000_000;

impl KeySet {
    fn new() -> Self {
        Self {
            memory: HashSet::new(),
            disk: None,
        }
    }

    fn text(key: &[String]) -> String {
        serde_json::to_string(key).unwrap_or_default()
    }

    /// Adds a key; false when it was already there.
    fn insert(&mut self, key: Vec<String>) -> Result<bool> {
        let failed = |e: rusqlite::Error| Error(format!("the scratch key store failed: {e}"));
        if let Some((connection, _)) = &self.disk {
            let added = connection
                .prepare_cached("INSERT OR IGNORE INTO k VALUES (?1)")
                .map_err(failed)?
                .execute([Self::text(&key)])
                .map_err(failed)?;
            return Ok(added == 1);
        }
        let added = self.memory.insert(key);
        if self.memory.len() > KEYS_IN_MEMORY || crate::force_disk() {
            static RUN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "pm-keys-{}-{}.sqlite",
                std::process::id(),
                RUN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            ));
            let _ = std::fs::remove_file(&path);
            let connection = rusqlite::Connection::open(&path).map_err(failed)?;
            connection
                .execute_batch(
                    "PRAGMA journal_mode = OFF; PRAGMA synchronous = OFF;
                     CREATE TABLE k (key TEXT PRIMARY KEY) WITHOUT ROWID; BEGIN;",
                )
                .map_err(failed)?;
            {
                let mut insert = connection
                    .prepare("INSERT OR IGNORE INTO k VALUES (?1)")
                    .map_err(failed)?;
                for key in self.memory.drain() {
                    insert.execute([Self::text(&key)]).map_err(failed)?;
                }
            }
            self.disk = Some((connection, path));
        }
        Ok(added)
    }

    fn contains(&self, key: &[String]) -> Result<bool> {
        match &self.disk {
            None => Ok(self.memory.contains(key)),
            Some((connection, _)) => connection
                .prepare_cached("SELECT 1 FROM k WHERE key = ?1")
                .and_then(|mut statement| statement.exists([Self::text(key)]))
                .map_err(|e| Error(format!("the scratch key store failed: {e}"))),
        }
    }
}

impl Drop for KeySet {
    fn drop(&mut self) {
        if let Some((connection, path)) = self.disk.take() {
            drop(connection);
            let _ = std::fs::remove_file(path);
        }
    }
}

fn keys(
    path: &Path,
    side_columns: &[KeyPart],
    filter: &[Condition],
    mut f: impl FnMut(Option<Vec<String>>),
) -> Result<()> {
    let mut file = open(path)?;
    let columns = side_columns
        .iter()
        .map(|part| {
            position(&file.headers, part.column())
                .ok_or_else(|| Error(format!("missing field {}", part.column())))
        })
        .collect::<Result<Vec<_>>>()?;
    let filter = filter
        .iter()
        .map(|c| {
            position(&file.headers, &c.column)
                .map(|at| (at, c))
                .ok_or_else(|| Error(format!("missing field {}", c.column)))
        })
        .collect::<Result<Vec<_>>>()?;
    for record in file.reader.records() {
        let record = record.map_err(|_| Error("invalid CSV record".into()))?;
        if !filter
            .iter()
            .all(|(at, c)| c.holds(record.get(*at).unwrap_or_default()))
        {
            continue;
        }
        let key = side_columns
            .iter()
            .zip(&columns)
            .map(|(part, at)| part.take(record.get(*at).unwrap_or_default()))
            .collect::<Vec<_>>();
        f((!key.iter().any(|v| v.trim().is_empty())).then_some(key));
    }
    Ok(())
}

fn profile(pack: &Pack, table: &Table, path: &Path) -> Result<TableReport> {
    let mut file = open(path)?;
    let headers = file.headers.clone();
    let known = table
        .required
        .iter()
        .chain(&table.fields)
        .collect::<BTreeSet<_>>();
    // A missing field with a stand-in column the export does have is used
    // through that column instead.
    let fallbacks_used = table
        .fallbacks
        .iter()
        .filter(|f| {
            position(&headers, &f.field).is_none() && position(&headers, &f.column).is_some()
        })
        .map(|f| (f.field.clone(), f.column.clone()))
        .collect::<BTreeMap<_, _>>();
    let missing_fields = table
        .required
        .iter()
        .filter(|f| {
            !pending(f) && position(&headers, f).is_none() && !fallbacks_used.contains_key(*f)
        })
        .cloned()
        .collect::<Vec<_>>();
    // A prefix such as `ZZ`, or `*__c` for a suffix (Salesforce custom fields).
    let custom = |name: &str| {
        pack.custom_prefixes
            .iter()
            .any(|p| match p.strip_prefix('*') {
                Some(suffix) => name.ends_with(suffix),
                None => name.starts_with(p.as_str()),
            })
    };
    let extra = headers
        .iter()
        .enumerate()
        .filter(|(_, h)| !known.contains(h) && custom(h))
        .take(64)
        .map(|(at, h)| (at, h.clone()))
        .collect::<Vec<_>>();
    let other_fields = headers
        .iter()
        .filter(|h| !known.contains(h) && !custom(h))
        .count();
    let date_at = table
        .date_field
        .as_deref()
        .map(|f| fallbacks_used.get(f).map_or(f, String::as_str))
        .and_then(|f| position(&headers, f));
    let scope_at = table
        .scope_field
        .as_deref()
        .and_then(|f| position(&headers, f));
    let mut rows = 0u64;
    let (mut first, mut last) = (None, None);
    let mut scope = BTreeMap::<String, u64>::new();
    let n = extra.len();
    let mut fill = vec![0u64; n];
    let mut dates = vec![0u64; n];
    let mut both = vec![vec![0u64; n]; n];
    let mut gaps = vec![Vec::<i64>::new(); n];
    let mut values = vec![HashMap::<String, u64>::new(); n];
    for record in file.reader.records() {
        let record = record.map_err(|_| Error("invalid CSV record".into()))?;
        rows += 1;
        let own = date_at.and_then(|at| erp_date(record.get(at).unwrap_or_default()));
        if let Some(day) = own {
            first = Some(first.map_or(day, |f: chrono::NaiveDate| f.min(day)));
            last = Some(last.map_or(day, |l: chrono::NaiveDate| l.max(day)));
        }
        if let Some(at) = scope_at {
            let value = record.get(at).unwrap_or_default();
            if scope.len() < 50 || scope.contains_key(value) {
                *scope.entry(value.to_owned()).or_default() += 1;
            }
        }
        let present = extra
            .iter()
            .map(|(at, _)| filled(record.get(*at).unwrap_or_default()))
            .collect::<Vec<_>>();
        for (i, (at, _)) in extra.iter().enumerate() {
            if !present[i] {
                continue;
            }
            fill[i] += 1;
            let value = record.get(*at).unwrap_or_default();
            if let Some(day) = erp_date(value) {
                dates[i] += 1;
                if let (Some(own), true) = (own, gaps[i].len() < 1_000_000) {
                    gaps[i].push((day - own).num_days());
                }
            } else if values[i].len() < 1000 || values[i].contains_key(value) {
                *values[i].entry(value.to_owned()).or_default() += 1;
            }
            for j in 0..n {
                if present[j] {
                    both[i][j] += 1;
                }
            }
        }
    }
    let custom_fields = extra
        .iter()
        .enumerate()
        .map(|(i, (_, column))| {
            let date_like = fill[i] > 0 && dates[i] * 10 >= fill[i] * 9;
            let timing = (date_like && !gaps[i].is_empty()).then(|| {
                let mut sorted = gaps[i].clone();
                sorted.sort_unstable();
                let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
                Timing {
                    median_days: at(0.5),
                    p10_days: at(0.1),
                    p90_days: at(0.9),
                    before_share: sorted.iter().filter(|d| **d < 0).count() as f64
                        / sorted.len() as f64,
                }
            });
            let mut common = values[i].iter().collect::<Vec<_>>();
            common.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            ExtraField {
                column: column.clone(),
                custom: true,
                filled: fill[i],
                filled_share: if rows == 0 {
                    0.0
                } else {
                    fill[i] as f64 / rows as f64
                },
                date_like,
                timing,
                filled_with: (0..n)
                    .filter(|j| *j != i && fill[i] > 0 && both[i][*j] * 10 >= fill[i] * 9)
                    .map(|j| extra[j].1.clone())
                    .collect(),
                examples: if date_like {
                    Vec::new()
                } else {
                    common.into_iter().take(3).map(|(v, _)| v.clone()).collect()
                },
            }
        })
        .collect();
    let mut scope = scope.into_iter().collect::<Vec<_>>();
    scope.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    scope.truncate(10);
    Ok(TableReport {
        table: table.name.clone(),
        label: table.label.clone(),
        rows,
        first_date: first.map(|d| d.to_string()),
        last_date: last.map(|d| d.to_string()),
        scope,
        custom_fields,
        other_fields,
        suggested: crate::reshape::suggest(&missing_fields, &headers),
        columns: if missing_fields.is_empty() {
            Vec::new()
        } else {
            headers.clone()
        },
        repeated_fields: table
            .required
            .iter()
            .chain(&table.fields)
            .filter(|f| headers.iter().filter(|h| h == f).count() > 1)
            .cloned()
            .collect(),
        absent_fields: table
            .fields
            .iter()
            .filter(|f| !pending(f) && position(&headers, f).is_none())
            .cloned()
            .collect(),
        missing_fields,
        fallbacks_used,
    })
}

/// Adapts the recipe to optional fields the export lacks: attributes they
/// would have filled are dropped, a lookup stops bringing them in, and a
/// rule that needs one to place, time, filter or link its rows is left out,
/// with whatever depended on it. Returns the rules left out, by field.
fn leave_out_absent(
    recipe: &mut Value,
    absent: &BTreeMap<String, BTreeSet<String>>,
) -> BTreeMap<String, BTreeSet<String>> {
    // Structural uses of a field: anything but an attribute.
    fn structural(value: &Value, fields: &BTreeSet<String>, found: &mut BTreeSet<String>) {
        match value {
            Value::Array(items) => items.iter().for_each(|v| structural(v, fields, found)),
            Value::Object(map) => {
                for (key, v) in map {
                    match key.as_str() {
                        "attributes" | "lookup" | "lookups" | "id" | "input" | "namespace"
                        | "object_type" | "qualifier" | "object" | "event" => {}
                        "column" | "date" | "time" | "timestamp" => {
                            if let Some(f) = v.as_str().filter(|f| fields.contains(*f)) {
                                found.insert(f.to_owned());
                            }
                            structural(v, fields, found);
                        }
                        "columns" | "key" | "matching" | "event_key" => {
                            for item in v.as_array().into_iter().flatten() {
                                match item.as_str() {
                                    Some(f) if fields.contains(f) => {
                                        found.insert(f.to_owned());
                                    }
                                    _ => structural(item, fields, found),
                                }
                            }
                        }
                        _ => structural(v, fields, found),
                    }
                }
            }
            _ => {}
        }
    }
    let none = BTreeSet::new();
    let mut gone = BTreeMap::<String, BTreeSet<String>>::new();
    for category in ["objects", "events", "relationships", "event_links"] {
        let rules = recipe.get_mut(category).and_then(Value::as_array_mut);
        for rule in rules.into_iter().flatten() {
            let id = rule["id"].as_str().unwrap_or_default().to_owned();
            let input = rule["input"].as_str().unwrap_or_default().to_owned();
            let mut unavailable = absent.get(&input).unwrap_or(&none).clone();
            let mut found = BTreeSet::new();
            // A lookup no longer brings in its absent columns; one keyed on an
            // absent column cannot join at all.
            for key in ["lookup", "lookups"] {
                let lookups = match rule.get_mut(key) {
                    Some(Value::Array(items)) => items.iter_mut().collect::<Vec<_>>(),
                    Some(lookup @ Value::Object(_)) => vec![lookup],
                    _ => Vec::new(),
                };
                for lookup in lookups {
                    let from = lookup["input"].as_str().unwrap_or_default();
                    let missing = absent.get(from).unwrap_or(&none).clone();
                    for part in ["key", "matching"] {
                        for f in lookup[part].as_array().into_iter().flatten() {
                            if let Some(f) = f
                                .as_str()
                                .filter(|f| missing.contains(*f) || unavailable.contains(*f))
                            {
                                found.insert(f.to_owned());
                            }
                        }
                    }
                    if let Some(columns) = lookup.get_mut("columns").and_then(Value::as_array_mut) {
                        columns.retain(|c| {
                            let keep = !c.as_str().is_some_and(|c| missing.contains(c));
                            if !keep {
                                unavailable.insert(c.as_str().unwrap_or_default().to_owned());
                            }
                            keep
                        });
                    }
                }
            }
            if unavailable.is_empty() && found.is_empty() {
                continue;
            }
            if let Some(attributes) = rule.get_mut("attributes").and_then(Value::as_array_mut) {
                attributes.retain(|a| {
                    !a["column"]
                        .as_str()
                        .is_some_and(|c| unavailable.contains(c))
                });
            }
            structural(rule, &unavailable, &mut found);
            for field in found {
                gone.entry(field).or_default().insert(id.clone());
            }
        }
    }
    let ids = gone.values().flatten().cloned().collect::<BTreeSet<_>>();
    if !ids.is_empty() {
        prune_rules(recipe, &|rule: &Value| {
            rule["id"].as_str().is_some_and(|id| ids.contains(id))
        });
    }
    gone
}

/// Points the rules that read `table` at a stand-in column: a timestamp of
/// the missing field becomes the stand-in's date (in the company's time zone
/// when the pack asks for it), and any other use names the stand-in column.
fn stand_in(recipe: &mut Value, table: &str, fallback: &Fallback, offset: bool) {
    let field = Value::String(fallback.field.clone());
    let column = Value::String(fallback.column.clone());
    fn rename(value: &mut Value, field: &Value, column: &Value) {
        match value {
            Value::Array(items) => items.iter_mut().for_each(|v| rename(v, field, column)),
            Value::Object(map) => {
                for (key, v) in map.iter_mut() {
                    if matches!(key.as_str(), "column" | "date" | "time") && *v == *field {
                        *v = column.clone();
                    } else if key == "columns" || key == "key" || key == "matching" {
                        if let Value::Array(items) = v {
                            for item in items.iter_mut() {
                                if *item == *field {
                                    *item = column.clone();
                                } else {
                                    rename(item, field, column);
                                }
                            }
                        }
                    } else if key != "lookup" && key != "lookups" {
                        rename(v, field, column);
                    }
                }
            }
            _ => {}
        }
    }
    for category in ["objects", "events", "relationships", "event_links"] {
        let rules = recipe.get_mut(category).and_then(Value::as_array_mut);
        for rule in rules.into_iter().flatten() {
            if rule["input"] != table {
                continue;
            }
            if rule.get("timestamp") == Some(&field) {
                rule["timestamp"] = if fallback.date_only {
                    json!({"date": fallback.column, "offset": if offset { "{setting:offset}" } else { "+00:00" }})
                } else {
                    column.clone()
                };
            }
            rename(rule, &field, &column);
        }
    }
}

/// Profiles exported files (table name → CSV) against a pack.
pub fn check(pack: &Pack, files: &BTreeMap<String, PathBuf>) -> Result<CheckReport> {
    let mut gaps = Vec::new();
    let mut data_problems = Vec::new();
    let mut tables = Vec::new();
    let mut missing_tables = Vec::new();
    // What a required table or field that is missing would stop, decided
    // below: only a build left without its case, steps or times stops.
    let mut wanting = Vec::<(Vec<String>, BTreeMap<String, BTreeSet<String>>, String)>::new();
    for table in &pack.tables {
        let Some(path) = files.get(&table.name) else {
            missing_tables.push(table.name.clone());
            if !table.optional {
                wanting.push((
                    vec![table.name.clone()],
                    BTreeMap::new(),
                    format!(
                        "Table {} ({}) is not in the export.",
                        table.name, table.label
                    ),
                ));
            }
            continue;
        };
        let report = profile(pack, table, path)?;
        if !report.missing_fields.is_empty() {
            let absent = BTreeMap::from([(
                table.name.clone(),
                report
                    .missing_fields
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>(),
            )]);
            wanting.push((Vec::new(), absent, format!(
                "Table {} ({}) is missing the fields {}.{}",
                table.name,
                table.label,
                report.missing_fields.join(", "),
                if report.suggested.is_empty() {
                    String::new()
                } else {
                    format!(
                        " It may name them differently ({}); confirm with the person and run workspace pack reshape.",
                        report
                            .suggested
                            .iter()
                            .map(|(field, column)| format!("{column} for {field}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                }
            )));
        }
        if !report.repeated_fields.is_empty() {
            gaps.push(format!(
                "Table {} ({}) has more than one column named {}; rename or remove the extra ones and check again.",
                table.name,
                table.label,
                report.repeated_fields.join(", ")
            ));
        }
        // An empty field-history table can be true (nothing changed, or
        // tracking is off); the row count shows it, and steps from it are
        // simply absent.
        if report.rows == 0 && !table.optional && table.history_of.is_none() {
            wanting.push((
                vec![table.name.clone()],
                BTreeMap::new(),
                format!("Table {} ({}) has no rows.", table.name, table.label),
            ));
        }
        tables.push(report);
    }
    // Missing pieces stop the check only when, without all of them, nothing
    // would be left to mine: no item of the case type, or no step. Otherwise
    // the steps that need them are left out and named.
    let mut left_out = Vec::new();
    {
        let all_tables = wanting.iter().flat_map(|w| w.0.clone()).collect::<Vec<_>>();
        let mut all_absent = BTreeMap::<String, BTreeSet<String>>::new();
        for (_, absent, _) in &wanting {
            for (table, fields) in absent {
                all_absent
                    .entry(table.clone())
                    .or_default()
                    .extend(fields.iter().cloned());
            }
        }
        if !wanting.is_empty() {
            if remains(pack, &all_tables, &all_absent).0 {
                for (gone_tables, absent, message) in &wanting {
                    let lost = remains(pack, gone_tables, absent).1;
                    left_out.push(if lost.is_empty() {
                        format!("{message} Nothing the build needs is lost.")
                    } else {
                        format!(
                            "{message} The build goes ahead without: {}.",
                            lost.into_iter().collect::<Vec<_>>().join(", ")
                        )
                    });
                }
            } else {
                gaps.extend(wanting.iter().map(|w| w.2.clone()));
            }
        }
    }
    let usable = |table: &str| {
        tables
            .iter()
            .any(|t| t.table == table && t.missing_fields.is_empty())
    };
    let mut unique = Vec::new();
    for rule in &pack.unique {
        if !usable(&rule.table) {
            continue;
        }
        // Check the same source identity the recipe will use after stand-ins.
        // A valid fallback must not make uniqueness profiling read an absent column.
        let fallbacks = &tables
            .iter()
            .find(|t| t.table == rule.table)
            .unwrap()
            .fallbacks_used;
        let source_column = |column: &String| fallbacks.get(column).unwrap_or(column).clone();
        let parts = rule
            .columns
            .iter()
            .map(source_column)
            .map(KeyPart::Column)
            .collect::<Vec<_>>();
        let filter = rule
            .filter
            .iter()
            .map(|condition| {
                let mut condition = condition.clone();
                condition.column = source_column(&condition.column);
                condition
            })
            .collect::<Vec<_>>();
        let (mut rows, mut repeated, mut seen) = (0, 0, KeySet::new());
        let mut failed = None;
        keys(&files[&rule.table], &parts, &filter, |key| {
            rows += 1;
            if let Some(key) = key {
                match seen.insert(key) {
                    Ok(false) => repeated += 1,
                    Ok(true) => {}
                    Err(error) => {
                        failed.get_or_insert(error);
                    }
                }
            }
        })?;
        if let Some(error) = failed {
            return Err(error);
        }
        if repeated > 0 {
            data_problems.push(format!(
                "{repeated} rows of {} repeat a key that should be listed once ({}: {}).",
                rule.table,
                rule.label,
                rule.columns.join(" + ")
            ));
        }
        unique.push(UniqueReport {
            id: rule.id.clone(),
            label: rule.label.clone(),
            rows,
            repeated,
        });
    }
    let mut links = Vec::new();
    let mut questions = Vec::new();
    for setting in &pack.settings {
        questions.push(Question {
            id: format!("setting:{}", setting.id),
            kind: "setting".into(),
            text: setting.question.clone(),
            options: vec![Choice {
                id: "value".into(),
                label: format!("Give the value, for example {}", setting.example),
            }],
            recommended: None,
            needs_value: Some(format!("for example {}", setting.example)),
            carries: Vec::new(),
        });
    }
    for rule in &pack.checks {
        let waits = rule
            .from
            .columns
            .iter()
            .chain(&rule.to.columns)
            .any(|part| pending(part.column()));
        if waits || !usable(&rule.from.table) || !usable(&rule.to.table) {
            continue;
        }
        let mut targets = KeySet::new();
        let mut failed = None;
        keys(
            &files[&rule.to.table],
            &rule.to.columns,
            &rule.to.filter,
            |key| {
                if let Some(key) = key
                    && let Err(error) = targets.insert(key)
                {
                    failed.get_or_insert(error);
                }
            },
        )?;
        let (mut rows, mut matched) = (0u64, 0u64);
        keys(
            &files[&rule.from.table],
            &rule.from.columns,
            &rule.from.filter,
            |key| {
                if key.is_some() || rule.require_nonblank {
                    rows += 1;
                }
                if let Some(key) = key {
                    match targets.contains(&key) {
                        Ok(true) => matched += 1,
                        Ok(false) => {}
                        Err(error) => {
                            failed.get_or_insert(error);
                        }
                    }
                }
            },
        )?;
        if let Some(error) = failed {
            return Err(error);
        }
        let missing = rows - matched;
        let share = if rows == 0 {
            1.0
        } else {
            matched as f64 / rows as f64
        };
        // Most rows matching points to a few records that do not fit; most
        // not matching, to a wrong period, filter or column in the export.
        if missing > 0 && rule.expect == "all" && share >= 0.5 && !rule.scope {
            data_problems.push(format!(
                "{missing} of {rows} {} do not match ({} matched).",
                rule.label,
                percent(share)
            ));
        } else if missing > 0 && rule.expect == "all" {
            gaps.push(format!(
                "{missing} of {rows} {} do not match ({} matched). The export looks incomplete.",
                rule.label,
                percent(share)
            ));
        } else if missing > 0
            && share < rule.threshold.unwrap_or(1.0)
            && let Some(template) = &rule.question
        {
            questions.push(Question {
                id: format!("link:{}", rule.id),
                kind: "link".into(),
                text: template
                    .text
                    .replace("{missing}", &missing.to_string())
                    .replace("{total}", &rows.to_string())
                    .replace("{missing_share}", &percent(1.0 - share)),
                options: template.options.clone(),
                recommended: Some(template.recommended.clone()),
                needs_value: None,
                carries: Vec::new(),
            });
        }
        links.push(LinkReport {
            id: rule.id.clone(),
            label: rule.label.clone(),
            rows,
            matched,
            missing,
            share,
        });
    }
    questions.extend(custom_questions(pack, &tables));
    if !data_problems.is_empty() {
        questions.insert(0, set_aside_question(&data_problems));
    }
    let status = if !gaps.is_empty() {
        "export_incomplete"
    } else {
        "ready"
    };
    Ok(CheckReport {
        pack: pack.id.clone(),
        status,
        tables,
        missing_tables,
        unique,
        links,
        gaps,
        data_problems,
        left_out,
        questions,
    })
}

/// Whether a build would still have something to mine without the given
/// tables and fields (an item of the case type and at least one step), and
/// the steps and links it would lose, by name.
pub fn remains(
    pack: &Pack,
    tables: &[String],
    absent: &BTreeMap<String, BTreeSet<String>>,
) -> (bool, BTreeSet<String>) {
    let mut recipe = pack.recipe.clone();
    let names = |recipe: &Value| {
        ["events", "relationships", "event_links"]
            .iter()
            .flat_map(|c| recipe[*c].as_array().cloned().unwrap_or_default())
            .map(|rule| {
                let name = match &rule["activity"]["value"] {
                    Value::String(step) => format!("\"{step}\""),
                    _ => rule["id"].as_str().unwrap_or_default().replace('_', " "),
                };
                (rule["id"].as_str().unwrap_or_default().to_owned(), name)
            })
            .collect::<BTreeMap<_, _>>()
    };
    let before = names(&recipe);
    prune(&mut recipe, tables);
    leave_out_absent(&mut recipe, absent);
    let after = names(&recipe);
    let alive = recipe["events"].as_array().is_some_and(|e| !e.is_empty())
        && recipe["objects"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|o| o["object_type"] == pack.case_object_type.as_str());
    let lost = before
        .into_iter()
        .filter(|(id, _)| !after.contains_key(id))
        .map(|(_, name)| name)
        .collect();
    (alive, lost)
}

/// Questions about the company's own fields on tables that become items.
fn custom_questions(pack: &Pack, tables: &[TableReport]) -> Vec<Question> {
    let mut questions = Vec::new();
    for report in tables {
        let Some(table) = pack.table(&report.table) else {
            continue;
        };
        if table.object.is_none() {
            continue;
        }
        let rows_word = table
            .noun
            .clone()
            .unwrap_or_else(|| report.label.to_lowercase());
        let mut asked_with_a_date = BTreeSet::new();
        for field in report.custom_fields.iter().filter(|f| f.date_like) {
            let mut text = format!(
                "{} of {rows_word} ({} of {}) have a date in the company's own field {}",
                percent(field.filled_share),
                field.filled,
                report.rows,
                field.column
            );
            if let (Some(timing), Some(date_field)) = (&field.timing, &table.date_field) {
                let after = table.date_label.as_deref().unwrap_or(date_field);
                text.push_str(&format!(
                    ", usually {} after {after} (most between {} and {} days)",
                    days(timing.median_days),
                    timing.p10_days,
                    timing.p90_days
                ));
            }
            let partners = field
                .filled_with
                .iter()
                .filter_map(|name| report.custom_fields.iter().find(|f| &f.column == name))
                .filter(|f| !f.date_like)
                .collect::<Vec<_>>();
            for partner in &partners {
                asked_with_a_date.insert(partner.column.clone());
                text.push_str(&format!(
                    ", filled together with {}{}",
                    partner.column,
                    if partner.examples.is_empty() {
                        String::new()
                    } else {
                        format!(" (for example {})", partner.examples.join(", "))
                    }
                ));
            }
            text.push_str(". Is that a real step in the process, such as an approval?");
            let consistent = field
                .timing
                .as_ref()
                .is_none_or(|timing| timing.before_share <= 0.1);
            questions.push(Question {
                id: format!("custom:{}.{}", report.table, field.column),
                kind: "custom_field".into(),
                text,
                options: vec![
                    Choice {
                        id: "step".into(),
                        label: "Yes, show it as a step (name the step)".into(),
                    },
                    Choice {
                        id: "detail".into(),
                        label: "No, keep it as a detail of the item".into(),
                    },
                    Choice {
                        id: "ignore".into(),
                        label: "No, leave it out".into(),
                    },
                ],
                recommended: Some(
                    if field.filled_share >= 0.05 && consistent {
                        "step"
                    } else {
                        "detail"
                    }
                    .into(),
                ),
                needs_value: Some("the step's name, for example \"Approve purchase order\"".into()),
                carries: partners.iter().map(|f| f.column.clone()).collect(),
            });
        }
        for field in report
            .custom_fields
            .iter()
            .filter(|f| !f.date_like && !asked_with_a_date.contains(&f.column) && f.filled > 0)
        {
            questions.push(Question {
                id: format!("custom:{}.{}", report.table, field.column),
                kind: "custom_field".into(),
                text: format!(
                    "{} of {rows_word} have a value in the company's own field {}{}. Should it be kept as a detail you can filter and group by?",
                    percent(field.filled_share),
                    field.column,
                    if field.examples.is_empty() {
                        String::new()
                    } else {
                        format!(" (for example {})", field.examples.join(", "))
                    }
                ),
                options: vec![
                    Choice { id: "detail".into(), label: "Yes, keep it as a detail (name it)".into() },
                    Choice { id: "ignore".into(), label: "No, leave it out".into() },
                ],
                recommended: Some(if field.filled_share >= 0.05 { "detail" } else { "ignore" }.into()),
                needs_value: Some("a business name, for example \"approver\"".into()),
                carries: Vec::new(),
            });
        }
    }
    questions
}

/// A saved answer to one question.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Answer {
    pub choice: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// Who confirmed it, in the company's words (name or role).
    pub confirmed_by: String,
    pub at: String,
    /// The question as it was asked.
    pub question: String,
    /// Fields the question said a step would carry along.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub carries: Vec<String>,
}

/// Checks an answer against the question it answers.
pub fn validate_answer(question: &Question, choice: &str, value: Option<&str>) -> Result<()> {
    if !question.options.iter().any(|o| o.id == choice) {
        return Err(Error(format!(
            "'{choice}' is not an option; choose one of {}",
            question
                .options
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    let needs = match question.kind.as_str() {
        "setting" => true,
        "custom_field" => choice != "ignore",
        _ => false,
    };
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    if needs && value.is_none() {
        return Err(Error(format!(
            "this answer needs a value: {}",
            question.needs_value.as_deref().unwrap_or("a value")
        )));
    }
    if let Some(value) = value
        && (value.len() > 120 || value.chars().any(char::is_control))
    {
        return Err(Error("the value must be a short single line".into()));
    }
    Ok(())
}

#[derive(Debug)]
pub struct Built {
    pub recipe: Recipe,
    /// What the answers changed in the standard recipe, in words.
    pub changes: Vec<String>,
    /// Tables left out because they are optional and were not exported.
    pub left_out: Vec<String>,
}

fn days(n: i64) -> String {
    if n.abs() == 1 {
        format!("{n} day")
    } else {
        format!("{n} days")
    }
}

/// A field name that waits for a setting, such as the company's own field
/// holding the SAP order number; checked once the setting is answered.
fn pending(name: &str) -> bool {
    name.starts_with("{setting:")
}

impl Pack {
    /// The pack with every answered setting filled in; unanswered ones stay
    /// as placeholders, and checks that need them wait.
    pub fn with_settings(&self, answers: &BTreeMap<String, Answer>) -> Result<Self> {
        let settings = self
            .settings
            .iter()
            .filter_map(|s| {
                answers
                    .get(&format!("setting:{}", s.id))
                    .and_then(|a| a.value.clone())
                    .map(|v| (s.id.clone(), v))
            })
            .collect::<BTreeMap<_, _>>();
        let mut value = serde_json::to_value(self).map_err(|e| Error(e.to_string()))?;
        fill(&mut value, &settings);
        serde_json::from_value(value).map_err(|e| Error(e.to_string()))
    }
}

fn fill(value: &mut Value, settings: &BTreeMap<String, String>) {
    match value {
        Value::String(text) => {
            if let Some(found) = text
                .strip_prefix("{setting:")
                .and_then(|rest| rest.strip_suffix('}'))
                .and_then(|id| settings.get(id))
            {
                *text = found.clone();
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|item| fill(item, settings)),
        Value::Object(map) => map.values_mut().for_each(|item| fill(item, settings)),
        _ => {}
    }
}

fn substitute(value: &mut Value, settings: &BTreeMap<String, String>) -> Result<()> {
    match value {
        Value::String(text) => {
            if let Some(id) = text
                .strip_prefix("{setting:")
                .and_then(|rest| rest.strip_suffix('}'))
            {
                *text = settings
                    .get(id)
                    .cloned()
                    .ok_or_else(|| Error(format!("the setting '{id}' has not been answered")))?;
            }
        }
        Value::Array(items) => {
            for item in items {
                substitute(item, settings)?;
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                substitute(item, settings)?;
            }
        }
        _ => {}
    }
    Ok(())
}

/// Removes every rule that reads one of `tables`, then everything that pointed
/// at what those rules produced: links to their objects, links of their
/// events, and steps left with nothing to belong to.
fn prune(recipe: &mut Value, tables: &[String]) {
    if tables.is_empty() {
        return;
    }
    let reads = |rule: &Value| {
        tables.iter().any(|t| {
            rule["input"] == *t
                || rule["lookup"]["input"] == *t
                || rule["lookups"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|l| l["input"] == *t)
        })
    };
    prune_rules(recipe, &reads);
}

/// Removes the rules `reads` names, and every rule that depends on them:
/// links to removed objects, events left with no object, and carried links
/// through removed rules.
fn prune_rules(recipe: &mut Value, reads: &dyn Fn(&Value) -> bool) {
    let ids = |recipe: &Value, category: &str, gone: &dyn Fn(&Value) -> bool| {
        recipe[category]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|r| gone(r))
            .filter_map(|r| r["id"].as_str().map(str::to_owned))
            .collect::<BTreeSet<_>>()
    };
    let objects = ids(recipe, "objects", &reads);
    let to_gone = |reference: &Value| {
        reference["object"]
            .as_str()
            .is_some_and(|o| objects.contains(o))
    };
    if let Some(rules) = recipe["events"].as_array_mut() {
        for rule in rules.iter_mut() {
            if let Some(references) = rule["objects"].as_array_mut() {
                references.retain(|r| !to_gone(r));
            }
        }
    }
    let relationships = ids(recipe, "relationships", &|r| {
        reads(r) || to_gone(&r["source"]) || to_gone(&r["target"])
    });
    let linked_events = recipe["event_links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|l| !reads(l) && !to_gone(&l["target"]))
        .filter_map(|l| l["event"].as_str().map(str::to_owned))
        .collect::<BTreeSet<_>>();
    let events = ids(recipe, "events", &|r| {
        reads(r)
            || (r["objects"].as_array().is_none_or(Vec::is_empty)
                && !r["id"]
                    .as_str()
                    .is_some_and(|id| linked_events.contains(id)))
    });
    let gone_event = |rule: &Value| rule["event"].as_str().is_some_and(|e| events.contains(e));
    for category in [
        "objects",
        "events",
        "relationships",
        "event_links",
        "carried_links",
    ] {
        let Some(rules) = recipe.get_mut(category).and_then(Value::as_array_mut) else {
            continue;
        };
        rules.retain(|r| {
            let id = r["id"].as_str().unwrap_or_default();
            match category {
                "objects" => !objects.contains(id),
                "events" => !events.contains(id),
                "relationships" => !relationships.contains(id),
                "event_links" => !reads(r) && !gone_event(r) && !to_gone(&r["target"]),
                _ => {
                    !gone_event(r)
                        && !r["through"].as_str().is_some_and(|o| objects.contains(o))
                        && !r["relationship"]
                            .as_str()
                            .is_some_and(|rel| relationships.contains(rel))
                }
            }
        });
    }
}

fn slug(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Builds the recipe from a pack, each exported table's SHA-256 and the
/// saved answers. Unanswered custom-field questions leave the standard
/// recipe unchanged; unanswered settings stop the build.
pub fn build_recipe(
    pack: &Pack,
    hashes: &BTreeMap<String, String>,
    answers: &BTreeMap<String, Answer>,
) -> Result<Built> {
    build_recipe_with(pack, hashes, answers, &BTreeMap::new())
}

/// What the check found about the export's columns, per table: required
/// fields with a stand-in column, and optional fields that are absent.
#[derive(Debug, Clone, Default)]
pub struct Exported {
    pub fallbacks: BTreeMap<String, String>,
    pub absent: Vec<String>,
}

impl Exported {
    /// Per table, from a saved check report's `tables`.
    pub fn from_report(tables: &[TableReport]) -> BTreeMap<String, Exported> {
        tables
            .iter()
            .map(|t| {
                (
                    t.table.clone(),
                    Exported {
                        fallbacks: t.fallbacks_used.clone(),
                        absent: t
                            .absent_fields
                            .iter()
                            .chain(&t.missing_fields)
                            .cloned()
                            .collect(),
                    },
                )
            })
            .collect()
    }
}

/// [`build_recipe`] for an export as the check found it: stand-in columns
/// for missing required fields, and absent optional fields left out.
pub fn build_recipe_with(
    pack: &Pack,
    hashes: &BTreeMap<String, String>,
    answers: &BTreeMap<String, Answer>,
    exported: &BTreeMap<String, Exported>,
) -> Result<Built> {
    let settings = pack
        .settings
        .iter()
        .filter_map(|s| {
            answers
                .get(&format!("setting:{}", s.id))
                .and_then(|a| a.value.clone())
                .map(|v| (s.id.clone(), v))
        })
        .collect::<BTreeMap<_, _>>();
    let mut recipe = pack.recipe.clone();
    let mut changes = Vec::new();
    // Company fields the person confirmed, applied to the standard recipe.
    for (id, answer) in answers {
        let Some((table_name, column)) = id
            .strip_prefix("custom:")
            .and_then(|rest| rest.split_once('.'))
        else {
            continue;
        };
        let table = pack
            .table(table_name)
            .ok_or_else(|| Error(format!("answer {id} names an unknown table")))?;
        let Some(object) = &table.object else {
            continue;
        };
        let value = answer.value.clone().unwrap_or_default();
        match answer.choice.as_str() {
            "step" => {
                let rule = format!("custom_{}_{}", slug(table_name), slug(column));
                let object_rule = recipe["objects"]
                    .as_array()
                    .and_then(|rules| rules.iter().find(|r| r["id"] == *object))
                    .ok_or_else(|| Error(format!("pack has no object rule {object}")))?
                    .clone();
                recipe["events"].as_array_mut().unwrap().push(json!({
                    "id": rule, "input": table_name, "namespace": object_rule["namespace"],
                    "key": table.key, "activity": {"value": value},
                    "timestamp": {"date": column, "offset": "{setting:offset}"},
                    "optional_timestamp": true,
                    "objects": [{"object": object, "columns": table.key, "qualifier": object_rule["object_type"]}],
                    "attributes": answer.carries.iter().map(|c| json!({"column": c, "name": c})).collect::<Vec<_>>(),
                }));
                for (n, link) in table.fan_out.iter().enumerate() {
                    let mut link = link.clone();
                    link["id"] = json!(format!("{rule}_lines_{n}"));
                    link["event"] = json!(rule);
                    // Rows whose item never took this step have no event.
                    link["optional_event"] = json!(true);
                    recipe["event_links"]
                        .as_array_mut()
                        .ok_or_else(|| Error("pack recipe needs event_links".into()))?
                        .push(link);
                }
                changes.push(format!(
                    "Step \"{value}\" from {table_name}.{column}, confirmed by {}",
                    answer.confirmed_by
                ));
            }
            "detail" => {
                let rules = recipe["objects"].as_array_mut().unwrap();
                let rule = rules
                    .iter_mut()
                    .find(|r| r["id"] == *object)
                    .ok_or_else(|| Error(format!("pack has no object rule {object}")))?;
                rule["attributes"]
                    .as_array_mut()
                    .map(|a| a.push(json!({"column": column, "name": value})))
                    .ok_or_else(|| Error("object rule needs attributes".into()))?;
                changes.push(format!(
                    "Detail \"{value}\" from {table_name}.{column}, confirmed by {}",
                    answer.confirmed_by
                ));
            }
            _ => {}
        }
    }
    // Leave out optional tables that were not exported, and their rules.
    let mut left_out = Vec::new();
    for table in &pack.tables {
        if hashes.contains_key(&table.name) {
            continue;
        }
        if !table.optional {
            changes.push(format!(
                "Table {} ({}) was not exported; the steps that need it are left out.",
                table.name, table.label
            ));
        }
        left_out.push(table.name.clone());
    }
    prune(&mut recipe, &left_out);
    for (table_name, export) in exported {
        let Some(table) = pack.table(table_name) else {
            continue;
        };
        for fallback in table
            .fallbacks
            .iter()
            .filter(|f| export.fallbacks.get(&f.field) == Some(&f.column))
        {
            stand_in(
                &mut recipe,
                table_name,
                fallback,
                pack.settings.iter().any(|s| s.id == "offset"),
            );
            changes.push(format!(
                "{table_name}: {} was not exported, so {} is used instead{}.",
                fallback.field,
                fallback.column,
                if fallback.date_only {
                    " (to the day)"
                } else {
                    ""
                }
            ));
        }
    }
    substitute(&mut recipe, &settings)?;
    // Pack exports give single-column times in UTC (the export instructions
    // ask for it). Saying so explicitly lets a time written without its zone
    // (`2026-01-05 14:30:00`, or a date alone) be read as the check reads it,
    // instead of every row failing; a time with its zone keeps it.
    for rule in recipe
        .get_mut("events")
        .and_then(Value::as_array_mut)
        .into_iter()
        .flatten()
    {
        if let Some(column) = rule["timestamp"].as_str().map(str::to_owned) {
            rule["timestamp"] = json!({"date": column, "offset": "+00:00"});
        }
    }
    // Optional fields the export lacks.
    let absent = exported
        .iter()
        .filter_map(|(table_name, export)| {
            let table = pack.table(table_name)?;
            let fields = export
                .absent
                .iter()
                .filter(|f| table.fields.contains(f) || table.required.contains(f))
                .cloned()
                .collect::<BTreeSet<_>>();
            (!fields.is_empty()).then(|| (table_name.clone(), fields))
        })
        .collect::<BTreeMap<_, _>>();
    for (field, rules) in leave_out_absent(&mut recipe, &absent) {
        changes.push(format!(
            "{field} was not exported, so these steps or links are left out: {}.",
            rules.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    let mut used = BTreeSet::new();
    for category in ["objects", "events", "relationships", "event_links"] {
        for rule in recipe[category].as_array().into_iter().flatten() {
            for input in [&rule["input"], &rule["lookup"]["input"]] {
                if let Some(input) = input.as_str() {
                    used.insert(input.to_owned());
                }
            }
            for lookup in rule["lookups"].as_array().into_iter().flatten() {
                if let Some(input) = lookup["input"].as_str() {
                    used.insert(input.to_owned());
                }
            }
        }
    }
    recipe["inputs"] = json!(
        used.iter()
            .map(|table| json!({"id": table, "sha256": hashes[table]}))
            .collect::<Vec<_>>()
    );
    if recipe["events"].as_array().is_none_or(Vec::is_empty) {
        return Err(Error(
            "nothing is left to build: no step can be made from the exported tables".into(),
        ));
    }
    let mut recipe: Recipe = serde_json::from_value(recipe)
        .map_err(|error| Error(format!("the pack's recipe is invalid: {error}")))?;
    if let Some(answer) = answers.get(SET_ASIDE).filter(|a| a.choice == "set_aside") {
        recipe.set_aside = true;
        changes.push(format!(
            "Records that do not fit are left out and listed as a data-quality finding (confirmed by {}).",
            answer.confirmed_by
        ));
    }
    Ok(Built {
        recipe,
        changes,
        left_out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack() -> Pack {
        Pack::parse(BUILT_IN[0].1).unwrap()
    }

    #[test]
    fn the_sap_pack_parses_and_lists_its_export() {
        let pack = pack();
        assert_eq!(pack.status, "tested");
        let list = pack.checklist();
        assert_eq!(list["tables"].as_array().unwrap().len(), 12);
        assert!(
            list["tables"][2]["fields"]
                .as_array()
                .unwrap()
                .contains(&json!("AEDAT"))
        );
    }

    #[test]
    fn settings_must_be_answered_before_building() {
        let pack = pack();
        let hashes = pack
            .tables
            .iter()
            .map(|t| (t.name.clone(), "a".repeat(64)))
            .collect();
        let error = build_recipe(&pack, &hashes, &BTreeMap::new()).unwrap_err();
        assert!(error.to_string().contains("offset"));
        let answers = BTreeMap::from([(
            "setting:offset".to_owned(),
            Answer {
                choice: "value".into(),
                value: Some("+01:00".into()),
                confirmed_by: "AP lead".into(),
                at: "2026-10-12T00:00:00Z".into(),
                question: "clock".into(),
                carries: Vec::new(),
            },
        )]);
        let built = build_recipe(&pack, &hashes, &answers).unwrap();
        assert!(built.changes.is_empty());
        assert_eq!(built.recipe.inputs.len(), 12);
    }

    #[test]
    fn answers_are_checked_against_their_question() {
        let question = Question {
            id: "custom:EKKO.ZZAPPR_DAT".into(),
            kind: "custom_field".into(),
            text: String::new(),
            options: vec![
                Choice {
                    id: "step".into(),
                    label: String::new(),
                },
                Choice {
                    id: "ignore".into(),
                    label: String::new(),
                },
            ],
            recommended: None,
            needs_value: Some("name".into()),
            carries: Vec::new(),
        };
        assert!(validate_answer(&question, "maybe", None).is_err());
        assert!(validate_answer(&question, "step", None).is_err());
        assert!(validate_answer(&question, "step", Some("Approve order")).is_ok());
        assert!(validate_answer(&question, "ignore", None).is_ok());
    }
}
