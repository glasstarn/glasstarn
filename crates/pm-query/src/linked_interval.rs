//! Cross-object intervals using explicit, one-hop recorded correlation.
use crate::exploration::{Exploration, ExplorationRequest};
use pm_core::{
    EventLog, EventTypeId, ObjectTypeId, OcelFilter,
    segment::{Occurrence, Segment},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkedInterval {
    pub root_type: String,
    pub from_type: String,
    pub to_type: String,
    pub segment: Segment,
    #[serde(default)]
    pub group_by: Option<String>,
}
#[derive(Serialize)]
pub struct LinkedExploration {
    pub report: Exploration,
    pub selected_filter: OcelFilter,
    pub definition: LinkedInterval,
}
/// Root attributes and activity contrasts describe `roots`, the active followed
/// population. Endpoint dates may belong to directly linked object types.
pub fn explore_linked(
    path: &std::path::Path,
    filter: &OcelFilter,
    roots: &EventLog,
    query: &LinkedInterval,
) -> Result<LinkedExploration, String> {
    let rows = endpoints(path, filter, query)?;
    let allowed = roots
        .traces
        .iter()
        .filter_map(|trace| roots.cases.label(trace.case.0))
        .collect::<std::collections::BTreeSet<_>>();
    let mut durations = std::collections::HashMap::new();
    let mut chosen = std::collections::BTreeSet::new();
    for (name, from, to) in rows {
        if !allowed.contains(name.as_str()) {
            continue;
        }
        let duration = from
            .zip(to)
            .and_then(|(a, b)| b.checked_sub(a))
            .filter(|d| *d >= 0);
        if duration.is_some_and(|d| query.segment.contains(d)) {
            chosen.insert(name.clone());
        }
        durations.insert(name, duration);
    }
    let request = ExplorationRequest {
        segment: Some(query.segment.clone()),
        group_by: query.group_by.clone(),
        ..Default::default()
    };
    let mut report = crate::exploration::explore_with_durations(roots, &request, Some(&durations))?;
    report.semantics="Each active root object is counted once. Endpoints come from the selected root type or one-hop linked objects through a recorded object relationship or shared event, within the active native scope. First/last is evaluated over all matching endpoints on those linked objects: last delivery means the latest related delivery, not the average shipment. No multi-hop traversal through a customer, product or vendor is invented. Missing/reversed endpoints have no duration. Histograms use upper-exclusive bins; typed range bounds are inclusive. Root attributes and contrasts describe the followed root history. Contrasts compare only measurable intervals in range versus measurable intervals outside it. Missing intervals remain in population totals, not the contrast baseline. Associations do not prove cause.".into();
    let mut selected_filter = filter.clone();
    selected_filter.cohort = Some(pm_core::OcelCohort {
        parent: filter.cohort.clone().map(Box::new),
        root_type: query.root_type.clone(),
        roots: chosen,
        paths: [&query.from_type, &query.to_type]
            .into_iter()
            .filter(|kind| **kind != query.root_type)
            .map(|kind| vec![query.root_type.clone(), kind.clone()])
            .collect(),
    });
    Ok(LinkedExploration {
        report,
        selected_filter,
        definition: query.clone(),
    })
}

type Endpoints = Vec<(String, Option<i64>, Option<i64>)>;
fn endpoints(
    path: &std::path::Path,
    filter: &OcelFilter,
    query: &LinkedInterval,
) -> Result<Endpoints, String> {
    let sqlite = path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("sqlite") || x.eq_ignore_ascii_case("sqlite3"));
    if !sqlite {
        return loaded_endpoints(path, filter, query);
    }
    let log = crate::ocel::open_disk(path).map_err(|e| e.to_string())?;
    if !log.quality().map_err(|e| e.to_string())?.ready_for_analysis {
        return Err("connected data checks must pass before measuring intervals".into());
    }
    let type_id = |name: &str| {
        log.object_types
            .iter()
            .position(|value| value == name)
            .map(|id| ObjectTypeId(id as u32))
            .ok_or_else(|| format!("unknown object type: {name}"))
    };
    let event_id = |name: &str| {
        log.event_types
            .iter()
            .position(|value| value == name)
            .map(|id| EventTypeId(id as u32))
            .ok_or_else(|| format!("unknown activity: {name}"))
    };
    let view = log.view(filter).map_err(|e| e.to_string())?;
    let rows = log
        .linked_interval(
            &view,
            [
                type_id(&query.root_type)?,
                type_id(&query.from_type)?,
                type_id(&query.to_type)?,
            ],
            [event_id(&query.segment.from)?, event_id(&query.segment.to)?],
            [
                query.segment.from_occurrence == Occurrence::Last,
                query.segment.to_occurrence == Occurrence::Last,
            ],
        )
        .map_err(|e| e.to_string())?;
    rows.into_iter()
        .map(|(id, from, to)| Ok((log.object_label(id).map_err(|e| e.to_string())?, from, to)))
        .collect()
}
fn loaded_endpoints(
    path: &std::path::Path,
    filter: &OcelFilter,
    query: &LinkedInterval,
) -> Result<Endpoints, String> {
    use std::collections::{BTreeMap, BTreeSet};
    let log = pm_ingest::read_ocel(path).map_err(|e| e.to_string())?;
    if !pm_ingest::assess_ocel_quality(&log).ready_for_analysis {
        return Err("connected data checks must pass before measuring intervals".into());
    }
    let types = [&query.root_type, &query.from_type, &query.to_type].map(|name| {
        log.object_types
            .iter()
            .position(|kind| kind.name == *name)
            .map(|id| ObjectTypeId(id as u32))
            .ok_or_else(|| format!("unknown object type: {name}"))
    });
    let [root, from_type, to_type] = types;
    let (root, from_type, to_type) = (root?, from_type?, to_type?);
    let event = |name: &str| {
        log.event_types
            .iter()
            .position(|kind| kind.name == name)
            .map(|id| EventTypeId(id as u32))
            .ok_or_else(|| format!("unknown activity: {name}"))
    };
    let from = event(&query.segment.from)?;
    let to = event(&query.segment.to)?;
    let view = pm_core::OcelView::apply(&log, filter).map_err(|e| e.to_string())?;
    let roots = view
        .objects
        .iter()
        .copied()
        .filter(|id| log.objects[id.0 as usize].object_type == root)
        .collect::<BTreeSet<_>>();
    let mut neighbours = roots
        .iter()
        .map(|id| (*id, BTreeSet::from([*id])))
        .collect::<BTreeMap<_, _>>();
    for index in &view.object_object_relations {
        let link = log.object_object_relations[*index];
        for (a, b) in [(link.source, link.target), (link.target, link.source)] {
            if let Some(items) = neighbours.get_mut(&a) {
                items.insert(b);
            }
        }
    }
    let mut shared = BTreeMap::<u32, BTreeSet<pm_core::ObjectId>>::new();
    let mut dates = BTreeMap::<pm_core::ObjectId, (Option<i64>, Option<i64>)>::new();
    let choose = |old: Option<i64>, new: i64, last: bool| {
        Some(old.map_or(
            new,
            |prior| if last { prior.max(new) } else { prior.min(new) },
        ))
    };
    for index in &view.event_object_relations {
        let link = log.event_object_relations[*index];
        let event = log.events[link.event.0 as usize];
        let kind = log.objects[link.object.0 as usize].object_type;
        shared.entry(link.event.0).or_default().insert(link.object);
        let date = dates.entry(link.object).or_default();
        if kind == from_type && event.event_type == from {
            date.0 = choose(
                date.0,
                event.at,
                query.segment.from_occurrence == Occurrence::Last,
            );
        }
        if kind == to_type && event.event_type == to {
            date.1 = choose(
                date.1,
                event.at,
                query.segment.to_occurrence == Occurrence::Last,
            );
        }
    }
    for items in shared.values() {
        for id in items.intersection(&roots) {
            neighbours
                .get_mut(id)
                .unwrap()
                .extend(items.iter().copied());
        }
    }
    Ok(neighbours
        .into_iter()
        .map(|(id, items)| {
            let mut source = None;
            let mut target = None;
            for item in items {
                if let Some((a, b)) = dates.get(&item) {
                    if let Some(a) = a {
                        source = choose(
                            source,
                            *a,
                            query.segment.from_occurrence == Occurrence::Last,
                        );
                    }
                    if let Some(b) = b {
                        target =
                            choose(target, *b, query.segment.to_occurrence == Occurrence::Last);
                    }
                }
            }
            (
                log.object_ids.label(id.0).unwrap_or("").to_owned(),
                source,
                target,
            )
        })
        .collect())
}

