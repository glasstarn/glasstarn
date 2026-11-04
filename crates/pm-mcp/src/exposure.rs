//! Explicit control over which identifiers may cross the AI boundary.

use hmac::{Hmac, Mac};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::Write;
use std::path::Path;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Show,
    Alias,
    Omit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct ExposurePolicy {
    pub project_names: Mode,
    pub source_names: Mode,
    pub source_metadata: Mode,
    pub column_names: Mode,
    pub activity_names: Mode,
    pub event_ids: Mode,
    pub object_type_names: Mode,
    pub object_ids: Mode,
    pub relationship_qualifiers: Mode,
    pub resource_names: Mode,
    pub case_ids: Mode,
    pub attribute_names: Mode,
    pub attribute_values: Mode,
    pub timestamps: Mode,
    pub costs: Mode,
    pub source_rows: Mode,
    /// Secret used only to make aliases stable and resistant to guessing.
    /// It is never included in an MCP result.
    pub alias_key: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ValuePreview {
    pub local_value: String,
    pub outbound_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FieldPreview {
    pub role: String,
    pub local_column: String,
    pub outbound_column: Option<String>,
    pub mode: Mode,
    pub examples: Vec<ValuePreview>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ExposurePreview {
    pub project_name: ValuePreview,
    pub source_file: ValuePreview,
    pub policy: serde_json::Value,
    pub fields: Vec<FieldPreview>,
    pub note: String,
}

impl Default for ExposurePolicy {
    fn default() -> Self {
        Self::safe()
    }
}

impl ExposurePolicy {
    /// Maximum-useful privacy default: structural names are pseudonymized and
    /// values unnecessary for aggregate analysis are withheld.
    pub fn safe() -> Self {
        let mut key = [0u8; 32];
        OsRng.fill_bytes(&mut key);
        Self {
            project_names: Mode::Alias,
            source_names: Mode::Omit,
            source_metadata: Mode::Omit,
            column_names: Mode::Alias,
            activity_names: Mode::Alias,
            event_ids: Mode::Alias,
            object_type_names: Mode::Alias,
            object_ids: Mode::Alias,
            relationship_qualifiers: Mode::Alias,
            resource_names: Mode::Alias,
            case_ids: Mode::Alias,
            attribute_names: Mode::Alias,
            attribute_values: Mode::Omit,
            timestamps: Mode::Omit,
            costs: Mode::Show,
            source_rows: Mode::Omit,
            alias_key: hex(&key),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref();
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("could not read {}: {error}", path.display()))?;
        let policy: Self = serde_json::from_str(&text).map_err(|error| {
            format!("{} is not a valid exposure policy: {error}", path.display())
        })?;
        policy.validate()?;
        Ok(policy)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), String> {
        let path = path.as_ref();
        self.validate()?;
        let text = serde_json::to_string_pretty(self).map_err(|error| error.to_string())?;
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(path).map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                path.display()
            )
        })?;
        file.write_all(format!("{text}\n").as_bytes())
            .map_err(|error| format!("could not write {}: {error}", path.display()))
    }

    pub fn validate(&self) -> Result<(), String> {
        let bytes = decode_hex(&self.alias_key)
            .ok_or("alias_key must contain exactly 64 hexadecimal characters")?;
        if bytes.len() != 32 {
            return Err("alias_key must contain exactly 64 hexadecimal characters".to_owned());
        }
        Ok(())
    }

    /// The policy description safe to return through MCP. The alias key is
    /// intentionally absent.
    pub fn public_summary(&self) -> serde_json::Value {
        serde_json::json!({
            "project_names": self.project_names,
            "source_names": self.source_names,
            "source_metadata": self.source_metadata,
            "column_names": self.column_names,
            "activity_names": self.activity_names,
            "event_ids": self.event_ids,
            "object_type_names": self.object_type_names,
            "object_ids": self.object_ids,
            "relationship_qualifiers": self.relationship_qualifiers,
            "resource_names": self.resource_names,
            "case_ids": self.case_ids,
            "attribute_names": self.attribute_names,
            "attribute_values": self.attribute_values,
            "timestamps": self.timestamps,
            "costs": self.costs,
            "source_rows": self.source_rows,
        })
    }

    /// Derives a separate bearer credential for an explicitly enabled live
    /// application. The alias key itself never crosses the process boundary,
    /// and this credential is not accepted by ordinary browser endpoints.
    pub fn delegation_token(&self) -> String {
        let key = decode_hex(&self.alias_key).expect("validated policy key");
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key size");
        mac.update(b"glasstarn\0live-delegation\0v1");
        hex(&mac.finalize().into_bytes())
    }

    /// A project credential cannot be used against another project session,
    /// even if the operating system later reuses a local port.
    pub fn project_delegation_token(&self, project_id: &str) -> String {
        let key = decode_hex(&self.alias_key).expect("validated policy key");
        let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key size");
        mac.update(b"glasstarn\0project-delegation\0v1\0");
        mac.update(project_id.as_bytes());
        hex(&mac.finalize().into_bytes())
    }

    pub fn transform(&self, mode: Mode, category: &str, value: &str) -> Option<String> {
        match mode {
            Mode::Show => Some(value.to_owned()),
            Mode::Omit => None,
            Mode::Alias => {
                let key = decode_hex(&self.alias_key).expect("validated policy key");
                let mut mac = HmacSha256::new_from_slice(&key).expect("HMAC accepts any key size");
                mac.update(category.as_bytes());
                mac.update(&[0]);
                mac.update(value.as_bytes());
                let digest = mac.finalize().into_bytes();
                Some(format!("{category}_{}", &hex(&digest)[..12]))
            }
        }
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0xf) as usize] as char);
    }
    output
}

