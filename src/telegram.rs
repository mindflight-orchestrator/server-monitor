//! Telegram Bot API for sending alerts.

use reqwest::Client;
use tracing::{debug, warn};

const TELEGRAM_API: &str = "https://api.telegram.org";

/// Send a message via Telegram Bot API.
pub async fn send_message(
    client: &Client,
    bot_token: &str,
    chat_id: &str,
    text: &str,
) -> Result<(), String> {
    let url = format!("{}/bot{}/sendMessage", TELEGRAM_API, bot_token);

    let body = serde_json::json!({
        "chat_id": chat_id,
        "text": text,
        "parse_mode": "HTML",
        "disable_web_page_preview": true,
    });

    let res = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("Telegram request failed: {}", e))?;

    if !res.status().is_success() {
        let status = res.status();
        let body = res.text().await.unwrap_or_default();
        warn!(status = %status, body = %body, "Telegram API error");
        return Err(format!("Telegram API error {}: {}", status, body));
    }

    debug!("Telegram message sent successfully");
    Ok(())
}

/// Format a failure alert message for Telegram.
pub fn format_alert(target: &str, failures: &[String]) -> String {
    let mut msg = format!("<b>ClosLamartine Monitor Alert</b>\n");
    msg.push_str(&format!("Target: <code>{}</code>\n", target));
    msg.push_str(&format!("Time: {}\n\n", chrono_now()));
    msg.push_str("<b>Failures:</b>\n");
    for f in failures {
        msg.push_str(&format!("• {}\n", f));
    }
    msg
}

/// Format a resolved alert message for Telegram.
pub fn format_resolved(target: &str) -> String {
    let mut msg = format!("<b>ClosLamartine Monitor Resolved</b>\n");
    msg.push_str(&format!("Target: <code>{}</code>\n", target));
    msg.push_str(&format!("Time: {}\n\n", chrono_now()));
    msg.push_str("✅ Issue is resolved and checks are healthy again.");
    msg
}

fn chrono_now() -> String {
    chrono::Utc::now()
        .format("%Y-%m-%d %H:%M:%S UTC")
        .to_string()
}
