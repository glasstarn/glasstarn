//! A page of items: never the whole log at once. Sorting and counting happen
//! here, so a list of 100,000 items costs one page to show.

use crate::timing::SlowCase;
use pm_core::{EventLog, Variants};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// The order of the page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemOrder {
    #[default]
    Slowest,
    Fastest,
    Newest,
    Oldest,
    Identifier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ItemPage {
    /// Items matching the search in the view.
    pub total: usize,
    pub offset: usize,
    pub items: Vec<SlowCase>,
}

/// Items of `log` in `order`, those whose identifier contains `search`
/// (ignoring case), from `offset`, at most `limit`.
pub fn item_page(
    log: &EventLog,
    order: ItemOrder,
    search: &str,
    offset: usize,
    limit: usize,
) -> ItemPage {
    let variants = Variants::build(log);
    let rank = variants
        .variants
        .iter()
        .enumerate()
        .map(|(index, variant)| (variant.sequence.clone(), index + 1))
        .collect::<BTreeMap<_, _>>();
    let needle = search.trim().to_lowercase();
    let mut rows = log
        .traces
        .iter()
        .filter(|trace| {
            needle.is_empty() || log.case_label(trace.case).to_lowercase().contains(&needle)
        })
        .collect::<Vec<_>>();
    match order {
        ItemOrder::Slowest => rows.sort_by(|a, b| {
            b.throughput()
                .cmp(&a.throughput())
                .then(a.case.cmp(&b.case))
        }),
        ItemOrder::Fastest => rows.sort_by(|a, b| {
            a.throughput()
                .cmp(&b.throughput())
                .then(a.case.cmp(&b.case))
        }),
        ItemOrder::Newest => {
            rows.sort_by(|a, b| b.start().cmp(&a.start()).then(a.case.cmp(&b.case)))
        }
        ItemOrder::Oldest => {
            rows.sort_by(|a, b| a.start().cmp(&b.start()).then(a.case.cmp(&b.case)))
        }
        ItemOrder::Identifier => {
            rows.sort_by(|a, b| log.case_label(a.case).cmp(log.case_label(b.case)))
        }
    }
    let total = rows.len();
    let items = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|trace| {
            let sequence = trace
                .events
                .iter()
                .map(|event| event.activity)
                .collect::<Vec<_>>();
            let distinct = sequence.iter().collect::<BTreeSet<_>>().len();
            SlowCase {
                case: log.case_label(trace.case).to_owned(),
                started_at_micros: trace.start(),
                duration_seconds: trace.throughput() / 1_000_000,
                events: trace.events.len(),
                route_rank: rank.get(&sequence).copied().unwrap_or(0),
                repeats_step: distinct < sequence.len(),
            }
        })
        .collect();
    ItemPage {
        total,
        offset,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{ActivityId, CaseId, CaseTrace, Dictionary, Event};

    fn log() -> EventLog {
        let mut log = EventLog::default();
        let mut cases = Dictionary::new();
        for (id, steps) in [
            ("A-1", &[0, 10][..]),
            ("B-2", &[5, 100]),
            ("C-3", &[20, 25, 30]),
        ] {
            let case = CaseId(cases.intern(id));
            let events = steps
                .iter()
                .enumerate()
                .map(|(i, at)| {
                    Event::new(
                        ActivityId(
                            log.activities
                                .intern(if i == 1 { "Pay" } else { "Receive" }),
                        ),
                        *at * 1_000_000,
                        i as u32 + 2,
                    )
                })
                .collect();
            log.traces.push(CaseTrace {
                case,
                events,
                attributes: vec![],
                incomplete: false,
            });
        }
        log.cases = cases;
        log
    }

    #[test]
    fn pages_are_sorted_counted_and_searchable() {
        let log = log();
        let page = item_page(&log, ItemOrder::Slowest, "", 0, 2);
        assert_eq!(page.total, 3);
        assert_eq!(
            page.items
                .iter()
                .map(|i| i.case.as_str())
                .collect::<Vec<_>>(),
            ["B-2", "A-1"]
        );
        assert_eq!(page.items[0].duration_seconds, 95);
        let next = item_page(&log, ItemOrder::Slowest, "", 2, 2);
        assert_eq!(next.items.len(), 1);
        assert!(next.items[0].repeats_step, "C-3 receives twice");
        let found = item_page(&log, ItemOrder::Identifier, "b-", 0, 50);
        assert_eq!((found.total, found.items[0].case.as_str()), (1, "B-2"));
        assert_eq!(
            item_page(&log, ItemOrder::Newest, "", 0, 1).items[0].case,
            "C-3"
        );
    }
}
