//! BPMN 2.0 import into the executable accepting Petri-net boundary.
//!
//! The importer intentionally supports a precise, auditable subset rather
//! than guessing at constructs whose execution semantics differ. Tasks,
//! start/end events, exclusive gateways, parallel gateways, and ordinary
//! intermediate events are supported. Inclusive/event-based/complex gateways,
//! subprocesses, and boundary events are rejected with the element identifier.

use pm_core::{
    ActivityId, Dictionary, ModelError, PetriNet, PetriTransition, PlaceId, TransitionId,
};
use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};
use quick_xml::name::QName;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

#[derive(Debug, Clone, PartialEq)]
pub struct BpmnModel {
    pub process_id: String,
    pub process_name: Option<String>,
    pub activities: Dictionary,
    pub net: PetriNet,
}

#[derive(Debug, thiserror::Error)]
pub enum BpmnError {
    #[error("could not read BPMN model '{path}': {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("could not parse BPMN model '{path}': {detail}")]
    Xml { path: String, detail: String },
    #[error("BPMN model must contain exactly one process with flow nodes; found {0}")]
    ProcessCount(usize),
    #[error("BPMN element <{element}> is missing required attribute '{attribute}'")]
    MissingAttribute {
        element: String,
        attribute: &'static str,
    },
    #[error("duplicate BPMN identifier '{0}'")]
    DuplicateId(String),
    #[error("unsupported BPMN element <{element}> with id '{id}'")]
    Unsupported { element: String, id: String },
    #[error("BPMN task '{id}' must have exactly one incoming and one outgoing sequence flow")]
    ImplicitGateway { id: String },
    #[error("BPMN node '{id}' has an invalid flow shape: {detail}")]
    InvalidFlowShape { id: String, detail: String },
    #[error("sequence flow '{flow}' refers to unknown node '{node}'")]
    UnknownNode { flow: String, node: String },
    #[error("BPMN process has no start event or no end event")]
    MissingBoundary,
    #[error(transparent)]
    InvalidModel(#[from] ModelError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NodeKind {
    Start,
    End,
    Task,
    Exclusive,
    Parallel,
    Intermediate,
}

#[derive(Debug)]
struct Node {
    id: String,
    name: Option<String>,
    kind: NodeKind,
}

#[derive(Debug)]
struct Flow {
    id: String,
    source: String,
    target: String,
}

#[derive(Default)]
struct RawProcess {
    id: String,
    name: Option<String>,
    nodes: BTreeMap<String, Node>,
    flows: BTreeMap<String, Flow>,
}

pub fn import_bpmn(path: impl AsRef<Path>) -> Result<BpmnModel, BpmnError> {
    let path = path.as_ref();
    let display = path.display().to_string();
    let file = File::open(path).map_err(|source| BpmnError::Io {
        path: display.clone(),
        source,
    })?;
    let mut reader = Reader::from_reader(BufReader::new(file));
    reader.config_mut().trim_text(true);
    let mut processes = Vec::new();
    let mut current: Option<RawProcess> = None;
    let mut depth = 0usize;
    let mut process_depth = None;
    let mut buffer = Vec::new();

    loop {
        match reader.read_event_into(&mut buffer) {
            Ok(Event::Start(element)) => {
                depth += 1;
                let name = local(element.name());
                if name == b"process" {
                    if current.is_some() {
                        return Err(BpmnError::Unsupported {
                            element: "process".to_owned(),
                            id: "nested process".to_owned(),
                        });
                    }
                    current = Some(RawProcess {
                        id: required_attr(&reader, &element, b"id", "process")?,
                        name: optional_attr(&reader, &element, b"name", "process")?,
                        ..RawProcess::default()
                    });
                    process_depth = Some(depth);
                } else if current.is_some() && depth == process_depth.unwrap_or(0) + 1 {
                    observe_element(&reader, &element, &mut current)?;
                }
            }
            Ok(Event::Empty(element)) => {
                if current.is_some() && depth + 1 == process_depth.unwrap_or(0) + 1 {
                    observe_element(&reader, &element, &mut current)?;
                }
            }
            Ok(Event::End(element)) => {
                if local(element.name()) == b"process" {
                    if let Some(process) = current.take() {
                        processes.push(process);
                    }
                    process_depth = None;
                }
                depth = depth.saturating_sub(1);
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(error) => {
                return Err(BpmnError::Xml {
                    path: display,
                    detail: error.to_string(),
                });
            }
        }
        buffer.clear();
    }

    processes.retain(|process| !process.nodes.is_empty() || !process.flows.is_empty());
    if processes.len() != 1 {
        return Err(BpmnError::ProcessCount(processes.len()));
    }
    compile(processes.remove(0))
}

fn observe_element(
    reader: &Reader<BufReader<File>>,
    element: &BytesStart<'_>,
    current: &mut Option<RawProcess>,
) -> Result<(), BpmnError> {
    let tag = String::from_utf8_lossy(local(element.name())).into_owned();
    let process = current.as_mut().unwrap();
    if tag == "sequenceFlow" {
        let flow = Flow {
            id: required_attr(reader, element, b"id", &tag)?,
            source: required_attr(reader, element, b"sourceRef", &tag)?,
            target: required_attr(reader, element, b"targetRef", &tag)?,
        };
        if process.flows.insert(flow.id.clone(), flow).is_some() {
            return Err(BpmnError::DuplicateId(required_attr(
                reader, element, b"id", &tag,
            )?));
        }
        return Ok(());
    }
    let kind = match tag.as_str() {
        "startEvent" => Some(NodeKind::Start),
        "endEvent" => Some(NodeKind::End),
        "task" | "userTask" | "serviceTask" | "manualTask" | "businessRuleTask" | "scriptTask"
        | "sendTask" | "receiveTask" => Some(NodeKind::Task),
        "exclusiveGateway" => Some(NodeKind::Exclusive),
        "parallelGateway" => Some(NodeKind::Parallel),
        "intermediateCatchEvent" | "intermediateThrowEvent" => Some(NodeKind::Intermediate),
        "inclusiveGateway" | "eventBasedGateway" | "complexGateway" | "subProcess"
        | "boundaryEvent" | "callActivity" => {
            return Err(BpmnError::Unsupported {
                element: tag.clone(),
                id: required_attr(reader, element, b"id", &tag)?,
            });
        }
        _ => None,
    };
    if let Some(kind) = kind {
        let id = required_attr(reader, element, b"id", &tag)?;
        let node = Node {
            id: id.clone(),
            name: optional_attr(reader, element, b"name", &tag)?,
            kind,
        };
        if process.nodes.insert(id.clone(), node).is_some() {
            return Err(BpmnError::DuplicateId(id));
        }
    }
    Ok(())
}

fn compile(process: RawProcess) -> Result<BpmnModel, BpmnError> {
    if let Some(duplicate) = process
        .nodes
        .keys()
        .find(|id| process.flows.contains_key(*id))
    {
        return Err(BpmnError::DuplicateId(duplicate.clone()));
    }
    let starts = process
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Start)
        .count();
    let ends = process
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::End)
        .count();
    if starts == 0 || ends == 0 {
        return Err(BpmnError::MissingBoundary);
    }
    for flow in process.flows.values() {
        if !process.nodes.contains_key(&flow.source) {
            return Err(BpmnError::UnknownNode {
                flow: flow.id.clone(),
                node: flow.source.clone(),
            });
        }
        if !process.nodes.contains_key(&flow.target) {
            return Err(BpmnError::UnknownNode {
                flow: flow.id.clone(),
                node: flow.target.clone(),
            });
        }
    }
    validate_reachability(&process)?;

