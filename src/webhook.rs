//! Telegram webhook HTTP server for interactive commands.

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::post, Json, Router};
use serde::Deserialize;
use std::process::Command;
use subtle::ConstantTimeEq;
use tracing::{info, warn};

use crate::checks;
use crate::config::Config;
use crate::config::IpFirewallBackend;
use crate::diagnose_core;
use crate::docker::{ContainerStatus, DockerClient, HealthStatus};
use crate::ip_admin;
use crate::telegram;
use crate::vitals;
use reqwest::Client;

/// Telegram Update payload (minimal fields we need).
#[derive(Debug, Deserialize)]
pub struct TelegramUpdate {
    pub update_id: i64,
    pub message: Option<TelegramMessage>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramMessage {
    pub chat: TelegramChat,
    pub text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TelegramChat {
    pub id: i64,
}

/// Shared state for the webhook handler.
#[derive(Clone)]
pub struct WebhookState {
    pub config: Config,
    pub http_client: Client,
}

/// Build the webhook router.
pub fn router(state: WebhookState) -> Router {
    Router::new()
        .route("/monitor/webhook", post(webhook_handler))
        .with_state(state)
}

/// Webhook handler: validate secret, parse update, dispatch command.
async fn webhook_handler(
    State(state): State<WebhookState>,
    headers: axum::http::HeaderMap,
    Json(payload): Json<TelegramUpdate>,
) -> impl IntoResponse {
    // 1. Secret validation (constant-time, before any processing)
    let secret = match state.config.webhook_secret.as_ref() {
        Some(s) => s.as_bytes(),
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "webhook not configured").into_response()
        }
    };

    let header_token = headers
        .get("x-telegram-bot-api-secret-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    if header_token.as_bytes().len() != secret.len()
        || !bool::from(secret.ct_eq(header_token.as_bytes()))
    {
        warn!("webhook: invalid or missing secret token");
        return (StatusCode::FORBIDDEN, "Forbidden").into_response();
    }

    // 2. Parse message
    let Some(msg) = payload.message else {
        return (StatusCode::OK, "ok").into_response();
    };

    let chat_id = msg.chat.id.to_string();
    let text = msg.text.as_deref().unwrap_or("").trim().to_string();
    let raw_cmd = text.split_whitespace().next().unwrap_or("").to_lowercase();
    let cmd = normalize_command(&raw_cmd);

    // 3. /myid, /start, /help: no TELEGRAM_ALLOWED_CHAT_IDS required (so new users can get ID and see commands)
    if cmd == "/myid" || cmd == "/start" {
        let config = state.config.clone();
        let client = state.http_client.clone();
        let reply = format!(
            "Your Chat ID: <code>{}</code>\n\nAdd this to TELEGRAM_ALLOWED_CHAT_IDS in the monitor config, then redeploy to get access.",
            chat_id
        );
        tokio::spawn(async move {
            if let Some(token) = &config.telegram_bot_token {
                let _ = telegram::send_message(&client, token, &chat_id, &reply).await;
            }
        });
        return (StatusCode::OK, "ok").into_response();
    }
    if cmd == "/help" {
        let config = state.config.clone();
        let client = state.http_client.clone();
        tokio::spawn(async move {
            if let Some(token) = &config.telegram_bot_token {
                let text = help_message_html(&config);
                let _ = telegram::send_message(&client, token, &chat_id, &text).await;
            }
        });
        return (StatusCode::OK, "ok").into_response();
    }

    // 4. Chat ID check (all other commands require TELEGRAM_ALLOWED_CHAT_IDS)
    if !state.config.allowed_chat_ids.contains(&chat_id) {
        info!(chat_id = %chat_id, "webhook: ignoring message from unknown chat");
        return (StatusCode::OK, "ok").into_response();
    }

    // 5. Dispatch command (spawn to avoid blocking Telegram's 60s timeout)
    let config = state.config.clone();
    let client = state.http_client.clone();
    tokio::spawn(async move {
        let reply = handle_command(&config, &client, &chat_id, &text).await;
        if let Some(reply_text) = reply {
            if let (Some(token), _) = (config.telegram_bot_token.as_ref(), &config.allowed_chat_ids)
            {
                if let Err(e) = telegram::send_message(&client, token, &chat_id, &reply_text).await
                {
                    warn!(error = %e, "webhook: failed to send reply");
                }
            }
        }
    });

    (StatusCode::OK, "ok").into_response()
}

/// Handle a command and return optional reply text.
async fn handle_command(
    config: &Config,
    _client: &Client,
    _chat_id: &str,
    text: &str,
) -> Option<String> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let raw_cmd = parts.first().copied().unwrap_or("").to_lowercase();
    let cmd = normalize_command(&raw_cmd);
    info!(raw_command = %raw_cmd, normalized_command = %cmd, "webhook command received");