/// Native entry point shared by CLI and MCP. The browser can supply its already
/// filtered root log to `explore_linked` instead.
pub fn native_roots(
    path: &std::path::Path,
    filter: &OcelFilter,
    root_type: &str,
) -> Result<EventLog, String> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    let summary = crate::follow_ocel(path, filter, root_type, file.as_file_mut())
        .map_err(|e| e.to_string())?;
    let loaded = crate::load_log(
        file.path(),
        Some(summary.mapping()),
        &pm_ingest::Policy::new(),
        Default::default(),
    )
    .map_err(|e| e.to_string())?
    .map_err(|_| "followed roots failed data checks")?;
    Ok(loaded.log)
}
pub fn explore_native(
    path: &std::path::Path,
    filter: &OcelFilter,
    query: &LinkedInterval,
) -> Result<LinkedExploration, String> {
    let roots = native_roots(path, filter, &query.root_type)?;
    explore_linked(path, filter, &roots, query)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn split_deliveries_count_roots_once_and_do_not_expand_through_another_order() {
        // O1 has two directly linked deliveries at day 2 and day 12.
        // O2 shares a vendor with O1 but has its own day-3 delivery.
        // O3 has no shipment. A vendor is not an implicit join path.
        let mut file = tempfile::Builder::new()
            .suffix(".jsonocel")
            .tempfile()
            .unwrap();
        let objects = vec![
            json!({"id":"O1","type":"order","attributes":[],"relationships":[{"objectId":"D1","qualifier":"shipment"},{"objectId":"D2","qualifier":"shipment"},{"objectId":"V","qualifier":"vendor"}]}),
            json!({"id":"O2","type":"order","attributes":[],"relationships":[{"objectId":"D3","qualifier":"shipment"},{"objectId":"V","qualifier":"vendor"}]}),
            json!({"id":"O3","type":"order","attributes":[],"relationships":[]}),
            json!({"id":"D1","type":"delivery","attributes":[],"relationships":[]}),
            json!({"id":"D2","type":"delivery","attributes":[],"relationships":[]}),
            json!({"id":"D3","type":"delivery","attributes":[],"relationships":[]}),
            json!({"id":"V","type":"vendor","attributes":[],"relationships":[]}),
        ];
        let event = |id: &str, kind: &str, date: &str, object: &str| json!({"id":id,"type":kind,"time":date,"attributes":[],"relationships":[{"objectId":object,"qualifier":"involves"}]});
        let data = json!({"objectTypes":[{"name":"order","attributes":[]},{"name":"delivery","attributes":[]},{"name":"vendor","attributes":[]}],"eventTypes":[{"name":"Create","attributes":[]},{"name":"Ship","attributes":[]}],"objects":objects,"events":[event("E1","Create","2026-01-01T00:00:00Z","O1"),event("E2","Create","2026-01-01T00:00:00Z","O2"),event("E3","Create","2026-01-01T00:00:00Z","O3"),event("S1","Ship","2026-01-03T00:00:00Z","D1"),event("S2","Ship","2026-01-13T00:00:00Z","D2"),event("S3","Ship","2026-01-04T00:00:00Z","D3")]});
        serde_json::to_writer(file.as_file_mut(), &data).unwrap();
        let mut query = LinkedInterval {
            root_type: "order".into(),
            from_type: "order".into(),
            to_type: "delivery".into(),
            segment: Segment {
                from: "Create".into(),
                to: "Ship".into(),
                to_occurrence: Occurrence::Last,
                min_micros: Some(10 * 86_400_000_000),
                max_micros: Some(18 * 86_400_000_000),
                ..Default::default()
            },
            group_by: None,
        };
        let report = explore_native(file.path(), &Default::default(), &query).unwrap();
        assert_eq!(
            (
                report.report.overall.items,
                report.report.overall.measurable
            ),
            (3, 2)
        );
        assert_eq!(report.report.selected.items, 1);
        assert_eq!(report.report.selected.examples, ["O1"]);
        assert_eq!(report.report.overall.mean_seconds, Some(648000));
        assert_eq!(
            report.selected_filter.cohort.unwrap().roots,
            std::collections::BTreeSet::from(["O1".into()])
        );
        query.segment.to_occurrence = Occurrence::First;
        let report = explore_native(file.path(), &Default::default(), &query).unwrap();
        assert_eq!(report.report.overall.mean_seconds, Some(216000));
        assert_eq!(report.report.selected.items, 0);
    }
}

