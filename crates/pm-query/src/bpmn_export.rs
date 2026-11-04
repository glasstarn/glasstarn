//! BPMN 2.0 export of the process map, for Signavio, ARIS, Visio and other
//! modelling tools.
//!
//! The file describes exactly what the map shows: one task per step, one
//! sequence flow per connection, a start and an end event. Where a step has
//! several ways in or out, an exclusive gateway makes the choice explicit, so
//! every task has one incoming and one outgoing flow as BPMN tools expect.
//! Observed order is not a claim about concurrency, so no parallel gateways
//! are written. A diagram section lays the model out left to right so tools
//! draw it on opening.

use crate::map_view::MapView;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::Write as _;

const TASK_WIDTH: f64 = 140.0;
const TASK_HEIGHT: f64 = 64.0;
const GATEWAY_SIZE: f64 = 44.0;
const EVENT_SIZE: f64 = 36.0;
const COLUMN: f64 = 190.0;
const ROW: f64 = 120.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Start,
    End,
    Task,
    Gateway,
}

struct Shape {
    id: String,
    kind: Kind,
    name: Option<String>,
    documentation: Option<String>,
}

/// Writes the map view as a BPMN 2.0 document with diagram layout.
pub fn map_view_bpmn(view: &MapView, process_name: &str) -> String {
    let mut shapes: Vec<Shape> = Vec::new();
    let mut flows: Vec<(String, usize, usize)> = Vec::new();
    let add = |shapes: &mut Vec<Shape>,
               id: String,
               kind: Kind,
               name: Option<String>,
               documentation: Option<String>| {
        shapes.push(Shape {
            id,
            kind,
            name,
            documentation,
        });
        shapes.len() - 1
    };

    let start = add(
        &mut shapes,
        "StartEvent_1".into(),
        Kind::Start,
        Some("Start".into()),
        None,
    );
    let end = add(
        &mut shapes,
        "EndEvent_1".into(),
        Kind::End,
        Some("End".into()),
        None,
    );
    let shown: BTreeSet<&str> = view
        .nodes
        .iter()
        .map(|node| node.activity.as_str())
        .collect();
    let edges: Vec<_> = view
        .edges
        .iter()
        .filter(|edge| {
            shown.contains(edge.edge.from.as_str()) && shown.contains(edge.edge.to.as_str())
        })
        .collect();

    // Every step's ways in and out: other steps, Start and End. A step with
    // no way in on this map is joined to Start, one with no way out to End.
    let begins = |node: &crate::analysis::Node| {
        node.starts > 0 || !edges.iter().any(|edge| edge.edge.to == node.activity)
    };
    let finishes = |node: &crate::analysis::Node| {
        node.ends > 0 || !edges.iter().any(|edge| edge.edge.from == node.activity)
    };
    let mut entry = BTreeMap::new();
    let mut exit = BTreeMap::new();
    for (index, node) in view.nodes.iter().enumerate() {
        let task = add(
            &mut shapes,
            format!("Task_{}", index + 1),
            Kind::Task,
            Some(node.activity.clone()),
            Some(format!(
                "Recorded {} times for {} items.",
                node.frequency, node.case_frequency
            )),
        );
        let ins = edges
            .iter()
            .filter(|edge| edge.edge.to == node.activity)
            .count()
            + usize::from(begins(node));
        let outs = edges
            .iter()
            .filter(|edge| edge.edge.from == node.activity)
            .count()
            + usize::from(finishes(node));
        let join = if ins > 1 {
            let gateway = add(
                &mut shapes,
                format!("Gateway_join_{}", index + 1),
                Kind::Gateway,
                None,
                None,
            );
            flows.push((format!("Flow_join_{}", index + 1), gateway, task));
            gateway
        } else {
            task
        };
        let split = if outs > 1 {
            let gateway = add(
                &mut shapes,
                format!("Gateway_split_{}", index + 1),
                Kind::Gateway,
                None,
                None,
            );
            flows.push((format!("Flow_split_{}", index + 1), task, gateway));
            gateway
        } else {
            task
        };
        entry.insert(node.activity.as_str(), join);
        exit.insert(node.activity.as_str(), split);
    }

    let starting: Vec<_> = view.nodes.iter().filter(|node| begins(node)).collect();
    let ending: Vec<_> = view.nodes.iter().filter(|node| finishes(node)).collect();
    let start_port = if starting.len() > 1 {
        let gateway = add(
            &mut shapes,
            "Gateway_start".into(),
            Kind::Gateway,
            None,
            None,
        );
        flows.push(("Flow_start".into(), start, gateway));
        gateway
    } else {
        start
    };
    let end_port = if ending.len() > 1 {
        let gateway = add(&mut shapes, "Gateway_end".into(), Kind::Gateway, None, None);
        flows.push(("Flow_end".into(), gateway, end));
        gateway
    } else {
        end
    };
    for node in &starting {
        flows.push((
            format!("Flow_{}", flows.len() + 1),
            start_port,
            entry[node.activity.as_str()],
        ));
    }
    for edge in &edges {
        flows.push((
            format!("Flow_{}", flows.len() + 1),
            exit[edge.edge.from.as_str()],
            entry[edge.edge.to.as_str()],
        ));
    }
    for node in &ending {
        flows.push((
            format!("Flow_{}", flows.len() + 1),
            exit[node.activity.as_str()],
            end_port,
        ));
    }

    let positions = layout(&shapes, &flows, start);
    write_document(process_name, &shapes, &flows, &positions)
}

