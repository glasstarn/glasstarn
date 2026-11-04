//! Deterministic object-centric timing rankings with source evidence.

use crate::{
    EventTypeId, ObjectTypeId, OcelDirectlyFollowsEvidence, OcelGraphs, OcelLog,
    OcelSynchronizationEvidence, OcelView, analyze_ocel_synchronization,
};
use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelLifecycleBottleneck {
    pub rank: usize,
    pub object_type: ObjectTypeId,
    pub source: EventTypeId,
    pub target: EventTypeId,
    pub occurrences: u64,
    pub objects: u64,
    pub total_elapsed_micros: i64,
    pub mean_elapsed_micros: i64,
    pub share_millionths: u64,
    pub evidence: Vec<OcelDirectlyFollowsEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelSynchronizationBottleneck {
    pub rank: usize,
    pub activity: EventTypeId,
    pub object_types: Vec<ObjectTypeId>,
    pub multi_object_events: u64,
    pub eligible_events: u64,
    pub insufficient_history_events: u64,
    pub total_arrival_spread_micros: i64,
    pub mean_arrival_spread_micros: i64,
    pub share_millionths: u64,
    pub evidence: Vec<OcelSynchronizationEvidence>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OcelBottleneckRanking {
    pub lifecycle_candidates: u64,
    pub lifecycle_positive_candidates: u64,
    pub total_lifecycle_elapsed_micros: i64,
    pub lifecycle: Vec<OcelLifecycleBottleneck>,
    pub synchronization_eligible_activities: u64,
    pub synchronization_positive_activities: u64,
    pub total_arrival_spread_micros: i64,
    pub synchronization: Vec<OcelSynchronizationBottleneck>,
}

/// Ranks two distinct observable burdens without combining them into an opaque
/// score: total lifecycle-transition elapsed time and total prior-point spread
/// at multi-object events. Zero-duration candidates remain in the disclosed
/// denominators but are not presented as bottlenecks.
pub fn rank_ocel_bottlenecks(log: &OcelLog, view: &OcelView) -> OcelBottleneckRanking {
    rank_bottlenecks(
        OcelGraphs::discover(log, view).directly_follows,
        analyze_ocel_synchronization(log, view),
    )
}

/// The ranking from the lifecycle transitions and the synchronization
/// measurements, however they were read.
pub fn rank_bottlenecks(
    directly_follows: Vec<crate::OcelDirectlyFollowsEdge>,
    synchronization_all: Vec<crate::OcelSynchronization>,
) -> OcelBottleneckRanking {
    let lifecycle_candidates = count(directly_follows.len());
    let mut lifecycle = directly_follows
        .into_iter()
        .filter(|edge| edge.total_elapsed_micros > 0)
        .collect::<Vec<_>>();
    lifecycle.sort_by_key(|edge| {
        (
            std::cmp::Reverse(edge.total_elapsed_micros),
            edge.object_type,
            edge.source,
            edge.target,
        )
    });
    let lifecycle_total = sum_positive(lifecycle.iter().map(|edge| edge.total_elapsed_micros));
    let lifecycle = lifecycle
        .into_iter()
        .enumerate()
        .map(|(index, edge)| OcelLifecycleBottleneck {
            rank: index + 1,
            object_type: edge.object_type,
            source: edge.source,
            target: edge.target,
            occurrences: edge.occurrences,
            objects: edge.objects,
            total_elapsed_micros: edge.total_elapsed_micros,
            mean_elapsed_micros: edge.mean_elapsed_micros,
            share_millionths: share(edge.total_elapsed_micros, lifecycle_total),
            evidence: edge.evidence,
        })
        .collect::<Vec<_>>();

    let synchronization_eligible_activities = count(
        synchronization_all
            .iter()
            .filter(|activity| activity.eligible_events > 0)
            .count(),
    );
    let mut synchronization = synchronization_all
        .into_iter()
        .filter(|activity| activity.total_arrival_spread_micros > 0)
        .collect::<Vec<_>>();
    synchronization.sort_by_key(|activity| {
        (
            std::cmp::Reverse(activity.total_arrival_spread_micros),
            activity.activity,
        )
    });
    let synchronization_total = sum_positive(
        synchronization
            .iter()
            .map(|activity| activity.total_arrival_spread_micros),
    );
    let synchronization = synchronization
        .into_iter()
        .enumerate()
        .map(|(index, activity)| OcelSynchronizationBottleneck {
            rank: index + 1,
            activity: activity.activity,
            object_types: activity.object_types,
            multi_object_events: activity.multi_object_events,
            eligible_events: activity.eligible_events,
            insufficient_history_events: activity.insufficient_history_events,
            total_arrival_spread_micros: activity.total_arrival_spread_micros,
            mean_arrival_spread_micros: activity.mean_arrival_spread_micros.unwrap_or(0),
            share_millionths: share(activity.total_arrival_spread_micros, synchronization_total),
            evidence: activity.evidence,
        })
        .collect::<Vec<_>>();

    OcelBottleneckRanking {
        lifecycle_candidates,
        lifecycle_positive_candidates: count(lifecycle.len()),
        total_lifecycle_elapsed_micros: lifecycle_total,
        lifecycle,
        synchronization_eligible_activities,
        synchronization_positive_activities: count(synchronization.len()),
        total_arrival_spread_micros: synchronization_total,
        synchronization,
    }
}

fn sum_positive(values: impl Iterator<Item = i64>) -> i64 {
    let total = values.fold(0_i128, |total, value| {
        total.saturating_add(i128::from(value.max(0)))
    });
    i64::try_from(total).unwrap_or(i64::MAX)
}

fn share(value: i64, total: i64) -> u64 {
    if value <= 0 || total <= 0 {
        return 0;
    }
    u64::try_from(
        i128::from(value)
            .saturating_mul(1_000_000)
            .saturating_div(i128::from(total)),
    )
    .unwrap_or(1_000_000)
}

fn count(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OcelBuilder, OcelFilter};

    fn log() -> OcelLog {
        let mut builder = OcelBuilder::new();
        for activity in ["a", "b", "c", "join slow", "join fast"] {
            builder.declare_event_type(activity, vec![]).unwrap();
        }
        builder.declare_object_type("order", vec![]).unwrap();
        for (object, _) in [("o1", 0), ("o2", 0), ("o3", 0), ("o4", 0)] {
            builder.add_object(object, "order").unwrap();
        }
        for (event, activity, at) in [
            ("a1", "a", 0),
            ("b1", "b", 10),
            ("a2", "a", 0),
            ("b2", "b", 20),
            ("c1", "c", 4),
            ("slow", "join slow", 30),
            ("c2", "c", 8),
            ("fast", "join fast", 31),
        ] {
            builder.add_event(event, activity, at).unwrap();
        }
        for (event, object) in [
            ("a1", "o1"),
            ("b1", "o1"),
            ("a2", "o2"),
            ("b2", "o2"),
            ("c1", "o3"),
            ("slow", "o1"),
            ("slow", "o3"),
            ("c2", "o4"),
            ("fast", "o2"),
            ("fast", "o4"),
        ] {
            builder
                .relate_event_object(event, object, "participant")
                .unwrap();
        }
        builder.build()
    }

    #[test]
    fn ranks_lifecycle_elapsed_burden_and_convergence_spread_separately() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let ranking = rank_ocel_bottlenecks(&log, &view);
        assert_eq!(ranking.lifecycle[0].source, EventTypeId(0));
        assert_eq!(ranking.lifecycle[0].target, EventTypeId(1));
        assert_eq!(ranking.lifecycle[0].total_elapsed_micros, 30);
        assert_eq!(ranking.lifecycle[0].rank, 1);
        assert_eq!(ranking.synchronization[0].activity, EventTypeId(4));
        assert_eq!(ranking.synchronization[0].total_arrival_spread_micros, 12);
        assert_eq!(ranking.synchronization[1].total_arrival_spread_micros, 6);
        assert_eq!(
            ranking
                .synchronization
                .iter()
                .map(|item| item.share_millionths)
                .sum::<u64>(),
            999_999
        );
    }

    #[test]
    fn zero_duration_candidates_are_disclosed_but_not_ranked() {
        let mut builder = OcelBuilder::new();
        builder.declare_event_type("a", vec![]).unwrap();
        builder.declare_event_type("b", vec![]).unwrap();
        builder.declare_object_type("order", vec![]).unwrap();
        builder.add_event("a", "a", 1).unwrap();
        builder.add_event("b", "b", 1).unwrap();
        builder.add_object("o", "order").unwrap();
        builder.relate_event_object("a", "o", "subject").unwrap();
        builder.relate_event_object("b", "o", "subject").unwrap();
        let log = builder.build();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        let ranking = rank_ocel_bottlenecks(&log, &view);
        assert_eq!(ranking.lifecycle_candidates, 0);
        assert!(ranking.lifecycle.is_empty());
    }

    #[test]
    fn ranking_is_deterministic() {
        let log = log();
        let view = OcelView::apply(&log, &OcelFilter::default()).unwrap();
        assert_eq!(
            rank_ocel_bottlenecks(&log, &view),
            rank_ocel_bottlenecks(&log, &view)
        );
    }
}
