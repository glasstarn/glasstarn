//! Keeping agent-facing answers small.
//!
//! An agent pays for every byte it reads, and a long list rarely changes the
//! answer: the first few routes, hand-overs or deviations do. So answers for
//! agents are held to a byte budget by default. Lists are shortened, longest
//! effect first, never silently: each shortened list gets a sibling
//! `<name>_omitted` count, and the answer says how to ask for everything.
//! Numbers are never changed or rounded.

use serde_json::{Value, json};

/// The default budget for one answer, in bytes of compact JSON.
pub const AGENT_BUDGET: usize = 4096;

/// Parts kept whole: the identifiers an agent needs to call the next tool
/// (`catalog`), the description of what was measured, and the headline's
/// findings, which a first answer must cover in full.
const KEEP_WHOLE: [&str; 5] = ["catalog", "filter", "view", "summary", "findings"];

/// Shortens lists in `value` until its compact JSON fits `budget` bytes.
/// The largest list is halved first, again and again, so short lists that
/// describe the answer stay whole while long evidence lists give way.
/// Returns the value unchanged when it already fits. A shortened answer
/// carries a `shortened` note naming `how_to_get_everything`.
pub fn fit(value: Value, budget: usize, how_to_get_everything: &str) -> Value {
    let full_bytes = size(&value);
    if full_bytes <= budget {
        return value;
    }
    let mut fitted = value;
    // Original lengths of the lists cut, by their path.
    let mut cut: Vec<(Vec<Step>, usize)> = Vec::new();
    while size(&fitted) > budget {
        let mut largest = None;
        let mut path = Vec::new();
        find_largest(&fitted, &mut path, &mut largest);
        let Some((_, path)) = largest else { break };
        let Some(Value::Array(items)) = at(&mut fitted, &path) else {
            break;
        };
        let before = items.len();
        items.truncate(before.div_ceil(2).min(before - 1).max(1));
        if !cut.iter().any(|(seen, _)| *seen == path) {
            cut.push((path, before));
        }
    }
    for (path, original) in &cut {
        let Some((Step::Key(key), parent)) = path.split_last() else {
            continue;
        };
        let kept = match at(&mut fitted, path) {
            Some(Value::Array(items)) => items.len(),
            _ => continue,
        };
        if let Some(Value::Object(map)) = at(&mut fitted, parent) {
            map.insert(format!("{key}_omitted"), json!(original - kept));
        }
    }
    if let Value::Object(map) = &mut fitted {
        map.insert(
            "shortened".to_owned(),
            json!({
                "budget_bytes": budget,
                "full_bytes": full_bytes,
                "how_to_get_everything": how_to_get_everything,
                "note": "The longest lists were cut to their first items, in the engine's order; each cut list has a sibling <name>_omitted count. Totals and figures are unchanged."
            }),
        );
    }
    fitted
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Key(String),
    Index(usize),
}

fn size(value: &Value) -> usize {
    serde_json::to_vec(value).map_or(usize::MAX, |bytes| bytes.len())
}

/// Finds the largest list (in bytes) that is an object field outside the
/// parts kept whole and still has more than one item. Returns its size.
fn find_largest(
    value: &Value,
    path: &mut Vec<Step>,
    largest: &mut Option<(usize, Vec<Step>)>,
) -> usize {
    match value {
        Value::Object(map) => {
            let mut total = 2;
            for (key, child) in map {
                path.push(Step::Key(key.clone()));
                let child_size = if KEEP_WHOLE.contains(&key.as_str()) {
                    size(child)
                } else {
                    let child_size = find_largest(child, path, largest);
                    if let Value::Array(items) = child
                        && items.len() > 1
                        && largest.as_ref().is_none_or(|(best, _)| child_size > *best)
                    {
                        *largest = Some((child_size, path.clone()));
                    }
                    child_size
                };
                path.pop();
                total += key.len() + 4 + child_size;
            }
            total
        }
        Value::Array(items) => {
            let mut total = 2;
            for (index, item) in items.iter().enumerate() {
                path.push(Step::Index(index));
                total += find_largest(item, path, largest) + 1;
                path.pop();
            }
            total
        }
        other => size(other),
    }
}

fn at<'a>(value: &'a mut Value, path: &[Step]) -> Option<&'a mut Value> {
    path.iter().try_fold(value, |current, step| match step {
        Step::Key(key) => current.get_mut(key.as_str()),
        Step::Index(index) => current.get_mut(*index),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_answers_are_untouched() {
        let value = json!({"cases": 3, "routes": [1, 2, 3]});
        assert_eq!(fit(value.clone(), 4096, "x"), value);
    }

    #[test]
    fn long_lists_are_cut_and_counted_until_the_answer_fits() {
        let routes = (0..500)
            .map(|n| json!({"rank": n, "steps": ["a", "b", "c"]}))
            .collect::<Vec<_>>();
        let value = json!({"cases": 9000, "routes": routes, "rules": [1, 2, 3]});
        let fitted = fit(value, 1024, "pass detail: full");
        assert!(size(&fitted) <= 1024 + 400, "{}", size(&fitted));
        assert_eq!(fitted["cases"], 9000);
        assert_eq!(fitted["routes"][0]["rank"], 0);
        // A short descriptive list beside it stays whole.
        let kept = fitted["routes"].as_array().unwrap().len();
        assert_eq!(fitted["routes_omitted"], 500 - kept);
        assert_eq!(
            fitted["shortened"]["how_to_get_everything"],
            "pass detail: full"
        );
    }

    #[test]
    fn identifiers_for_the_next_call_are_kept_whole() {
        let value = json!({"catalog": {"event_types": (0..300).collect::<Vec<_>>()},
            "items": (0..300).collect::<Vec<_>>()});
        let fitted = fit(value, 500, "x");
        assert_eq!(
            fitted["catalog"]["event_types"].as_array().unwrap().len(),
            300
        );
        assert_eq!(fitted["items"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn nested_lists_are_counted_beside_themselves() {
        let value = json!({"variants": [{"sequence": (0..50).collect::<Vec<_>>()}]});
        let fitted = fit(value, 60, "x");
        assert_eq!(
            fitted["variants"][0]["sequence_omitted"],
            50 - fitted["variants"][0]["sequence"].as_array().unwrap().len()
        );
    }
}
