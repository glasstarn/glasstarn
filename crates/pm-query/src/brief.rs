//! The analyst's brief — a compact markdown summary for an AI assistant.
//!
//! # Why this exists
//!
//! Connecting an MCP server is the richer path, but it requires the user to be
//! able to install and configure one. Many corporate users cannot, yet do have
//! an approved chat tool. For them a file they can paste is the only route to
//! assistance, and it works with any assistant with no integration at all.
//!
//! # What it is not
//!
//! Pasting this into a hosted chat tool sends activity names, metrics, and case
//! identifiers to a model provider — exactly as an MCP tool call would. This
//! solves *integration friction*, not the privacy boundary. The brief says so
//! in its own footer, so nobody is misled by the file itself.
//!
//! # Claim labelling
//!
//! Every number here is a **calculated fact** produced by the deterministic
//! engine. The brief says that explicitly and instructs the assistant to label
//! its own additions as interpretation or hypothesis, so the distinction
//! survives into whatever the user writes up.

use crate::analysis::Analysis;
use std::fmt::Write;

/// Renders an analysis as a markdown brief for an AI assistant.
pub fn brief(a: &Analysis) -> String {
    let mut s = String::new();

    let _ = writeln!(s, "# Process analysis: {}", a.source);
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Produced by a deterministic process-mining engine. **Every figure below is a \
         calculated fact** measured from the event log — not an estimate, a model output, or \
         an interpretation."
    );
    let _ = writeln!(s);

    if let Some(columns) = &a.unpivoted_from {
        let _ = writeln!(
            s,
            "The source was one row per case with several timestamp columns. These columns \
             were turned into activities: {}.",
            columns.join("; ")
        );
        let _ = writeln!(s);
    }

    if a.view.narrowed {
        let _ = writeln!(
            s,
            "**This is a filtered view.** It covers {} of {} cases ({:.0}%). Every figure below \
             describes only those cases, not the whole process.",
            a.view.cases_shown, a.view.cases_total, a.view.percent_shown
        );
        let _ = writeln!(s);
    }

    // --- Shape ------------------------------------------------------------
    let _ = writeln!(s, "## Shape of the data");
    let _ = writeln!(s);
    let _ = writeln!(s, "| Measure | Value |");
    let _ = writeln!(s, "|---|---|");
    let _ = writeln!(s, "| Events | {} |", a.summary.events);
    let _ = writeln!(s, "| Cases | {} |", a.summary.cases);
    let _ = writeln!(s, "| Distinct activities | {} |", a.summary.activities);
    let _ = writeln!(s, "| Distinct paths (variants) | {} |", a.summary.variants);
    let _ = writeln!(
        s,
        "| Paths covering 80% of cases | {} |",
        a.summary.variants_for_80_percent
    );
    let _ = writeln!(
        s,
        "| Paths followed by exactly one case | {} |",
        a.summary.unique_variants
    );
    if a.summary.single_event_cases > 0 {
        let _ = writeln!(
            s,
            "| Cases with a single event | {} |",
            a.summary.single_event_cases
        );
    }
    if let Some(cost) = a.summary.total_cost_microunits {
        let _ = writeln!(
            s,
            "| Total mapped cost | {} source units |",
            cost_units(cost)
        );
    }
    let throughput = if a.summary.duration_eligible_cases == 0 {
        "unavailable".to_owned()
    } else {
        duration(a.summary.mean_throughput_seconds)
    };
    let median_throughput = if a.summary.duration_eligible_cases == 0 {
        "unavailable".to_owned()
    } else {
        duration(a.summary.median_throughput_seconds)
    };
    let _ = writeln!(
        s,
        "| Mean throughput | {} ({} of {} cases eligible) |",
        throughput, a.summary.duration_eligible_cases, a.summary.cases
    );
    let _ = writeln!(
        s,
        "| Median throughput | {} ({} of {} cases eligible) |",
        median_throughput, a.summary.duration_eligible_cases, a.summary.cases
    );
    let _ = writeln!(s);

    // --- Where the time goes ---------------------------------------------
    let _ = writeln!(s, "## Where the time goes");
    let _ = writeln!(s);
    if a.edges.is_empty() {
        let _ = writeln!(s, "No transitions — every case has a single event.");
    } else if a.bottlenecks.is_empty() {
        let _ = writeln!(
            s,
            "Transitions were observed, but none were eligible for duration analysis because all affected cases were incomplete."
        );
    } else {
        let _ = writeln!(
            s,
            "{} Ranked by the **total** time each transition accounts for across all cases.",
            if a.measures_true_waiting {
                "Waiting time between one activity finishing and the next starting — an end \
                 timestamp was available, so this excludes processing time."
            } else {
                "Elapsed time from the start of one activity to the start of the next. **This \
                 is not purely waiting**: it includes however long the earlier activity itself \
                 took, because the log records only one timestamp per event."
            }
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| Rank | Transition | {} | Share of total | Timed / observed |",
            if a.measures_true_waiting {
                "Mean wait"
            } else {
                "Mean elapsed"
            }
        );
        let _ = writeln!(s, "|---|---|---|---|---|");
        for b in a.bottlenecks.iter().take(8) {
            let _ = writeln!(
                s,
                "| {} | {} → {} | {} | {:.1}% | {} |",
                b.rank,
                b.from,
                b.to,
                duration(b.mean_seconds),
                b.share_percent,
                format_args!(
                    "{} / {}",
                    b.duration_eligible_occurrences, b.total_occurrences
                )
            );
        }
    }
    let _ = writeln!(s);

    // --- Paths ------------------------------------------------------------
    let _ = writeln!(s, "## Most common paths");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "| Cases | Share | Mean throughput | Timed cases | Path | Example cases |"
    );
    let _ = writeln!(s, "|---|---|---|---|---|---|");
    for v in a.variants.iter().take(10) {
        let _ = writeln!(
            s,
            "| {} | {:.1}% | {} | {} / {} | {} | {} |",
            v.cases,
            v.percent,
            if v.duration_eligible_cases == 0 {
                "unavailable".to_owned()
            } else {
                duration(v.mean_throughput_seconds)
            },
            v.duration_eligible_cases,
            v.cases,
            v.sequence.join(" → "),
            v.example_cases.join(", ")
        );
    }
    let _ = writeln!(s);

    // --- Activity inventory ----------------------------------------------
    // The raw labels, so an assistant can spot coded names, near-duplicates,
    // and system noise, and propose a rule set to address them.
    if !a.nodes.is_empty() {
        let _ = writeln!(s, "## Activities as recorded");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "Labels exactly as they appear in the source data. Nothing has been renamed or \
             merged unless listed under preparation rules below."
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| Activity | Occurrences | Cases | Starts a case | Ends a case |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|");
        let mut nodes: Vec<&crate::analysis::Node> = a.nodes.iter().collect();
        nodes.sort_by(|x, y| {
            y.frequency
                .cmp(&x.frequency)
                .then(x.activity.cmp(&y.activity))
        });
        for n in nodes.iter().take(60) {
            let _ = writeln!(
                s,
                "| `{}` | {} | {} | {} | {} |",
                n.activity, n.frequency, n.case_frequency, n.starts, n.ends
            );
        }
        if a.nodes.len() > 60 {
            let _ = writeln!(s, "| … {} more | | | | |", a.nodes.len() - 60);
        }
        let _ = writeln!(s);
    }

    // --- Rework -----------------------------------------------------------
    if a.rework.cases_affected > 0 {
        let _ = writeln!(s, "## Rework and observed loops");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "{} cases ({:.1}%) repeated at least one activity. {} immediate repetitions; {} returns after intervening work.",
            a.rework.cases_affected,
            a.rework.case_percent,
            a.rework.immediate_repetitions,
            a.rework.returns_after_intervening
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| Activity | Repetitions | Immediate | Returned later | Cases | Example cases |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|");
        for activity in a.rework.activities.iter().take(10) {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} | {} | {} |",
                activity.activity,
                activity.repetitions,
                activity.immediate_repetitions,
                activity.returns_after_intervening,
                activity.cases_affected,
                activity.example_cases.join(", ")
            );
        }
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "Observed loop paths (from actual cases, not cycles inferred from the graph):"
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| Loop | Traversals | Cases | Timed traversals | Mean duration | Example cases |"
        );
        let _ = writeln!(s, "|---|---|---|---|---|---|");
        for loop_path in a.rework.loops.iter().take(10) {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {} of {} | {} | {} |",
                loop_path.sequence.join(" → "),
                loop_path.occurrences,
                loop_path.cases_affected,
                loop_path.duration_eligible_occurrences,
                loop_path.occurrences,
                if loop_path.duration_eligible_occurrences == 0 {
                    "unavailable".to_owned()
                } else {
                    duration(loop_path.mean_duration_seconds)
                },
                loop_path.example_cases.join(", ")
            );
        }
        if a.rework.omitted_loop_candidates > 0 {
            let _ = writeln!(
                s,
                "{} oversized or excess loop-path candidates were omitted from path detail; activity-level repetition counts remain complete.",
                a.rework.omitted_loop_candidates
            );
        }
        let _ = writeln!(s);
    }

    // --- Organization -----------------------------------------------------
    if let Some(org) = &a.organization {
        let _ = writeln!(s, "## Who passes work to whom");
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "{} resources. {:.0}% of consecutive steps changed hands.",
            org.resources, org.fragmentation_percent
        );
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "| From | To | {} | Share of handover time | Timed / observed |",
            if a.measures_true_waiting {
                "Mean wait"
            } else {
                "Mean elapsed"
            }
        );
        let _ = writeln!(s, "|---|---|---|---|---|");
        for h in org.handovers.iter().take(8) {
            let _ = writeln!(
                s,
                "| {} | {} | {} | {:.1}% | {} |",
                h.from,
                h.to,
                duration(h.mean_seconds),
                h.share_percent,
                format_args!("{} / {}", h.duration_eligible_occurrences, h.occurrences)
            );
        }
        let _ = writeln!(s);
        let _ = writeln!(
            s,
            "A handover is one resource acting immediately after a different one in the same \
             case. It records **sequence**, not that work was deliberately passed or that either \
             person was slow. Waiting time belongs to the transition, not to a person — do not \
             use this to characterise individuals."
        );
        let _ = writeln!(s);
    }

    // --- Automation targeting ---------------------------------------------
    if let Some(t) = &a.targeting {
        let _ = writeln!(s, "## Where the machine stops and people start");
        let _ = writeln!(s);
        if t.resources_mapped && !t.unclassified_resources.is_empty() {
            // Until someone says which accounts are systems, a step by an
            // undecided account is neither manual nor automated.
            let _ = writeln!(
                s,
                "**Not yet classified.** Nobody has confirmed which accounts are people and which \
                 are systems: {} accounts are undecided, covering {} steps. Until they are, \
                 the straight-through rate is not shown and the steps below may include system \
                 work. Undecided: {}.",
                t.unclassified_resources.len(),
                t.unclassified_events,
                capped_list(&t.unclassified_resources, 20)
            );
            let _ = writeln!(s);
            if !t.suggested_system_resources.is_empty() {
                let _ = writeln!(
                    s,
                    "These names look like system accounts; ask the person to confirm: {}.",
                    capped_list(&t.suggested_system_resources, 20)
                );
                let _ = writeln!(s);
            }
        } else {
            match t.straight_through_percent {
                Some(percent) => {
                    let _ = writeln!(
                        s,
                        "Among cases whose performers are known, {percent:.0}% ran straight through — \
                         {} untouched, {} needed a person. Performer coverage is {:.0}%; {} cases \
                         remain undetermined.",
                        t.straight_through_cases,
                        t.touched_cases,
                        t.classification_coverage_percent,
                        t.undetermined_cases
                    );
                }
                None => {
                    let _ = writeln!(
                        s,
                        "The straight-through rate is unknown: no resource column was mapped, so who \
                         performed each step was never recorded."
                    );
                }
            }
            let _ = writeln!(s);
        }

        if !t.manual_steps_ranked.is_empty() {
            let _ = writeln!(
                s,
                "Manual steps are ordered by {} × routing-context concentration. **Routing-context \
                 concentration** describes how concentrated the observed predecessor/successor \
                 pairs are. It does not show whether task inputs or decisions are rules-based. \
                 This is an investigation order, not an automation recommendation.",
                match t.ranking_basis {
                    pm_core::RankingBasis::ProcessingTime => "observed processing time",
                    pm_core::RankingBasis::Occurrences => "occurrences",
                }
            );
            let _ = writeln!(s);
            let _ = writeln!(
                s,
                "| Activity | Processing | Wait before | Start-to-start before | Cases | Routing concentration | Example cases |"
            );
            let _ = writeln!(s, "|---|---|---|---|---|---|---|");
            for step in t.manual_steps_ranked.iter().take(10) {
                let _ = writeln!(
                    s,
                    "| {} | {} ({} observations) | {} ({} observations) | {} ({} observations) | {} | {:.0}% | {} |",
                    step.activity,
                    duration(step.processing_seconds),
                    step.processing_observations,
                    duration(step.waiting_before_seconds),
                    step.waiting_observations,
                    duration(step.start_to_start_before_seconds),
                    step.start_to_start_observations,
                    step.cases,
                    step.routing_context_concentration,
                    step.example_cases.join(", ")
                );
            }
            let _ = writeln!(s);
        }

        if !t.divergences.is_empty() {
            let _ = writeln!(
                s,
                "Where cases leave the most frequent observed variant ({} cases, {:.1}%):",
                t.dominant_variant_cases, t.dominant_variant_percent
            );
            let _ = writeln!(s);
            let _ = writeln!(
                s,
                "| Cases | After | Did | Dominant observed variant next | Example cases |"
            );
            let _ = writeln!(s, "|---|---|---|---|---|");
            for d in t.divergences.iter().take(8) {
                let _ = writeln!(
                    s,
                    "| {} | {} | {} | {} | {} |",
                    d.cases,
                    d.after.as_deref().unwrap_or("(the start)"),
                    d.took.as_deref().unwrap_or("(nothing)"),
                    d.dominant_path_next.as_deref().unwrap_or("(nothing)"),
                    d.example_cases.join(", ")
                );
            }
            let _ = writeln!(s);
        }

        let _ = writeln!(s, "Interpretation limits:");
        for limitation in &t.interpretation_limits {
            let _ = writeln!(s, "- {limitation}");
        }
        let _ = writeln!(s);
    }

    // --- Caveats ----------------------------------------------------------
    let _ = writeln!(s, "## What this analysis does and does not establish");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "- The transition table is a **directly-follows graph**: it records what followed what. \
         It is not a validated process model, and activities that run concurrently appear as if \
         they loop between each other. The path table is the honest account of what actually \
         happened."
    );
    let _ = writeln!(
        s,
        "- Transition times are measured between recorded timestamps. They include queueing, \
         handoffs, and non-working hours, and cannot distinguish those from each other."
    );
    let _ = writeln!(
        s,
        "- Nothing here establishes **cause**. A slow transition shows where time accumulates, \
         not why."
    );
    if !a.notes.is_empty() {
        for note in &a.notes {
            let _ = writeln!(s, "- {note}");
        }
    }
    let _ = writeln!(s);

    // --- Instructions to the assistant -----------------------------------
    let _ = writeln!(s, "## How to use this");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "You are helping a process analyst investigate this process. Please:"
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "1. Treat every figure above as a **calculated fact**. Do not recompute or adjust them."
    );
    let _ = writeln!(
        s,
        "2. Label anything you add: **interpretation** (an explanation you are offering) or \
         **hypothesis** (a proposal needing investigation). Never present either as a finding."
    );
    let _ = writeln!(
        s,
        "3. Do not claim causation from these numbers. Suggest what would need to be checked to \
         establish it."
    );
    let _ = writeln!(
        s,
        "4. When you reference a figure, cite it — including the example case identifiers, so \
         the analyst can look at real cases."
    );
    let _ = writeln!(
        s,
        "5. If a question cannot be answered from this brief, say so and say what additional \
         data would answer it."
    );
    let _ = writeln!(s);

    // --- Rule proposal ----------------------------------------------------
    let _ = writeln!(s, "## Proposing data preparation");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "If the activity list contains coded names, several spellings of the same step, or \
         system noise, you may propose a preparation rule set. Reading meaning out of labels is \
         judgment, which is where you are useful; applying rules exactly is the engine's job."
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Propose it as JSON in this shape, and **say why for each entry**. The user reviews it \
         before anything is applied. It never modifies their data — it is recorded with the \
         project and applied at analysis time."
    );
    let _ = writeln!(s);
    let _ = writeln!(s, "```json");
    let _ = writeln!(s, "{{");
    let _ = writeln!(s, "  \"rename\": {{");
    let _ = writeln!(s, "    \"STAT_03\": \"Credit Check\",");
    let _ = writeln!(s, "    \"APPROVAL_GRANTED\": \"Approve\",");
    let _ = writeln!(s, "    \"Approved\": \"Approve\"");
    let _ = writeln!(s, "  }},");
    let _ = writeln!(s, "  \"exclude_activities\": [\"System Auto-Update\"],");
    let _ = writeln!(s, "  \"exclude_cases\": [],");
    let _ = writeln!(s, "  \"min_events_per_case\": 0,");
    let _ = writeln!(s, "  \"drop_incomplete_cases\": false");
    let _ = writeln!(s, "}}");
    let _ = writeln!(s, "```");
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "Saved as `rules.json`, it is applied with \
         `glasstarn analyze <file> --rules rules.json`."
    );
    let _ = writeln!(s);
    let _ = writeln!(
        s,
        "**Do not merge labels that might be genuinely different steps.** `Cancel` and \
         `Cancelled` may be distinct states in a real workflow. When unsure, say so and let the \
         user decide rather than proposing the merge."
    );
    let _ = writeln!(s);

    let _ = writeln!(
        s,
        "---\n\n*Privacy note: this brief contains activity names, metrics, and case \
         identifiers from the source log. Sending it to a hosted assistant discloses them to \
         that provider.*"
    );

    s
}

