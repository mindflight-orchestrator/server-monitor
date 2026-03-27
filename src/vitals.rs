//! Server vitals: memory, disk, systemd services, SSL certs.

use crate::config::Config;
use std::path::Path;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};
use tracing::{debug, warn};

/// Systemd services to check (Ubuntu: ssh not sshd; nginx for reverse proxy).
const REQUIRED_SERVICES: &[&str] = &["docker", "ssh", "nginx"];
const REQUIRED_SERVICES_DEV: &[&str] = &["docker"]; // ssh optional locally, nginx usually not in dev

/// Run all server vital checks.
pub fn run_vitals(config: &Config) -> Vec<String> {
    let mut failures = Vec::new();

    // Memory
    if let Err(e) = check_memory(config, &mut failures) {
        failures.push(format!("Memory check failed: {}", e));
    }

    // Disk (space + read-only)
    if let Err(e) = check_disk(config, &mut failures) {
        failures.push(format!("Disk check failed: {}", e));
    }

    // Systemd services (skip nginx in dev - often not running locally)
    check_services(config, &mut failures);

    // SSL cert expiration (skip in dev - /etc/letsencrypt usually not present)
    if !config.dev {
        check_ssl_certs(config, &mut failures);
    }

    failures
}

fn check_memory(config: &Config, failures: &mut Vec<String>) -> Result<(), String> {
    let min_mem = config.min_available_memory_bytes;
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();

    let available = sys.available_memory();
    let total = sys.total_memory();

    debug!(
        available_mb = available / 1024 / 1024,
        total_mb = total / 1024 / 1024,
        "memory check"
    );

    if available < min_mem {
        failures.push(format!(
            "Low memory: {} MB available (min {} MB)",
            available / 1024 / 1024,
            min_mem / 1024 / 1024
        ));
    }

    Ok(())
}

fn check_disk(config: &Config, failures: &mut Vec<String>) -> Result<(), String> {
    let min_disk = config.min_available_disk_bytes;

    let disks = sysinfo::Disks::new_with_refreshed_list();

    // Focus on root (/) and data mounts. Skip Docker overlay/volumes - they report 0 free.
    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy();

        if mount_str != "/" && !mount_str.starts_with("/var") && !mount_str.starts_with("/home") {
            continue;
        }
        if mount_str.starts_with("/var/lib/docker") {
            continue;
        }

        if disk.is_read_only() {
            failures.push(format!(
                "Disk read-only: {} ({}) - possible crash/failure",
                disk.name().to_string_lossy(),
                mount_str
            ));
        }

        let available = disk.available_space();
        let total = disk.total_space();

        debug!(
            mount = %mount_str,
            available_gb = available / 1024 / 1024 / 1024,
            total_gb = total / 1024 / 1024 / 1024,
            "disk check"
        );

        if available < min_disk {
            failures.push(format!(
                "Low disk space: {} has {} MB free (min {} MB)",
                mount_str,
                available / 1024 / 1024,
                min_disk / 1024 / 1024
            ));
        }
    }

    Ok(())
}

fn check_services(config: &Config, failures: &mut Vec<String>) {
    let services = if config.dev {
        REQUIRED_SERVICES_DEV
    } else {
        REQUIRED_SERVICES
    };
    for service in services {
        match Command::new("systemctl")
            .args(["is-active", "--quiet", service])
            .status()
        {
            Ok(status) => {
                if !status.success() {
                    failures.push(format!("Service not running: {}", service));
                }
            }
            Err(e) => {
                warn!(service = %service, error = %e, "systemctl failed");
                failures.push(format!("Cannot check service {}: {}", service, e));
            }
        }
    }
}

/// Check Let's Encrypt / Certbot SSL certs in /etc/letsencrypt/live/
fn check_ssl_certs(config: &Config, failures: &mut Vec<String>) {
    let live_dir = Path::new("/etc/letsencrypt/live");
    if !live_dir.exists() {
        return;
    }

    let Ok(entries) = std::fs::read_dir(live_dir) else {
        return;
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let domain = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        // Skip README
        if domain == "README" {
            continue;
        }
        let cert_path = path.join("fullchain.pem");
        if !cert_path.exists() {
            continue;
        }

        let output = match Command::new("openssl")
            .args([
                "x509",
                "-in",
                cert_path.to_str().unwrap_or(""),
                "-noout",
                "-enddate",
            ])
            .output()
        {
            Ok(o) => o,
            Err(e) => {
                warn!(domain = %domain, error = %e, "openssl x509 failed");
                continue;
            }
        };

        if !output.status.success() {
            continue;
        }

        let out_str = String::from_utf8_lossy(&output.stdout);
        // Format: notAfter=Mon Mar 12 15:00:00 2026 GMT
        let not_after = match out_str.strip_prefix("notAfter=") {
            Some(s) => s.trim(),
            None => continue,
        };

        let expiry_secs = match parse_openssl_date(not_after) {
            Some(t) => t,
            None => continue,
        };

        let days_left = (expiry_secs - now) / 86400;

        if days_left < 0 {
            failures.push(format!(
                "SSL cert expired: {} ({} days ago) - renew with certbot",
                domain, -days_left
            ));
        } else if days_left < config.cert_warn_days as i64 {
            failures.push(format!(
                "SSL cert expires soon: {} ({} days left) - renew with certbot",
                domain, days_left
            ));
        }
    }
}

