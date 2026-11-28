#![forbid(unsafe_code)]
mod invitation_mailer;
use axum::{Json, Router, http::StatusCode, routing::get};
use finance_shared::API_LIVENESS;

pub mod organizations;
pub mod pilot;
pub mod synthetic;

pub fn router() -> Router {
    Router::new()
        .route("/health/live", get(|| async { Json(API_LIVENESS) }))
        // Readiness deliberately fails closed until real dependency checks exist.
        .route(
            "/health/ready",
            get(|| async { StatusCode::SERVICE_UNAVAILABLE }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use tower::ServiceExt;

    #[tokio::test]
    async fn liveness_is_operational_only() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/health/live")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), 1024).await.unwrap();
        assert!(
            std::str::from_utf8(&body)
                .unwrap()
                .contains("\"phase\":\"foundation\"")
        );
    }

    #[tokio::test]
    async fn readiness_fails_closed() {
        let response = router()
            .oneshot(
                Request::builder()
                    .uri("/health/ready")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn financial_routes_are_not_exposed() {
        let response = router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/dossiers")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
