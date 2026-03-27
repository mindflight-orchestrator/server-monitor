//! Health checks for prod, staging, and server vitals.

use crate::config::Config;
use crate::config::MonitorScope;
use crate::docker::{ContainerInfo, ContainerStatus, DockerClient, HealthStatus};
use crate::telegram;
use crate::vitals;
use reqwest::Client;
use std::time::Duration;
use tracing::{info, warn};


/// Result of a full check run.
#[derive(Debug)]
pub struct CheckResult {
    pub server_ok: bool,
    pub prod_ok: bool,
    pub staging_ok: bool,
    pub prod_monitored: bool,
    pub staging_monitored: bool,
    pub server_failures: Vec<String>,
    pub prod_failures: Vec<String>,
    pub staging_failures: Vec<String>,
}

/// Last known alert state, used to avoid repeated loop notifications.
#[derive(Debug, Clone, Default)]
pub struct AlertState {
    initialized: bool,
    server_ok: bool,
    prod_ok: bool,
    staging_ok: bool,
}

impl CheckResult {
    pub fn all_ok(&self) -> bool {
        self.server_ok && self.prod_ok && self.staging_ok
    }

    pub fn has_failures(&self) -> bool {
        !self.server_failures.is_empty()
            || !self.prod_failures.is_empty()
            || !self.staging_failures.is_empty()
    }

    /// Format as status message for Telegram.
    pub fn format_status(&self) -> String {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let mut msg = format!("<b>ClosLamartine Monitor — Status</b>\nTime: {}\n\n", now);

        msg.push_str("<b>Server:</b> ");
        if self.server_ok {
            msg.push_str("✅ OK\n");
        } else {
            msg.push_str("❌\n");
            for f in &self.server_failures {
                msg.push_str(&format!("  • {}\n", f));
            }
        }

        msg.push_str("<b>Production:</b> ");
        if !self.prod_monitored {
            msg.push_str("ℹ️ Not monitored\n");
        } else if self.prod_ok {
            msg.push_str("✅ OK\n");
        } else {
            msg.push_str("❌\n");
            for f in &self.prod_failures {
                msg.push_str(&format!("  • {}\n", f));
            }
        }

        msg.push_str("<b>Staging:</b> ");
        if !self.staging_monitored {
            msg.push_str("ℹ️ Not monitored\n");
        } else if self.staging_ok {
            msg.push_str("✅ OK\n");
        } else {
            msg.push_str("❌\n");
            for f in &self.staging_failures {
                msg.push_str(&format!("  • {}\n", f));
            }
        }

        msg
    }

    /// Format prod-only status.
    pub fn format_status_prod(&self) -> String {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let mut msg = format!("<b>Production status</b>\nTime: {}\n\n", now);
        if !self.prod_monitored {
            msg.push_str("ℹ️ Not monitored by this instance");
        } else if self.prod_ok {
            msg.push_str("✅ OK");
        } else {
            for f in &self.prod_failures {
                msg.push_str(&format!("• {}\n", f));
            }
        }
        msg
    }

    /// Format staging-only status.
    pub fn format_status_staging(&self) -> String {
        let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
        let mut msg = format!("<b>Staging status</b>\nTime: {}\n\n", now);
        if !self.staging_monitored {
            msg.push_str("ℹ️ Not monitored by this instance");
        } else if self.staging_ok {
            msg.push_str("✅ OK");
        } else {
            for f in &self.staging_failures {
                msg.push_str(&format!("• {}\n", f));
            }
        }
        msg
    }
}

