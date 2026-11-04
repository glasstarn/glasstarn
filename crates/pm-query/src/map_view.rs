//! A readable process map: which part of the process the map shows.
//!
//! A full directly-follows graph is unreadable on real logs. The explorer
//! starts from the most common route and adds detail on request, in one of two
//! ways:
//!
//! - **Routes** (the default): the map is exactly the union of the selected
//!   variants. Every drawn connection occurred in a covered case, and coverage
//!   is the share of cases that followed one of the selected routes.
//! - **Limits**: the most frequent activities and connections, as in a
//!   frequency-filtered map. Useful when variants are fragmented. Retained
//!   connections can combine into paths no case took, so coverage counts only
//!   cases whose whole recorded path stays inside the map.
//!
//! Limits preserve the most common route; exact variant selections use their
//! most frequent selected route as the layout spine.
//! Every count and the coverage line are computed here, never in a front end.

use crate::analysis::{Edge, Node, map_elements};
use pm_core::dfg::Dfg;
use pm_core::dict::ActivityId;
use pm_core::log::EventLog;
use pm_core::variants::Variants;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Ranked variants listed with a map view; the rest are counted, not listed.
pub const LISTED_VARIANTS: usize = 100;

/// A connection is slow when its average time is at least this multiple of the
/// median average across all measured connections in the active view.
pub const SLOW_MULTIPLE: i64 = 2;

/// Fewest measured connections needed before any is called slow.
pub const SLOW_MIN_CONNECTIONS: usize = 3;

/// Which part of the process to draw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum MapSelection {
    /// The exact union of these variants, by one-based frequency rank. An
    /// empty selection defaults to rank 1.
    Routes {
        #[serde(default)]
        variants: Vec<usize>,
    },
    /// The most frequent activities and connections. Both are raised, if
    /// needed, to include the most common route.
    Limits {
        activities: usize,
        connections: usize,
    },
}

impl Default for MapSelection {
    fn default() -> Self {
        MapSelection::Routes { variants: vec![1] }
    }
}

/// Why a map view could not be produced.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum MapViewError {
    #[error("the active view contains no cases")]
    Empty,
    #[error("variant {rank} does not exist; the active view has {available} variants")]
    UnknownVariant { rank: usize, available: usize },
}

/// One activity in the frequency ranking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RankedActivity {
    /// One-based, most frequent first; ties broken by label.
    pub rank: usize,
    pub activity: String,
    pub frequency: u64,
    pub case_frequency: u64,
    pub shown: bool,
}

/// One route in the variant ranking.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RankedVariant {
    /// One-based, most frequent first, matching the analysis variant order.
    pub rank: usize,
    pub sequence: Vec<String>,
    pub cases: usize,
    pub percent: f64,
    /// Whether this route is drawn (Routes) or fully inside the map (Limits).
    pub shown: bool,
    pub example_cases: Vec<String>,
    pub duration_eligible_cases: usize,
    pub mean_throughput_seconds: i64,
}

/// A connection on the map, with the engine's slowness judgement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapEdge {
    #[serde(flatten)]
    pub edge: Edge,
    /// Average time at least [`SLOW_MULTIPLE`] times the median connection
    /// average in the active view.
    pub slow: bool,
}