    match cmd.as_str() {
        "/status" => {
            let result = checks::run_checks(config).await;
            Some(result.format_status())
        }
        "/status_prod" => {
            let result = checks::run_checks(config).await;
            Some(result.format_status_prod())
        }
        "/status_staging" => {
            let result = checks::run_checks(config).await;
            Some(result.format_status_staging())
        }
        "/status_server" => Some(vitals::format_server_status(config)),
        "/space_left" => Some(vitals::format_space_left()),
        "/uptime_stats" => Some(vitals::format_uptime_stats()),
        "/memory" => Some(vitals::format_memory()),
        "/certs" => Some(vitals::format_certs(config)),
        "/docker" => Some(run_docker_list(config).await),
        "/self" => {
            let report = diagnose_core::run_full_diagnostic(config).await;
            Some(diagnose_core::format_report_telegram(&report, 3900))
        }
        "/help" => Some(help_message_html(config)),
        "/prod_backup" => Some(run_prod_backup(config).await),
        "/prod_restart" => Some(run_prod_restart(config).await),
        "/staging_backup" => Some(run_staging_backup(config).await),
        "/staging_restart" => Some(run_staging_restart(config).await),
        "/ip_list" => {
            if !config.ip_admin_enabled() {
                Some(ip_admin_disabled_reply())
            } else {
                let pw = parts.get(1).copied().unwrap_or("");
                if pw.is_empty() {
                    Some("Usage: <code>/ip_list password</code>".to_string())
                } else if !ip_admin::verify_password(config, pw) {
                    Some(ip_password_invalid_reply())
                } else {
                    Some(ip_admin::ip_list(config).await)
                }
            }
        }
        "/ip_ban" => {
            if !config.ip_admin_enabled() {
                Some(ip_admin_disabled_reply())
            } else {
                let ip = parts.get(1).copied().unwrap_or("");
                let pw = parts.get(2).copied().unwrap_or("");
                if ip.is_empty() || pw.is_empty() {
                    Some("Usage: <code>/ip_ban x.x.x.x password</code>".to_string())
                } else if !ip_admin::verify_password(config, pw) {
                    Some(ip_password_invalid_reply())
                } else {
                    Some(ip_admin::ip_ban(config, ip).await)
                }
            }
        }
        "/ip_unban" => {
            if !config.ip_admin_enabled() {
                Some(ip_admin_disabled_reply())
            } else {
                let ip = parts.get(1).copied().unwrap_or("");
                let pw = parts.get(2).copied().unwrap_or("");
                if ip.is_empty() || pw.is_empty() {
                    Some("Usage: <code>/ip_unban x.x.x.x password</code>".to_string())
                } else if !ip_admin::verify_password(config, pw) {
                    Some(ip_password_invalid_reply())
                } else {
                    Some(ip_admin::ip_unban(config, ip).await)
                }
            }
        }
        "/kylit_backup_db" => {
            if !config.kylit_webhook_enabled {
                Some(kylit_disabled_reply())
            } else {
                Some(run_kylit_backup_db(config).await)
            }
        }
        "/kylit_backup_minio" => {
            if !config.kylit_webhook_enabled {
                Some(kylit_disabled_reply())
            } else {
                Some(run_kylit_backup_minio(config).await)
            }
        }
        "/kylit_backup_all" => {
            if !config.kylit_webhook_enabled {
                Some(kylit_disabled_reply())
            } else {
                Some(run_kylit_backup_all(config).await)
            }
        }
        "/kylit_docker" => {
            if !config.kylit_webhook_enabled {
                Some(kylit_disabled_reply())
            } else {
                Some(run_kylit_docker(config).await)
            }
        }
        _ => {
            if cmd.starts_with('/') {
                Some("Unknown command. Send /help for the full command list.".to_string())
            } else {
                None
            }
        }
    }
}

