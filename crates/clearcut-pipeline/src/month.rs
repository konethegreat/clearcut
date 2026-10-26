// Written by Kone Tshivhinda & Claude
//! Where a month begins, for the monthly budget (ADR-0011, ADR-0018 section 6).
//!
//! A tenant's month is a calendar month **in its own time zone**, not in UTC: a month that
//! ended at 22:00 the night before would otherwise start and stop two hours off the one the
//! owner of the key sees on their statement. South Africa has no summer time, so one fixed
//! offset (UTC+2) is exact; a tenant elsewhere would set its own.

use time::{OffsetDateTime, Time, UtcOffset};

/// South African Standard Time, in seconds east of UTC.
pub const SOUTH_AFRICA_OFFSET_SECONDS: i32 = 2 * 60 * 60;

/// The instant (in UTC) at which the calendar month containing `now` began in the time zone
/// `offset`: midnight on the first.
pub fn month_start(now: OffsetDateTime, offset: UtcOffset) -> OffsetDateTime {
    let local = now.to_offset(offset);
    // The first of a month always exists; the fallback is only for the compiler.
    let first = local.date().replace_day(1).unwrap_or_else(|_| local.date());
    first
        .with_time(Time::MIDNIGHT)
        .assume_offset(offset)
        .to_offset(UtcOffset::UTC)
}

#[cfg(test)]
mod tests {
    use super::*;
    use time::macros::datetime;

    fn sast() -> UtcOffset {
        UtcOffset::from_whole_seconds(SOUTH_AFRICA_OFFSET_SECONDS).unwrap()
    }

    #[test]
    fn a_month_in_south_africa_starts_two_hours_before_midnight_utc() {
        assert_eq!(
            month_start(datetime!(2026-10-15 10:00 UTC), sast()),
            datetime!(2026-09-30 22:00 UTC)
        );
    }

    #[test]
    fn the_last_two_hours_of_a_utc_month_already_belong_to_the_next_south_african_one() {
        // 22:30 UTC on 31 October is 00:30 on 1 November in South Africa.
        assert_eq!(
            month_start(datetime!(2026-10-31 22:30 UTC), sast()),
            datetime!(2026-10-31 22:00 UTC)
        );
        // Half an hour earlier it is still October.
        assert_eq!(
            month_start(datetime!(2026-10-31 21:30 UTC), sast()),
            datetime!(2026-09-30 22:00 UTC)
        );
    }

    #[test]
    fn the_instant_a_month_starts_is_in_that_month() {
        let start = datetime!(2026-10-31 22:00 UTC);
        assert_eq!(month_start(start, sast()), start);
        assert_eq!(
            month_start(start - time::Duration::nanoseconds(1), sast()),
            datetime!(2026-09-30 22:00 UTC)
        );
    }

    #[test]
    fn the_new_year_is_a_month_boundary_like_any_other() {
        assert_eq!(
            month_start(datetime!(2026-12-31 23:00 UTC), sast()),
            datetime!(2026-12-31 22:00 UTC)
        );
        assert_eq!(
            month_start(datetime!(2026-12-31 20:00 UTC), sast()),
            datetime!(2026-11-30 22:00 UTC)
        );
    }

    #[test]
    fn utc_is_just_the_calendar_month() {
        assert_eq!(
            month_start(datetime!(2026-10-15 10:00 UTC), UtcOffset::UTC),
            datetime!(2026-10-01 00:00 UTC)
        );
    }

    #[test]
    fn a_time_given_in_another_offset_gives_the_same_month() {
        let in_new_york = datetime!(2026-10-15 06:00 -04:00);
        assert_eq!(month_start(in_new_york, sast()), datetime!(2026-09-30 22:00 UTC));
    }

    #[test]
    fn the_south_african_offset_is_two_hours_east() {
        assert_eq!(SOUTH_AFRICA_OFFSET_SECONDS, 7_200);
    }
}
