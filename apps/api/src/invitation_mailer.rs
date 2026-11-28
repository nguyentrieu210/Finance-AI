//! Resend dispatcher for the durable staff invitation outbox.
use serde_json::json;
use sqlx::PgPool;
use std::{env, time::Duration};
use uuid::Uuid;

struct Settings {
    key: String,
    from: String,
    public_url: String,
}
#[derive(sqlx::FromRow)]
struct Delivery {
    delivery_id: Uuid,
    email: String,
    company_name: String,
}

impl Settings {
    fn from_env() -> Option<Self> {
        let key = env::var("RESEND_API_KEY").ok()?;
        let from = env::var("RESEND_FROM_EMAIL").ok()?;
        let public_url = env::var("FINANCE_PUBLIC_URL")
            .unwrap_or_else(|_| "https://namviet.gpcds.site".to_owned());
        Self::validated(key, from, public_url)
    }

    fn validated(key: String, from: String, public_url: String) -> Option<Self> {
        let host = public_url
            .strip_prefix("https://")
            .and_then(|value| value.split('/').next());
        if host.is_none_or(|value| value.is_empty() || value.contains('@'))
            || !key.starts_with("re_")
            || key.len() > 256
            || from.trim().is_empty()
            || from.len() > 320
            || from.chars().any(char::is_control)
            || !public_url.starts_with("https://")
            || public_url.len() > 512
            || public_url
                .chars()
                .any(|c| matches!(c, '?' | '#' | '\\' | ' '))
        {
            return None;
        }
        Some(Self {
            key,
            from,
            public_url: public_url.trim_end_matches('/').to_owned(),
        })
    }
}

pub(crate) fn spawn(pool: PgPool) {
    let Some(settings) = Settings::from_env() else {
        eprintln!("finance invitation delivery disabled: Resend configuration unavailable");
        return;
    };
    tokio::spawn(async move {
        let Ok(client) = reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        else {
            eprintln!("finance invitation delivery client unavailable");
            return;
        };
        let mut tick = tokio::time::interval(Duration::from_secs(10));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            dispatch_batch(&pool, &client, &settings).await;
        }
    });
}

async fn dispatch_batch(pool: &PgPool, client: &reqwest::Client, settings: &Settings) {
    let worker = Uuid::from_bytes(rand::random());
    let claimed: Result<Vec<Delivery>, sqlx::Error> = sqlx::query_as(
        "SELECT delivery_id,email,company_name FROM finance.staff_invitation_delivery_claim($1,$2)",
    )
    .bind(worker)
    .bind(5_i32)
    .fetch_all(pool)
    .await;
    let Ok(claimed) = claimed else { return };
    for delivery in claimed {
        let link = format!("{}/auth/start?rd=%2Finvite", settings.public_url);
        let company = html_escape(&delivery.company_name);
        let safe_link = html_escape(&link);
        let html = format!(
            "<p>Bạn được mời tham gia <strong>{company}</strong> trên Payment &amp; Advanced NV.</p><p><a href=\"{safe_link}\">Đăng nhập và gửi yêu cầu tham gia</a></p><p>Lời mời có hiệu lực trong 7 ngày. Việc gửi yêu cầu chưa cấp quyền truy cập; quản trị viên công ty cần phê duyệt.</p>"
        );
        let plain = format!(
            "Bạn được mời tham gia {} trên Payment & Advanced NV.\n\nĐăng nhập và gửi yêu cầu tham gia: {}\n\nLời mời có hiệu lực trong 7 ngày. Quản trị viên công ty cần phê duyệt trước khi cấp quyền truy cập.",
            delivery.company_name, link
        );
        let result = client
            .post("https://api.resend.com/emails")
            .bearer_auth(&settings.key)
            .header("Idempotency-Key", delivery.delivery_id.to_string())
            .json(&json!({
                "from": settings.from.as_str(),
                "to": [delivery.email],
                "subject": format!("Lời mời tham gia {} · Payment & Advanced NV", delivery.company_name),
                "html": html,
                "text": plain
            }))
            .send()
            .await;
        let (sent, error_code) = match result {
            Ok(response) if response.status().is_success() => (true, None),
            Ok(response) if response.status().as_u16() == 429 => (false, Some("PROVIDER_429")),
            Ok(response) if response.status().as_u16() == 408 => (false, Some("PROVIDER_5XX")),
            Ok(response) if response.status().is_server_error() => (false, Some("PROVIDER_5XX")),
            Ok(_) => (false, Some("PROVIDER_4XX")),
            Err(_) => (false, Some("NETWORK")),
        };
        let _ = sqlx::query_scalar::<_, bool>(
            "SELECT finance.staff_invitation_delivery_complete($1,$2,$3,$4)",
        )
        .bind(worker)
        .bind(delivery.delivery_id)
        .bind(sent)
        .bind(error_code)
        .fetch_one(pool)
        .await;
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaped_invitation_content_cannot_inject_markup() {
        assert_eq!(
            html_escape("<img src=x onerror='x'>&"),
            "&lt;img src=x onerror=&#39;x&#39;&gt;&amp;"
        );
    }

    #[test]
    fn public_url_requires_https_and_no_query_or_fragment() {
        let base = || {
            (
                "re_test-key".to_owned(),
                "Finance <noreply@example.invalid>".to_owned(),
            )
        };
        let (key, from) = base();
        assert!(
            Settings::validated(key, from, "https://namviet.gpcds.site/path".to_owned()).is_some()
        );
        let (key, from) = base();
        assert!(Settings::validated(key, from, "http://example.invalid".to_owned()).is_none());
        let (key, from) = base();
        assert!(Settings::validated(key, from, "https://".to_owned()).is_none());
        let (key, from) = base();
        assert!(
            Settings::validated(key, from, "https://user@example.invalid".to_owned()).is_none()
        );
        let (key, from) = base();
        assert!(
            Settings::validated(key, from, "https://example.invalid/?x=1".to_owned()).is_none()
        );
    }
}
