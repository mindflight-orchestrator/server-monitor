//! Shared diagnostic checks for CLI `diagnose` and Telegram `/self`.

use crate::config::{Config, ReverseProxyKind};
use reqwest::Client;
use serde::Deserialize;
use std::net::SocketAddr;
use std::str::FromStr;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagSeverity {
    Ok,
    Warn,
    Fail,
}

#[derive(Debug, Clone)]
pub struct DiagLine {
    pub severity: DiagSeverity,
    pub message: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DiagSection {
    pub title: String,
    pub lines: Vec<DiagLine>,
}

#[derive(Debug, Clone, Default)]
pub struct DiagReport {
    pub sections: Vec<DiagSection>,
    pub passed: usize,
    pub failed: usize,
    pub warned: usize,
}

impl DiagReport {
    pub fn push_section(&mut self, section: DiagSection) {
        for line in &section.lines {
            match line.severity {
                DiagSeverity::Ok => self.passed += 1,
                DiagSeverity::Warn => self.warned += 1,
                DiagSeverity::Fail => self.failed += 1,
            }
        }
        self.sections.push(section);
    }
}

/// Run all checks and return a structured report (used by CLI and Telegram).
pub async fn run_full_diagnostic(config: &Config) -> DiagReport {
    let mut report = DiagReport::default();
    report.push_section(check_env(config));
    report.push_section(check_pid(config));
    report.push_section(check_webhook_port(config).await);
    report.push_section(check_telegram_api(config).await);
    report.push_section(check_reverse_proxy(config));
    report
}

/// After the webhook listener is up: confirm Telegram `getWebhookInfo` matches `MONITOR_WEBHOOK_EXPECTED_URL`.
pub async fn verify_telegram_webhook_startup(config: &Config) -> Result<(), String> {
    let result = fetch_webhook_result(config).await?;
    let registered = result.url.as_deref().unwrap_or("").trim();
    if registered.is_empty() {
        return Err(
            "Telegram webhook URL is empty — register with setWebhook (see DEPLOY_CHECKLIST.md)"
                .to_string(),
        );
    }
    let expected = config.webhook_expected_url.trim();
    if registered != expected {
        return Err(format!(
            "webhook mismatch: registered={} expected={}",
            registered, expected
        ));
    }
    Ok(())
}

fn check_env(config: &Config) -> DiagSection {
    let mut lines = Vec::new();
    let title = "Environment".to_string();

    let env_paths = [
        std::env::var("MONITOR_ENV_FILE").ok().map(std::path::PathBuf::from),
        Some(std::path::PathBuf::from("/etc/clos-monitor.env")),
        Some(std::path::PathBuf::from(".env")),
    ];
    let loaded_env = env_paths.into_iter().flatten().find(|p| p.exists());
    match loaded_env {
        Some(p) => lines.push(DiagLine {
            severity: DiagSeverity::Ok,
            message: format!("Env file: {} (exists)", p.display()),
            detail: None,
        }),
        None => lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: "No env file found (MONITOR_ENV_FILE / /etc/clos-monitor.env / .env)".to_string(),
            detail: None,
        }),
    }

    match &config.telegram_bot_token {
        Some(token) => lines.push(DiagLine {
            severity: DiagSeverity::Ok,
            message: format!("TELEGRAM_BOT_TOKEN: set ({})", mask_token(&token)),
            detail: None,
        }),
        None => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: "TELEGRAM_BOT_TOKEN: not set — alerts and getWebhookInfo will fail".to_string(),
            detail: None,
        }),
    }

    if config.allowed_chat_ids.is_empty() {
        lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: "TELEGRAM_CHAT_ID / TELEGRAM_ALLOWED_CHAT_IDS: not set — commands ignored"
                .to_string(),
            detail: None,
        });
    } else {
        lines.push(DiagLine {
            severity: DiagSeverity::Ok,
            message: format!("Allowed chat IDs: {}", config.allowed_chat_ids.join(", ")),
            detail: None,
        });
    }

    match &config.webhook_secret {
        Some(secret) => {
            let preview: String = secret.chars().take(8).collect();
            lines.push(DiagLine {
                severity: DiagSeverity::Ok,
                message: format!("MONITOR_WEBHOOK_SECRET: set ({}...)", preview),
                detail: None,
            });
        }
        None => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: "MONITOR_WEBHOOK_SECRET: not set — webhook server will NOT start".to_string(),
            detail: None,
        }),
    }

    lines.push(DiagLine {
        severity: DiagSeverity::Ok,
        message: format!("MONITOR_WEBHOOK_PORT: {}", config.webhook_port),
        detail: None,
    });

    let scope = match config.scope {
        crate::config::MonitorScope::Both => "both",
        crate::config::MonitorScope::Prod => "prod",
        crate::config::MonitorScope::Staging => "staging",
    };
    lines.push(DiagLine {
        severity: DiagSeverity::Ok,
        message: format!("MONITOR_SCOPE: {}", scope),
        detail: None,
    });

    let rp = match config.reverse_proxy {
        ReverseProxyKind::Nginx => "nginx",
        ReverseProxyKind::Traefik => "traefik",
        ReverseProxyKind::None => "none",
    };
    lines.push(DiagLine {
        severity: DiagSeverity::Ok,
        message: format!("MONITOR_REVERSE_PROXY: {}", rp),
        detail: None,
    });

    lines.push(DiagLine {
        severity: DiagSeverity::Ok,
        message: format!(
            "Expected Telegram webhook URL: {}",
            config.webhook_expected_url
        ),
        detail: None,
    });

    DiagSection { title, lines }
}

