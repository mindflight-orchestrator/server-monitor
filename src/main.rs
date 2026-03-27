//! ClosLamartine Docker monitor - checks prod/staging containers and sends Telegram alerts.

mod checks;
mod config;
mod diagnose;
mod diagnose_core;
mod docker;
mod ip_admin;
mod telegram;
mod vitals;
mod webhook;

use crate::config::MonitorScope;
use clap::Parser;
use std::path::Path;
use std::time::Duration;
use tracing::{info, warn, Level};
use tracing_subscriber::FmtSubscriber;

/// Load .env from first existing path. Order: MONITOR_ENV_FILE env, /etc/clos-monitor.env, .env
fn load_env_file() {
    let paths: Vec<std::path::PathBuf> = [
        std::env::var("MONITOR_ENV_FILE")
            .ok()
            .map(std::path::PathBuf::from),
        Some(Path::new("/etc/clos-monitor.env").to_path_buf()),
        Some(Path::new(".env").to_path_buf()),
    ]
    .into_iter()
    .flatten()
    .collect();

    for path in paths {
        if path.exists() {
            if dotenvy::from_path(&path).is_ok() {
                info!(path = %path.display(), "loaded env file");
            }
            return;
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "clos-monitor")]
#[command(
    about = "Monitor ClosLamartine Docker containers (prod/staging) and send Telegram alerts"
)]
struct Cli {
    /// Dev mode: check local docker-compose (backend 58081, frontend 58080)
    #[arg(long)]
    dev: bool,

    /// Monitor scope: both, prod, or staging.
    #[arg(long, value_parser = ["both", "prod", "staging"])]
    scope: Option<String>,

    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand, Debug)]
enum Command {
    /// Run checks once and print status
    Check,

    /// Daemon mode: run checks periodically and send Telegram alerts on failure
    Run,

    /// Diagnose Telegram webhook chain (env, PID, port, API, nginx or Traefik)
    Diagnose,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let subscriber = FmtSubscriber::builder()
        .with_max_level(Level::INFO)
        .with_target(false)
        .with_thread_ids(false)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env().add_directive(Level::INFO.into()),
        )
        .finish();
    tracing::subscriber::set_global_default(subscriber)?;

    load_env_file();

    let cli = Cli::parse();
    let config = {
        let c = config::Config::from_env();
        let dev = c.dev || cli.dev;
        let scope = cli
            .scope
            .as_deref()
            .map(MonitorScope::from_str)
            .unwrap_or(c.scope);
        c.with_dev(dev).with_scope(scope)
    };

    match cli.command {
        Command::Check => run_check(&config).await,
        Command::Run => run_daemon(&config).await,
        Command::Diagnose => {
            diagnose::run(&config).await;
            Ok(())
        }
    }
}

async fn run_check(
    config: &config::Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let result = checks::run_checks(config).await;

    if result.all_ok() {
        println!("OK: all checks passed");
        return Ok(());
    }

    if !result.server_ok {
        println!("SERVER FAILURES:");
        for f in &result.server_failures {
            println!("  - {}", f);
        }
    }
    if !result.prod_ok {
        println!("PROD FAILURES:");
        for f in &result.prod_failures {
            println!("  - {}", f);
        }
    }
    if !result.staging_ok {
        println!("STAGING FAILURES:");
        for f in &result.staging_failures {
            println!("  - {}", f);
        }
    }

    if config.telegram_configured() {
        let mut state = checks::AlertState::default();
        checks::send_alerts_on_transition(config, &result, &mut state).await;
    }

    std::process::exit(1);
}

async fn run_daemon(
    config: &config::Config,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    if !config.telegram_configured() {
        eprintln!(
            "Warning: TELEGRAM_BOT_TOKEN and TELEGRAM_CHAT_ID not set - alerts will not be sent"
        );
    }

    // Write PID file so monitor-kill can target this exact process
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&config.pid_file, pid.to_string()) {
        eprintln!("Warning: failed to write PID file {}: {}", config.pid_file.display(), e);
    } else {
        info!(pid = pid, path = %config.pid_file.display(), "wrote PID file");
    }

    let interval = Duration::from_secs(config.interval_secs);
    info!(
        interval_secs = config.interval_secs,
        "starting monitor daemon"
    );

    // Spawn webhook server if configured
    if config.webhook_enabled() {
        let config = config.clone();
        let state = webhook::WebhookState {
            config: config.clone(),
            http_client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("reqwest client"),
        };
        let listener = tokio::net::TcpListener::bind(format!("127.0.0.1:{}", config.webhook_port))
            .await
            .map_err(|e| format!("Failed to bind webhook port {}: {}", config.webhook_port, e))?;
        info!(port = config.webhook_port, "webhook server listening");
        tokio::spawn(async move {
            let app = webhook::router(state);
            axum::serve(listener, app)
                .await
                .expect("webhook server error");
        });

        if config.webhook_startup_verify {
            if config.telegram_bot_token.is_some() {
                tokio::time::sleep(Duration::from_millis(400)).await;
                match diagnose_core::verify_telegram_webhook_startup(&config).await {
                    Ok(()) => info!(
                        url = %config.webhook_expected_url,
                        "Telegram webhook registration OK"
                    ),
                    Err(e) => {
                        warn!(
                            error = %e,
                            expected = %config.webhook_expected_url,
                            "Telegram webhook registration check failed — run `clos-monitor diagnose` or fix setWebhook"
                        );
                        if config.webhook_startup_strict {
                            return Err(e.into());
                        }
                    }
                }
            } else {
                warn!("webhook startup verification skipped: TELEGRAM_BOT_TOKEN not set");
            }
        }
    } else {
        info!("webhook disabled (MONITOR_WEBHOOK_SECRET not set)");
    }

    let mut alert_state = checks::AlertState::default();

    loop {
        let result = checks::run_checks(config).await;

        checks::send_alerts_on_transition(config, &result, &mut alert_state).await;

        tokio::time::sleep(interval).await;
    }
}