#[cfg(test)]
mod disk_tests {
    use super::*;
    #[test]
    fn sqlite_uses_direct_links_and_shared_events_without_multihop_expansion() {
        // Canonical order-management fixture: two items are placed at day 0,
        // packed into one package at day 1 and delivered at day 2. The order
        // itself has no direct relationship/shared event with the package.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../testdata/ocel/order-management.sqlite");
        let mut query = LinkedInterval {
            root_type: "item".into(),
            from_type: "item".into(),
            to_type: "package".into(),
            segment: Segment {
                from: "place order".into(),
                to: "deliver package".into(),
                to_occurrence: Occurrence::Last,
                min_micros: Some(2 * 86_400_000_000),
                max_micros: Some(2 * 86_400_000_000),
                ..Default::default()
            },
            group_by: None,
        };
        let report = explore_native(&path, &Default::default(), &query).unwrap();
        assert_eq!(
            (
                report.report.overall.items,
                report.report.overall.measurable,
                report.report.selected.items
            ),
            (2, 2, 2)
        );
        assert_eq!(report.report.overall.mean_seconds, Some(172800));
        query.root_type = "order".into();
        query.from_type = "order".into();
        let report = explore_native(&path, &Default::default(), &query).unwrap();
        assert_eq!(
            (
                report.report.overall.items,
                report.report.overall.measurable
            ),
            (1, 0)
        );
    }
}
