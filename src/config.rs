//! Configuration loaded from environment variables.

use std::env;
use std::path::PathBuf;

/// Environment scope monitored by this instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MonitorScope {
    Both,
    Prod,
    Staging,
}

impl MonitorScope {
    pub fn from_str(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "prod" | "production" => Self::Prod,
            "staging" => Self::Staging,
            _ => Self::Both,
        }
    }
}

/// Backend for password-gated IP admin commands (`/ip_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpFirewallBackend {
    Ufw,
    Crowdsec,
}

impl IpFirewallBackend {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "ufw" => Some(Self::Ufw),
            "crowdsec" => Some(Self::Crowdsec),
            _ => None,
        }
    }
}

/// Which reverse proxy `diagnose` / `/self` should check on disk (local routing to the webhook).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReverseProxyKind {
    /// grep `/etc/nginx/sites-enabled/` + `telegram-webhook-secret.conf`
    Nginx,
    /// grep `MONITOR_TRAEFIK_CONFIG_SCAN_PATH` for `monitor/webhook`
    Traefik,
    /// Skip local proxy file checks (e.g. cloud LB only)
    None,
}

impl ReverseProxyKind {
    pub fn from_str(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "nginx" => Some(Self::Nginx),
            "traefik" => Some(Self::Traefik),
            "none" | "off" => Some(Self::None),
            _ => None,
        }
    }
}

/// Monitor configuration.
#[derive(Debug, Clone)]
pub struct Config {
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
    /// Allowed chat IDs for webhook commands (TELEGRAM_CHAT_ID or comma-separated TELEGRAM_ALLOWED_CHAT_IDS).
    pub allowed_chat_ids: Vec<String>,
    pub interval_secs: u64,
    pub docker_socket: PathBuf,
    /// Min available memory (bytes) before alert. Default: 100 MiB.
    pub min_available_memory_bytes: u64,
    /// Min available disk space (bytes) before alert. Default: 1 GiB.
    pub min_available_disk_bytes: u64,
    /// Warn when SSL cert expires within this many days. Default: 30.
    pub cert_warn_days: u32,
    /// Dev mode: check local docker-compose (backend 58081, frontend 58080), skip nginx/SSL.
    pub dev: bool,
    /// Webhook HTTP server port. Default: 9090.
    pub webhook_port: u16,
    /// Secret token for webhook validation (X-Telegram-Bot-Api-Secret-Token). Required when webhook enabled.
    pub webhook_secret: Option<String>,
    /// Prod deploy path for backup/restart. Default: /home/closlamartine/app.
    pub prod_deploy_path: PathBuf,
    /// Staging deploy path for backup/restart. Default: /home/clsmstaging/app.
    pub staging_deploy_path: PathBuf,
    /// Scope of monitored environments (both|prod|staging). Default: both.
    pub scope: MonitorScope,
    /// Production domain shown in alert messages.
    pub prod_domain: String,
    /// Staging domain shown in alert messages.
    pub staging_domain: String,
    /// Path for PID file (daemon mode). Default: /tmp/clos-monitor.pid.
    pub pid_file: PathBuf,
    /// Expected `comm` names for PID file check / diagnose (MONITOR_PROCESS_COMM comma-separated).
    pub expected_process_names: Vec<String>,

    /// When true, Telegram webhook exposes /kylit_* backup commands (MONITOR_KYLIT_WEBHOOK=1).
    pub kylit_webhook_enabled: bool,
    /// Kylit deploy directory on server. Default: /home/kylit/kylit
    pub kylit_deploy_path: PathBuf,
    /// Full path to kylit-prod-backup.sh. Default: {kylit_deploy_path}/scripts/kylit-prod-backup.sh
    pub kylit_backup_script: PathBuf,
    /// Passed to backup script as KYLIT_ENV_FILE (e.g. .env.prod).
    pub kylit_env_file: Option<PathBuf>,
    /// Postgres container name. Default: kylit-postgres
    pub kylit_pg_container: String,
    pub kylit_pg_user: String,
    pub kylit_pg_db: String,
    /// SQL dumps from /kylit_backup_db. Default: /var/backups/kylit
    pub kylit_backup_root: PathBuf,
    /// Kylit Docker container names for /kylit_docker (comma-separated).
    pub kylit_container_names: Vec<String>,

