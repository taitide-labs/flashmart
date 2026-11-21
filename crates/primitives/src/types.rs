use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum PrimitiveError {
    #[error("买家标识格式无效：需为 1-64 位字母、数字、下划线或连字符")]
    InvalidBuyerId,
    #[error("幂等键格式无效：需为 1-128 位非空可见字符")]
    InvalidIdempotencyKey,
    #[error("购买数量必须大于等于 1")]
    InvalidQuantity,
    #[error("金额（分）不能为负数或溢出")]
    InvalidMoneyCents,
    #[error("商品品类仅支持 digital 或 physical")]
    InvalidItemKind,
    #[error("币种仅支持 CNY")]
    UnsupportedCurrency,
    #[error("版本号必须为大于等于 1 的正整数")]
    InvalidOrderVersion,
}

macro_rules! uuid_id {
    ($name:ident, $doc:expr) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, ToSchema)]
        #[serde(transparent)]
        pub struct $name(Uuid);

        impl $name {
            pub fn new(id: Uuid) -> Self {
                Self(id)
            }

            pub fn into_inner(self) -> Uuid {
                self.0
            }

            pub fn as_uuid(&self) -> &Uuid {
                &self.0
            }
        }

        impl From<Uuid> for $name {
            fn from(value: Uuid) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

uuid_id!(ItemId, "限量商品强类型标识");
uuid_id!(OrderId, "抢购订单强类型标识");
uuid_id!(ReservationId, "库存预占强类型标识");

/// 轻量买家标识：1-64 位字母、数字、下划线或连字符
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct BuyerId(String);

impl BuyerId {
    pub fn try_new(raw: impl AsRef<str>) -> Result<Self, PrimitiveError> {
        let trimmed = raw.as_ref().trim();
        if trimmed.is_empty()
            || trimmed.len() > 64
            || !trimmed
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
        {
            return Err(PrimitiveError::InvalidBuyerId);
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for BuyerId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::try_new(s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for BuyerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 写接口幂等键：1-128 位非空且不含控制字符
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    pub fn try_new(raw: impl AsRef<str>) -> Result<Self, PrimitiveError> {
        let trimmed = raw.as_ref().trim();
        if trimmed.is_empty()
            || trimmed.len() > 128
            || trimmed.chars().any(|c| c.is_control())
        {
            return Err(PrimitiveError::InvalidIdempotencyKey);
        }
        Ok(Self(trimmed.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl<'de> Deserialize<'de> for IdempotencyKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::try_new(s).map_err(serde::de::Error::custom)
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 购买或库存操作正整数件数（>= 1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct Quantity(i32);

impl Quantity {
    pub fn try_new(value: i32) -> Result<Self, PrimitiveError> {
        if value < 1 {
            return Err(PrimitiveError::InvalidQuantity);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i32 {
        self.0
    }
}

impl<'de> Deserialize<'de> for Quantity {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = i32::deserialize(deserializer)?;
        Self::try_new(v).map_err(serde::de::Error::custom)
    }
}

/// 非负金额（单位：分）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct MoneyCents(i64);

impl MoneyCents {
    pub fn try_new(value: i64) -> Result<Self, PrimitiveError> {
        if value < 0 {
            return Err(PrimitiveError::InvalidMoneyCents);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i64 {
        self.0
    }

    pub fn checked_mul_quantity(self, quantity: Quantity) -> Result<Self, PrimitiveError> {
        self.0
            .checked_mul(i64::from(quantity.get()))
            .map(Self)
            .ok_or(PrimitiveError::InvalidMoneyCents)
    }
}

impl<'de> Deserialize<'de> for MoneyCents {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = i64::deserialize(deserializer)?;
        Self::try_new(v).map_err(serde::de::Error::custom)
    }
}

/// 币种（首期固定 CNY）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, ToSchema)]
pub enum Currency {
    #[default]
    Cny,
}

impl Currency {
    pub fn as_str(self) -> &'static str {
        "CNY"
    }

    pub fn parse(raw: &str) -> Result<Self, PrimitiveError> {
        match raw {
            "CNY" => Ok(Self::Cny),
            _ => Err(PrimitiveError::UnsupportedCurrency),
        }
    }
}

impl Serialize for Currency {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Currency {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Self::parse(&s).map_err(serde::de::Error::custom)
    }
}

/// 限量商品品类标记
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    Digital,
    Physical,
}

impl ItemKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Digital => "digital",
            Self::Physical => "physical",
        }
    }

    pub fn parse(raw: &str) -> Result<Self, PrimitiveError> {
        match raw {
            "digital" => Ok(Self::Digital),
            "physical" => Ok(Self::Physical),
            _ => Err(PrimitiveError::InvalidItemKind),
        }
    }
}

/// 订单乐观锁版本号（>= 1）
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, ToSchema)]
#[serde(transparent)]
pub struct OrderVersion(i32);

impl OrderVersion {
    pub const INITIAL: Self = Self(1);

    pub fn try_new(value: i32) -> Result<Self, PrimitiveError> {
        if value < 1 {
            return Err(PrimitiveError::InvalidOrderVersion);
        }
        Ok(Self(value))
    }

    pub fn get(self) -> i32 {
        self.0
    }

    pub fn next(self) -> Self {
        Self(self.0 + 1)
    }
}

impl<'de> Deserialize<'de> for OrderVersion {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let v = i32::deserialize(deserializer)?;
        Self::try_new(v).map_err(serde::de::Error::custom)
    }
}