/// What the map shows and what it leaves out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MapView {
    /// The selection after normalization (empty routes defaulted, limits raised).
    pub selection: MapSelection,
    /// The most frequent selected route (or global main route for Limits),
    /// drawn as the map's spine.
    pub top_route: Vec<String>,
    pub nodes: Vec<Node>,
    pub edges: Vec<MapEdge>,
    pub activities_shown: usize,
    pub activities_total: usize,
    pub connections_shown: usize,
    pub connections_total: usize,
    /// Routes drawn (Routes) or routes lying fully inside the map (Limits).
    pub variants_shown: usize,
    pub variants_total: usize,
    /// Cases whose whole recorded path is drawn on the map.
    pub cases_covered: usize,
    pub cases_total: usize,
    pub percent_covered: f64,
    /// Whether connection times exclude processing time.
    pub measures_true_waiting: bool,
    /// Average time from which a connection counts as slow; `None` when too
    /// few connections are measurable to judge.
    pub slow_threshold_seconds: Option<i64>,
    /// Every activity, most frequent first.
    pub activity_ranking: Vec<RankedActivity>,
    /// The most common routes, up to [`LISTED_VARIANTS`].
    pub variants: Vec<RankedVariant>,
    /// What the counts mean, for any front end to show verbatim.
    pub notes: Vec<String>,
    /// The money passing through each step and connection, when the items
    /// carry an amount. Absent from older caches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub values: Option<MapValues>,
}

/// Amounts summed over the items that pass through each step or connection
/// (an item counts once per step, however often it repeats it).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct MapValues {
    pub field: String,
    pub steps: std::collections::BTreeMap<String, f64>,
    /// "from\u{1}to" → amount.
    pub connections: std::collections::BTreeMap<String, f64>,
}

/// The amount through each step and connection, for the log's main amount
/// field; `None` when no field holds numbers.
pub(crate) fn map_values(log: &EventLog) -> Option<MapValues> {
    let fields = pm_core::number_fields(log);
    let field = pm_core::main_number_field(&fields)?;
    let index = log.attribute_index(&field.name)?;
    let mut values = MapValues {
        field: field.name.clone(),
        ..Default::default()
    };
    for trace in &log.traces {
        let Some(amount) = pm_core::case_number(log, trace, index) else {
            continue;
        };
        let mut steps = std::collections::BTreeSet::new();
        let mut pairs = std::collections::BTreeSet::new();
        for (position, event) in trace.events.iter().enumerate() {
            steps.insert(event.activity.0);
            if let Some(next) = trace.events.get(position + 1) {
                pairs.insert((event.activity.0, next.activity.0));
            }
        }
        for step in steps {
            *values
                .steps
                .entry(log.activities.label(step).unwrap_or("").to_owned())
                .or_default() += amount;
        }
        for (from, to) in pairs {
            let key = format!(
                "{}\u{1}{}",
                log.activities.label(from).unwrap_or(""),
                log.activities.label(to).unwrap_or("")
            );
            *values.connections.entry(key).or_default() += amount;
        }
    }
    Some(values)
}

