//! User-facing formal discovery results.
//!
//! Core mining uses compact activity identifiers. This boundary resolves them
//! to labels once, so CLI, browser, and agent clients receive the same stable,
//! self-contained model rather than each interpreting identifiers separately.

use crate::View;
use pm_core::{
    EventLog, Filter, HeuristicsMinerConfig, InductiveMinerConfig, PetriNet, ProcessTree,
    discover_alpha, discover_heuristics, discover_inductive,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LabelledProcessTree {
    Activity {
        activity: String,
    },
    Tau,
    Sequence {
        children: Vec<LabelledProcessTree>,
    },
    Exclusive {
        children: Vec<LabelledProcessTree>,
    },
    Parallel {
        children: Vec<LabelledProcessTree>,
    },
    Loop {
        body: Box<LabelledProcessTree>,
        redo: Box<LabelledProcessTree>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelledTransition {
    pub id: u32,
    /// `None` is an invisible routing transition.
    pub activity: Option<String>,
    pub inputs: Vec<u32>,
    pub outputs: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelledPetriNet {
    pub place_count: usize,
    pub transitions: Vec<LabelledTransition>,
    pub initial_marking: Vec<u32>,
    pub final_marking: Vec<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FormalDiscovery {
    pub source: String,
    pub algorithm: String,
    /// Integer thousandths: 200 means 20.0%.
    pub noise_threshold_milli: u16,
    pub filtered_edges: usize,
    pub used_flower_fallback: bool,
    pub view: View,
    pub tree: LabelledProcessTree,
    pub net: LabelledPetriNet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NetDiscovery {
    pub source: String,
    pub algorithm: String,
    pub view: View,
    pub net: LabelledPetriNet,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelledHeuristicArc {
    pub from: String,
    pub to: String,
    pub observations: u64,
    pub dependency_millionths: u64,
    pub kind: pm_core::DependencyKind,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LabelledAndBinding {
    pub anchor: String,
    pub activities: [String; 2],
    pub score_millionths: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HeuristicsDiscovery {
    pub source: String,
    pub algorithm: String,
    pub view: View,
    pub config: HeuristicsMinerConfig,
    pub starts: Vec<(String, u64)>,
    pub ends: Vec<(String, u64)>,
    pub arcs: Vec<LabelledHeuristicArc>,
    pub and_splits: Vec<LabelledAndBinding>,
    pub and_joins: Vec<LabelledAndBinding>,
}

/// An externally documented executable model, currently imported from BPMN.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ImportedFormalModel {
    pub source: String,
    pub model_kind: String,
    pub process_id: String,
    pub process_name: Option<String>,
    pub net: LabelledPetriNet,
}

/// Imports the supported executable BPMN subset without guessing at unsupported
/// gateway or subprocess semantics.
pub fn import_bpmn_model(
    path: impl AsRef<std::path::Path>,
) -> Result<ImportedFormalModel, pm_ingest::BpmnError> {
    let path = path.as_ref();
    let model = pm_ingest::import_bpmn(path)?;
    let labels = EventLog {
        activities: model.activities,
        ..EventLog::default()
    };
    Ok(ImportedFormalModel {
        source: path.display().to_string(),
        model_kind: "bpmn_2_0".to_owned(),
        process_id: model.process_id,
        process_name: model.process_name,
        net: label_net(&model.net, &labels),
    })
}

/// Discovers a sound formal model for a filtered view of a normalized log.
pub fn discover_formal(
    source: &str,
    log: &EventLog,
    filter: &Filter,
    config: InductiveMinerConfig,
) -> Result<FormalDiscovery, pm_core::DiscoveryError> {
    let filtered = filter.apply(log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    let discovered = discover_inductive(&filtered.log, config)?;
    Ok(FormalDiscovery {
        source: source.to_owned(),
        algorithm: "inductive_miner_infrequent".to_owned(),
        noise_threshold_milli: discovered.noise_threshold_milli,
        filtered_edges: discovered.filtered_edges,
        used_flower_fallback: discovered.used_flower_fallback,
        view,
        tree: label_tree(&discovered.tree, &filtered.log),
        net: label_net(&discovered.net, &filtered.log),
    })
}

/// Runs the classic Alpha Miner for teaching and compatibility. Unlike the
/// default Inductive Miner, this does not promise a sound model.
pub fn discover_alpha_formal(
    source: &str,
    log: &EventLog,
    filter: &Filter,
) -> Result<NetDiscovery, pm_core::AlphaError> {
    let filtered = filter.apply(log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    let discovered = discover_alpha(&filtered.log)?;
    Ok(NetDiscovery {
        source: source.to_owned(),
        algorithm: "alpha_miner_classic".to_owned(),
        view,
        net: label_net(&discovered.net, &filtered.log),
    })
}

/// Discovers a labelled heuristic net without claiming accepting-net
/// soundness or silently converting it to one.
pub fn discover_heuristics_formal(
    source: &str,
    log: &EventLog,
    filter: &Filter,
    config: HeuristicsMinerConfig,
) -> Result<HeuristicsDiscovery, pm_core::HeuristicsError> {
    let filtered = filter.apply(log);
    let view = View {
        cases_shown: filtered.log.case_count(),
        cases_total: filtered.cases_before,
        events_total: filtered.events_before,
        percent_shown: filtered.cases_kept_percent(),
        narrowed: filtered.narrowed(),
    };
    let discovered = discover_heuristics(&filtered.log, config)?;
    let label = |activity| filtered.log.activity_label(activity).to_owned();
    let bindings = |items: Vec<pm_core::AndBinding>| {
        items
            .into_iter()
            .map(|binding| LabelledAndBinding {
                anchor: label(binding.anchor),
                activities: [label(binding.activities[0]), label(binding.activities[1])],
                score_millionths: binding.score_millionths,
            })
            .collect()
    };
    Ok(HeuristicsDiscovery {
        source: source.to_owned(),
        algorithm: "heuristics_miner".to_owned(),
        view,
        config: discovered.config,
        starts: discovered
            .starts
            .into_iter()
            .map(|(activity, count)| (label(activity), count))
            .collect(),
        ends: discovered
            .ends
            .into_iter()
            .map(|(activity, count)| (label(activity), count))
            .collect(),
        arcs: discovered
            .arcs
            .into_iter()
            .map(|arc| LabelledHeuristicArc {
                from: label(arc.from),
                to: label(arc.to),
                observations: arc.observations,
                dependency_millionths: arc.dependency_millionths,
                kind: arc.kind,
            })
            .collect(),
        and_splits: bindings(discovered.and_splits),
        and_joins: bindings(discovered.and_joins),
    })
}

fn label_tree(tree: &ProcessTree, log: &EventLog) -> LabelledProcessTree {
    match tree {
        ProcessTree::Activity { activity } => LabelledProcessTree::Activity {
            activity: log.activity_label(*activity).to_owned(),
        },
        ProcessTree::Tau => LabelledProcessTree::Tau,
        ProcessTree::Sequence { children } => LabelledProcessTree::Sequence {
            children: children
                .iter()
                .map(|child| label_tree(child, log))
                .collect(),
        },
        ProcessTree::Exclusive { children } => LabelledProcessTree::Exclusive {
            children: children
                .iter()
                .map(|child| label_tree(child, log))
                .collect(),
        },
        ProcessTree::Parallel { children } => LabelledProcessTree::Parallel {
            children: children
                .iter()
                .map(|child| label_tree(child, log))
                .collect(),
        },
        ProcessTree::Loop { body, redo } => LabelledProcessTree::Loop {
            body: Box::new(label_tree(body, log)),
            redo: Box::new(label_tree(redo, log)),
        },
    }
}

/// The executable net for a process tree written with step names, such as
/// the expected process an agent reads from an SOP: steps in order, choices,
/// steps that may repeat, and optional steps.
pub fn tree_to_net(tree: &LabelledProcessTree) -> Result<LabelledPetriNet, String> {
    fn resolve(tree: &LabelledProcessTree, log: &mut EventLog) -> ProcessTree {
        let all = |children: &[LabelledProcessTree], log: &mut EventLog| {
            children
                .iter()
                .map(|child| resolve(child, log))
                .collect::<Vec<_>>()
        };
        match tree {
            LabelledProcessTree::Activity { activity } => ProcessTree::Activity {
                activity: pm_core::ActivityId(log.activities.intern(activity)),
            },
            LabelledProcessTree::Tau => ProcessTree::Tau,
            LabelledProcessTree::Sequence { children } => ProcessTree::Sequence {
                children: all(children, log),
            },
            LabelledProcessTree::Exclusive { children } => ProcessTree::Exclusive {
                children: all(children, log),
            },
            LabelledProcessTree::Parallel { children } => ProcessTree::Parallel {
                children: all(children, log),
            },
            LabelledProcessTree::Loop { body, redo } => ProcessTree::Loop {
                body: Box::new(resolve(body, log)),
                redo: Box::new(resolve(redo, log)),
            },
        }
    }
    let mut log = EventLog::default();
    let tree = resolve(tree, &mut log);
    tree.validate().map_err(|error| error.to_string())?;
    let net = tree.to_petri_net().map_err(|error| error.to_string())?;
    Ok(label_net(&net, &log))
}

fn label_net(net: &PetriNet, log: &EventLog) -> LabelledPetriNet {
    LabelledPetriNet {
        place_count: net.place_count,
        transitions: net
            .transitions
            .iter()
            .map(|transition| LabelledTransition {
                id: transition.id.0,
                activity: transition
                    .activity
                    .map(|activity| log.activity_label(activity).to_owned()),
                inputs: transition.inputs.iter().map(|place| place.0).collect(),
                outputs: transition.outputs.iter().map(|place| place.0).collect(),
            })
            .collect(),
        initial_marking: net.initial_marking.clone(),
        final_marking: net.final_marking.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{ActivityId, CaseId, CaseTrace, Dictionary, Event};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_BPMN: AtomicU64 = AtomicU64::new(1);

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

    #[test]
    fn alpha_discovery_resolves_activity_labels() {
        let log = log(&[&["A", "B", "C"]]);
        let result = discover_alpha_formal("orders.csv", &log, &Filter::new()).unwrap();
        assert_eq!(result.algorithm, "alpha_miner_classic");
        assert_eq!(
            result
                .net
                .transitions
                .iter()
                .filter_map(|transition| transition.activity.as_deref())
                .collect::<Vec<_>>(),
            vec!["A", "B", "C"]
        );
    }

    #[test]
    fn heuristics_discovery_resolves_scored_arcs() {
        let log = log(&[&["A", "B"], &["A", "B"]]);
        let result = discover_heuristics_formal(
            "orders.csv",
            &log,
            &Filter::new(),
            HeuristicsMinerConfig::default(),
        )
        .unwrap();
        assert_eq!(result.algorithm, "heuristics_miner");
        assert_eq!(result.arcs[0].from, "A");
        assert_eq!(result.arcs[0].to, "B");
        assert_eq!(result.arcs[0].dependency_millionths, 666_666);
    }

    #[test]
    fn resolves_all_core_identifiers_to_labels() {
        let log = log(&[&["Receive", "Approve"], &["Receive", "Approve"]]);
        let result = discover_formal(
            "orders.csv",
            &log,
            &Filter::new(),
            InductiveMinerConfig::default(),
        )
        .unwrap();
        assert_eq!(
            result.tree,
            LabelledProcessTree::Sequence {
                children: vec![
                    LabelledProcessTree::Activity {
                        activity: "Receive".to_owned(),
                    },
                    LabelledProcessTree::Activity {
                        activity: "Approve".to_owned(),
                    },
                ],
            }
        );
        assert!(
            result
                .net
                .transitions
                .iter()
                .filter_map(|transition| transition.activity.as_deref())
                .all(|activity| activity == "Receive" || activity == "Approve")
        );
    }

    #[test]
    fn reports_the_scope_of_the_discovered_view() {
        let log = log(&[&["A", "B"], &["A", "C"], &["A", "B"]]);
        let result = discover_formal(
            "orders.csv",
            &log,
            &Filter::new().with_activity("C"),
            InductiveMinerConfig::default(),
        )
        .unwrap();
        assert_eq!(result.view.cases_shown, 1);
        assert_eq!(result.view.cases_total, 3);
        assert!(result.view.narrowed);
        assert!((result.view.percent_shown - 33.333).abs() < 0.01);
    }

    #[test]
    fn json_is_self_contained_and_deterministic() {
        let log = log(&[&["A", "B"], &["A", "B"]]);
        let first = discover_formal(
            "orders.csv",
            &log,
            &Filter::new(),
            InductiveMinerConfig::default(),
        )
        .unwrap();
        let second = discover_formal(
            "orders.csv",
            &log,
            &Filter::new(),
            InductiveMinerConfig::default(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&first).unwrap(),
            serde_json::to_string(&second).unwrap()
        );
        let json = serde_json::to_value(first).unwrap();
        assert_eq!(json["algorithm"], "inductive_miner_infrequent");
        assert_eq!(json["tree"]["children"][0]["activity"], "A");
    }

    #[test]
    fn bpmn_import_resolves_activity_labels_for_all_front_ends() {
        let path = std::env::temp_dir().join(format!(
            "open-process-query-bpmn-{}-{}.bpmn",
            std::process::id(),
            NEXT_BPMN.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(
            &path,
            r#"<definitions xmlns="http://www.omg.org/spec/BPMN/20100524/MODEL">
<process id="p1" name="Approval"><startEvent id="s"/><task id="a" name="Approve"/>
<endEvent id="e"/><sequenceFlow id="f1" sourceRef="s" targetRef="a"/>
<sequenceFlow id="f2" sourceRef="a" targetRef="e"/></process></definitions>"#,
        )
        .unwrap();
        let imported = import_bpmn_model(&path).unwrap();
        std::fs::remove_file(path).unwrap();
        assert_eq!(imported.model_kind, "bpmn_2_0");
        assert_eq!(imported.process_name.as_deref(), Some("Approval"));
        assert_eq!(
            imported
                .net
                .transitions
                .iter()
                .filter_map(|transition| transition.activity.as_deref())
                .collect::<Vec<_>>(),
            vec!["Approve"]
        );
    }
}
