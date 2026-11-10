//! Read-only parser for the selected day's X Assistant log.
//!
//! Responsibilities:
//! - Resolve the daily x-assistant log using the existing app-log naming
//!   helper and Beijing-time date.
//! - Parse structured automation start, stage, and terminal outcome events.
//! - Preserve timestamps and round identifiers when available.
//! - Never create, truncate, rotate, or modify log files.
//!
//! Parsing rules and event patterns will be implemented after reviewing the
//! actual current log format. Avoid inferring a successful send from a click
//! alone; require an explicit success confirmation event.
