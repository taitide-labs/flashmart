use rstest::rstest;

use crate::{
    BuyerId, Currency, IdempotencyKey, ItemKind, MoneyCents, OrderVersion, PrimitiveError, Quantity,
};

#[rstest]
#[case("buyer-demo", true)]
#[case("user_01", true)]
#[case("  buyer-1  ", true)]
#[case("", false)]
#[case("   ", false)]
#[case("buyer with space", false)]
#[case("buyer@domain", false)]
fn buyer_id_validation(#[case] input: &str, #[case] ok: bool) {
    let res = BuyerId::try_new(input);
    assert_eq!(res.is_ok(), ok);
    if !ok {
        assert_eq!(res.unwrap_err(), PrimitiveError::InvalidBuyerId);
    }
}

#[test]
fn buyer_id_rejects_over_64_chars() {
    let long = "a".repeat(65);
    assert_eq!(BuyerId::try_new(long), Err(PrimitiveError::InvalidBuyerId));
}

#[rstest]
#[case("idem-123", true)]
#[case("", false)]
#[case("   ", false)]
#[case("bad\nkey", false)]
fn idempotency_key_validation(#[case] input: &str, #[case] ok: bool) {
    assert_eq!(IdempotencyKey::try_new(input).is_ok(), ok);
}

#[rstest]
#[case(1, true)]
#[case(5, true)]
#[case(0, false)]
#[case(-2, false)]
fn quantity_validation(#[case] input: i32, #[case] ok: bool) {
    assert_eq!(Quantity::try_new(input).is_ok(), ok);
}

#[test]
fn money_cents_checked_multiplication() {
    let unit = MoneyCents::try_new(9900).unwrap();
    let qty = Quantity::try_new(3).unwrap();
    assert_eq!(unit.checked_mul_quantity(qty).unwrap().get(), 29700);
    assert_eq!(
        MoneyCents::try_new(-1),
        Err(PrimitiveError::InvalidMoneyCents)
    );
}

#[test]
fn currency_and_item_kind_roundtrip() {
    let cny_json = serde_json::to_string(&Currency::Cny).unwrap();
    assert_eq!(cny_json, "\"CNY\"");
    assert_eq!(
        serde_json::from_str::<Currency>("\"CNY\"").unwrap(),
        Currency::Cny
    );
    assert!(serde_json::from_str::<Currency>("\"USD\"").is_err());

    assert_eq!(ItemKind::Digital.as_str(), "digital");
    assert_eq!(ItemKind::Physical.as_str(), "physical");
    assert_eq!(ItemKind::parse("digital").unwrap(), ItemKind::Digital);
    assert_eq!(ItemKind::parse("bad"), Err(PrimitiveError::InvalidItemKind));
}

#[test]
fn order_version_progression() {
    assert_eq!(OrderVersion::INITIAL.get(), 1);
    assert_eq!(OrderVersion::INITIAL.next().get(), 2);
    assert_eq!(
        OrderVersion::try_new(0),
        Err(PrimitiveError::InvalidOrderVersion)
    );
}