/// Run all checks and return aggregated result.
pub async fn run_checks(config: &Config) -> CheckResult {
    // Server vitals (memory, disk, services)
    let server_failures = vitals::run_vitals(config);
    let server_ok = server_failures.is_empty();

    let http_client = Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("reqwest client");

    let mut prod_failures = Vec::new();
    let mut staging_failures = Vec::new();

    if config.dev {
        // Dev mode: only check backend 58081, frontend 58080
        if let Err(e) = check_backend_health(&http_client, &config.dev_backend_url).await {
            prod_failures.push(format!("Backend: {}", e));
        }
        if !check_http_ok(&http_client, &config.dev_frontend_url, "dev frontend").await {
            prod_failures.push("Frontend unreachable".to_string());
        }
        let prod_ok = prod_failures.is_empty();
        let staging_ok = true; // N/A in dev

        if server_ok && prod_ok {
            info!("All dev checks passed");
        } else {
            warn!(
                server_failures = ?server_failures,
                prod_failures = ?prod_failures,
                "dev checks failed"
            );
        }

        return CheckResult {
            server_ok,
            prod_ok,
            staging_ok,
            prod_monitored: true,
            staging_monitored: false,
            server_failures,
            prod_failures,
            staging_failures,
        };
    }

    // Prod/staging: Docker + HTTP
    let docker = match DockerClient::connect(&config.docker_socket) {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "failed to connect to Docker");
            return CheckResult {
                server_ok,
                prod_ok: false,
                staging_ok: false,
                prod_monitored: matches!(config.scope, MonitorScope::Both | MonitorScope::Prod),
                staging_monitored: matches!(
                    config.scope,
                    MonitorScope::Both | MonitorScope::Staging
                ),
                server_failures,
                prod_failures: vec![format!("Docker connection failed: {}", e)],
                staging_failures: vec![format!("Docker connection failed: {}", e)],
            };
        }
    };

    // Docker container checks
    if matches!(config.scope, MonitorScope::Both | MonitorScope::Prod) {
        let prod_refs: Vec<&str> = config.prod_containers.iter().map(|s| s.as_str()).collect();
        let prod_containers = docker.inspect_containers(&prod_refs).await;
        if let Ok(containers) = prod_containers {
            for c in containers {
                collect_container_failures(&c, &mut prod_failures);
            }
        } else if let Err(e) = prod_containers {
            prod_failures.push(format!("Docker inspect failed: {}", e));
        }
    }

    if matches!(config.scope, MonitorScope::Both | MonitorScope::Staging) {
        let staging_refs: Vec<&str> = config.staging_containers.iter().map(|s| s.as_str()).collect();
        let staging_containers = docker.inspect_containers(&staging_refs).await;
        if let Ok(containers) = staging_containers {
            for c in containers {
                collect_container_failures(&c, &mut staging_failures);
            }
        } else if let Err(e) = staging_containers {
            staging_failures.push(format!("Docker inspect failed: {}", e));
        }
    }

    // HTTP health checks
    if matches!(config.scope, MonitorScope::Both | MonitorScope::Prod) {
        if let Err(e) = check_backend_health(&http_client, &config.prod_backend_url).await {
            prod_failures.push(format!("Backend: {}", e));
        }
        if !check_http_ok(&http_client, &config.prod_frontend_url, "prod frontend").await {
            prod_failures.push("Frontend unreachable".to_string());
        }
    }
    if matches!(config.scope, MonitorScope::Both | MonitorScope::Staging) {
        if let Err(e) = check_backend_health(&http_client, &config.staging_backend_url).await
        {
            staging_failures.push(format!("Backend: {}", e));
        }
        if !check_http_ok(&http_client, &config.staging_frontend_url, "staging frontend").await
        {
            staging_failures.push("Frontend unreachable".to_string());
        }
    }

    let prod_ok = if matches!(config.scope, MonitorScope::Both | MonitorScope::Prod) {
        prod_failures.is_empty()
    } else {
        true
    };
    let staging_ok = if matches!(config.scope, MonitorScope::Both | MonitorScope::Staging) {
        staging_failures.is_empty()
    } else {
        true
    };

    if server_ok && prod_ok && staging_ok {
        info!("All checks passed");
    } else {
        warn!(
            server_failures = ?server_failures,
            prod_failures = ?prod_failures,
            staging_failures = ?staging_failures,
            "checks failed"
        );
    }

    CheckResult {
        server_ok,
        prod_ok,
        staging_ok,
        prod_monitored: matches!(config.scope, MonitorScope::Both | MonitorScope::Prod),
        staging_monitored: matches!(config.scope, MonitorScope::Both | MonitorScope::Staging),
        server_failures,
        prod_failures,
        staging_failures,
    }
}