pub fn mask_token(token: &str) -> String {
    if token.len() <= 4 {
        return "***".to_string();
    }
    let tail: String = token
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("***...{}", tail)
}

fn check_pid(config: &Config) -> DiagSection {
    let title = "Daemon Process".to_string();
    let mut lines = Vec::new();
    let pid_path = &config.pid_file;

    if !pid_path.exists() {
        lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!(
                "PID file not found: {} — daemon may not be running",
                pid_path.display()
            ),
            detail: None,
        });
        return DiagSection { title, lines };
    }

    let pid_str = match std::fs::read_to_string(pid_path) {
        Ok(s) => s.trim().to_string(),
        Err(e) => {
            lines.push(DiagLine {
                severity: DiagSeverity::Fail,
                message: format!("PID file unreadable ({}): {}", pid_path.display(), e),
                detail: None,
            });
            return DiagSection { title, lines };
        }
    };

    let pid: u32 = match pid_str.parse() {
        Ok(p) => p,
        Err(_) => {
            lines.push(DiagLine {
                severity: DiagSeverity::Fail,
                message: format!("PID file contains invalid value: '{}'", pid_str),
                detail: None,
            });
            return DiagSection { title, lines };
        }
    };

    let comm_path = format!("/proc/{}/comm", pid);
    match std::fs::read_to_string(&comm_path) {
        Ok(comm) => {
            let comm = comm.trim();
            let ok = config
                .expected_process_names
                .iter()
                .any(|name| comm == name.as_str() || comm.starts_with(name.as_str()));
            if ok {
                lines.push(DiagLine {
                    severity: DiagSeverity::Ok,
                    message: format!("Daemon running: PID {} ({})", pid, comm),
                    detail: None,
                });
            } else {
                lines.push(DiagLine {
                    severity: DiagSeverity::Warn,
                    message: format!(
                        "PID {} in PID file but process comm is '{}' — expected one of [{}] (stale PID file?)",
                        pid,
                        comm,
                        config.expected_process_names.join(", ")
                    ),
                    detail: None,
                });
            }
        }
        Err(_) => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: format!(
                "PID {} from PID file is not running (process not found in /proc)",
                pid
            ),
            detail: None,
        }),
    }

    DiagSection { title, lines }
}

