//! Generates event logs from known process models.
//!
//! This crate exists to solve the **oracle problem**: a test whose expected
//! output was produced by the code under test proves nothing. By starting from
//! a model and generating the log, the correct answer is known by
//! construction.
//!
//! Test-support and benchmark tooling only — not part of the shipped engine.

pub mod concur;
mod csv;
pub mod dynamics;
pub mod hubspot;
pub mod jde;
pub mod jira;
pub mod log;
pub mod model;
pub mod netsuite;
pub mod oracle;
pub mod peoplesoft;
pub mod rng;
pub mod salesforce;
pub mod sap_business_one;
pub mod sap_bydesign;
pub mod sap_o2c;
pub mod sap_operations;
pub mod sap_p2p;
pub mod servicenow;
pub mod successfactors;
pub mod workflow_platforms;

pub use log::{Event, LogBuilder, Wait};
pub use model::Model;
pub use rng::Rng;