/// Parse openssl date format: "Mar 12 15:00:00 2026 GMT" or "Mon Mar 12 15:00:00 2026 GMT"
fn parse_openssl_date(s: &str) -> Option<i64> {
    let s = s.trim_end_matches(" GMT").trim();
    chrono::DateTime::parse_from_str(s, "%b %d %H:%M:%S %Y")
        .or_else(|_| chrono::DateTime::parse_from_str(s, "%a %b %d %H:%M:%S %Y"))
        .ok()
        .map(|dt| dt.timestamp())
}

// --- Stats (for Telegram commands) ---

/// Format disk space for /, /var, /home (skip Docker overlay).
pub fn format_space_left() -> String {
    let disks = sysinfo::Disks::new_with_refreshed_list();
    let mut lines = vec!["<b>Disk space</b>".to_string()];

    for disk in disks.list() {
        let mount = disk.mount_point();
        let mount_str = mount.to_string_lossy();

        if mount_str != "/" && !mount_str.starts_with("/var") && !mount_str.starts_with("/home") {
            continue;
        }
        if mount_str.starts_with("/var/lib/docker") {
            continue;
        }

        let available = disk.available_space();
        let total = disk.total_space();
        let avail_gb = available as f64 / 1024.0 / 1024.0 / 1024.0;
        let total_gb = total as f64 / 1024.0 / 1024.0 / 1024.0;
        lines.push(format!(
            "{} {:.1} GB free / {:.1} GB",
            mount_str, avail_gb, total_gb
        ));
    }

    if lines.len() == 1 {
        lines.push("(no mounts)".to_string());
    }
    lines.join("\n")
}

/// Format memory stats.
pub fn format_memory() -> String {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();

    let available = sys.available_memory();
    let total = sys.total_memory();
    let used = total.saturating_sub(available);

    let avail_gb = available as f64 / 1024.0 / 1024.0 / 1024.0;
    let total_gb = total as f64 / 1024.0 / 1024.0 / 1024.0;
    let used_gb = used as f64 / 1024.0 / 1024.0 / 1024.0;

    format!(
        "<b>Memory</b>\n{:.2} GB used / {:.2} GB total\n{:.2} GB available",
        used_gb, total_gb, avail_gb
    )
}

/// Format uptime and load average.
pub fn format_uptime_stats() -> String {
    let uptime_secs = sysinfo::System::uptime();
    let days = uptime_secs / 86400;
    let hours = (uptime_secs % 86400) / 3600;
    let mins = (uptime_secs % 3600) / 60;

    let uptime_str = if days > 0 {
        format!("{}d {}h {}m", days, hours, mins)
    } else if hours > 0 {
        format!("{}h {}m", hours, mins)
    } else {
        format!("{}m", mins)
    };

    let load = sysinfo::System::load_average();
    let load_str = format!(
        "{:.2} {:.2} {:.2} (1/5/15 min)",
        load.one, load.five, load.fifteen
    );

    format!("<b>Uptime</b>\n{}\n\n<b>Load</b>\n{}", uptime_str, load_str)
}

/// Format SSL cert expiry info (all certs in /etc/letsencrypt/live/).
pub fn format_certs(config: &Config) -> String {
    let live_dir = Path::new("/etc/letsencrypt/live");
    if !live_dir.exists() {
        return "<b>SSL certs</b>\n(no certs found)".to_string();
    }

    let Ok(entries) = std::fs::read_dir(live_dir) else {
        return "<b>SSL certs</b>\n(cannot read)".to_string();
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;

    let mut lines = vec!["<b>SSL certs</b>".to_string()];

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let domain = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        if domain == "README" {
            continue;
        }
        let cert_path = path.join("fullchain.pem");
        if !cert_path.exists() {
            continue;
        }

        let output = match Command::new("openssl")
            .args([
                "x509",
                "-in",
                cert_path.to_str().unwrap_or(""),
                "-noout",
                "-enddate",
            ])
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => continue,
        };

        let out_str = String::from_utf8_lossy(&output.stdout);
        let not_after = match out_str.strip_prefix("notAfter=") {
            Some(s) => s.trim(),
            None => continue,
        };

        let expiry_secs = match parse_openssl_date(not_after) {
            Some(t) => t,
            None => continue,
        };

        let days_left = (expiry_secs - now) / 86400;

        let status = if days_left < 0 {
            format!("❌ expired {}d ago", -days_left)
        } else if days_left < config.cert_warn_days as i64 {
            format!("⚠️ {}d left", days_left)
        } else {
            format!("✅ {}d left", days_left)
        };

        lines.push(format!("{}: {}", domain, status));
    }

    if lines.len() == 1 {
        lines.push("(no certs)".to_string());
    }
    lines.join("\n")
}

/// Format server vitals status (for /status:server).
pub fn format_server_status(config: &Config) -> String {
    let failures = run_vitals(config);
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC");
    let mut msg = format!("<b>Server status</b>\nTime: {}\n\n", now);
    if failures.is_empty() {
        msg.push_str("✅ All OK");
    } else {
        msg.push_str("❌ Failures:\n");
        for f in &failures {
            msg.push_str(&format!("  • {}\n", f));
        }
    }
    msg
}
