//! The headline of an analysis: what an agent needs first, in a few hundred
//! bytes. The full analysis (every step, link, route and timing bin) can run
//! to tens of kilobytes; an agent asks for it, or for one part, only when the
//! question needs it. Every figure is copied from the analysis, never
//! recomputed.

use crate::analysis::Analysis;
use crate::values::ValueStats;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadlineSummary {
    pub cases: usize,
    pub events: usize,
    pub activities: usize,
    pub variants: usize,
    pub top_five_coverage_percent: f64,
    pub median_throughput_seconds: i64,
    pub p90_throughput_seconds: i64,
    pub duration_eligible_cases: usize,
    pub incomplete_cases: usize,
    pub rework_case_percent: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadlineRoute {
    pub steps: Vec<String>,
    pub cases: usize,
    pub percent: f64,
    pub mean_throughput_seconds: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadlineHandover {
    pub from: String,
    pub to: String,
    pub mean_seconds: i64,
    /// Share of all waiting time spent on this hand-over.
    pub share_percent: f64,
    pub occurrences: u64,
}

/// Totals of the field most likely to be money.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeadlineValue {
    pub field: String,
    pub stats: ValueStats,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Headline {
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub history_semantics: Option<String>,
    pub summary: HeadlineSummary,
    /// The most common routes, most common first.
    pub routes: Vec<HeadlineRoute>,
    /// The hand-overs with the most waiting time, most first.
    pub slowest_handovers: Vec<HeadlineHandover>,
    /// The top calculated findings, with items called cases.
    pub findings: Vec<crate::findings::KeyFinding>,
    pub number_fields: Vec<String>,
    pub value: Option<HeadlineValue>,
    /// Where the cases sit against the extract window, when there is one.
    pub window: Option<pm_core::WindowSummary>,
    /// How much was left out, so the reader knows to ask.
    pub routes_available: usize,
    pub handovers_available: usize,
}

/// The headline of `analysis`, with at most `limit` routes and hand-overs.
/// Long routes keep their first and last steps and say how many lie between.
pub fn headline(analysis: &Analysis, value: Option<HeadlineValue>, limit: usize) -> Headline {
    let summary = &analysis.summary;
    Headline {
        history_semantics: analysis
            .mapping
            .object_history
            .then(|| crate::OBJECT_HISTORY_SEMANTICS.to_owned()),
        // The file name, not its folder: the path says nothing about the process.
        source: analysis
            .source
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or_default()
            .to_owned(),
        summary: HeadlineSummary {
            cases: summary.cases,
            events: summary.events,
            activities: summary.activities,
            variants: summary.variants,
            top_five_coverage_percent: summary.top_five_coverage_percent,
            median_throughput_seconds: summary.median_throughput_seconds,
            p90_throughput_seconds: summary.p90_throughput_seconds,
            duration_eligible_cases: summary.duration_eligible_cases,
            incomplete_cases: summary.incomplete_cases,
            rework_case_percent: analysis.rework.case_percent,
        },
        routes: analysis
            .variants
            .iter()
            .take(limit)
            .map(|variant| HeadlineRoute {
                steps: shorten(&variant.sequence),
                cases: variant.cases,
                percent: variant.percent,
                mean_throughput_seconds: variant.mean_throughput_seconds,
            })
            .collect(),
        slowest_handovers: analysis
            .bottlenecks
            .iter()
            .take(limit)
            .map(|bottleneck| HeadlineHandover {
                from: bottleneck.from.clone(),
                to: bottleneck.to.clone(),
                mean_seconds: bottleneck.mean_seconds,
                share_percent: bottleneck.share_percent,
                occurrences: bottleneck.total_occurrences,
            })
            .collect(),
        // The agent reads these as they are: name the items plainly.
        // Every finding, short: what was found, how big and the filter for
        // its items, without the evidence fields the full analysis keeps.
        findings: analysis
            .key_findings
            .iter()
            .take(12)
            .cloned()
            .map(|mut finding| {
                finding.value = None;
                finding.target = None;
                finding.tags.clear();
                for text in [&mut finding.title, &mut finding.detail, &mut finding.impact] {
                    *text = crate::findings::render(text, "case", "cases");
                }
                finding
            })
            .collect(),
        number_fields: analysis
            .number_fields
            .iter()
            .map(|field| field.name.clone())
            .collect(),
        value,
        window: analysis.window.clone(),
        routes_available: analysis.variants.len(),
        handovers_available: analysis.bottlenecks.len(),
    }
}

/// Routes longer than eight steps keep the first four and last three.
fn shorten(steps: &[String]) -> Vec<String> {
    if steps.len() <= 8 {
        return steps.to_vec();
    }
    let mut short = steps[..4].to_vec();
    short.push(format!("… {} more steps …", steps.len() - 7));
    short.extend_from_slice(&steps[steps.len() - 3..]);
    short
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_routes_keep_their_ends() {
        let steps = (1..=12).map(|n| format!("s{n}")).collect::<Vec<_>>();
        assert_eq!(
            shorten(&steps),
            [
                "s1",
                "s2",
                "s3",
                "s4",
                "… 5 more steps …",
                "s10",
                "s11",
                "s12"
            ]
        );
        assert_eq!(shorten(&steps[..8]), steps[..8].to_vec());
    }
}
