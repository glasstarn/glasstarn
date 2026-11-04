//! Reading case attributes as numbers.
//!
//! Case attributes are stored as text, because a source file does not say
//! which of its columns hold amounts. A value counts as a number when it reads
//! as one without guessing: digits with an optional sign, decimal point and
//! exponent, an optional leading currency symbol, and comma thousands
//! separators only in their exact positions ("1,234,567.89"). Anything else,
//! such as "12,5", "N/A" or "1.2.3", is not a number and is counted as such
//! rather than coerced.

use crate::log::{CaseTrace, EventLog};
use serde::{Deserialize, Serialize};

/// Reads `text` as a number, or `None` when it is not unambiguously one.
pub fn parse_number(text: &str) -> Option<f64> {
    let text = text.trim();
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", text.strip_prefix('+').unwrap_or(text)),
    };
    let rest = rest
        .strip_prefix(['$', '€', '£', '¥'])
        .unwrap_or(rest)
        .trim_start();
    if rest.is_empty()
        || !rest.starts_with(|c: char| c.is_ascii_digit() || c == '.')
        || !rest
            .chars()
            .all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'e' | 'E' | '+' | '-'))
    {
        return None;
    }
    let plain = if rest.contains(',') {
        let (whole, fraction) = rest.split_once('.').unwrap_or((rest, ""));
        let mut groups = whole.split(',');
        let first = groups.next()?;
        let grouped = (1..=3).contains(&first.len())
            && first.chars().all(|c| c.is_ascii_digit())
            && groups.all(|group| group.len() == 3 && group.chars().all(|c| c.is_ascii_digit()))
            && fraction.chars().all(|c| c.is_ascii_digit());
        if !grouped {
            return None;
        }
        rest.replace(',', "")
    } else {
        rest.to_owned()
    };
    format!("{sign}{plain}")
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

/// The number a case holds for the attribute at `index`, if any. When the
/// number changes between steps (an amount revised), the latest one: each
/// case still counts once, at its current amount.
pub fn case_number(log: &EventLog, trace: &CaseTrace, index: usize) -> Option<f64> {
    let value = log.latest_value(trace, index)?;
    parse_number(log.attribute_values.label(value)?)
}

/// Keeps cases whose number for `attribute` lies within the bounds, both
/// inclusive. A case with no value, or a value that is not a number, never
/// matches: an amount that is not recorded is not "below 50,000".
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NumberRange {
    pub attribute: String,
    #[serde(default)]
    pub at_least: Option<f64>,
    #[serde(default)]
    pub at_most: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub greater_than: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub less_than: Option<f64>,
}

// Bounds come from JSON or from checked command-line input, neither of which
// can produce NaN, so equality is reflexive.
impl Eq for NumberRange {}

impl NumberRange {
    pub fn contains(&self, value: f64) -> bool {
        self.at_least.is_none_or(|low| value >= low)
            && self.at_most.is_none_or(|high| value <= high)
            && self.greater_than.is_none_or(|low| value > low)
            && self.less_than.is_none_or(|high| value < high)
    }
}

/// How one case attribute reads as numbers across the cases of a log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NumberField {
    pub name: String,
    /// Cases whose value reads as a number.
    pub numbers: usize,
    /// Cases with a value that does not read as a number.
    pub not_numbers: usize,
    /// Cases with no value.
    pub missing: usize,
}

/// The case attributes that hold numbers: at least one case has a number and
/// at least 99% of the recorded values read as numbers. The exceptions are
/// counted, never hidden.
pub fn number_fields(log: &EventLog) -> Vec<NumberField> {
    log.attribute_names
        .iter()
        .enumerate()
        .filter_map(|(index, name)| {
            let mut field = NumberField {
                name: name.clone(),
                numbers: 0,
                not_numbers: 0,
                missing: 0,
            };
            // Values are interned, so each distinct one is parsed once.
            let mut parsed = std::collections::HashMap::new();
            for trace in &log.traces {
                match trace.attribute(index) {
                    None => field.missing += 1,
                    Some(value) => {
                        let number = *parsed.entry(value).or_insert_with(|| {
                            log.attribute_values
                                .label(value)
                                .and_then(parse_number)
                                .is_some()
                        });
                        if number {
                            field.numbers += 1;
                        } else {
                            field.not_numbers += 1;
                        }
                    }
                }
            }
            (field.numbers > 0
                && field.not_numbers * 100 <= field.numbers + field.not_numbers
                && !names_an_identifier(name))
            .then_some(field)
        })
        .collect()
}

/// Whether a field's name says it holds identifiers, such as "Purchasing
/// Document" or "Order ID": numbers, but not amounts to add up.
fn names_an_identifier(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.split(|c: char| !c.is_alphanumeric()).any(|word| {
        matches!(
            word,
            "document" | "doc" | "id" | "number" | "no" | "nr" | "key" | "code"
        )
    })
}

/// The number field most likely to be money: one whose name says so, else
/// the one with the most numbers. Ties go to the alphabetically first name.
pub fn main_number_field(fields: &[NumberField]) -> Option<&NumberField> {
    const MONEY: [&str; 9] = [
        "amount", "value", "price", "total", "gross", "revenue", "cost", "betrag", "wert",
    ];
    let money = |field: &&NumberField| {
        let name = field.name.to_lowercase();
        MONEY.iter().any(|word| name.contains(word))
    };
    fn best<'a>(pool: impl Iterator<Item = &'a NumberField>) -> Option<&'a NumberField> {
        pool.max_by(|a, b| a.numbers.cmp(&b.numbers).then(b.name.cmp(&a.name)))
    }
    best(fields.iter().filter(money)).or_else(|| best(fields.iter()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifiers_are_not_amounts() {
        assert!(names_an_identifier("Purchasing Document"));
        assert!(names_an_identifier("Order ID"));
        assert!(!names_an_identifier("Net value"));
        assert!(!names_an_identifier("Cumulative net worth (EUR)"));
    }

    #[test]
    fn plain_and_formatted_amounts_read_as_numbers() {
        for (text, number) in [
            ("50000", 50_000.0),
            (" 12.5 ", 12.5),
            ("-3", -3.0),
            ("+4", 4.0),
            ("$1,234,567.89", 1_234_567.89),
            ("€ 99", 99.0),
            ("-£2,000", -2_000.0),
            (".5", 0.5),
            ("1e3", 1_000.0),
        ] {
            assert_eq!(parse_number(text), Some(number), "{text}");
        }
    }

    #[test]
    fn ambiguous_or_textual_values_are_not_numbers() {
        for text in [
            "", "N/A", "12,5", "1,23", "1.2.3", "inf", "NaN", "abc1", "1,2345", "--1", "$",
        ] {
            assert_eq!(parse_number(text), None, "{text}");
        }
    }

    #[test]
    fn the_money_field_is_chosen_by_name_first() {
        let field = |name: &str, numbers| NumberField {
            name: name.into(),
            numbers,
            not_numbers: 0,
            missing: 0,
        };
        let fields = [field("customer_no", 90), field("order_value", 40)];
        assert_eq!(main_number_field(&fields).unwrap().name, "order_value");
        let fields = [field("a", 3), field("b", 9)];
        assert_eq!(main_number_field(&fields).unwrap().name, "b");
        assert!(main_number_field(&[]).is_none());
    }

    #[test]
    fn ranges_include_their_bounds() {
        let range = NumberRange {
            attribute: "amount".into(),
            at_least: Some(50_000.0),
            at_most: None,
            ..Default::default()
        };
        assert!(range.contains(50_000.0) && range.contains(80_000.0));
        assert!(!range.contains(49_999.99));
    }
}
