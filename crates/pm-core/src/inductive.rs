//! Inductive Miner with local infrequent-edge filtering.
//!
//! The miner operates on weighted trace variants, so its working set follows
//! behavioural variety rather than case count. At every recursion it first
//! tries the unfiltered Inductive Miner cuts. Only when those fail does it
//! apply the configured outgoing-edge frequency threshold and try again.

use crate::{ActivityId, EventLog, PetriNet, ProcessTree};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

const MAX_RECURSION_DEPTH: usize = 128;

/// Deterministic IMf settings. `noise_threshold_milli = 200` means 20%.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InductiveMinerConfig {
    pub noise_threshold_milli: u16,
}

impl Default for InductiveMinerConfig {
    fn default() -> Self {
        Self {
            noise_threshold_milli: 200,
        }
    }
}

/// A discovered sound model and transparent filtering metadata.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InductiveModel {
    pub tree: ProcessTree,
    pub net: PetriNet,
    pub noise_threshold_milli: u16,
    pub filtered_edges: usize,
    pub used_flower_fallback: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DiscoveryError {
    #[error("noise threshold must be between 0 and 1000 thousandths")]
    InvalidNoiseThreshold,
    #[error(transparent)]
    InvalidModel(#[from] crate::ModelError),
}

/// Discovers a block-structured model from the normalized event log.
pub fn discover_inductive(
    log: &EventLog,
    config: InductiveMinerConfig,
) -> Result<InductiveModel, DiscoveryError> {
    discover_inductive_from_variants(WeightedLog::from_event_log(log).traces, config)
}

/// [`discover_inductive`] from the variants alone: each activity sequence
/// with how many cases follow it. The miner reads nothing else, so the same
/// variants give the same model however the cases were read.
pub fn discover_inductive_from_variants(
    variants: BTreeMap<Vec<ActivityId>, u64>,
    config: InductiveMinerConfig,
) -> Result<InductiveModel, DiscoveryError> {
    if config.noise_threshold_milli > 1000 {
        return Err(DiscoveryError::InvalidNoiseThreshold);
    }
    let traces = WeightedLog { traces: variants };
    let mut stats = MiningStats::default();
    let tree = mine(&traces, config, &mut stats, 0);
    let net = tree.to_petri_net()?;
    Ok(InductiveModel {
        tree,
        net,
        noise_threshold_milli: config.noise_threshold_milli,
        filtered_edges: stats.filtered_edges,
        used_flower_fallback: stats.used_flower_fallback,
    })
}

#[derive(Debug, Clone, Default)]
struct WeightedLog {
    traces: BTreeMap<Vec<ActivityId>, u64>,
}

impl WeightedLog {
    fn from_event_log(log: &EventLog) -> Self {
        let mut traces = BTreeMap::new();
        for case in &log.traces {
            let trace: Vec<_> = case.events.iter().map(|event| event.activity).collect();
            *traces.entry(trace).or_insert(0) += 1;
        }
        Self { traces }
    }

    fn add(&mut self, trace: Vec<ActivityId>, frequency: u64) {
        *self.traces.entry(trace).or_insert(0) += frequency;
    }

    fn activities(&self) -> BTreeSet<ActivityId> {
        self.traces
            .keys()
            .flat_map(|trace| trace.iter().copied())
            .collect()
    }

    fn total_traces(&self) -> u64 {
        self.traces.values().sum()
    }

    fn empty_traces(&self) -> u64 {
        self.traces.get(&Vec::new()).copied().unwrap_or(0)
    }
}

#[derive(Default)]
struct MiningStats {
    filtered_edges: usize,
    used_flower_fallback: bool,
}

#[derive(Clone)]
enum Cut {
    Exclusive(Vec<BTreeSet<ActivityId>>),
    Sequence(Vec<BTreeSet<ActivityId>>),
    Parallel(Vec<BTreeSet<ActivityId>>),
    Loop {
        groups: Vec<BTreeSet<ActivityId>>,
        starts: BTreeSet<ActivityId>,
        ends: BTreeSet<ActivityId>,
    },
}

fn mine(
    log: &WeightedLog,
    config: InductiveMinerConfig,
    stats: &mut MiningStats,
    depth: usize,
) -> ProcessTree {
    let activities = log.activities();
    if activities.is_empty() {
        return ProcessTree::Tau;
    }
    if activities.len() == 1 {
        return single_activity_base(log, *activities.iter().next().unwrap(), config);
    }
    if depth >= MAX_RECURSION_DEPTH {
        stats.used_flower_fallback = true;
        return flower(&activities);
    }

    let graph = Relations::build(log, false);
    if let Some(cut) = find_cut(log, &graph) {
        return apply_cut(log, cut, config, stats, depth);
    }

    if config.noise_threshold_milli > 0 {
        let filtered_direct = Relations::build(log, false).filtered(config.noise_threshold_milli);
        let filtered_eventual = Relations::build(log, true).filtered(config.noise_threshold_milli);
        if filtered_direct.removed_edges > 0
            && let Some(cut) = find_filtered_cut(log, &filtered_direct, &filtered_eventual)
        {
            stats.filtered_edges += filtered_direct.removed_edges;
            return apply_cut(log, cut, config, stats, depth);
        }
    }

    stats.used_flower_fallback = true;
    flower(&activities)
}

fn single_activity_base(
    log: &WeightedLog,
    activity: ActivityId,
    config: InductiveMinerConfig,
) -> ProcessTree {
    let total = log.total_traces();
    let empty = log.empty_traces();
    let occurrences: u64 = log
        .traces
        .iter()
        .map(|(trace, frequency)| trace.len() as u64 * frequency)
        .sum();
    let nonempty = total.saturating_sub(empty);
    let excess = occurrences.saturating_sub(nonempty);
    let repeated = excess > 0
        && (config.noise_threshold_milli == 0
            || excess * 1000 >= nonempty.max(1) * u64::from(config.noise_threshold_milli));
    let core = if repeated {
        ProcessTree::loop_of(ProcessTree::activity(activity), ProcessTree::Tau)
    } else {
        ProcessTree::activity(activity)
    };
    if empty > 0
        && (config.noise_threshold_milli == 0
            || empty * 1000 >= total * u64::from(config.noise_threshold_milli))
    {
        ProcessTree::exclusive(vec![ProcessTree::Tau, core])
    } else {
        core
    }
}

fn flower(activities: &BTreeSet<ActivityId>) -> ProcessTree {
    let choices: Vec<_> = activities
        .iter()
        .copied()
        .map(ProcessTree::activity)
        .collect();
    let redo = if choices.len() == 1 {
        choices.into_iter().next().unwrap()
    } else {
        ProcessTree::exclusive(choices)
    };
    ProcessTree::loop_of(ProcessTree::Tau, redo)
}

fn apply_cut(
    log: &WeightedLog,
    cut: Cut,
    config: InductiveMinerConfig,
    stats: &mut MiningStats,
    depth: usize,
) -> ProcessTree {
    match cut {
        Cut::Exclusive(groups) => {
            let children = split_exclusive(log, &groups)
                .iter()
                .map(|sublog| mine(sublog, config, stats, depth + 1))
                .collect();
            ProcessTree::exclusive(children)
        }
        Cut::Sequence(groups) => {
            let children = split_project(log, &groups)
                .iter()
                .map(|sublog| mine(sublog, config, stats, depth + 1))
                .collect();
            ProcessTree::sequence(children)
        }
        Cut::Parallel(groups) => {
            let children = split_project(log, &groups)
                .iter()
                .map(|sublog| mine(sublog, config, stats, depth + 1))
                .collect();
            ProcessTree::parallel(children)
        }
        Cut::Loop {
            groups,
            starts,
            ends,
        } => {
            let mut sublogs = split_loop(log, &groups, &starts, &ends);
            let body = mine(&sublogs.remove(0), config, stats, depth + 1);
            let redo_children: Vec<_> = sublogs
                .iter()
                .map(|sublog| mine(sublog, config, stats, depth + 1))
                .collect();
            let redo = match redo_children.len() {
                0 => ProcessTree::Tau,
                1 => redo_children.into_iter().next().unwrap(),
                _ => ProcessTree::exclusive(redo_children),
            };
            ProcessTree::loop_of(body, redo)
        }
    }
}

#[derive(Clone, Default)]
struct Relations {
    activities: BTreeSet<ActivityId>,
    direct: BTreeMap<(ActivityId, ActivityId), u64>,
    starts: BTreeSet<ActivityId>,
    ends: BTreeSet<ActivityId>,
    removed_edges: usize,
}

impl Relations {
    fn build(log: &WeightedLog, eventually: bool) -> Self {
        let mut relations = Self::default();
        for (trace, frequency) in &log.traces {
            relations.activities.extend(trace.iter().copied());
            if let Some(first) = trace.first() {
                relations.starts.insert(*first);
            }
            if let Some(last) = trace.last() {
                relations.ends.insert(*last);
            }
            if eventually {
                for left in 0..trace.len() {
                    let mut seen = BTreeSet::new();
                    for right in (left + 1)..trace.len() {
                        if seen.insert(trace[right]) {
                            *relations
                                .direct
                                .entry((trace[left], trace[right]))
                                .or_insert(0) += frequency;
                        }
                    }
                }
            } else {
                for pair in trace.windows(2) {
                    *relations.direct.entry((pair[0], pair[1])).or_insert(0) += frequency;
                }
            }
        }
        relations
    }

    fn filtered(mut self, threshold_milli: u16) -> Self {
        let mut strongest = BTreeMap::<ActivityId, u64>::new();
        for ((from, _), frequency) in &self.direct {
            strongest
                .entry(*from)
                .and_modify(|value| *value = (*value).max(*frequency))
                .or_insert(*frequency);
        }
        let before = self.direct.len();
        self.direct.retain(|(from, _), frequency| {
            frequency.saturating_mul(1000)
                >= strongest[from].saturating_mul(u64::from(threshold_milli))
        });
        self.removed_edges = before - self.direct.len();
        self
    }

    fn has(&self, from: ActivityId, to: ActivityId) -> bool {
        self.direct.contains_key(&(from, to))
    }
}

fn find_cut(log: &WeightedLog, graph: &Relations) -> Option<Cut> {
    find_exclusive(graph)
        .map(Cut::Exclusive)
        .or_else(|| find_sequence(graph).map(Cut::Sequence))
        .or_else(|| find_parallel(graph).map(Cut::Parallel))
        .or_else(|| find_loop(log, graph))
}

fn find_filtered_cut(log: &WeightedLog, direct: &Relations, eventual: &Relations) -> Option<Cut> {
    find_exclusive(direct)
        .map(Cut::Exclusive)
        .or_else(|| find_sequence(eventual).map(Cut::Sequence))
        // IMf deliberately does not frequency-filter parallel cuts: sparse
        // interleavings do not provide safe evidence that concurrency is noise.
        .or_else(|| find_loop(log, direct))
}

fn find_exclusive(graph: &Relations) -> Option<Vec<BTreeSet<ActivityId>>> {
    let groups = components(&graph.activities, |a, b| graph.has(a, b) || graph.has(b, a));
    (groups.len() > 1).then_some(groups)
}

fn find_sequence(graph: &Relations) -> Option<Vec<BTreeSet<ActivityId>>> {
    let nodes: Vec<_> = graph.activities.iter().copied().collect();
    let reach: BTreeMap<_, _> = nodes
        .iter()
        .copied()
        .map(|source| (source, reachable(source, graph)))
        .collect();
    let mut remaining = graph.activities.clone();
    let mut strongly_connected = Vec::new();
    while let Some(seed) = remaining.iter().next().copied() {
        let group: BTreeSet<_> = remaining
            .iter()
            .copied()
            .filter(|candidate| {
                reach[&seed].contains(candidate) && reach[candidate].contains(&seed)
            })
            .collect();
        for activity in &group {
            remaining.remove(activity);
        }
        strongly_connected.push(group);
    }
    if strongly_connected.len() < 2 {
        return None;
    }
    let mut pending: BTreeSet<usize> = (0..strongly_connected.len()).collect();
    let mut layers = Vec::new();
    while !pending.is_empty() {
        let sources: Vec<_> = pending
            .iter()
            .copied()
            .filter(|candidate| {
                !pending.iter().copied().any(|other| {
                    other != *candidate
                        && strongly_connected[other].iter().any(|from| {
                            strongly_connected[*candidate]
                                .iter()
                                .any(|to| reach[from].contains(to))
                        })
                })
            })
            .collect();
        if sources.is_empty() {
            return None;
        }
        let mut layer = BTreeSet::new();
        for source in sources {
            layer.extend(strongly_connected[source].iter().copied());
            pending.remove(&source);
        }
        layers.push(layer);
    }
    (layers.len() > 1).then_some(layers)
}

fn reachable(source: ActivityId, graph: &Relations) -> BTreeSet<ActivityId> {
    let mut found = BTreeSet::from([source]);
    let mut queue = VecDeque::from([source]);
    while let Some(from) = queue.pop_front() {
        for &(edge_from, to) in graph.direct.keys() {
            if edge_from == from && found.insert(to) {
                queue.push_back(to);
            }
        }
    }
    found
}

fn find_parallel(graph: &Relations) -> Option<Vec<BTreeSet<ActivityId>>> {
    let groups = components(&graph.activities, |a, b| {
        !(graph.has(a, b) && graph.has(b, a))
    });
    if groups.len() < 2
        || groups
            .iter()
            .any(|group| group.is_disjoint(&graph.starts) || group.is_disjoint(&graph.ends))
    {
        None
    } else {
        Some(groups)
    }
}

fn find_loop(log: &WeightedLog, graph: &Relations) -> Option<Cut> {
    if graph.starts.is_empty() || graph.ends.is_empty() {
        return None;
    }
    let mut body = graph
        .starts
        .union(&graph.ends)
        .copied()
        .collect::<BTreeSet<_>>();
    loop {
        let additions: Vec<_> = graph
            .activities
            .difference(&body)
            .copied()
            .filter(|candidate| {
                graph.direct.keys().any(|(from, to)| {
                    (*to == *candidate && body.contains(from) && !graph.ends.contains(from))
                        || (*from == *candidate && body.contains(to) && !graph.starts.contains(to))
                })
            })
            .collect();
        if additions.is_empty() {
            break;
        }
        body.extend(additions);
    }
    let redo = graph
        .activities
        .difference(&body)
        .copied()
        .collect::<BTreeSet<_>>();
    if redo.is_empty() {
        let repeated_boundary = log.traces.keys().any(|trace| {
            trace
                .windows(2)
                .any(|pair| graph.ends.contains(&pair[0]) && graph.starts.contains(&pair[1]))
        });
        return repeated_boundary.then(|| Cut::Loop {
            groups: vec![body],
            starts: graph.starts.clone(),
            ends: graph.ends.clone(),
        });
    }
    let redo_groups = components(&redo, |a, b| graph.has(a, b) || graph.has(b, a));
    let every_group_returns = redo_groups.iter().all(|group| {
        graph
            .direct
            .keys()
            .any(|(from, to)| graph.ends.contains(from) && group.contains(to))
            && graph
                .direct
                .keys()
                .any(|(from, to)| group.contains(from) && graph.starts.contains(to))
    });
    if !every_group_returns {
        return None;
    }
    let mut groups = vec![body];
    groups.extend(redo_groups);
    Some(Cut::Loop {
        groups,
        starts: graph.starts.clone(),
        ends: graph.ends.clone(),
    })
}

fn components(
    activities: &BTreeSet<ActivityId>,
    connected: impl Fn(ActivityId, ActivityId) -> bool,
) -> Vec<BTreeSet<ActivityId>> {
    let mut remaining = activities.clone();
    let mut groups = Vec::new();
    while let Some(seed) = remaining.iter().next().copied() {
        let mut group = BTreeSet::from([seed]);
        let mut queue = VecDeque::from([seed]);
        remaining.remove(&seed);
        while let Some(from) = queue.pop_front() {
            let neighbours: Vec<_> = remaining
                .iter()
                .copied()
                .filter(|to| connected(from, *to))
                .collect();
            for neighbour in neighbours {
                remaining.remove(&neighbour);
                group.insert(neighbour);
                queue.push_back(neighbour);
            }
        }
        groups.push(group);
    }
    groups.sort_by_key(|group| group.iter().next().copied());
    groups
}

fn split_project(log: &WeightedLog, groups: &[BTreeSet<ActivityId>]) -> Vec<WeightedLog> {
    let mut sublogs = vec![WeightedLog::default(); groups.len()];
    for (trace, frequency) in &log.traces {
        for (index, group) in groups.iter().enumerate() {
            sublogs[index].add(
                trace
                    .iter()
                    .copied()
                    .filter(|activity| group.contains(activity))
                    .collect(),
                *frequency,
            );
        }
    }
    sublogs
}

fn split_exclusive(log: &WeightedLog, groups: &[BTreeSet<ActivityId>]) -> Vec<WeightedLog> {
    let mut sublogs = vec![WeightedLog::default(); groups.len()];
    for (trace, frequency) in &log.traces {
        let selected = groups
            .iter()
            .enumerate()
            .max_by_key(|(index, group)| {
                (
                    trace
                        .iter()
                        .filter(|activity| group.contains(activity))
                        .count(),
                    std::cmp::Reverse(*index),
                )
            })
            .map(|(index, _)| index)
            .unwrap_or(0);
        let projected = trace
            .iter()
            .copied()
            .filter(|activity| groups[selected].contains(activity))
            .collect();
        sublogs[selected].add(projected, *frequency);
    }
    sublogs
}

fn split_loop(
    log: &WeightedLog,
    groups: &[BTreeSet<ActivityId>],
    starts: &BTreeSet<ActivityId>,
    ends: &BTreeSet<ActivityId>,
) -> Vec<WeightedLog> {
    let mut sublogs = vec![WeightedLog::default(); groups.len().max(2)];
    for (trace, frequency) in &log.traces {
        if trace.is_empty() {
            sublogs[0].add(Vec::new(), *frequency);
            continue;
        }
        let mut segment = Vec::new();
        let mut current_group = group_index(trace[0], groups).unwrap_or(0);
        for (index, activity) in trace.iter().copied().enumerate() {
            let group = group_index(activity, groups).unwrap_or(current_group);
            let body_boundary = current_group == 0
                && group == 0
                && index > 0
                && ends.contains(&trace[index - 1])
                && starts.contains(&activity);
            if group != current_group || body_boundary {
                sublogs[current_group].add(std::mem::take(&mut segment), *frequency);
                if body_boundary && groups.len() == 1 {
                    sublogs[1].add(Vec::new(), *frequency);
                }
                current_group = group;
            }
            segment.push(activity);
        }
        sublogs[current_group].add(segment, *frequency);
    }
    sublogs
}

fn group_index(activity: ActivityId, groups: &[BTreeSet<ActivityId>]) -> Option<usize> {
    groups.iter().position(|group| group.contains(&activity))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{CaseId, CaseTrace, Dictionary, Event};

    fn log(spec: &[&[&str]]) -> EventLog {
        let mut cases = Dictionary::new();
        let mut activities = Dictionary::new();
        let mut traces = Vec::new();
        let mut row = 2;
        for (index, labels) in spec.iter().enumerate() {
            let case = CaseId(cases.intern(&format!("C{}", index + 1)));
            let events = labels
                .iter()
                .enumerate()
                .map(|(position, label)| {
                    let event =
                        Event::new(ActivityId(activities.intern(label)), position as i64, row);
                    row += 1;
                    event
                })
                .collect();
            traces.push(CaseTrace {
                case,
                events,
                incomplete: false,
                attributes: Vec::new(),
            });
        }
        EventLog {
            cases,
            activities,
            step_values: Default::default(),
            traces,
            ..EventLog::default()
        }
    }

    fn id(log: &EventLog, label: &str) -> ActivityId {
        ActivityId(log.activities.get(label).unwrap())
    }

    #[test]
    fn discovers_a_hand_constructed_sequence() {
        let event_log = log(&[&["A", "B", "C"], &["A", "B", "C"]]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        assert_eq!(
            model.tree,
            ProcessTree::sequence(vec![
                ProcessTree::activity(id(&event_log, "A")),
                ProcessTree::activity(id(&event_log, "B")),
                ProcessTree::activity(id(&event_log, "C")),
            ])
        );
        assert!(!model.used_flower_fallback);
    }

    #[test]
    fn discovers_choice_between_shared_prefix_and_suffix() {
        let event_log = log(&[&["A", "B", "D"], &["A", "C", "D"]]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        assert_eq!(
            model.tree,
            ProcessTree::sequence(vec![
                ProcessTree::activity(id(&event_log, "A")),
                ProcessTree::exclusive(vec![
                    ProcessTree::activity(id(&event_log, "B")),
                    ProcessTree::activity(id(&event_log, "C")),
                ]),
                ProcessTree::activity(id(&event_log, "D")),
            ])
        );
    }

    #[test]
    fn distinguishes_parallel_interleaving_from_a_loop() {
        let event_log = log(&[&["A", "B", "C", "D"], &["A", "C", "B", "D"]]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        assert_eq!(
            model.tree,
            ProcessTree::sequence(vec![
                ProcessTree::activity(id(&event_log, "A")),
                ProcessTree::parallel(vec![
                    ProcessTree::activity(id(&event_log, "B")),
                    ProcessTree::activity(id(&event_log, "C")),
                ]),
                ProcessTree::activity(id(&event_log, "D")),
            ])
        );
    }

    #[test]
    fn discovers_redo_loop_from_constructed_traces() {
        let event_log = log(&[
            &["Submit", "Review", "Approve"],
            &["Submit", "Review", "Request", "Review", "Approve"],
            &[
                "Submit", "Review", "Request", "Review", "Request", "Review", "Approve",
            ],
        ]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        assert_eq!(
            model.tree,
            ProcessTree::sequence(vec![
                ProcessTree::activity(id(&event_log, "Submit")),
                ProcessTree::loop_of(
                    ProcessTree::activity(id(&event_log, "Review")),
                    ProcessTree::activity(id(&event_log, "Request")),
                ),
                ProcessTree::activity(id(&event_log, "Approve")),
            ])
        );
    }

    #[test]
    fn discovers_a_self_loop_without_inventing_parallelism() {
        let event_log = log(&[&["A", "B", "C"], &["A", "B", "B", "B", "C"]]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        assert_eq!(
            model.tree,
            ProcessTree::sequence(vec![
                ProcessTree::activity(id(&event_log, "A")),
                ProcessTree::loop_of(ProcessTree::activity(id(&event_log, "B")), ProcessTree::Tau,),
                ProcessTree::activity(id(&event_log, "C")),
            ])
        );
    }

    #[test]
    fn every_discovery_returns_a_valid_accepting_net() {
        let event_log = log(&[&["A", "B"], &["C", "A"], &["B", "C", "B"]]);
        let model = discover_inductive(&event_log, InductiveMinerConfig::default()).unwrap();
        model.tree.validate().unwrap();
        model.net.validate().unwrap();
    }

    #[test]
    fn rejects_an_invalid_noise_threshold() {
        let event_log = log(&[&["A"]]);
        assert_eq!(
            discover_inductive(
                &event_log,
                InductiveMinerConfig {
                    noise_threshold_milli: 1001,
                },
            ),
            Err(DiscoveryError::InvalidNoiseThreshold)
        );
    }

    #[test]
    fn local_filter_removes_only_edges_weak_relative_to_their_source() {
        let a = ActivityId(0);
        let b = ActivityId(1);
        let c = ActivityId(2);
        let d = ActivityId(3);
        let mut weighted = WeightedLog::default();
        weighted.add(vec![a, b, c, d], 100);
        weighted.add(vec![c, a, b, c, d], 1);

        let direct = Relations::build(&weighted, false).filtered(200);
        assert_eq!(direct.removed_edges, 1);
        assert!(!direct.has(c, a));
        assert!(direct.has(c, d));

        let eventual = Relations::build(&weighted, true).filtered(200);
        assert_eq!(
            find_sequence(&eventual),
            Some(vec![
                BTreeSet::from([a]),
                BTreeSet::from([b]),
                BTreeSet::from([c]),
                BTreeSet::from([d]),
            ])
        );
    }

    #[test]
    fn base_case_filters_a_rare_extra_occurrence() {
        let activity = ActivityId(0);
        let mut weighted = WeightedLog::default();
        weighted.add(vec![activity], 100);
        weighted.add(vec![activity, activity], 1);
        assert_eq!(
            single_activity_base(&weighted, activity, InductiveMinerConfig::default()),
            ProcessTree::activity(activity)
        );
    }
}