    let source = PlaceId(0);
    let sink = PlaceId(1);
    let flow_places: BTreeMap<_, _> = process
        .flows
        .keys()
        .enumerate()
        .map(|(index, id)| (id.clone(), PlaceId(u32::try_from(index + 2).unwrap())))
        .collect();
    let mut activities = Dictionary::new();
    let mut transitions = Vec::new();
    for node in process.nodes.values() {
        let incoming: Vec<_> = process
            .flows
            .values()
            .filter(|flow| flow.target == node.id)
            .map(|flow| flow_places[&flow.id])
            .collect();
        let outgoing: Vec<_> = process
            .flows
            .values()
            .filter(|flow| flow.source == node.id)
            .map(|flow| flow_places[&flow.id])
            .collect();
        match node.kind {
            NodeKind::Start => {
                require_exact_shape(node, &incoming, &outgoing, Some(0), None)?;
                push_transition(&mut transitions, None, vec![source], outgoing);
            }
            NodeKind::End => {
                require_exact_shape(node, &incoming, &outgoing, Some(1), Some(0))?;
                push_transition(&mut transitions, None, incoming, vec![sink]);
            }
            NodeKind::Task => {
                if incoming.len() != 1 || outgoing.len() != 1 {
                    return Err(BpmnError::ImplicitGateway {
                        id: node.id.clone(),
                    });
                }
                let label = node
                    .name
                    .as_deref()
                    .filter(|name| !name.trim().is_empty())
                    .ok_or(BpmnError::MissingAttribute {
                        element: node.id.clone(),
                        attribute: "name",
                    })?;
                let activity = ActivityId(activities.intern(label.trim()));
                push_transition(&mut transitions, Some(activity), incoming, outgoing);
            }
            NodeKind::Exclusive => {
                require_exact_shape(node, &incoming, &outgoing, None, None)?;
                for input in &incoming {
                    for output in &outgoing {
                        push_transition(&mut transitions, None, vec![*input], vec![*output]);
                    }
                }
            }
            NodeKind::Parallel => {
                require_exact_shape(node, &incoming, &outgoing, None, None)?;
                push_transition(&mut transitions, None, incoming, outgoing);
            }
            NodeKind::Intermediate => {
                require_exact_shape(node, &incoming, &outgoing, Some(1), Some(1))?;
                push_transition(&mut transitions, None, incoming, outgoing);
            }
        }
    }

