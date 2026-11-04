//! Dictionary encoding of case and activity labels into compact identifiers.
//!
//! One million rows means one million *events*, not one million *cases*.
//! Labels are interned once; everything downstream works on `u32` identifiers,
//! which keeps sequences compact and comparisons cheap.

use std::fmt;
use std::hash::BuildHasher;

/// A dictionary-encoded activity label.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct ActivityId(pub u32);

/// A dictionary-encoded case identifier.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
pub struct CaseId(pub u32);

impl fmt::Display for ActivityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "a{}", self.0)
    }
}

impl fmt::Display for CaseId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

/// Interns string labels, assigning each a compact identifier.
///
/// Identifiers are assigned in **first-seen order**, which makes encoding
/// deterministic for a given input ordering. Lookups by identifier are O(1)
/// index operations; the map is only consulted when interning.
///
/// Every label is stored once, in one shared text buffer; the lookup table
/// holds only identifiers. A large connected log has millions of item and
/// step identifiers, and keeping each as two separate strings (one for the
/// list, one for the map) used most of the analysis's memory.
#[derive(Clone, Default)]
pub struct Dictionary {
    text: String,
    /// End offset of each label in `text`, in identifier order.
    ends: Vec<usize>,
    index: hashbrown::HashTable<u32>,
    hasher: std::hash::RandomState,
}

impl fmt::Debug for Dictionary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list().entries(self.iter()).finish()
    }
}

impl PartialEq for Dictionary {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.ends == other.ends
    }
}

impl Eq for Dictionary {}

fn label_at<'a>(text: &'a str, ends: &[usize], id: u32) -> &'a str {
    let id = id as usize;
    let start = if id == 0 { 0 } else { ends[id - 1] };
    &text[start..ends[id]]
}

impl Dictionary {
    /// Creates an empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty dictionary with space reserved for `capacity` labels.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            ends: Vec::with_capacity(capacity),
            index: hashbrown::HashTable::with_capacity(capacity),
            ..Self::default()
        }
    }

    /// Interns `label`, returning its identifier.
    ///
    /// Returns the existing identifier if the label has been seen before.
    /// Labels are matched exactly — normalization (trimming, case folding) is
    /// a validation concern and must happen before interning, so that
    /// `"Approve"` and `"approve "` are reported as a data-quality finding
    /// rather than silently merged.
    pub fn intern(&mut self, label: &str) -> u32 {
        let hash = self.hasher.hash_one(label);
        let Self {
            text,
            ends,
            index,
            hasher,
        } = self;
        if let Some(&id) = index.find(hash, |&id| label_at(text, ends, id) == label) {
            return id;
        }
        let id = u32::try_from(ends.len()).expect("dictionary exceeded u32::MAX distinct labels");
        text.push_str(label);
        ends.push(text.len());
        index.insert_unique(hash, id, |&id| hasher.hash_one(label_at(text, ends, id)));
        id
    }

    /// Returns the identifier for `label`, if it has been interned.
    pub fn get(&self, label: &str) -> Option<u32> {
        let hash = self.hasher.hash_one(label);
        self.index
            .find(hash, |&id| label_at(&self.text, &self.ends, id) == label)
            .copied()
    }

    /// Returns the label for `id`, if it exists.
    pub fn label(&self, id: u32) -> Option<&str> {
        ((id as usize) < self.ends.len()).then(|| label_at(&self.text, &self.ends, id))
    }

    /// Returns the number of distinct labels interned.
    pub fn len(&self) -> usize {
        self.ends.len()
    }

    /// Returns `true` if no labels have been interned.
    pub fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// All labels in identifier order.
    pub fn iter(&self) -> impl Iterator<Item = &str> {
        (0..self.ends.len() as u32).map(|id| label_at(&self.text, &self.ends, id))
    }

    /// Returns all labels in identifier order.
    pub fn labels(&self) -> Vec<&str> {
        self.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assigns_ids_in_first_seen_order() {
        let mut d = Dictionary::new();
        assert_eq!(d.intern("Receive Order"), 0);
        assert_eq!(d.intern("Check Credit"), 1);
        assert_eq!(d.intern("Approve"), 2);
        assert_eq!(d.len(), 3);
    }

    #[test]
    fn repeated_label_returns_same_id() {
        let mut d = Dictionary::new();
        let first = d.intern("Approve");
        let second = d.intern("Approve");
        assert_eq!(first, second);
        assert_eq!(d.len(), 1);
    }

    #[test]
    fn round_trips_label_and_id() {
        let mut d = Dictionary::new();
        let id = d.intern("Ship");
        assert_eq!(d.label(id), Some("Ship"));
        assert_eq!(d.get("Ship"), Some(id));
    }

    #[test]
    fn unknown_lookups_return_none() {
        let d = Dictionary::new();
        assert_eq!(d.get("Missing"), None);
        assert_eq!(d.label(0), None);
    }

    /// Labels differing only by whitespace or case are distinct here by
    /// design; merging them is a validation decision, not an encoding one.
    #[test]
    fn does_not_normalize_labels() {
        let mut d = Dictionary::new();
        let a = d.intern("Approve");
        let b = d.intern("approve");
        let c = d.intern("Approve ");
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(d.len(), 3);
    }

    /// Encoding the same label stream twice must produce identical output.
    #[test]
    fn encoding_is_deterministic() {
        let stream = ["A", "B", "A", "C", "B", "A"];
        let encode = || {
            let mut d = Dictionary::new();
            let ids: Vec<u32> = stream.iter().map(|s| d.intern(s)).collect();
            (
                ids,
                d.labels().iter().map(|l| l.to_string()).collect::<Vec<_>>(),
            )
        };
        assert_eq!(encode(), encode());
    }
}
