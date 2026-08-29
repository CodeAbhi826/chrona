//! Chrona core: window-event domain model, categorisation rules and the
//! usage-statistics engine. This crate is pure computation — no I/O, no
//! system integration — which keeps it fully unit-testable.

pub mod model;
pub mod rules;
pub mod stats;

pub use model::*;
pub use rules::{default_rules, Field, Rule, RuleSet};