/// Columns by longest path from Start over the flows that move forward;
/// rows in order of first appearance within each column.
fn layout(shapes: &[Shape], flows: &[(String, usize, usize)], start: usize) -> Vec<(f64, f64)> {
    let count = shapes.len();
    let mut successors = vec![Vec::new(); count];
    for (_, source, target) in flows {
        successors[*source].push(*target);
    }
    // Forward flows: those not closing a cycle in a depth-first walk.
    let mut state = vec![0u8; count];
    let mut forward = vec![Vec::new(); count];
    let mut stack = vec![(start, 0usize)];
    state[start] = 1;
    while let Some((node, next)) = stack.pop() {
        if next < successors[node].len() {
            stack.push((node, next + 1));
            let target = successors[node][next];
            match state[target] {
                0 => {
                    forward[node].push(target);
                    state[target] = 1;
                    stack.push((target, 0));
                }
                2 => forward[node].push(target),
                _ => {}
            }
        } else {
            state[node] = 2;
        }
    }
    let mut indegree = vec![0usize; count];
    for targets in &forward {
        for target in targets {
            indegree[*target] += 1;
        }
    }
    let mut column = vec![0usize; count];
    let mut queue: VecDeque<usize> = (0..count).filter(|node| indegree[*node] == 0).collect();
    while let Some(node) = queue.pop_front() {
        for target in &forward[node] {
            column[*target] = column[*target].max(column[node] + 1);
            indegree[*target] -= 1;
            if indegree[*target] == 0 {
                queue.push_back(*target);
            }
        }
    }
    let mut rows: BTreeMap<usize, usize> = BTreeMap::new();
    (0..count)
        .map(|node| {
            let row = rows.entry(column[node]).or_insert(0);
            let position = (
                80.0 + column[node] as f64 * COLUMN,
                80.0 + *row as f64 * ROW,
            );
            *row += 1;
            position
        })
        .collect()
}

