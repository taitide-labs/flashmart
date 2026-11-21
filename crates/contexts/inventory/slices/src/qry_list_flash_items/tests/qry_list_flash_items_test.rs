use chrono::{Duration, TimeZone, Utc};
use flashmart_contracts::WindowStatus;
use rstest::rstest;
use uuid::Uuid;

use super::super::handler::{FlashItemRow, row_to_summary};

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
    let summary = row_to_summary(
        FlashItemRow {
            id: Uuid::from_u128(0x1111),
            title: "限定测试商品".to_owned(),
            description: "描述".to_owned(),
            item_kind: "digital".to_owned(),
            price_cents: 9900,
            currency: "CNY".to_owned(),
            total_stock: 10,
            available_stock: 4,
            per_user_limit: 2,
            sale_starts_at: starts_at,
            sale_ends_at: ends_at,
        },
        now,
    )
    .unwrap();
    assert_eq!(summary.window_status, expected);
}