fn help_message_html(config: &Config) -> String {
    let mut s = String::from(
        r#"<b>Monitor — Commands</b>

<b>Status</b>
/status — Full status (prod, staging, server)
/status_prod — Production only
/status_staging — Staging only
/status_server — Server vitals only
/self — Self-check (webhook chain, CLI diagnose equivalent)

<b>Server stats</b>
/space_left — Disk space (/ /var /home)
/uptime_stats — Uptime + load average
/memory — RAM usage
/certs — SSL cert expiry
/docker — Container list

<b>Production</b>
/prod_backup — Backup production DB
/prod_restart — Restart prod containers

<b>Staging</b>
/staging_backup — Backup staging DB
/staging_restart — Restart staging containers

/help — This message
/myid — Get your Chat ID (no auth required)
"#,
    );

    if config.kylit_webhook_enabled {
        s.push_str(
            r#"
<b>Kylit</b>
/kylit_backup_db — Postgres dump under KYLIT_BACKUP_ROOT
/kylit_backup_minio — mc mirror (see deploy env)
/kylit_backup_all — kylit-prod-backup.sh
/kylit_docker — Kylit container status
"#,
        );
    }

    if config.ip_admin_enabled() {
        let be = match config.ip_backend {
            Some(IpFirewallBackend::Ufw) => "ufw",
            Some(IpFirewallBackend::Crowdsec) => "crowdsec",
            None => "none",
        };
        s.push_str(&format!(
            r#"
<b>IP admin ({be})</b>
/ip_list &lt;password&gt;
/ip_ban &lt;ipv4&gt; &lt;password&gt;
/ip_unban &lt;ipv4&gt; &lt;password&gt;
"#
        ));
    }

    s
}

fn ip_admin_disabled_reply() -> String {
    "ℹ️ IP admin is off. Set MONITOR_IP_BACKEND=ufw or crowdsec and MONITOR_IP_ADMIN_SECRET."
        .to_string()
}

fn ip_password_invalid_reply() -> String {
    "❌ Invalid password.".to_string()
}

fn kylit_disabled_reply() -> String {
    "ℹ️ Kylit commands are off. Set MONITOR_KYLIT_WEBHOOK=1 and see cmd/server-monitor/env.kylit.example."
        .to_string()
}

fn normalize_command(cmd: &str) -> String {
    // Strip bot suffix in group chats: /help@closlamartineBot -> /help
    let no_bot_suffix = cmd.split('@').next().unwrap_or(cmd);
    // Accept -, _, :, and mobile long dashes as separators.
    no_bot_suffix
        .replace(['—', '–'], "-")
        .replace(['-', ':'], "_")
}

async fn run_docker_list(config: &Config) -> String {
    let docker = match DockerClient::connect(&config.docker_socket) {
        Ok(d) => d,
        Err(e) => return format!("❌ <b>Docker</b>\nConnection failed: {}", e),
    };

    let prod_names: Vec<&str> = config.prod_containers.iter().map(|s| s.as_str()).collect();
    let staging_names: Vec<&str> = config.staging_containers.iter().map(|s| s.as_str()).collect();

    let prod = docker.inspect_containers(&prod_names).await;
    let staging = docker.inspect_containers(&staging_names).await;

    let mut lines = vec!["<b>Docker containers</b>".to_string()];

    lines.push("\n<b>Prod</b>".to_string());
    if let Ok(containers) = prod {
        for c in containers {
            lines.push(format_container(&c));
        }
    } else {
        lines.push("  (inspect failed)".to_string());
    }

    lines.push("\n<b>Staging</b>".to_string());
    if let Ok(containers) = staging {
        for c in containers {
            lines.push(format_container(&c));
        }
    } else {
        lines.push("  (inspect failed)".to_string());
    }

    lines.join("\n")
}