    let place_count = flow_places.len() + 2;
    let mut initial_marking = vec![0; place_count];
    initial_marking[source.0 as usize] = 1;
    let mut final_marking = vec![0; place_count];
    final_marking[sink.0 as usize] = 1;
    let net = PetriNet {
        place_count,
        transitions,
        initial_marking,
        final_marking,
    };
    net.validate()?;
    Ok(BpmnModel {
        process_id: process.id,
        process_name: process.name,
        activities,
        net,
    })
}

fn validate_reachability(process: &RawProcess) -> Result<(), BpmnError> {
    let starts: Vec<_> = process
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::Start)
        .map(|node| node.id.clone())
        .collect();
    let ends: Vec<_> = process
        .nodes
        .values()
        .filter(|node| node.kind == NodeKind::End)
        .map(|node| node.id.clone())
        .collect();
    let reachable = walk(starts, |node| {
        process
            .flows
            .values()
            .filter(|flow| flow.source == node)
            .map(|flow| flow.target.clone())
            .collect()
    });
    let can_finish = walk(ends, |node| {
        process
            .flows
            .values()
            .filter(|flow| flow.target == node)
            .map(|flow| flow.source.clone())
            .collect()
    });
    for node in process.nodes.keys() {
        if !reachable.contains(node) {
            return Err(BpmnError::InvalidFlowShape {
                id: node.clone(),
                detail: "not reachable from any start event".to_owned(),
            });
        }
        if !can_finish.contains(node) {
            return Err(BpmnError::InvalidFlowShape {
                id: node.clone(),
                detail: "cannot reach any end event".to_owned(),
            });
        }
    }
    Ok(())
}

