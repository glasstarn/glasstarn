//! Explicit endpoint selection for recorded elapsed time. No inferred adjacency.
use crate::{ActivityId, CaseTrace, EventLog, Micros};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Occurrence {
    #[default]
    First,
    Last,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Segment {
    pub from: String,
    pub to: String,
    pub from_occurrence: Occurrence,
    pub to_occurrence: Occurrence,
    pub min_micros: Option<Micros>,
    pub max_micros: Option<Micros>,
}
impl Segment {
    pub fn duration(&self, log: &EventLog, trace: &CaseTrace) -> Option<Micros> {
        if trace.incomplete {
            return None;
        }
        let pick = |name: &str, occurrence: Occurrence| {
            let id = ActivityId(log.activities.get(name)?);
            let mut events = trace.events.iter().filter(|event| event.activity == id);
            match occurrence {
                Occurrence::First => events.next(),
                Occurrence::Last => events.next_back(),
            }
            .map(|event| event.at)
        };
        let duration = pick(&self.to, self.to_occurrence)?
            .checked_sub(pick(&self.from, self.from_occurrence)?)?;
        (duration >= 0).then_some(duration)
    }
    pub fn contains(&self, duration: Micros) -> bool {
        self.min_micros.is_none_or(|min| duration >= min)
            && self.max_micros.is_none_or(|max| duration <= max)
    }
}
