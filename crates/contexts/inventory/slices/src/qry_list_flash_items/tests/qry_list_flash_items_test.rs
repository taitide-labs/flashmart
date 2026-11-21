use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::WindowStatus;
use rstest::rstest;

use super::super::handler::compute_window_status;

#[rstest]
#[case(-1, WindowStatus::Upcoming)]
#[case(0, WindowStatus::Active)]
#[case(30, WindowStatus::Active)]
#[case(60, WindowStatus::Active)]
#[case(61, WindowStatus::Ended)]
fn window_status_follows_server_clock(#[case] offset_mins: i64, #[case] expected: WindowStatus) {
    let starts_at = Utc.with_ymd_and_hms(2026, 11, 21, 12, 0, 0).unwrap();
    let ends_at = starts_at + Duration::minutes(60);
    let now = starts_at + Duration::minutes(offset_mins);
    assert_eq!(compute_window_status(now, starts_at, ends_at), expected);
}