/// Builds the map view of an (already filtered) log.
pub fn map_view(log: &EventLog, selection: &MapSelection) -> Result<MapView, MapViewError> {
    let variants = Variants::build(log);
    let Some(top) = variants.variants.first() else {
        return Err(MapViewError::Empty);
    };
    let full = Dfg::build(log);
    let slow_threshold = slow_threshold(&full);
    let ranking = activity_ranking(&full, log);
    let top_activities: BTreeSet<ActivityId> = top.sequence.iter().copied().collect();
    let top_edges: BTreeSet<(ActivityId, ActivityId)> = pairs(&top.sequence).collect();

    let (selection, drawn_log, retained_activities, retained_edges, shown_routes) = match selection
    {
        MapSelection::Routes { variants: ranks } => {
            let mut ranks: BTreeSet<usize> = ranks.iter().copied().collect();
            if ranks.is_empty() {
                ranks.insert(1);
            }
            if let Some(&rank) = ranks
                .iter()
                .find(|&&rank| rank == 0 || rank > variants.len())
            {
                return Err(MapViewError::UnknownVariant {
                    rank,
                    available: variants.len(),
                });
            }
            let chosen: Vec<_> = ranks
                .iter()
                .map(|rank| &variants.variants[rank - 1])
                .collect();
            let cases: BTreeSet<_> = chosen
                .iter()
                .flat_map(|v| v.cases.iter().copied())
                .collect();
            let activities = chosen
                .iter()
                .flat_map(|v| v.sequence.iter().copied())
                .collect();
            let edges = chosen.iter().flat_map(|v| pairs(&v.sequence)).collect();
            let drawn = EventLog {
                traces: log
                    .traces
                    .iter()
                    .filter(|trace| cases.contains(&trace.case))
                    .cloned()
                    .collect(),
                ..sub_log_frame(log)
            };
            (
                MapSelection::Routes {
                    variants: ranks.iter().copied().collect(),
                },
                Some(drawn),
                activities,
                edges,
                ranks,
            )
        }
        MapSelection::Limits {
            activities,
            connections,
        } => {
            let mut retained: BTreeSet<ActivityId> = top_activities.clone();
            for (activity, _) in ranking.iter().take(*activities) {
                retained.insert(*activity);
            }
            let mut candidates: Vec<(&(ActivityId, ActivityId), u64)> = full
                .edges
                .iter()
                .filter(|((from, to), _)| retained.contains(from) && retained.contains(to))
                .map(|(key, edge)| (key, edge.frequency))
                .collect();
            candidates.sort_by(|left, right| {
                right
                    .1
                    .cmp(&left.1)
                    .then_with(|| label_pair(log, left.0).cmp(&label_pair(log, right.0)))
            });
            let mut edges: BTreeSet<(ActivityId, ActivityId)> = top_edges.clone();
            for (key, _) in candidates.iter().take(*connections) {
                edges.insert(**key);
            }
            // Keep every retained activity reachable: its most frequent
            // incoming and outgoing connection among the candidates.
            for activity in &retained {
                for incoming in [true, false] {
                    // Natural starts need no incoming edge, and natural ends
                    // need no outgoing edge. Do not force rare returns into
                    // an otherwise complete main flow.
                    let node = full.nodes.get(activity).expect("retained activity");
                    if (incoming && node.starts > 0) || (!incoming && node.ends > 0) {
                        continue;
                    }

                    let touches = |key: &(ActivityId, ActivityId)| {
                        if incoming {
                            key.1 == *activity
                        } else {
                            key.0 == *activity
                        }
                    };
                    if edges.iter().any(touches) {
                        continue;
                    }
                    if let Some((key, _)) = candidates.iter().find(|(key, _)| touches(key)) {
                        edges.insert(**key);
                    }
                }
            }
            let shown: BTreeSet<usize> = variants
                .variants
                .iter()
                .enumerate()
                .filter(|(_, variant)| {
                    variant.sequence.iter().all(|a| retained.contains(a))
                        && pairs(&variant.sequence).all(|pair| edges.contains(&pair))
                })
                .map(|(index, _)| index + 1)
                .collect();
            (
                MapSelection::Limits {
                    activities: retained.len(),
                    connections: edges.len(),
                },
                None,
                retained,
                edges,
                shown,
            )
        }
    };

    // Routes draw counts from the covered cases only; limits keep the counts
    // of the whole active view for the retained elements.
    let drawn_dfg = drawn_log.as_ref().map(Dfg::build);
    let (nodes, edges) = match (&drawn_log, &drawn_dfg) {
        (Some(drawn), Some(dfg)) => map_elements(drawn, dfg),
        _ => map_elements(log, &full),
    };
    let retained_labels: BTreeSet<&str> = retained_activities
        .iter()
        .map(|a| log.activity_label(*a))
        .collect();
    let retained_pairs: BTreeSet<(&str, &str)> = retained_edges
        .iter()
        .map(|pair| label_pair(log, pair))
        .collect();
    let nodes: Vec<Node> = nodes
        .into_iter()
        .filter(|node| retained_labels.contains(node.activity.as_str()))
        .collect();
    let edges: Vec<MapEdge> = edges
        .into_iter()
        .filter(|edge| retained_pairs.contains(&(edge.from.as_str(), edge.to.as_str())))
        .map(|edge| MapEdge {
            slow: is_slow(&edge, slow_threshold),
            edge,
        })
        .collect();

    let cases_covered: usize = shown_routes
        .iter()
        .map(|rank| variants.variants[rank - 1].case_count())
        .sum();
    let cases_total = log.case_count();
    let percent_covered = if cases_total == 0 {
        0.0
    } else {
        cases_covered as f64 / cases_total as f64 * 100.0
    };
    let activity_ranking = ranking
        .iter()
        .enumerate()
        .map(|(index, (activity, node))| RankedActivity {
            rank: index + 1,
            activity: log.activity_label(*activity).to_owned(),
            frequency: node.frequency,
            case_frequency: node.case_frequency,
            shown: retained_activities.contains(activity),
        })
        .collect();
    let listed = variants
        .variants
        .iter()
        .take(LISTED_VARIANTS)
        .enumerate()
        .map(|(index, variant)| RankedVariant {
            rank: index + 1,
            sequence: labels(log, &variant.sequence),
            cases: variant.case_count(),
            percent: variant.case_count() as f64 / variants.case_count.max(1) as f64 * 100.0,
            shown: shown_routes.contains(&(index + 1)),
            duration_eligible_cases: variant.duration_cases,
            mean_throughput_seconds: variant.mean_throughput() / 1_000_000,
            example_cases: variant
                .cases
                .iter()
                .take(3)
                .map(|case| log.case_label(*case).to_owned())
                .collect(),
        })
        .collect();
    let mut notes = vec![match &selection {
        MapSelection::Routes { .. } => "Shows only the selected variants; every connection drawn occurred in a covered case, and counts are for those cases.".to_owned(),
        MapSelection::Limits { .. } => "Shows the most frequent steps and connections with counts for the whole active view. Connections can combine into paths no case took; coverage counts only cases whose whole path is drawn.".to_owned(),
    }];
    if slow_threshold.is_some() {
        notes.push(format!(
            "A connection is marked slow when its average time is at least {SLOW_MULTIPLE} times the median average of all measured connections in the active view."
        ));
    }

    Ok(MapView {
        top_route: labels(
            log,
            &variants.variants[match &selection {
                MapSelection::Routes { variants: ranks } => ranks[0] - 1,
                MapSelection::Limits { .. } => 0,
            }]
            .sequence,
        ),
        selection,
        nodes,
        edges,
        activities_shown: retained_activities.len(),
        activities_total: full.nodes.len(),
        connections_shown: retained_edges.len(),
        connections_total: full.edges.len(),
        variants_shown: shown_routes.len(),
        variants_total: variants.len(),
        cases_covered,
        cases_total,
        percent_covered,
        measures_true_waiting: full.measures_true_waiting,
        slow_threshold_seconds: slow_threshold,
        activity_ranking,
        variants: listed,
        notes,
        values: map_values(log),
    })
}