async fn check_webhook_port(config: &Config) -> DiagSection {
    let title = "Webhook Server (local port)".to_string();
    let mut lines = Vec::new();

    let addr_str = format!("127.0.0.1:{}", config.webhook_port);
    let addr = match SocketAddr::from_str(&addr_str) {
        Ok(a) => a,
        Err(e) => {
            lines.push(DiagLine {
                severity: DiagSeverity::Fail,
                message: format!("Invalid webhook address {}: {}", addr_str, e),
                detail: None,
            });
            return DiagSection { title, lines };
        }
    };

    match tokio::time::timeout(Duration::from_secs(2), tokio::net::TcpStream::connect(addr)).await
    {
        Ok(Ok(_)) => lines.push(DiagLine {
            severity: DiagSeverity::Ok,
            message: format!(
                "Port {} is open — webhook server is listening",
                config.webhook_port
            ),
            detail: None,
        }),
        Ok(Err(e)) => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: format!(
                "Port {} refused: {} — webhook not running?",
                config.webhook_port, e
            ),
            detail: None,
        }),
        Err(_) => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: format!(
                "Port {} timed out — webhook not responding",
                config.webhook_port
            ),
            detail: None,
        }),
    }

    DiagSection { title, lines }
}

#[derive(Debug, Deserialize)]
struct WebhookInfo {
    ok: bool,
    result: Option<WebhookResult>,
}

#[derive(Debug, Deserialize)]
struct WebhookResult {
    url: Option<String>,
    has_custom_certificate: Option<bool>,
    pending_update_count: Option<u64>,
    last_error_date: Option<u64>,
    last_error_message: Option<String>,
    max_connections: Option<u64>,
}

async fn fetch_webhook_result(config: &Config) -> Result<WebhookResult, String> {
    let token = config
        .telegram_bot_token
        .as_ref()
        .ok_or_else(|| "TELEGRAM_BOT_TOKEN not set".to_string())?
        .clone();

    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let url = format!("https://api.telegram.org/bot{}/getWebhookInfo", token);
    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Telegram API unreachable: {}", e))?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err("Telegram API 401 — TELEGRAM_BOT_TOKEN invalid".to_string());
    }
    if !status.is_success() {
        return Err(format!("Telegram API HTTP {}", status));
    }

    let info: WebhookInfo = response
        .json()
        .await
        .map_err(|e| format!("Could not parse Telegram response: {}", e))?;

    if !info.ok {
        return Err("Telegram API response: ok=false".to_string());
    }

    info.result
        .ok_or_else(|| "Telegram API returned empty result".to_string())
}

