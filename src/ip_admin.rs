//! Password-gated IP list / ban / unban via UFW or CrowdSec (`cscli`).

use crate::config::{Config, IpFirewallBackend};
use std::process::Command;
use std::time::Duration;
use subtle::ConstantTimeEq;
use tokio::task;

const OUTPUT_CAP: usize = 3500;
const CMD_TIMEOUT: Duration = Duration::from_secs(45);

pub fn verify_password(config: &Config, provided: &str) -> bool {
    let Some(expected) = config.ip_admin_secret.as_ref() else {
        return false;
    };
    if expected.is_empty() {
        return false;
    }
    if provided.len() != expected.len() {
        return false;
    }
    bool::from(expected.as_bytes().ct_eq(provided.as_bytes()))
}

pub async fn ip_list(config: &Config) -> String {
    let Some(backend) = config.ip_backend else {
        return "❌ IP admin not configured.".to_string();
    };
    match backend {
        IpFirewallBackend::Ufw => run_program_args("ufw", &["status", "numbered"]).await,
        IpFirewallBackend::Crowdsec => run_program_args("cscli", &["decisions", "list"]).await,
    }
}

pub async fn ip_ban(config: &Config, ip: &str) -> String {
    if !looks_like_ip(ip) {
        return format!("❌ Invalid IP: <code>{}</code>", ip);
    }
    let Some(backend) = config.ip_backend else {
        return "❌ IP admin not configured.".to_string();
    };
    match backend {
        IpFirewallBackend::Ufw => {
            let script = format!(
                "printf 'y\\n' | ufw deny from {} to any 2>&1",
                shell_escape_ip(ip)
            );
            run_sh_c(script).await
        }
        IpFirewallBackend::Crowdsec => {
            let script = format!(
                "cscli decisions add --ip {} -d {} --reason {} 2>&1",
                shell_escape(ip),
                shell_escape(&config.crowdsec_ban_duration),
                shell_escape(&config.crowdsec_ban_reason)
            );
            run_sh_c(script).await
        }
    }
}

pub async fn ip_unban(config: &Config, ip: &str) -> String {
    if !looks_like_ip(ip) {
        return format!("❌ Invalid IP: <code>{}</code>", ip);
    }
    let Some(backend) = config.ip_backend else {
        return "❌ IP admin not configured.".to_string();
    };
    match backend {
        IpFirewallBackend::Ufw => {
            let script = format!(
                "printf 'y\\n' | ufw delete deny from {} to any 2>&1",
                shell_escape_ip(ip)
            );
            run_sh_c(script).await
        }
        IpFirewallBackend::Crowdsec => {
            let script = format!("cscli decisions delete --ip {} 2>&1", shell_escape(ip));
            run_sh_c(script).await
        }
    }
}

fn looks_like_ip(s: &str) -> bool {
    let parts: Vec<&str> = s.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| p.parse::<u8>().is_ok())
}

fn shell_escape(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}

fn shell_escape_ip(s: &str) -> String {
    if s.chars().all(|c| c.is_ascii_digit() || c == '.') {
        s.to_string()
    } else {
        shell_escape(s)
    }
}

async fn run_program_args(program: &str, args: &[&str]) -> String {
    let program = program.to_string();
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    match tokio::time::timeout(
        CMD_TIMEOUT,
        task::spawn_blocking(move || exec_capture(&program, &args)),
    )
    .await
    {
        Ok(Ok(Ok(s))) => wrap_output(&s),
        Ok(Ok(Err(e))) => format!("❌ <pre>{}</pre>", html_escape(&e)),
        Ok(Err(e)) => format!("❌ Task join error: {}", e),
        Err(_) => "❌ Command timed out".to_string(),
    }
}

async fn run_sh_c(script: String) -> String {
    match tokio::time::timeout(
        CMD_TIMEOUT,
        task::spawn_blocking(move || {
            exec_capture("sh", &["-c".to_string(), script])
        }),
    )
    .await
    {
        Ok(Ok(Ok(s))) => wrap_output(&s),
        Ok(Ok(Err(e))) => format!("❌ <pre>{}</pre>", html_escape(&e)),
        Ok(Err(e)) => format!("❌ Task join error: {}", e),
        Err(_) => "❌ Command timed out".to_string(),
    }
}

fn exec_capture(program: &str, args: &[String]) -> Result<String, String> {
    let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = Command::new(program)
        .args(arg_refs)
        .output()
        .map_err(|e| e.to_string())?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let status = if out.status.success() { "ok" } else { "fail" };
    Ok(format!(
        "[{}] status={}\nstdout:\n{}\nstderr:\n{}",
        status,
        out.status,
        stdout,
        stderr
    ))
}

fn wrap_output(s: &str) -> String {
    let t = s.trim();
    let capped = if t.len() > OUTPUT_CAP {
        format!(
            "{}…",
            t.chars()
                .take(OUTPUT_CAP.saturating_sub(1))
                .collect::<String>()
        )
    } else {
        t.to_string()
    };
    format!("<pre>{}</pre>", html_escape(&capped))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