fn walk(seeds: Vec<String>, neighbours: impl Fn(&str) -> Vec<String>) -> BTreeSet<String> {
    let mut found: BTreeSet<_> = seeds.iter().cloned().collect();
    let mut pending = std::collections::VecDeque::from(seeds);
    while let Some(node) = pending.pop_front() {
        for neighbour in neighbours(&node) {
            if found.insert(neighbour.clone()) {
                pending.push_back(neighbour);
            }
        }
    }
    found
}

fn require_exact_shape(
    node: &Node,
    incoming: &[PlaceId],
    outgoing: &[PlaceId],
    exact_incoming: Option<usize>,
    exact_outgoing: Option<usize>,
) -> Result<(), BpmnError> {
    let incoming_valid =
        exact_incoming.map_or(!incoming.is_empty(), |count| incoming.len() == count);
    let outgoing_valid =
        exact_outgoing.map_or(!outgoing.is_empty(), |count| outgoing.len() == count);
    if !incoming_valid || !outgoing_valid {
        return Err(BpmnError::InvalidFlowShape {
            id: node.id.clone(),
            detail: format!(
                "{} incoming and {} outgoing sequence flows",
                incoming.len(),
                outgoing.len()
            ),
        });
    }
    Ok(())
}

fn push_transition(
    transitions: &mut Vec<PetriTransition>,
    activity: Option<ActivityId>,
    inputs: Vec<PlaceId>,
    outputs: Vec<PlaceId>,
) {
    transitions.push(PetriTransition {
        id: TransitionId(u32::try_from(transitions.len()).unwrap()),
        activity,
        inputs,
        outputs,
    });
}

fn required_attr(
    reader: &Reader<BufReader<File>>,
    element: &BytesStart<'_>,
    wanted: &[u8],
    tag: &str,
) -> Result<String, BpmnError> {
    optional_attr(reader, element, wanted, tag)?.ok_or_else(|| BpmnError::MissingAttribute {
        element: tag.to_owned(),
        attribute: if wanted == b"id" {
            "id"
        } else if wanted == b"sourceRef" {
            "sourceRef"
        } else if wanted == b"targetRef" {
            "targetRef"
        } else {
            "attribute"
        },
    })
}