fn format_container(c: &crate::docker::ContainerInfo) -> String {
    let (emoji, status_str) = match &c.status {
        ContainerStatus::Running => {
            let health = if c.name.contains("_db") {
                match &c.health {
                    HealthStatus::Healthy => "",
                    HealthStatus::Unhealthy => " (unhealthy)",
                    HealthStatus::Starting => " (starting)",
                    HealthStatus::None => "",
                }
            } else {
                ""
            };
            ("✅", format!("running{}", health))
        }
        ContainerStatus::Exited => ("❌", "exited".to_string()),
        ContainerStatus::Restarting => ("🔄", "restarting".to_string()),
        ContainerStatus::Paused => ("⏸", "paused".to_string()),
        ContainerStatus::Dead => ("❌", "dead".to_string()),
        ContainerStatus::Other(s) => ("❓", s.clone()),
    };
    format!("  {} {} {}", emoji, c.name, status_str)
}

async fn run_kylit_backup_db(config: &Config) -> String {
    let backup_root = &config.kylit_backup_root;
    if let Err(e) = std::fs::create_dir_all(backup_root) {
        return format!("❌ <b>Kylit DB backup</b>\n{}", e);
    }
    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let run_dir = backup_root.join(timestamp.to_string());
    if let Err(e) = std::fs::create_dir_all(&run_dir) {
        return format!("❌ <b>Kylit DB backup</b>\n{}", e);
    }
    let sql_path = run_dir.join("postgres.sql");
    let log_path = run_dir.join("postgres.log");
    let sh = format!(
        "docker exec {} pg_dump -U {} -d {} --clean --if-exists --no-owner --no-acl > {} 2> {}",
        config.kylit_pg_container,
        shell_escape(&config.kylit_pg_user),
        shell_escape(&config.kylit_pg_db),
        sql_path.display(),
        log_path.display()
    );
    let output = Command::new("sh").args(["-c", &sh]).output();
    match output {
        Ok(o) if o.status.success() => {
            let size = std::fs::metadata(&sql_path).map(|m| m.len()).unwrap_or(0);
            format!(
                "✅ <b>Kylit DB backup</b>\n<code>{}</code>\n{}",
                sql_path.display(),
                format_size(size)
            )
        }
        Ok(o) => {
            let err = String::from_utf8_lossy(&o.stderr);
            let log_tail = std::fs::read_to_string(&log_path).unwrap_or_default();
            format!(
                "❌ <b>Kylit DB backup failed</b>\n{}\n{}",
                err.trim(),
                log_tail.trim().chars().take(500).collect::<String>()
            )
        }
        Err(e) => format!("❌ <b>Kylit DB backup failed</b>\n{}", e),
    }
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

async fn run_kylit_backup_minio(config: &Config) -> String {
    let script = config
        .kylit_deploy_path
        .join("scripts")
        .join("minio-mirror-backup.sh");
    if !script.is_file() {
        return format!(
            "❌ <b>Kylit MinIO backup</b>\nMissing script <code>{}</code>",
            script.display()
        );
    }
    let output = Command::new("bash")
        .current_dir(&config.kylit_deploy_path)
        .arg(&script)
        .output();
    match output {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            let tail: String = out.chars().rev().take(800).collect::<String>().chars().rev().collect();
            format!("✅ <b>Kylit MinIO mirror</b>\n<pre>{}</pre>", tail)
        }
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            format!(
                "❌ <b>Kylit MinIO mirror failed</b>\nstderr: {}\nstdout: {}",
                stderr.trim().chars().take(400).collect::<String>(),
                stdout.trim().chars().take(400).collect::<String>()
            )
        }
        Err(e) => format!("❌ <b>Kylit MinIO mirror failed</b>\n{}", e),
    }
}

async fn run_kylit_backup_all(config: &Config) -> String {
    if !config.kylit_backup_script.is_file() {
        return format!(
            "❌ <b>Kylit full backup</b>\nMissing <code>{}</code>",
            config.kylit_backup_script.display()
        );
    }
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(&config.kylit_backup_script);
    cmd.current_dir(&config.kylit_deploy_path);
    cmd.env("KYLIT_BACKUP_ROOT", &config.kylit_backup_root);
    cmd.env("KYLIT_PG_CONTAINER", &config.kylit_pg_container);
    if let Some(p) = &config.kylit_env_file {
        cmd.env("KYLIT_ENV_FILE", p);
    }
    let output = cmd.output();
    match output {
        Ok(o) if o.status.success() => {
            let out = String::from_utf8_lossy(&o.stdout);
            let tail: String = out.chars().rev().take(1200).collect::<String>().chars().rev().collect();
            format!("✅ <b>Kylit full backup</b>\n<pre>{}</pre>", tail)
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            let stdout = String::from_utf8_lossy(&o.stdout);
            format!(
                "❌ <b>Kylit full backup failed</b>\nstderr: {}\nstdout: {}",
                stderr.trim().chars().take(500).collect::<String>(),
                stdout.trim().chars().take(500).collect::<String>()
            )
        }
        Err(e) => format!("❌ <b>Kylit full backup failed</b>\n{}", e),
    }
}