async fn check_telegram_api(config: &Config) -> DiagSection {
    let title = "Telegram API (getWebhookInfo)".to_string();
    let mut lines = Vec::new();

    let result = match fetch_webhook_result(config).await {
        Ok(r) => r,
        Err(e) => {
            lines.push(DiagLine {
                severity: DiagSeverity::Fail,
                message: e,
                detail: None,
            });
            return DiagSection { title, lines };
        }
    };

    let webhook_url = result.url.as_deref().unwrap_or("(not set)").trim();
    let expected_url = config.webhook_expected_url.trim();
    let pending = result.pending_update_count.unwrap_or(0);
    let custom_cert = result.has_custom_certificate.unwrap_or(false);
    let max_conn = result.max_connections.unwrap_or(0);

    let mut detail_lines = vec![
        format!("url:                    {}", webhook_url),
        format!("has_custom_certificate: {}", custom_cert),
        format!("pending_update_count:    {}", pending),
        format!("max_connections:         {}", max_conn),
    ];

    if let Some(err_msg) = &result.last_error_message {
        if let Some(err_date) = result.last_error_date {
            let ts = chrono::DateTime::from_timestamp(err_date as i64, 0)
                .map(|dt: chrono::DateTime<chrono::Utc>| {
                    dt.format("%Y-%m-%d %H:%M:%S UTC").to_string()
                })
                .unwrap_or_else(|| format!("unix:{}", err_date));
            detail_lines.push(format!("last_error_date:         {}", ts));
        }
        detail_lines.push(format!("last_error_message:      {}", err_msg));
    } else {
        detail_lines.push("last_error_message:      (none)".to_string());
    }

    let detail = detail_lines.join("\n");

    if webhook_url == "(not set)" || webhook_url.is_empty() {
        lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: format!(
                "Webhook not registered. Expected URL: {}",
                expected_url
            ),
            detail: Some(detail),
        });
    } else if webhook_url != expected_url {
        lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!(
                "Webhook URL mismatch — registered: {}  expected: {}",
                webhook_url, expected_url
            ),
            detail: Some(detail),
        });
    } else {
        lines.push(DiagLine {
            severity: DiagSeverity::Ok,
            message: "Telegram webhook URL matches MONITOR_WEBHOOK_EXPECTED_URL".to_string(),
            detail: Some(detail),
        });
    }

    if pending > 10 {
        lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!(
                "{} pending updates — webhook may be failing",
                pending
            ),
            detail: None,
        });
    }

    if result.last_error_message.is_some() {
        lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: "Telegram reports last_error_message (see detail above)".to_string(),
            detail: None,
        });
    }

    DiagSection { title, lines }
}

fn check_reverse_proxy(config: &Config) -> DiagSection {
    match config.reverse_proxy {
        ReverseProxyKind::Nginx => check_nginx_proxy(),
        ReverseProxyKind::Traefik => check_traefik_proxy(config),
        ReverseProxyKind::None => DiagSection {
            title: "Reverse proxy".to_string(),
            lines: vec![DiagLine {
                severity: DiagSeverity::Ok,
                message: "MONITOR_REVERSE_PROXY=none — skipped local routing file scan".to_string(),
                detail: None,
            }],
        },
    }
}

fn check_nginx_proxy() -> DiagSection {
    let title = "Reverse proxy (nginx)".to_string();
    let mut lines = Vec::new();

    let grep = std::process::Command::new("grep")
        .args(["-rl", "monitor/webhook", "/etc/nginx/sites-enabled/"])
        .output();

    match grep {
        Ok(out) if out.status.success() => {
            let files = String::from_utf8_lossy(&out.stdout);
            let file_list: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
            if file_list.is_empty() {
                lines.push(DiagLine {
                    severity: DiagSeverity::Fail,
                    message: "No nginx site contains 'monitor/webhook'".to_string(),
                    detail: None,
                });
            } else {
                lines.push(DiagLine {
                    severity: DiagSeverity::Ok,
                    message: format!(
                        "/monitor/webhook block found in: {}",
                        file_list.join(", ")
                    ),
                    detail: None,
                });
            }
        }
        Ok(_) => lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: "No nginx site contains 'monitor/webhook' (grep exit non-zero)".to_string(),
            detail: None,
        }),
        Err(e) => lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!(
                "Could not grep nginx config ({}): skipped (permissions?)",
                e
            ),
            detail: None,
        }),
    }

    let secret_path = std::path::Path::new("/etc/nginx/telegram-webhook-secret.conf");
    if !secret_path.exists() {
        lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: "/etc/nginx/telegram-webhook-secret.conf missing".to_string(),
            detail: None,
        });
        return DiagSection { title, lines };
    }

    match std::fs::read_to_string(secret_path) {
        Ok(content) => {
            let content = content.trim();
            if content.is_empty() || content.contains("run-monitor-deploy-to-update") {
                lines.push(DiagLine {
                    severity: DiagSeverity::Warn,
                    message: "telegram-webhook-secret.conf looks like a placeholder".to_string(),
                    detail: None,
                });
            } else {
                lines.push(DiagLine {
                    severity: DiagSeverity::Ok,
                    message: "/etc/nginx/telegram-webhook-secret.conf has a value".to_string(),
                    detail: None,
                });
            }
        }
        Err(e) => lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!(
                "telegram-webhook-secret.conf unreadable ({}): skipped",
                e
            ),
            detail: None,
        }),
    }

    DiagSection { title, lines }
}