    /// Prod stack container names (MONITOR_PROD_CONTAINERS).
    pub prod_containers: Vec<String>,
    /// Staging stack container names (MONITOR_STAGING_CONTAINERS).
    pub staging_containers: Vec<String>,
    pub prod_backend_url: String,
    pub prod_frontend_url: String,
    pub staging_backend_url: String,
    pub staging_frontend_url: String,
    pub dev_backend_url: String,
    pub dev_frontend_url: String,
    /// Postgres container for /prod_backup (docker exec).
    pub prod_pg_container: String,
    pub staging_pg_container: String,
    /// Shell fragment run after `cd prod_deploy_path` for prod restart (see MONITOR_PROD_COMPOSE_RESTART_SH).
    pub prod_compose_restart_sh: String,
    pub staging_compose_restart_sh: String,

    /// IP admin: backend when MONITOR_IP_BACKEND=ufw|crowdsec and MONITOR_IP_ADMIN_SECRET set.
    pub ip_backend: Option<IpFirewallBackend>,
    pub ip_admin_secret: Option<String>,
    pub crowdsec_ban_duration: String,
    pub crowdsec_ban_reason: String,

    /// Local reverse proxy type for diagnose (`MONITOR_REVERSE_PROXY`).
    pub reverse_proxy: ReverseProxyKind,
    /// Directory or file scanned with `grep -r` for `monitor/webhook` when using Traefik.
    pub traefik_config_scan_path: Option<PathBuf>,
    /// Public webhook URL Telegram must call (default `https://MONITOR_PROD_DOMAIN/monitor/webhook`).
    pub webhook_expected_url: String,
    /// After daemon start, call `getWebhookInfo` and compare to `webhook_expected_url` (default: on if webhook enabled).
    pub webhook_startup_verify: bool,
    /// If true, daemon exits with error when startup webhook check fails (`MONITOR_WEBHOOK_STARTUP_STRICT`).
    pub webhook_startup_strict: bool,
}

fn parse_csv(name: &str, default: &[&str]) -> Vec<String> {
    match env::var(name) {
        Ok(s) => {
            let parsed: Vec<String> = s
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect();
            if parsed.is_empty() {
                default.iter().map(|s| (*s).to_string()).collect()
            } else {
                parsed
            }
        }
        Err(_) => default.iter().map(|s| (*s).to_string()).collect(),
    }
}

impl Config {
    pub fn from_env() -> Self {
        let interval_secs = env::var("MONITOR_INTERVAL_SECS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(300);

        let docker_socket = env::var("DOCKER_SOCKET")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/var/run/docker.sock"));

        let min_available_memory_bytes = env::var("MONITOR_MIN_MEMORY_MB")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(|mb| mb * 1024 * 1024)
            .unwrap_or(100 * 1024 * 1024); // 100 MiB

        let min_available_disk_bytes = env::var("MONITOR_MIN_DISK_GB")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(|gb| gb * 1024 * 1024 * 1024)
            .unwrap_or(1024 * 1024 * 1024); // 1 GiB

        let cert_warn_days = env::var("MONITOR_CERT_WARN_DAYS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(30);

        let dev = env::var("MONITOR_DEV")
            .ok()
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let webhook_port = env::var("MONITOR_WEBHOOK_PORT")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(9090);

        let webhook_secret = env::var("MONITOR_WEBHOOK_SECRET").ok();

        let allowed_chat_ids: Vec<String> = env::var("TELEGRAM_ALLOWED_CHAT_IDS")
            .ok()
            .map(|s| {
                s.split(',')
                    .map(|x| x.trim().to_string())
                    .filter(|x| !x.is_empty())
                    .collect()
            })
            .or_else(|| env::var("TELEGRAM_CHAT_ID").ok().map(|id| vec![id]))
            .unwrap_or_default();

        let prod_deploy_path = env::var("MONITOR_PROD_DEPLOY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/home/closlamartine/app"));

        let staging_deploy_path = env::var("MONITOR_STAGING_DEPLOY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/home/clsmstaging/app"));

        let scope = env::var("MONITOR_SCOPE")
            .ok()
            .map(|s| MonitorScope::from_str(&s))
            .unwrap_or(MonitorScope::Both);

        let prod_domain = env::var("MONITOR_PROD_DOMAIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "www.mobile-i-love.com".to_string());

        let staging_domain = env::var("MONITOR_STAGING_DOMAIN")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| "dlc.netsrv.be".to_string());

        let pid_file = env::var("MONITOR_PID_FILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp/clos-monitor.pid"));