/// Formats seconds as a compact human duration.
/// Names joined with commas, at most `max` of them and a count of the rest.
fn capped_list(names: &[String], max: usize) -> String {
    let shown = names
        .iter()
        .take(max)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    match names.len().saturating_sub(max) {
        0 => shown,
        rest => format!("{shown} and {rest} more"),
    }
}

fn duration(seconds: i64) -> String {
    let s = seconds.abs();
    let out = if s >= 86_400 {
        format!("{:.1}d", s as f64 / 86_400.0)
    } else if s >= 3_600 {
        format!("{:.1}h", s as f64 / 3_600.0)
    } else if s >= 60 {
        format!("{:.0}m", s as f64 / 60.0)
    } else {
        format!("{s}s")
    };
    if seconds < 0 { format!("-{out}") } else { out }
}

fn cost_units(microunits: i128) -> String {
    let sign = if microunits < 0 { "-" } else { "" };
    let absolute = microunits.abs();
    let whole = absolute / 1_000_000;
    let fraction = absolute % 1_000_000;
    if fraction == 0 {
        format!("{sign}{whole}")
    } else {
        let fraction = format!("{fraction:06}");
        format!("{sign}{whole}.{}", fraction.trim_end_matches('0'))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{
        Analysis, Bottleneck, Edge, LoopSummary, Node, ReworkActivitySummary, ReworkSummary,
        Summary, VariantSummary,
    };

    fn analysis() -> Analysis {
        Analysis {
            recorded_window: None,
            source: "orders.csv".to_owned(),
            case_column: "case_id".to_owned(),
            activity_column: "activity".to_owned(),
            timestamp_column: "timestamp".to_owned(),
            timestamp_format: Some("iso_space_seconds".to_owned()),
            case_attributes: Vec::new(),
            step_level_fields: Vec::new(),
            mapping: pm_ingest::Mapping::new("case_id", "activity", "timestamp"),
            optional_columns: Vec::new(),
            unpivoted_from: None,
            excluded_events: 0,
            excluded_cases: 0,
            measures_true_waiting: false,
            view: crate::analysis::View {
                cases_shown: 200,
                cases_total: 200,
                events_total: 1184,
                percent_shown: 100.0,
                narrowed: false,
            },
            summary: Summary {
                events: 1184,
                cases: 200,
                activities: 8,
                variants: 7,
                single_event_cases: 0,
                unique_variants: 0,
                top_five_coverage_percent: 93.5,
                variants_for_80_percent: 4,
                mean_throughput_seconds: 216_000,
                median_throughput_seconds: 198_000,
                p90_throughput_seconds: 600_000,
                duration_eligible_cases: 200,
                incomplete_cases: 0,
                total_cost_microunits: None,
            },
            nodes: vec![Node {
                activity: "Check Credit".to_owned(),
                frequency: 200,
                case_frequency: 200,
                starts: 0,
                ends: 0,
                example_cases: vec!["ORDER-00001".to_owned()],
            }],
            edges: vec![
                Edge {
                    from: "Receive Order".to_owned(),
                    to: "Check Credit".to_owned(),
                    frequency: 200,
                    case_frequency: 200,
                    duration_eligible_occurrences: 200,
                    mean_seconds: 172_000,
                    total_seconds: 34_400_000,
                    min_seconds: 86_400,
                    max_seconds: 259_200,
                    frequency_share_percent: 94.3,
                    performance_share_percent: 99.9,
                    frequency_rank: 1,
                    performance_rank: Some(1),
                    example_cases: vec!["ORDER-00001".to_owned()],
                    same_time_occurrences: 0,
                },
                Edge {
                    from: "Investigate".to_owned(),
                    to: "Investigate".to_owned(),
                    frequency: 12,
                    case_frequency: 9,
                    duration_eligible_occurrences: 12,
                    mean_seconds: 3_600,
                    total_seconds: 43_200,
                    min_seconds: 600,
                    max_seconds: 7_200,
                    frequency_share_percent: 5.7,
                    performance_share_percent: 0.1,
                    frequency_rank: 2,
                    performance_rank: Some(2),
                    example_cases: vec!["ORDER-00006".to_owned()],
                    same_time_occurrences: 0,
                },
            ],
            bottlenecks: vec![Bottleneck {
                rank: 1,
                from: "Receive Order".to_owned(),
                to: "Check Credit".to_owned(),
                total_seconds: 34_400_000,
                mean_seconds: 172_000,
                total_occurrences: 200,
                duration_eligible_occurrences: 200,
                share_percent: 80.6,
                example_cases: vec!["ORDER-00001".to_owned(), "ORDER-00002".to_owned()],
            }],
            rework: ReworkSummary {
                cases_affected: 9,
                case_percent: 4.5,
                repeated_activity_occurrences: 12,
                immediate_repetitions: 12,
                returns_after_intervening: 0,
                loop_traversals: 12,
                omitted_loop_candidates: 0,
                activities: vec![ReworkActivitySummary {
                    activity: "Investigate".to_owned(),
                    repetitions: 12,
                    immediate_repetitions: 12,
                    returns_after_intervening: 0,
                    cases_affected: 9,
                    duration_eligible_occurrences: 12,
                    mean_repeat_interval_seconds: 3_600,
                    total_repeat_interval_seconds: 43_200,
                    example_cases: vec!["ORDER-00006".to_owned()],
                }],
                loops: vec![LoopSummary {
                    sequence: vec!["Investigate".to_owned(), "Investigate".to_owned()],
                    occurrences: 12,
                    cases_affected: 9,
                    duration_eligible_occurrences: 12,
                    mean_duration_seconds: 3_600,
                    total_duration_seconds: 43_200,
                    example_cases: vec!["ORDER-00006".to_owned()],
                }],
            },
            variants: vec![VariantSummary {
                sequence: vec!["Receive Order".to_owned(), "Check Credit".to_owned()],
                cases: 60,
                percent: 30.0,
                duration_eligible_cases: 60,
                mean_throughput_seconds: 181_000,
                example_cases: vec!["ORDER-00006".to_owned(), "ORDER-00007".to_owned()],
            }],
            notes: vec!["Time zone policy not set: read as written.".to_owned()],
            offers: Vec::new(),
            organization: None,
            targeting: None,
            window: None,
            number_fields: Vec::new(),
            key_findings: Vec::new(),
            drivers: Default::default(),
            step_drivers: Vec::new(),
            skipped_steps: Vec::new(),
            people: Vec::new(),
            process: None,
            timing: Default::default(),
        }
    }

    #[test]
    fn formats_exact_costs_without_floating_point_noise() {
        assert_eq!(cost_units(12_625_000), "12.625");
        assert_eq!(cost_units(-500_000), "-0.5");
        assert_eq!(cost_units(2_000_000), "2");
    }

    #[test]
    fn includes_the_headline_numbers() {
        let b = brief(&analysis());
        assert!(b.contains("1184"));
        assert!(b.contains("200"));
        assert!(b.contains("orders.csv"));
    }

    #[test]
    fn includes_bottlenecks_with_their_share() {
        let b = brief(&analysis());
        assert!(b.contains("| Rank | Transition |"));
        assert!(b.contains("Receive Order → Check Credit"));
        assert!(b.contains("80.6%"));
        assert!(b.contains("200 / 200"));
    }

    #[test]
    fn says_when_duration_statistics_have_no_eligible_cases() {
        let mut a = analysis();
        a.summary.duration_eligible_cases = 0;
        a.summary.incomplete_cases = a.summary.cases;
        a.variants[0].duration_eligible_cases = 0;
        a.bottlenecks.clear();
        for edge in &mut a.edges {
            edge.duration_eligible_occurrences = 0;
            edge.performance_rank = None;
        }

        let b = brief(&a);
        assert!(b.contains("unavailable (0 of 200 cases eligible)"));
        assert!(b.contains("none were eligible for duration analysis"));
        assert!(b.contains("unavailable | 0 / 60"));
    }

    /// Evidence must reach the assistant, or it cannot cite real cases.
    #[test]
    fn carries_example_case_identifiers() {
        let b = brief(&analysis());
        assert!(b.contains("ORDER-00006"));
    }

    /// The claim taxonomy must survive into the brief, or standard 4 stops at
    /// our own boundary.
    #[test]
    fn states_that_figures_are_calculated_facts() {
        let b = brief(&analysis());
        assert!(b.contains("calculated fact"));
    }

    #[test]
    fn instructs_the_assistant_to_label_its_own_claims() {
        let b = brief(&analysis());
        assert!(b.contains("interpretation"));
        assert!(b.contains("hypothesis"));
    }

    /// The DFG's limitation must be stated where the reader will see it.
    #[test]
    fn warns_that_the_graph_misrepresents_concurrency() {
        let b = brief(&analysis());
        assert!(b.contains("concurrently"));
        assert!(b.contains("directly-follows graph"));
    }

    #[test]
    fn forbids_claiming_causation() {
        let b = brief(&analysis());
        assert!(b.to_lowercase().contains("causation"));
    }

    /// The file must disclose its own privacy consequence, since we cannot
    /// control where the user pastes it.
    #[test]
    fn discloses_the_privacy_consequence() {
        let b = brief(&analysis());
        assert!(b.contains("Privacy note"));
        assert!(b.contains("discloses"));
    }

    #[test]
    fn reports_rework_when_present() {
        let b = brief(&analysis());
        assert!(b.contains("Rework and observed loops"));
        assert!(b.contains("Investigate"));
    }

    #[test]
    fn omits_the_rework_section_when_there_is_none() {
        let mut a = analysis();
        a.rework = ReworkSummary::default();
        assert!(!brief(&a).contains("Rework and observed loops"));
    }

    #[test]
    fn explains_an_unpivot_when_one_happened() {
        let mut a = analysis();
        a.unpivoted_from = Some(vec!["created_at → Created".to_owned()]);
        let b = brief(&a);
        assert!(b.contains("created_at → Created"));
        assert!(b.contains("one row per case"));
    }

    #[test]
    fn omits_the_unpivot_note_when_none_happened() {
        assert!(!brief(&analysis()).contains("one row per case"));
    }

    #[test]
    fn carries_advisory_notes_through() {
        let b = brief(&analysis());
        assert!(b.contains("Time zone policy not set"));
    }

    #[test]
    fn lists_activities_as_recorded() {
        let b = brief(&analysis());
        assert!(b.contains("Activities as recorded"));
        assert!(b.contains("`Check Credit`"));
    }

    /// The agent needs the rule vocabulary, or it cannot propose anything the
    /// engine can actually apply.
    #[test]
    fn explains_how_to_propose_preparation_rules() {
        let b = brief(&analysis());
        assert!(b.contains("Proposing data preparation"));
        assert!(b.contains("exclude_activities"));
        assert!(b.contains("--rules rules.json"));
    }

    /// And it must be warned off the merge that looks obvious but is not.
    #[test]
    fn warns_against_merging_possibly_distinct_steps() {
        let b = brief(&analysis());
        assert!(b.contains("Cancelled"));
        assert!(b.contains("genuinely different"));
    }

    #[test]
    fn reports_handovers_and_warns_against_judging_people() {
        let mut a = analysis();
        a.organization = Some(crate::analysis::OrganizationSummary {
            resources: 2,
            fragmentation_percent: 50.0,
            handovers: vec![crate::analysis::HandoverSummary {
                from: "alice".to_owned(),
                to: "bob".to_owned(),
                occurrences: 2,
                cases: 2,
                duration_eligible_occurrences: 2,
                mean_seconds: 129_600,
                total_seconds: 259_200,
                share_percent: 100.0,
                example_cases: vec!["ORDER-00001".to_owned()],
            }],
        });
        let b = brief(&a);
        assert!(b.contains("Who passes work to whom"));
        assert!(b.contains("alice"));
        assert!(b.contains("not to a person"));
    }

    #[test]
    fn omits_handovers_when_no_resource_was_mapped() {
        assert!(!brief(&analysis()).contains("Who passes work to whom"));
    }

    /// An assistant must never be handed a filtered view without being told.
    #[test]
    fn declares_a_filtered_view_prominently() {
        let mut a = analysis();
        a.view.narrowed = true;
        a.view.cases_shown = 24;
        a.view.percent_shown = 12.0;
        let b = brief(&a);
        assert!(b.contains("filtered view"));
        assert!(b.contains("12%"));
        assert!(b.contains("not the whole process"));
    }

    #[test]
    fn says_nothing_about_filtering_when_nothing_was_filtered() {
        assert!(!brief(&analysis()).contains("filtered view"));
    }

    fn targeting() -> crate::targeting::AutomationReport {
        crate::targeting::AutomationReport {
            source: "orders.csv".to_owned(),
            straight_through_percent: Some(40.0),
            straight_through_cases: 2,
            touched_cases: 3,
            undetermined_cases: 0,
            classification_coverage_percent: 100.0,
            manual_events: 3,
            system_events: 15,
            unrecorded_events: 0,
            manual_processing_seconds: 270_000,
            waiting_before_manual_seconds: 180_000,
            start_to_start_before_manual_seconds: 0,
            ranking_basis: pm_core::RankingBasis::ProcessingTime,
            manual_steps_ranked: vec![crate::targeting::ManualStep {
                activity: "Fix Tax Code".to_owned(),
                occurrences: 2,
                cases: 2,
                processing_seconds: 270_000,
                processing_observations: 2,
                waiting_before_seconds: 180_000,
                waiting_observations: 2,
                start_to_start_before_seconds: 0,
                start_to_start_observations: 0,
                total_cost_microunits: 0,
                routing_context_concentration: 100.0,
                distinct_contexts: 1,
                routing_contexts: vec![crate::targeting::RoutingContext {
                    before: Some("Check Credit".to_owned()),
                    after: Some("Post Invoice".to_owned()),
                    occurrences: 2,
                    share_percent: 100.0,
                }],
                example_cases: vec!["ORD-3".to_owned()],
            }],
            divergences: vec![crate::targeting::Divergence {
                after: Some("Check Credit".to_owned()),
                dominant_path_next: Some("Post Invoice".to_owned()),
                took: Some("Fix Tax Code".to_owned()),
                cases: 2,
                example_cases: vec!["ORD-3".to_owned()],
            }],
            dominant_variant_cases: 2,
            dominant_variant_percent: 40.0,
            unclassified_resources: Vec::new(),
            unclassified_events: 0,
            suggested_system_resources: Vec::new(),
            resources_mapped: true,
            interpretation_limits: vec![
                "Routing-context concentration does not establish that task inputs or decisions are rules-based."
                    .to_owned(),
                "Automation suitability requires human validation.".to_owned(),
            ],
        }
    }

    #[test]
    fn reports_where_the_machine_stops() {
        let mut a = analysis();
        a.targeting = Some(targeting());
        let b = brief(&a);
        assert!(b.contains("Where the machine stops"));
        assert!(b.contains("40% ran straight through"));
        assert!(b.contains("Fix Tax Code"));
    }

    /// The routing metric must be explained, or an assistant will read a
    /// sort order as a recommendation.
    #[test]
    fn explains_routing_concentration_and_disclaims_the_ordering() {
        let mut a = analysis();
        a.targeting = Some(targeting());
        let b = brief(&a);
        assert!(b.contains("investigation order, not an automation recommendation"));
        assert!(b.contains("does not show whether task inputs or decisions are rules-based"));
    }

    /// The boundary must travel with the numbers.
    #[test]
    fn states_that_it_cannot_say_how_the_work_is_done() {
        let mut a = analysis();
        a.targeting = Some(targeting());
        let b = brief(&a);
        assert!(b.contains("Automation suitability requires human validation"));
    }

    /// An incomplete classification is the margin of error on every figure, so
    /// it is stated rather than buried.
    #[test]
    fn declares_an_incomplete_resource_classification() {
        let mut a = analysis();
        let mut t = targeting();
        t.unclassified_resources = vec!["EDI_GATEWAY".to_owned()];
        a.targeting = Some(t);
        let b = brief(&a);
        assert!(b.contains("EDI_GATEWAY"));
        assert!(b.contains("Not yet classified"));
    }

    #[test]
    fn says_not_yet_classified_and_suggests_system_accounts() {
        let mut a = analysis();
        let mut t = targeting();
        t.unclassified_resources = vec!["batch_00".to_owned(), "user_001".to_owned()];
        t.unclassified_events = 3;
        t.suggested_system_resources = vec!["batch_00".to_owned()];
        a.targeting = Some(t);
        let b = brief(&a);
        assert!(b.contains("Not yet classified"));
        assert!(b.contains("look like system accounts; ask the person to confirm: batch_00."));
        assert!(
            !b.contains("ran straight through"),
            "no rate before the split is known"
        );
    }

    #[test]
    fn omits_targeting_when_there_is_none() {
        assert!(!brief(&analysis()).contains("Where the machine stops"));
    }

    #[test]
    fn is_deterministic() {
        let a = analysis();
        assert_eq!(brief(&a), brief(&a));
    }
}
