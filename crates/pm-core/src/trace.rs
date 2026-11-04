//! Traces.
//!
//! A [`Trace`] is the ordered sequence of activities for one case, without
//! timings. Grouping identical traces yields variants — see
//! [`crate::variants`].

use crate::dict::{ActivityId, CaseId};

/// The ordered sequence of activities for one case.
///
/// Ordering is established during ingestion by the documented rules — explicit
/// order field, then timestamp, then lifecycle rank, then original row — and is
/// treated as authoritative here.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    /// The case this trace belongs to.
    pub case: CaseId,
    /// Activities in execution order.
    pub activities: Vec<ActivityId>,
}

impl Trace {
    /// Creates a trace for `case` from an ordered activity sequence.
    pub fn new(case: CaseId, activities: Vec<ActivityId>) -> Self {
        Self { case, activities }
    }

    /// Number of events in the trace.
    pub fn len(&self) -> usize {
        self.activities.len()
    }

    /// Returns `true` if the trace has no events.
    ///
    /// A valid ingested trace always has at least one event; an empty trace
    /// indicates a validation failure upstream.
    pub fn is_empty(&self) -> bool {
        self.activities.is_empty()
    }

    /// Iterates over directly-follows pairs within this trace.
    ///
    /// For `A → B → C` this yields `(A, B)` and `(B, C)`. A single-event trace
    /// yields nothing, which is correct: it contributes a node but no edge.
    pub fn directly_follows(&self) -> impl Iterator<Item = (ActivityId, ActivityId)> + '_ {
        self.activities.windows(2).map(|w| (w[0], w[1]))
    }

    /// Returns the number of immediate repetitions (self-loops) in the trace.
    pub fn self_loops(&self) -> usize {
        self.activities.windows(2).filter(|w| w[0] == w[1]).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace(case: u32, acts: &[u32]) -> Trace {
        Trace::new(CaseId(case), acts.iter().map(|&a| ActivityId(a)).collect())
    }

    #[test]
    fn directly_follows_yields_consecutive_pairs() {
        let t = trace(1, &[10, 20, 30]);
        let pairs: Vec<_> = t.directly_follows().collect();
        assert_eq!(
            pairs,
            vec![
                (ActivityId(10), ActivityId(20)),
                (ActivityId(20), ActivityId(30)),
            ]
        );
    }

    /// A single-event case contributes a node but no edge. This is a real
    /// shape in production data, not an edge case to reject.
    #[test]
    fn single_event_trace_yields_no_pairs() {
        let t = trace(1, &[10]);
        assert_eq!(t.directly_follows().count(), 0);
        assert_eq!(t.len(), 1);
    }

    #[test]
    fn counts_self_loops() {
        // A → B → B → B → C has two immediate repetitions.
        let t = trace(1, &[1, 2, 2, 2, 3]);
        assert_eq!(t.self_loops(), 2);
    }

    #[test]
    fn no_self_loops_in_distinct_sequence() {
        let t = trace(1, &[1, 2, 3, 2]);
        assert_eq!(t.self_loops(), 0);
    }
}