fn size(kind: Kind) -> (f64, f64) {
    match kind {
        Kind::Task => (TASK_WIDTH, TASK_HEIGHT),
        Kind::Gateway => (GATEWAY_SIZE, GATEWAY_SIZE),
        Kind::Start | Kind::End => (EVENT_SIZE, EVENT_SIZE),
    }
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn write_document(
    name: &str,
    shapes: &[Shape],
    flows: &[(String, usize, usize)],
    positions: &[(f64, f64)],
) -> String {
    let mut incoming = vec![Vec::new(); shapes.len()];
    let mut outgoing = vec![Vec::new(); shapes.len()];
    for (id, source, target) in flows {
        outgoing[*source].push(id.as_str());
        incoming[*target].push(id.as_str());
    }
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    xml.push_str("<bpmn:definitions xmlns:bpmn=\"http://www.omg.org/spec/BPMN/20100524/MODEL\" xmlns:bpmndi=\"http://www.omg.org/spec/BPMN/20100524/DI\" xmlns:dc=\"http://www.omg.org/spec/DD/20100524/DC\" xmlns:di=\"http://www.omg.org/spec/DD/20100524/DI\" id=\"Definitions_1\" targetNamespace=\"https://glasstarn.dev/bpmn\" exporter=\"glasstarn\">\n");
    let _ = writeln!(
        xml,
        "  <bpmn:process id=\"Process_1\" name=\"{}\" isExecutable=\"false\">",
        escape(name)
    );
    for (index, shape) in shapes.iter().enumerate() {
        let element = match shape.kind {
            Kind::Start => "startEvent",
            Kind::End => "endEvent",
            Kind::Task => "task",
            Kind::Gateway => "exclusiveGateway",
        };
        let name = shape
            .name
            .as_deref()
            .map(|name| format!(" name=\"{}\"", escape(name)))
            .unwrap_or_default();
        let _ = writeln!(xml, "    <bpmn:{element} id=\"{}\"{name}>", shape.id);
        if let Some(documentation) = &shape.documentation {
            let _ = writeln!(
                xml,
                "      <bpmn:documentation>{}</bpmn:documentation>",
                escape(documentation)
            );
        }
        for flow in &incoming[index] {
            let _ = writeln!(xml, "      <bpmn:incoming>{flow}</bpmn:incoming>");
        }
        for flow in &outgoing[index] {
            let _ = writeln!(xml, "      <bpmn:outgoing>{flow}</bpmn:outgoing>");
        }
        let _ = writeln!(xml, "    </bpmn:{element}>");
    }
    for (id, source, target) in flows {
        let _ = writeln!(
            xml,
            "    <bpmn:sequenceFlow id=\"{id}\" sourceRef=\"{}\" targetRef=\"{}\" />",
            shapes[*source].id, shapes[*target].id
        );
    }
    xml.push_str("  </bpmn:process>\n");
    xml.push_str("  <bpmndi:BPMNDiagram id=\"Diagram_1\">\n    <bpmndi:BPMNPlane id=\"Plane_1\" bpmnElement=\"Process_1\">\n");
    let centre = |index: usize| {
        let (x, y) = positions[index];
        let (width, height) = size(shapes[index].kind);
        (x + width / 2.0, y + TASK_HEIGHT / 2.0, width, height)
    };
    for (index, shape) in shapes.iter().enumerate() {
        let (cx, cy, width, height) = centre(index);
        let _ = writeln!(
            xml,
            "      <bpmndi:BPMNShape id=\"{}_di\" bpmnElement=\"{}\"{}>\n        <dc:Bounds x=\"{:.0}\" y=\"{:.0}\" width=\"{width:.0}\" height=\"{height:.0}\" />\n      </bpmndi:BPMNShape>",
            shape.id,
            shape.id,
            if shape.kind == Kind::Gateway {
                " isMarkerVisible=\"true\""
            } else {
                ""
            },
            cx - width / 2.0,
            cy - height / 2.0
        );
    }
    for (id, source, target) in flows {
        let (sx, sy, sw, _) = centre(*source);
        let (tx, ty, tw, th) = centre(*target);
        let from = (sx + sw / 2.0, sy);
        let waypoints = if tx > sx {
            vec![from, (tx - tw / 2.0, ty)]
        } else {
            // Back to an earlier column: run below both shapes.
            let below = sy.max(ty) + th / 2.0 + 30.0;
            vec![
                from,
                (from.0 + 16.0, sy),
                (from.0 + 16.0, below),
                (tx, below),
                (tx, ty + th / 2.0),
            ]
        };
        let _ = writeln!(
            xml,
            "      <bpmndi:BPMNEdge id=\"{id}_di\" bpmnElement=\"{id}\">"
        );
        for (x, y) in waypoints {
            let _ = writeln!(xml, "        <di:waypoint x=\"{x:.0}\" y=\"{y:.0}\" />");
        }
        xml.push_str("      </bpmndi:BPMNEdge>\n");
    }
    xml.push_str("    </bpmndi:BPMNPlane>\n  </bpmndi:BPMNDiagram>\n</bpmn:definitions>\n");
    xml
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map_view::{MapSelection, map_view};
    use pm_core::dict::{ActivityId, CaseId, Dictionary};
    use pm_core::log::{CaseTrace, Event, EventLog};

    fn log(routes: &[(&[&str], usize)]) -> EventLog {
        let mut cases = Dictionary::default();
        let mut activities = Dictionary::default();
        let mut traces = Vec::new();
        let mut row = 0;
        for (route, count) in routes {
            for _ in 0..*count {
                let case = CaseId(cases.intern(&format!("case-{}", traces.len())));
                let events = route
                    .iter()
                    .enumerate()
                    .map(|(index, label)| {
                        row += 1;
                        Event::new(
                            ActivityId(activities.intern(label)),
                            index as i64 * 86_400_000_000,
                            row,
                        )
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

    fn roundtrip(xml: &str, name: &str) -> pm_ingest::BpmnModel {
        let path =
            std::env::temp_dir().join(format!("glasstarn-{name}-{}.bpmn", std::process::id()));
        std::fs::write(&path, xml).unwrap();
        let model = pm_ingest::import_bpmn(&path);
        let _ = std::fs::remove_file(&path);
        model.unwrap_or_else(|error| panic!("exported BPMN does not import: {error}\n{xml}"))
    }

    #[test]
    fn a_straight_route_is_start_tasks_end_with_no_gateways() {
        let view = map_view(
            &log(&[(&["Receive", "Check", "Pay"], 3)]),
            &MapSelection::default(),
        )
        .unwrap();
        let xml = map_view_bpmn(&view, "Invoices");
        assert_eq!(xml.matches("<bpmn:task ").count(), 3);
        assert_eq!(xml.matches("<bpmn:exclusiveGateway ").count(), 0);
        assert_eq!(xml.matches("<bpmn:sequenceFlow ").count(), 4);
        assert!(xml.contains("name=\"Invoices\""));
        assert!(xml.contains("<bpmndi:BPMNDiagram"));
        let model = roundtrip(&xml, "straight");
        assert_eq!(model.activities.len(), 3);
    }

    #[test]
    fn choices_and_loops_become_exclusive_gateways_that_reimport() {
        let routes: [(&[&str], usize); 3] = [
            (&["Receive", "Check", "Approve", "Pay"], 5),
            (&["Receive", "Check", "Reject"], 2),
            (
                &[
                    "Receive",
                    "Check",
                    "Ask for more",
                    "Check",
                    "Approve",
                    "Pay",
                ],
                1,
            ),
        ];
        let view = map_view(
            &log(&routes),
            &MapSelection::Routes {
                variants: vec![1, 2, 3],
            },
        )
        .unwrap();
        let xml = map_view_bpmn(&view, "Claims & refunds");
        assert!(xml.contains("Claims &amp; refunds"));
        assert!(xml.matches("<bpmn:exclusiveGateway ").count() >= 2);
        assert!(
            !xml.contains("parallelGateway"),
            "observed order is never written as concurrency"
        );
        let model = roundtrip(&xml, "choices");
        assert_eq!(model.activities.len(), 6);
        // Every shape and flow is placed so modelling tools can draw it.
        assert_eq!(
            xml.matches("<bpmndi:BPMNShape ").count(),
            xml.matches("<bpmn:task ").count() + xml.matches("<bpmn:exclusiveGateway ").count() + 2
        );
        assert_eq!(
            xml.matches("<bpmndi:BPMNEdge ").count(),
            xml.matches("<bpmn:sequenceFlow ").count()
        );
    }

    #[test]
    fn a_step_that_repeats_itself_keeps_one_way_in_and_out() {
        let view = map_view(
            &log(&[(&["Scan", "Scan", "File"], 2), (&["Scan", "File"], 3)]),
            &MapSelection::Routes {
                variants: vec![1, 2],
            },
        )
        .unwrap();
        let model = roundtrip(&map_view_bpmn(&view, "Scanning"), "loop");
        assert_eq!(model.activities.len(), 2);
    }
}