fn collect_container_failures(c: &ContainerInfo, failures: &mut Vec<String>) {
    match &c.status {
        ContainerStatus::Running => {
            // DB has healthcheck; backend/frontend don't
            if c.name.contains("_db") && c.health != HealthStatus::Healthy {
                match &c.health {
                    HealthStatus::Unhealthy => {
                        failures.push(format!("{}: DB unhealthy", c.name));
                    }
                    HealthStatus::Starting => {
                        failures.push(format!("{}: DB still starting", c.name));
                    }
                    HealthStatus::None => {
                        // Postgres has healthcheck, so None might mean old inspect
                        failures.push(format!("{}: DB health unknown", c.name));
                    }
                    _ => {}
                }
            }
        }
        ContainerStatus::Exited => {
            failures.push(format!("{}: container exited", c.name));
        }
        ContainerStatus::Restarting => {
            failures.push(format!("{}: container restarting", c.name));
        }
        ContainerStatus::Dead => {
            failures.push(format!("{}: container dead", c.name));
        }
        ContainerStatus::Paused => {
            failures.push(format!("{}: container paused", c.name));
        }
        ContainerStatus::Other(s) => {
            failures.push(format!("{}: status={}", c.name, s));
        }
    }
}

async fn check_http_ok(client: &Client, url: &str, label: &str) -> bool {
    match client.get(url).send().await {
        Ok(res) => {
            if res.status().is_success() {
                true
            } else {
                warn!(url = %url, status = %res.status(), "{} check failed", label);
                false
            }
        }
        Err(e) => {
            warn!(url = %url, error = %e, "{} unreachable", label);
            false
        }
    }
}

async fn check_backend_health(client: &Client, url: &str) -> Result<(), String> {
    let res = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("unreachable: {}", e))?;
    if !res.status().is_success() {
        return Err(format!("HTTP {}", res.status()));
    }
    let body = res.text().await.map_err(|e| format!("read error: {}", e))?;
    if body.contains("\"status\"") && body.contains("ok") {
        Ok(())
    } else {
        Err("invalid health response".to_string())
    }
}

/// Send failure/resolved notifications only when state changes.
pub async fn send_alerts_on_transition(
    config: &Config,
    result: &CheckResult,
    state: &mut AlertState,
) {
    if !config.telegram_configured() {
        return;
    }

    let Some(token) = config.telegram_bot_token.as_ref() else {
        return;
    };
    let Some(chat_id) = config.telegram_chat_id.as_ref() else {
        return;
    };

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .expect("reqwest client");

    // Initial run: emit only current failures.
    if !state.initialized {
        if !result.server_ok {
            let msg = telegram::format_alert("server", &result.server_failures);
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram alert");
            }
        }
        if result.prod_monitored && !result.prod_ok {
            let msg =
                telegram::format_alert(&target_label(config, "production"), &result.prod_failures);
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram alert");
            }
        }
        if result.staging_monitored && !result.staging_ok {
            let msg =
                telegram::format_alert(&target_label(config, "staging"), &result.staging_failures);
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram alert");
            }
        }
        state.initialized = true;
        state.server_ok = result.server_ok;
        state.prod_ok = result.prod_ok;
        state.staging_ok = result.staging_ok;
        return;
    }

    // Server transitions
    if state.server_ok && !result.server_ok {
        let msg = telegram::format_alert("server", &result.server_failures);
        if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
            warn!(error = %e, "failed to send Telegram alert");
        }
    } else if !state.server_ok && result.server_ok {
        let msg = telegram::format_resolved("server");
        if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
            warn!(error = %e, "failed to send Telegram resolved message");
        }
    }

    // Production transitions
    if result.prod_monitored {
        if state.prod_ok && !result.prod_ok {
            let msg =
                telegram::format_alert(&target_label(config, "production"), &result.prod_failures);
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram alert");
            }
        } else if !state.prod_ok && result.prod_ok {
            let msg = telegram::format_resolved(&target_label(config, "production"));
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram resolved message");
            }
        }
    }

    // Staging transitions
    if result.staging_monitored {
        if state.staging_ok && !result.staging_ok {
            let msg =
                telegram::format_alert(&target_label(config, "staging"), &result.staging_failures);
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram alert");
            }
        } else if !state.staging_ok && result.staging_ok {
            let msg = telegram::format_resolved(&target_label(config, "staging"));
            if let Err(e) = telegram::send_message(&client, token, chat_id, &msg).await {
                warn!(error = %e, "failed to send Telegram resolved message");
            }
        }
    }

    state.server_ok = result.server_ok;
    state.prod_ok = result.prod_ok;
    state.staging_ok = result.staging_ok;
}

fn target_label(config: &Config, env: &str) -> String {
    match env {
        "production" => format!("production ({})", config.prod_domain),
        "staging" => format!("staging ({})", config.staging_domain),
        _ => env.to_string(),
    }
}
