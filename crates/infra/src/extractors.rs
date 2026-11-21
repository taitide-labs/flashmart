use flashmart_primitives::{BuyerId, IdempotencyKey};
use http::HeaderMap;

use crate::problem::{ApiProblem, FieldValidationError, ProblemDetails};

pub fn extract_buyer_id(headers: &HeaderMap, instance: &str) -> Result<BuyerId, ApiProblem> {
    let raw = headers
        .get("X-Buyer-Id")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiProblem::from(ProblemDetails::missing_header(
                instance,
                "缺少买家标识",
                "请求头必须携带 X-Buyer-Id。",
            ))
        })?;

    BuyerId::try_new(raw).map_err(|_| {
        ApiProblem::from(ProblemDetails::validation(
            instance,
            "买家标识格式无效",
            "X-Buyer-Id 仅支持 1-64 位字母、数字、下划线或连字符。",
            vec![FieldValidationError {
                field: "X-Buyer-Id".to_owned(),
                message: "格式需匹配 ^[A-Za-z0-9_-]{1,64}$".to_owned(),
            }],
        ))
    })
}

pub fn extract_idempotency_key(
    headers: &HeaderMap,
    instance: &str,
) -> Result<IdempotencyKey, ApiProblem> {
    let raw = headers
        .get("Idempotency-Key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            ApiProblem::from(ProblemDetails::missing_header(
                instance,
                "缺少幂等键",
                "写请求必须携带有效 Idempotency-Key。",
            ))
        })?;

    IdempotencyKey::try_new(raw).map_err(|_| {
        ApiProblem::from(ProblemDetails::missing_header(
            instance,
            "幂等键格式无效",
            "Idempotency-Key 长度需在 1-128 字符之间且不含控制字符。",
        ))
    })
}