/// Activities most frequent first, ties broken by label.
fn activity_ranking<'a>(dfg: &'a Dfg, log: &EventLog) -> Vec<(ActivityId, &'a pm_core::dfg::Node)> {
    let mut ranking: Vec<_> = dfg.nodes.iter().map(|(a, n)| (*a, n)).collect();
    ranking.sort_by(|left, right| {
        right
            .1
            .frequency
            .cmp(&left.1.frequency)
            .then_with(|| log.activity_label(left.0).cmp(log.activity_label(right.0)))
    });
    ranking
}

/// Twice the median average time of measured connections, in seconds.
fn slow_threshold(dfg: &Dfg) -> Option<i64> {
    let mut means: Vec<i64> = dfg
        .edges
        .values()
        .filter(|edge| edge.duration_frequency > 0)
        .map(|edge| edge.mean_duration() / 1_000_000)
        .collect();
    if means.len() < SLOW_MIN_CONNECTIONS {
        return None;
    }
    means.sort_unstable();
    let median = means[(means.len() - 1) / 2];
    (median > 0).then_some(median * SLOW_MULTIPLE)
}

fn is_slow(edge: &Edge, threshold: Option<i64>) -> bool {
    threshold
        .is_some_and(|limit| edge.duration_eligible_occurrences > 0 && edge.mean_seconds >= limit)
}

