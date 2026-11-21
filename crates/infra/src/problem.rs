use std::ops::Deref;

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, header};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FieldValidationError {
    pub field: String,
    pub message: String,
}

/// RFC 9457 问题详情响应，字段严格对齐客户端 `ProblemDetails` 契约
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct ProblemDetails {
    #[serde(rename = "type")]
    pub problem_type: String,
    pub title: String,
    pub status: u16,
    pub detail: String,
    pub instance: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub errors: Option<Vec<FieldValidationError>>,
}

impl ProblemDetails {
    pub fn new(
        status: StatusCode,
        code: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        instance: impl Into<String>,
    ) -> Self {
        let code = code.into();
        Self {
            problem_type: format!("urn:flashmart:problem:{code}"),
            title: title.into(),
            status: status.as_u16(),
            detail: detail.into(),
            instance: instance.into(),
            code,
            errors: None,
        }
    }

    pub fn with_errors(mut self, errors: Vec<FieldValidationError>) -> Self {
        self.errors = Some(errors);
        self
    }

    pub fn validation(
        instance: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        errors: Vec<FieldValidationError>,
    ) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "validation_error",
            title,
            detail,
            instance,
        )
        .with_errors(errors)
    }

    pub fn missing_header(
        instance: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            "missing_required_header",
            title,
            detail,
            instance,
        )
    }
}

impl IntoResponse for ProblemDetails {
    fn into_response(self) -> Response {
        let status = StatusCode::from_u16(self.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let body = serde_json::to_vec(&self).unwrap_or_else(|_| {
            br#"{"type":"urn:flashmart:problem:internal_error","title":"Internal Error","status":500,"detail":"Failed to serialize problem details","instance":"/","code":"internal_error"}"#.to_vec()
        });
        let mut response = (status, body).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/problem+json"),
        );
        response
    }
}

/// 轻量栈指针包装，避免 `Result<T, ProblemDetails>` 触发 `clippy::result_large_err`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiProblem(pub Box<ProblemDetails>);

impl ApiProblem {
    pub fn new(problem: ProblemDetails) -> Self {
        Self(Box::new(problem))
    }
}

impl From<ProblemDetails> for ApiProblem {
    fn from(value: ProblemDetails) -> Self {
        Self::new(value)
    }
}

impl Deref for ApiProblem {
    type Target = ProblemDetails;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl IntoResponse for ApiProblem {
    fn into_response(self) -> Response {
        (*self.0).into_response()
    }
}
