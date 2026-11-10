//! Compact GPUI rendering for the daily statistics dashboard.
//!
//! Layout target: fit the existing X Assistant page's usable viewport without
//! vertical scrolling.
//!
//! Planned composition:
//! 1. A compact header with the Beijing-time date.
//! 2. One short row of four totals: rounds, success, failure, uncertain.
//! 3. Two small charts side by side: outcome distribution and failures by
//!    automation stage.
//! 4. One short-width chart along the bottom: hourly failure trend.
//!
//! Keep chart labels concise, minimize internal padding, and do not add
//! separate tabs or a second page. This module will reuse the project's
//! existing GPUI/chart dependencies rather than introducing a new UI stack.