fn pairs(sequence: &[ActivityId]) -> impl Iterator<Item = (ActivityId, ActivityId)> + '_ {
    sequence.windows(2).map(|pair| (pair[0], pair[1]))
}

fn labels(log: &EventLog, sequence: &[ActivityId]) -> Vec<String> {
    sequence
        .iter()
        .map(|a| log.activity_label(*a).to_owned())
        .collect()
}

fn label_pair<'a>(log: &'a EventLog, pair: &(ActivityId, ActivityId)) -> (&'a str, &'a str) {
    (log.activity_label(pair.0), log.activity_label(pair.1))
}

/// The dictionaries of a log without its traces, for building a subset.
fn sub_log_frame(log: &EventLog) -> EventLog {
    EventLog {
        cases: log.cases.clone(),
        activities: log.activities.clone(),
        resources: log.resources.clone(),
        attribute_names: log.attribute_names.clone(),
        attribute_values: log.attribute_values.clone(),
        costs: log.costs.clone(),
        step_values: log.step_values.clone(),
        traces: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::dict::Dictionary;
    use pm_core::log::{CaseTrace, Event};

    const DAY: i64 = 86_400 * 1_000_000;

    /// 6 × A→B→D, 3 × A→C→D, 1 × A→B→C→D. C→D takes ten days; others one.
    fn log() -> EventLog {
        let mut cases = Dictionary::default();
        let mut activities = Dictionary::default();
        let mut traces = Vec::new();
        let routes: [(&[&str], usize); 3] = [
            (&["A", "B", "D"], 6),
            (&["A", "C", "D"], 3),
            (&["A", "B", "C", "D"], 1),
        ];
        let mut row = 0;
        for (route, count) in routes {
            for _ in 0..count {
                let case = pm_core::dict::CaseId(cases.intern(&format!("case-{}", traces.len())));
                let mut at = 0;
                let events = route
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        if index > 0 {
                            at += if *label == "D" && route[index - 1] == "C" {
                                10 * DAY
                            } else {
                                DAY
                            };
                        }
                        row += 1;
                        Event::new(ActivityId(activities.intern(label)), at, row)
                    })
                    .collect();
                traces.push(CaseTrace {
                    case,
                    events,
                    incomplete: false,
                    attributes: Vec::new(),
                });
            }
        }
        EventLog {
            cases,
            activities,
            traces,
            ..EventLog::default()
        }
    }

    fn edge_pairs(view: &MapView) -> Vec<(String, String)> {
        view.edges
            .iter()
            .map(|e| (e.edge.from.clone(), e.edge.to.clone()))
            .collect()
    }

    #[test]
    fn opens_on_the_most_common_route_with_exact_coverage() {
        let view = map_view(&log(), &MapSelection::default()).unwrap();
        assert_eq!(view.top_route, ["A", "B", "D"]);
        assert_eq!(view.selection, MapSelection::Routes { variants: vec![1] });
        assert_eq!(
            edge_pairs(&view),
            [("A".into(), "B".into()), ("B".into(), "D".into())]
        );
        assert_eq!((view.activities_shown, view.activities_total), (3, 4));
        assert_eq!((view.connections_shown, view.connections_total), (2, 5));
        assert_eq!((view.variants_shown, view.variants_total), (1, 3));
        assert_eq!((view.cases_covered, view.cases_total), (6, 10));
        assert_eq!(view.percent_covered, 60.0);
        // Counts on the drawn route come from its own cases.
        assert_eq!(
            view.nodes
                .iter()
                .find(|n| n.activity == "A")
                .unwrap()
                .frequency,
            6
        );
    }

    #[test]
    fn selected_routes_are_exact_and_do_not_silently_include_the_top_route() {
        let view = map_view(&log(), &MapSelection::Routes { variants: vec![2] }).unwrap();
        assert_eq!(view.selection, MapSelection::Routes { variants: vec![2] });
        assert_eq!((view.cases_covered, view.percent_covered), (3, 30.0));
        assert!(
            view.variants
                .iter()
                .filter(|v| v.shown)
                .map(|v| v.rank)
                .eq([2])
        );
        assert_eq!(view.top_route, view.variants[1].sequence);
        assert!(view.nodes.iter().all(|node| node.case_frequency <= 3));
        let combined = map_view(
            &log(),
            &MapSelection::Routes {
                variants: vec![1, 2],
            },
        )
        .unwrap();
        assert_eq!(
            (combined.cases_covered, combined.percent_covered),
            (9, 90.0)
        );
        let defaulted = map_view(&log(), &MapSelection::Routes { variants: vec![] }).unwrap();
        assert_eq!(defaulted.cases_covered, 6);
        assert_eq!(
            map_view(&log(), &MapSelection::Routes { variants: vec![9] }),
            Err(MapViewError::UnknownVariant {
                rank: 9,
                available: 3
            })
        );
    }

    #[test]
    fn limits_rank_activities_and_connections_and_count_whole_paths_only() {
        let view = map_view(
            &log(),
            &MapSelection::Limits {
                activities: 0,
                connections: 0,
            },
        )
        .unwrap();
        // Raised to the top route.
        assert_eq!(
            view.selection,
            MapSelection::Limits {
                activities: 3,
                connections: 2
            }
        );
        assert_eq!(view.activity_ranking[0].activity, "A");
        assert_eq!(
            view.activity_ranking
                .iter()
                .map(|a| a.rank)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );

        let view = map_view(
            &log(),
            &MapSelection::Limits {
                activities: 4,
                connections: 0,
            },
        )
        .unwrap();
        // C is kept reachable by its strongest incoming and outgoing connection.
        assert!(edge_pairs(&view).contains(&("A".into(), "C".into())));
        assert!(edge_pairs(&view).contains(&("C".into(), "D".into())));
        assert_eq!(view.connections_shown, 4);
        // A→B→C→D needs B→C, which is not drawn: 9 of 10 cases are covered.
        assert_eq!((view.cases_covered, view.variants_shown), (9, 2));
        // Limits keep counts for the whole view.
        assert_eq!(
            view.nodes
                .iter()
                .find(|n| n.activity == "A")
                .unwrap()
                .frequency,
            10
        );

        let all = map_view(
            &log(),
            &MapSelection::Limits {
                activities: 9,
                connections: 9,
            },
        )
        .unwrap();
        assert_eq!((all.connections_shown, all.cases_covered), (5, 10));
    }

    #[test]
    fn slow_connections_are_judged_against_the_median_connection() {
        let view = map_view(
            &log(),
            &MapSelection::Routes {
                variants: vec![1, 2, 3],
            },
        )
        .unwrap();
        // Means are 1 day except C→D (10 days); the median is 1 day.
        assert_eq!(view.slow_threshold_seconds, Some(2 * 86_400));
        let slow: Vec<_> = view
            .edges
            .iter()
            .filter(|e| e.slow)
            .map(|e| (e.edge.from.as_str(), e.edge.to.as_str()))
            .collect();
        assert_eq!(slow, [("C", "D")]);
    }

    #[test]
    fn selection_reads_the_documented_json() {
        let routes: MapSelection =
            serde_json::from_str(r#"{"mode":"routes","variants":[1,3]}"#).unwrap();
        assert_eq!(
            routes,
            MapSelection::Routes {
                variants: vec![1, 3]
            }
        );
        let limits: MapSelection =
            serde_json::from_str(r#"{"mode":"limits","activities":8,"connections":14}"#).unwrap();
        assert_eq!(
            limits,
            MapSelection::Limits {
                activities: 8,
                connections: 14
            }
        );
    }
}