fn decode_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    text.as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| {
            let high = (pair[0] as char).to_digit(16)?;
            let low = (pair[1] as char).to_digit(16)?;
            Some(((high << 4) | low) as u8)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static ID: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn safe_policy_never_shows_identifiers() {
        let policy = ExposurePolicy::safe();
        assert_ne!(
            policy.transform(policy.case_ids, "case", "ORDER-1042"),
            Some("ORDER-1042".to_owned())
        );
        assert_eq!(
            policy.transform(policy.source_names, "source", "payroll.csv"),
            None
        );
    }

    #[test]
    fn keyed_aliases_are_stable_and_category_separated() {
        let policy = ExposurePolicy::safe();
        let one = policy.transform(Mode::Alias, "case", "A");
        assert_eq!(one, policy.transform(Mode::Alias, "case", "A"));
        assert_ne!(one, policy.transform(Mode::Alias, "activity", "A"));
    }

    #[test]
    fn public_summary_never_contains_the_key() {
        let policy = ExposurePolicy::safe();
        assert!(
            !policy
                .public_summary()
                .to_string()
                .contains(&policy.alias_key)
        );
    }

    #[test]
    fn delegation_credential_is_stable_separate_and_does_not_reveal_the_key() {
        let policy = ExposurePolicy::safe();
        let token = policy.delegation_token();
        assert_eq!(token, policy.delegation_token());
        assert_eq!(token.len(), 64);
        assert_ne!(token, policy.alias_key);
        assert!(!token.contains(&policy.alias_key));

        let other = ExposurePolicy::safe();
        assert_ne!(token, other.delegation_token());
    }

    #[test]
    fn policy_file_round_trips_and_is_never_overwritten() {
        let path = std::env::temp_dir().join(format!(
            "pm_exposure_policy_{}_{}.json",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        let policy = ExposurePolicy::safe();
        policy.save(&path).unwrap();
        assert_eq!(ExposurePolicy::load(&path).unwrap(), policy);
        assert!(policy.save(&path).is_err());
    }
}
