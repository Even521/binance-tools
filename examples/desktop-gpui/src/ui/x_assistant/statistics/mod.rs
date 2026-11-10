//! Daily automatic-reply statistics for the X Assistant UI.
//!
//! This module is intentionally not wired into the parent page yet. Its
//! boundaries are separated so log parsing, aggregation, and GPUI rendering
//! can be reviewed independently before the existing UI is changed.

mod aggregation;
mod log_parser;
mod model;
mod render;