        let expected_process_names = parse_csv(
            "MONITOR_PROCESS_COMM",
            &["clos-monitor", "server-monitor"],
        );

        let kylit_webhook_enabled = env::var("MONITOR_KYLIT_WEBHOOK")
            .ok()
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let kylit_deploy_path = env::var("KYLIT_DEPLOY_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/home/kylit/kylit"));

        let kylit_backup_script = env::var("KYLIT_BACKUP_SCRIPT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                kylit_deploy_path.join("scripts").join("kylit-prod-backup.sh")
            });

        let kylit_env_file = env::var("KYLIT_ENV_FILE").ok().map(PathBuf::from);

        let kylit_pg_container = env::var("KYLIT_PG_CONTAINER")
            .unwrap_or_else(|_| "kylit-postgres".to_string());

        let kylit_pg_user = env::var("KYLIT_PG_USER")
            .or_else(|_| env::var("POSTGRES_USER"))
            .unwrap_or_else(|_| "kylit".to_string());

        let kylit_pg_db = env::var("KYLIT_PG_DB")
            .or_else(|_| env::var("POSTGRES_DB"))
            .unwrap_or_else(|_| "kylit".to_string());

        let kylit_backup_root = env::var("KYLIT_BACKUP_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/var/backups/kylit"));

        let kylit_container_names = parse_csv(
            "MONITOR_KYLIT_CONTAINERS",
            &[
                "kylit-postgres",
                "kylit-backend",
                "kylit-minio",
                "kylit-app",
            ],
        );

        let prod_containers = parse_csv(
            "MONITOR_PROD_CONTAINERS",
            &[
                "closlamartine_db",
                "closlamartine_backend",
                "closlamartine_frontend",
            ],
        );
        let staging_containers = parse_csv(
            "MONITOR_STAGING_CONTAINERS",
            &[
                "clsmstaging_db",
                "clsmstaging_backend",
                "clsmstaging_frontend",
            ],
        );

        let prod_backend_url = env::var("MONITOR_PROD_BACKEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8087/api/health".to_string());
        let prod_frontend_url = env::var("MONITOR_PROD_FRONTEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3009/".to_string());
        let staging_backend_url = env::var("MONITOR_STAGING_BACKEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:8088/api/health".to_string());
        let staging_frontend_url = env::var("MONITOR_STAGING_FRONTEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:3010/".to_string());
        let dev_backend_url = env::var("MONITOR_DEV_BACKEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:58081/api/health".to_string());
        let dev_frontend_url = env::var("MONITOR_DEV_FRONTEND_URL")
            .unwrap_or_else(|_| "http://127.0.0.1:58080/".to_string());

        let prod_pg_container = env::var("MONITOR_PROD_PG_CONTAINER")
            .unwrap_or_else(|_| "closlamartine_db".to_string());
        let staging_pg_container = env::var("MONITOR_STAGING_PG_CONTAINER")
            .unwrap_or_else(|_| "clsmstaging_db".to_string());

        let prod_compose_restart_sh = env::var("MONITOR_PROD_COMPOSE_RESTART_SH").unwrap_or_else(|_| {
            "docker compose --env-file .env.prod -p closlamartine -f docker-compose.prod.yml restart 2>&1"
                .to_string()
        });
        let staging_compose_restart_sh =
            env::var("MONITOR_STAGING_COMPOSE_RESTART_SH").unwrap_or_else(|_| {
                "docker compose --env-file .env.staging -p clsmstaging -f docker-compose.staging.yml restart 2>&1"
                    .to_string()
            });

        let ip_backend = env::var("MONITOR_IP_BACKEND")
            .ok()
            .and_then(|s| IpFirewallBackend::from_str(&s));

        let ip_admin_secret = env::var("MONITOR_IP_ADMIN_SECRET").ok().filter(|s| !s.trim().is_empty());

        let crowdsec_ban_duration = env::var("MONITOR_CROWDSEC_BAN_DURATION")
            .unwrap_or_else(|_| "4h".to_string());
        let crowdsec_ban_reason = env::var("MONITOR_CROWDSEC_BAN_REASON")
            .unwrap_or_else(|_| "monitor".to_string());

        let reverse_proxy = env::var("MONITOR_REVERSE_PROXY")
            .ok()
            .and_then(|s| ReverseProxyKind::from_str(&s))
            .unwrap_or(ReverseProxyKind::Nginx);

        let traefik_config_scan_path =
            env::var("MONITOR_TRAEFIK_CONFIG_SCAN_PATH").ok().map(PathBuf::from);

        let webhook_expected_url = env::var("MONITOR_WEBHOOK_EXPECTED_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| {
                let host = prod_domain
                    .trim()
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                format!("https://{}/monitor/webhook", host)
            });

        let webhook_startup_verify = env::var("MONITOR_WEBHOOK_STARTUP_VERIFY")
            .ok()
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(true);

        let webhook_startup_strict = env::var("MONITOR_WEBHOOK_STARTUP_STRICT")
            .ok()
            .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        Self {
            telegram_bot_token: env::var("TELEGRAM_BOT_TOKEN").ok(),
            telegram_chat_id: env::var("TELEGRAM_CHAT_ID").ok(),
            allowed_chat_ids,
            interval_secs,
            docker_socket,
            min_available_memory_bytes,
            min_available_disk_bytes,
            cert_warn_days,
            dev,
            webhook_port,
            webhook_secret,
            prod_deploy_path,
            staging_deploy_path,
            scope,
            prod_domain,
            staging_domain,
            pid_file,
            expected_process_names,
            kylit_webhook_enabled,
            kylit_deploy_path,
            kylit_backup_script,
            kylit_env_file,
            kylit_pg_container,
            kylit_pg_user,
            kylit_pg_db,
            kylit_backup_root,
            kylit_container_names,
            prod_containers,
            staging_containers,
            prod_backend_url,
            prod_frontend_url,
            staging_backend_url,
            staging_frontend_url,
            dev_backend_url,
            dev_frontend_url,
            prod_pg_container,
            staging_pg_container,
            prod_compose_restart_sh,
            staging_compose_restart_sh,
            ip_backend,
            ip_admin_secret,
            crowdsec_ban_duration,
            crowdsec_ban_reason,
            reverse_proxy,
            traefik_config_scan_path,
            webhook_expected_url,
            webhook_startup_verify,
            webhook_startup_strict,
        }
    }

    pub fn with_dev(mut self, dev: bool) -> Self {
        self.dev = dev;
        self
    }

    pub fn telegram_configured(&self) -> bool {
        self.telegram_bot_token.is_some() && !self.allowed_chat_ids.is_empty()
    }

    pub fn with_scope(mut self, scope: MonitorScope) -> Self {
        self.scope = scope;
        self
    }

    /// Webhook is enabled when secret is set (required for security).
    pub fn webhook_enabled(&self) -> bool {
        self.webhook_secret.is_some()
    }

    /// Password-gated `/ip_*` commands when secret and backend are set.
    pub fn ip_admin_enabled(&self) -> bool {
        self.ip_admin_secret.is_some() && self.ip_backend.is_some()
    }
}
