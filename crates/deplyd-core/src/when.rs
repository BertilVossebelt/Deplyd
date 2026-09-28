//! Turning GitHub's timestamps into the one deplyd prints.
//!
//! Every date on screen has to read in the same zone, or a column of them
//! cannot be compared. Git is asked for its dates with `--date=format-local`;
//! these arrive from the API as UTC, so they are converted here rather than
//! shown as they came.

use chrono::{DateTime, Local};

/// `2026-09-28T19:52:40Z` as `2026-09-28 21:52`, in the reader's zone.
/// `None` for anything that will not parse: a missing date beats a wrong one.
pub fn local_minute(timestamp: &str) -> Option<String> {
    let parsed = DateTime::parse_from_rfc3339(timestamp.trim()).ok()?;
    Some(
        parsed
            .with_timezone(&Local)
            .format("%Y-%m-%d %H:%M")
            .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_utc_stamp_becomes_a_local_one() {
        // Whatever this machine's zone is, the same instant written two ways has
        // to come out the same - which is the whole point of converting.
        let utc = local_minute("2026-09-28T19:52:40Z").expect("parses");
        let offset = local_minute("2026-09-28T21:52:40+02:00").expect("parses");
        assert_eq!(utc, offset);
        assert_eq!(utc.len(), 16, "YYYY-MM-DD HH:MM, got {utc:?}");
    }

    #[test]
    fn anything_unparseable_is_nothing_rather_than_a_guess() {
        assert_eq!(local_minute(""), None);
        assert_eq!(local_minute("yesterday"), None);
        assert_eq!(local_minute("2026-09-28"), None);
    }
}