fn check_traefik_proxy(config: &Config) -> DiagSection {
    let title = "Reverse proxy (Traefik)".to_string();
    let mut lines = Vec::new();

    let Some(scan_path) = config.traefik_config_scan_path.as_ref() else {
        lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: "MONITOR_TRAEFIK_CONFIG_SCAN_PATH not set — set it to a file or directory containing your router rule for /monitor/webhook".to_string(),
            detail: None,
        });
        return DiagSection { title, lines };
    };

    if !scan_path.exists() {
        lines.push(DiagLine {
            severity: DiagSeverity::Fail,
            message: format!(
                "MONITOR_TRAEFIK_CONFIG_SCAN_PATH does not exist: {}",
                scan_path.display()
            ),
            detail: None,
        });
        return DiagSection { title, lines };
    }

    let grep = std::process::Command::new("grep")
        .arg("-rl")
        .arg("monitor/webhook")
        .arg(scan_path)
        .output();

    match grep {
        Ok(out) if out.status.success() => {
            let files = String::from_utf8_lossy(&out.stdout);
            let file_list: Vec<&str> = files.lines().filter(|l| !l.is_empty()).collect();
            if file_list.is_empty() {
                lines.push(DiagLine {
                    severity: DiagSeverity::Warn,
                    message: "No file under MONITOR_TRAEFIK_CONFIG_SCAN_PATH contains 'monitor/webhook'".to_string(),
                    detail: None,
                });
            } else {
                lines.push(DiagLine {
                    severity: DiagSeverity::Ok,
                    message: format!(
                        "monitor/webhook referenced in: {}",
                        file_list.join(", ")
                    ),
                    detail: None,
                });
            }
        }
        Ok(_) => lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: "grep found no monitor/webhook in Traefik scan path".to_string(),
            detail: None,
        }),
        Err(e) => lines.push(DiagLine {
            severity: DiagSeverity::Warn,
            message: format!("Could not grep Traefik config ({}): skipped", e),
            detail: None,
        }),
    }

    lines.push(DiagLine {
        severity: DiagSeverity::Ok,
        message: "Nginx secret include not used for Traefik — forward X-Telegram-Bot-Api-Secret-Token in your middleware/router".to_string(),
        detail: None,
    });

    DiagSection { title, lines }
}

/// Format report for Telegram HTML (truncated).
pub fn format_report_telegram(report: &DiagReport, max_len: usize) -> String {
    let prefix = format!(
        "<b>Monitor self-check</b> v{}\n",
        env!("CARGO_PKG_VERSION")
    );
    let mut body = String::new();
    for sec in &report.sections {
        body.push_str(&format!("\n<b>{}</b>\n", sec.title));
        for line in &sec.lines {
            let icon = match line.severity {
                DiagSeverity::Ok => "✅",
                DiagSeverity::Warn => "⚠️",
                DiagSeverity::Fail => "❌",
            };
            body.push_str(&format!("{} {}\n", icon, line.message));
            if let Some(d) = &line.detail {
                let snippet: String = d.chars().take(600).collect();
                body.push_str(&format!("<pre>{}</pre>\n", html_escape_pre(&snippet)));
            }
        }
    }
    body.push_str(&format!(
        "\n<i>{} ok, {} warn, {} fail</i>",
        report.passed, report.warned, report.failed
    ));

    let full = prefix + &body;
    truncate_msg(&full, max_len)
}

fn html_escape_pre(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn truncate_msg(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    format!("{}…", s.chars().take(max.saturating_sub(1)).collect::<String>())
}