fn optional_attr(
    reader: &Reader<BufReader<File>>,
    element: &BytesStart<'_>,
    wanted: &[u8],
    tag: &str,
) -> Result<Option<String>, BpmnError> {
    for attribute in element.attributes().with_checks(false) {
        let attribute = attribute.map_err(|error| BpmnError::Xml {
            path: tag.to_owned(),
            detail: error.to_string(),
        })?;
        if local(attribute.key) == wanted {
            let value = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
                .map_err(|error| BpmnError::Xml {
                    path: tag.to_owned(),
                    detail: error.to_string(),
                })?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn local(name: QName<'_>) -> &[u8] {
    name.local_name().into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pm_core::{CaseId, CaseTrace, Event, EventLog, replay_tokens};
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

    fn with_bpmn<T>(xml: &str, run: impl FnOnce(&Path) -> T) -> T {
        let path = std::env::temp_dir().join(format!(
            "open-process-bpmn-{}-{}.bpmn",
            std::process::id(),
            NEXT_FILE.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, xml).unwrap();
        let result = run(&path);
        std::fs::remove_file(path).unwrap();
        result
    }

    fn document(body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<bpmn:definitions xmlns:bpmn="http://www.omg.org/spec/BPMN/20100524/MODEL">
  <bpmn:process id="Process_1" name="Order process">{body}</bpmn:process>
</bpmn:definitions>"#
        )
    }

    #[test]
    fn imports_a_sequential_task_process() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start" />
    <bpmn:userTask id="receive" name="Receive Order" />
    <bpmn:serviceTask id="ship" name="Ship Order" />
    <bpmn:endEvent id="end" />
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="receive" />
    <bpmn:sequenceFlow id="f2" sourceRef="receive" targetRef="ship" />
    <bpmn:sequenceFlow id="f3" sourceRef="ship" targetRef="end" />"#,
        );
        let model = with_bpmn(&xml, |path| import_bpmn(path).unwrap());
        assert_eq!(model.process_id, "Process_1");
        assert_eq!(model.process_name.as_deref(), Some("Order process"));
        assert_eq!(model.activities.labels(), &["Receive Order", "Ship Order"]);
        model.net.validate().unwrap();
    }

    #[test]
    fn rejects_an_identifier_shared_by_a_node_and_flow() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start" />
    <bpmn:task id="shared" name="A" />
    <bpmn:endEvent id="end" />
    <bpmn:sequenceFlow id="shared" sourceRef="start" targetRef="shared" />
    <bpmn:sequenceFlow id="f2" sourceRef="shared" targetRef="end" />"#,
        );
        let error = with_bpmn(&xml, |path| import_bpmn(path).unwrap_err());
        assert!(matches!(error, BpmnError::DuplicateId(id) if id == "shared"));
    }

    #[test]
    fn exclusive_and_parallel_gateways_have_distinct_token_semantics() {
        let exclusive = document(
            r#"
    <bpmn:startEvent id="start"/><bpmn:exclusiveGateway id="choice"/>
    <bpmn:task id="a" name="A"/><bpmn:task id="b" name="B"/>
    <bpmn:exclusiveGateway id="merge"/><bpmn:endEvent id="end"/>
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="choice"/>
    <bpmn:sequenceFlow id="f2" sourceRef="choice" targetRef="a"/>
    <bpmn:sequenceFlow id="f3" sourceRef="choice" targetRef="b"/>
    <bpmn:sequenceFlow id="f4" sourceRef="a" targetRef="merge"/>
    <bpmn:sequenceFlow id="f5" sourceRef="b" targetRef="merge"/>
    <bpmn:sequenceFlow id="f6" sourceRef="merge" targetRef="end"/>"#,
        );
        let parallel = exclusive
            .replace(
                "exclusiveGateway id=\"choice\"",
                "parallelGateway id=\"choice\"",
            )
            .replace(
                "exclusiveGateway id=\"merge\"",
                "parallelGateway id=\"merge\"",
            );
        let xor = with_bpmn(&exclusive, |path| import_bpmn(path).unwrap());
        let and = with_bpmn(&parallel, |path| import_bpmn(path).unwrap());
        assert!(xor.net.transitions.len() > and.net.transitions.len());
        let split = and
            .net
            .transitions
            .iter()
            .find(|transition| transition.outputs.len() == 2)
            .unwrap();
        let join = and
            .net
            .transitions
            .iter()
            .find(|transition| transition.inputs.len() == 2)
            .unwrap();
        assert!(split.activity.is_none());
        assert!(join.activity.is_none());
    }

    #[test]
    fn imported_parallel_model_replays_both_interleavings() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start"/><bpmn:parallelGateway id="split"/>
    <bpmn:task id="a" name="A"/><bpmn:task id="b" name="B"/>
    <bpmn:parallelGateway id="join"/><bpmn:endEvent id="end"/>
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="split"/>
    <bpmn:sequenceFlow id="f2" sourceRef="split" targetRef="a"/>
    <bpmn:sequenceFlow id="f3" sourceRef="split" targetRef="b"/>
    <bpmn:sequenceFlow id="f4" sourceRef="a" targetRef="join"/>
    <bpmn:sequenceFlow id="f5" sourceRef="b" targetRef="join"/>
    <bpmn:sequenceFlow id="f6" sourceRef="join" targetRef="end"/>"#,
        );
        let model = with_bpmn(&xml, |path| import_bpmn(path).unwrap());
        let a = ActivityId(model.activities.get("A").unwrap());
        let b = ActivityId(model.activities.get("B").unwrap());
        let mut cases = Dictionary::new();
        let log = EventLog {
            cases: {
                cases.intern("C1");
                cases.intern("C2");
                cases
            },
            activities: model.activities.clone(),
            step_values: Default::default(),
            traces: vec![
                CaseTrace {
                    case: CaseId(0),
                    events: vec![Event::new(a, 0, 2), Event::new(b, 1, 3)],
                    incomplete: false,
                    attributes: Vec::new(),
                },
                CaseTrace {
                    case: CaseId(1),
                    events: vec![Event::new(b, 0, 4), Event::new(a, 1, 5)],
                    incomplete: false,
                    attributes: Vec::new(),
                },
            ],
            ..EventLog::default()
        };
        assert_eq!(replay_tokens(&log, &model.net).unwrap().fit_cases, 2);
    }

    #[test]
    fn imports_and_replays_a_gateway_loop() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start"/><bpmn:exclusiveGateway id="merge"/>
    <bpmn:task id="review" name="Review"/><bpmn:exclusiveGateway id="choice"/>
    <bpmn:task id="rework" name="Rework"/><bpmn:endEvent id="end"/>
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="merge"/>
    <bpmn:sequenceFlow id="f2" sourceRef="merge" targetRef="review"/>
    <bpmn:sequenceFlow id="f3" sourceRef="review" targetRef="choice"/>
    <bpmn:sequenceFlow id="f4" sourceRef="choice" targetRef="end"/>
    <bpmn:sequenceFlow id="f5" sourceRef="choice" targetRef="rework"/>
    <bpmn:sequenceFlow id="f6" sourceRef="rework" targetRef="merge"/>"#,
        );
        let model = with_bpmn(&xml, |path| import_bpmn(path).unwrap());
        let review = ActivityId(model.activities.get("Review").unwrap());
        let rework = ActivityId(model.activities.get("Rework").unwrap());
        let mut cases = Dictionary::new();
        let case = CaseId(cases.intern("C1"));
        let log = EventLog {
            cases,
            activities: model.activities.clone(),
            traces: vec![CaseTrace {
                case,
                events: vec![
                    Event::new(review, 0, 2),
                    Event::new(rework, 1, 3),
                    Event::new(review, 2, 4),
                ],
                incomplete: false,
                attributes: Vec::new(),
            }],
            ..EventLog::default()
        };
        assert_eq!(replay_tokens(&log, &model.net).unwrap().fit_cases, 1);
    }

    #[test]
    fn rejects_unsupported_constructs_instead_of_guessing() {
        let xml = document(r#"<bpmn:inclusiveGateway id="or_gateway"/>"#);
        assert!(matches!(
            with_bpmn(&xml, |path| import_bpmn(path)),
            Err(BpmnError::Unsupported { element, id })
                if element == "inclusiveGateway" && id == "or_gateway"
        ));
    }

    #[test]
    fn rejects_implicit_task_gateways() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start"/><bpmn:task id="task" name="Work"/>
    <bpmn:endEvent id="left"/><bpmn:endEvent id="right"/>
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="task"/>
    <bpmn:sequenceFlow id="f2" sourceRef="task" targetRef="left"/>
    <bpmn:sequenceFlow id="f3" sourceRef="task" targetRef="right"/>"#,
        );
        assert!(matches!(
            with_bpmn(&xml, |path| import_bpmn(path)),
            Err(BpmnError::ImplicitGateway { id }) if id == "task"
        ));
    }

    #[test]
    fn rejects_disconnected_model_fragments() {
        let xml = document(
            r#"
    <bpmn:startEvent id="start"/><bpmn:task id="work" name="Work"/>
    <bpmn:endEvent id="end"/><bpmn:task id="orphan" name="Orphan"/>
    <bpmn:sequenceFlow id="f1" sourceRef="start" targetRef="work"/>
    <bpmn:sequenceFlow id="f2" sourceRef="work" targetRef="end"/>"#,
        );
        assert!(matches!(
            with_bpmn(&xml, |path| import_bpmn(path)),
            Err(BpmnError::InvalidFlowShape { id, detail })
                if id == "orphan" && detail.contains("not reachable")
        ));
    }
}