async fn run_kylit_docker(config: &Config) -> String {
    let docker = match DockerClient::connect(&config.docker_socket) {
        Ok(d) => d,
        Err(e) => return format!("❌ <b>Kylit Docker</b>\n{}", e),
    };
    let names: Vec<&str> = config.kylit_container_names.iter().map(|s| s.as_str()).collect();
    let list = docker.inspect_containers(&names).await;
    let mut lines = vec!["<b>Kylit containers</b>".to_string()];
    match list {
        Ok(containers) => {
            for c in containers {
                lines.push(format_container(&c));
            }
        }
        Err(e) => lines.push(format!("  (inspect failed: {})", e)),
    }
    lines.join("\n")
}

async fn run_prod_backup(config: &Config) -> String {
    let backup_dir = std::path::Path::new("/tmp/clos-monitor-backups");
    let _ = std::fs::create_dir_all(backup_dir);

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let out_path = backup_dir.join(format!("prod_backup_{}.sql", timestamp));

    // Container has POSTGRES_USER, POSTGRES_DBNAME from .env.prod
    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                "docker exec {} sh -c 'pg_dump -U $POSTGRES_USER -d $POSTGRES_DBNAME --clean --if-exists --no-owner --no-acl' > {} 2>&1",
                config.prod_pg_container,
                out_path.display()
            ),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            format!(
                "✅ <b>Prod backup done</b>\nPath: <code>{}</code>\nSize: {}",
                out_path.display(),
                format_size(size)
            )
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            format!("❌ <b>Prod backup failed</b>\n{}", stderr.trim())
        }
        Err(e) => format!("❌ <b>Prod backup failed</b>\n{}", e),
    }
}

async fn run_staging_backup(config: &Config) -> String {
    let backup_dir = std::path::Path::new("/tmp/clos-monitor-backups");
    let _ = std::fs::create_dir_all(backup_dir);

    let timestamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let out_path = backup_dir.join(format!("staging_backup_{}.sql", timestamp));

    let output = Command::new("sh")
        .args([
            "-c",
            &format!(
                "docker exec {} sh -c 'pg_dump -U $POSTGRES_USER -d $POSTGRES_DBNAME --clean --if-exists --no-owner --no-acl' > {} 2>&1",
                config.staging_pg_container,
                out_path.display()
            ),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            format!(
                "✅ <b>Staging backup done</b>\nPath: <code>{}</code>\nSize: {}",
                out_path.display(),
                format_size(size)
            )
        }
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            format!("❌ <b>Staging backup failed</b>\n{}", stderr.trim())
        }
        Err(e) => format!("❌ <b>Staging backup failed</b>\n{}", e),
    }
}

async fn run_prod_restart(config: &Config) -> String {
    let output = Command::new("bash")
        .args([
            "-c",
            &format!(
                "cd {} && {}",
                config.prod_deploy_path.display(),
                config.prod_compose_restart_sh
            ),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => "✅ <b>Prod containers restarted</b>".to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            format!("❌ <b>Prod restart failed</b>\n{}", stderr.trim())
        }
        Err(e) => format!("❌ <b>Prod restart failed</b>\n{}", e),
    }
}

async fn run_staging_restart(config: &Config) -> String {
    let output = Command::new("bash")
        .args([
            "-c",
            &format!(
                "cd {} && {}",
                config.staging_deploy_path.display(),
                config.staging_compose_restart_sh
            ),
        ])
        .output();

    match output {
        Ok(o) if o.status.success() => "✅ <b>Staging containers restarted</b>".to_string(),
        Ok(o) => {
            let stderr = String::from_utf8_lossy(&o.stderr);
            format!("❌ <b>Staging restart failed</b>\n{}", stderr.trim())
        }
        Err(e) => format!("❌ <b>Staging restart failed</b>\n{}", e),
    }
}

fn format_size(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}
