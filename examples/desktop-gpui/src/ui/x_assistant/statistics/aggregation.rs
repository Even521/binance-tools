//! Deduplicate parsed events and aggregate one day's reply statistics.
//!
//! Responsibilities:
//! - Group multiple log lines belonging to the same automation round.
//! - Classify each round as success, explicit failure, uncertain, or skipped.
//! - Attribute at most one primary failure stage to each failed round.
//! - Calculate stage counts, hourly failure counts, and durations from
//!   explicit start/end timestamps.
//! - Keep uncertain and skipped rounds out of the explicit failure-rate
//!   denominator.
