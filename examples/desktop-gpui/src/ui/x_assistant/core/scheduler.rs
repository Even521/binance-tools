//! Single-threaded X assistant task scheduling primitives.
//!
//! This module deliberately does not spawn workers or touch the browser.
//! The existing X assistant main loop owns the browser and calls these
//! selectors between safe task boundaries.

use std::time::Duration;

/// Evenly spaces a configured number of posts across a rolling hour.
/// Returns the remaining wait from the age of the most recent published post.
pub fn next_post_delay(
    posts_per_hour: u8,
    seconds_since_last_post: Option<u64>,
) -> Duration {
    let count = posts_per_hour.clamp(1, 10) as u64;
    let interval = 3_600_u64 / count;
    let Some(age) = seconds_since_last_post else {
        return Duration::ZERO;
    };
    Duration::from_secs(interval.saturating_sub(age))
}

#[cfg(test)]
mod tests {
    use super::next_post_delay;
    use std::time::Duration;

    #[test]
    fn spreads_posts_over_the_hour() {
        assert_eq!(next_post_delay(3, None), Duration::ZERO);
        assert_eq!(next_post_delay(3, Some(500)), Duration::from_secs(700));
        assert_eq!(next_post_delay(3, Some(1_500)), Duration::ZERO);
    }
}
