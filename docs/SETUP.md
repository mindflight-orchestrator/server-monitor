# clos-monitor Setup Guide

Complete setup for the ClosLamartine Docker monitor: Telegram bot, ports reference, and deployment.

**Monitoring runs on the server only.** Do not run `clos-monitor run` locally in production; use `make monitor-deploy` to deploy to the server. To stop any stray local process: `make monitor-kill`.

---

## 1. Telegram Bot Setup

### 1.1 Create the bot

1. Open Telegram and search for **@BotFather**
2. Send: `/newbot`
3. Enter a display name (e.g. `ClosLamartine Monitor`)
4. Enter a username ending in `bot` (e.g. `closlamartine_monitor_bot`)
5. Copy the token (format: `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

### 1.2 Get your Chat ID

**Personal chat (alerts to you):**

1. Send `/start` to your new bot
2. Open in browser: `https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates`
3. Find `"chat":{"id":123456789}` — that number is your `TELEGRAM_CHAT_ID`

**Group chat (alerts to a team):**

1. Add the bot to the group
2. Send a message in the group
3. Open the getUpdates URL — group IDs are negative (e.g. `-1001234567890`)

### 1.3 Interactive setup script

```bash
./cmd/clos-monitor/scripts/setup-telegram-bot.sh
```

This script walks through the steps and can send a test message.

---

## 2. Ports Reference

| Environment | Service   | Port | Container name           | Health check                    |
|-------------|-----------|------|--------------------------|---------------------------------|
| **Production** | Frontend  | 3009 | closlamartine_frontend   | `GET http://127.0.0.1:3009/`    |
| **Production** | Backend   | 8087 | closlamartine_backend    | `GET http://127.0.0.1:8087/api/health` |
| **Production** | Postgres  | 5439 | closlamartine_db         | Docker healthcheck (pg_isready)  |
| **Staging** | Frontend  | 3010 | clsmstaging_frontend    | `GET http://127.0.0.1:3010/`    |
| **Staging** | Backend   | 8088 | clsmstaging_backend     | `GET http://127.0.0.1:8088/api/health` |
| **Staging** | Postgres  | 5441 | clsmstaging_db          | Docker healthcheck (pg_isready)  |

All checks run on **localhost** because the monitor runs on the same server as the containers.

---

## 3. What the Monitor Checks

### 3.1 Server vitals

| Check | Threshold | Alert when |
|-------|-----------|------------|
| **Memory** | Min 100 MB available (config: `MONITOR_MIN_MEMORY_MB`) | Available RAM below threshold |
| **Disk space** | Min 1 GB free (config: `MONITOR_MIN_DISK_GB`) | Root, /var, /home below threshold |
| **Disk read-only** | - | Any monitored mount is read-only (possible crash) |
| **Docker** | - | `systemctl is-active docker` not active |
| **SSH** | - | `systemctl is-active ssh` not active |
| **Nginx** | - | `systemctl is-active nginx` not active |
| **SSL certs** | Warn if &lt; 30 days (config: `MONITOR_CERT_WARN_DAYS`) | Certbot certs in /etc/letsencrypt/live/ |

### 3.2 Containers (prod / staging)

| Check type | Prod | Staging |
|------------|------|---------|
| Container running | closlamartine_db, _backend, _frontend | clsmstaging_db, _backend, _frontend |
| DB health | Postgres healthcheck = healthy | Same |
| Backend | HTTP 200 + `{"status":"ok"}` | Same |
| Frontend | HTTP 200 on root | Same |

Alerts are state-based: one alert when an environment becomes unhealthy, and one resolved message when it returns to healthy.

### 3.3 Optional future checks (not yet implemented)

| Check | Why useful |
|-------|-------------|
| **Load average** | High load = CPU saturation, runaway process |
| **Swap usage** | Heavy swap = memory pressure, OOM risk |
| **Systemd degraded** | `systemctl is-system-running` = degraded |
| **Failed units** | `systemctl --failed` = any failed services |
| **Uptime** | Very low = recent reboot/crash |
| **Inode usage** | Exhausted inodes = can't create files |

---

## 4. Configuration

| Variable | Description | Example |
|----------|-------------|---------|
| `TELEGRAM_BOT_TOKEN` | Bot token from @BotFather | `123456789:ABC...` |
| `TELEGRAM_CHAT_ID` | Chat or group ID for alerts | `-1001234567890` |
| `TELEGRAM_ALLOWED_CHAT_IDS` | Comma-separated chat IDs (webhook commands) | `123,456,-789` |
| `MONITOR_INTERVAL_SECS` | Seconds between checks (daemon mode) | `300` (default) |
| `MONITOR_WEBHOOK_PORT` | Webhook HTTP port | `9090` (default) |
| `MONITOR_WEBHOOK_SECRET` | **Required for webhook.** Secret token for validation | `openssl rand -hex 32` |
| `MONITOR_WEBHOOK_EXPECTED_URL` | URL Telegram uses (must match `setWebhook`); default `https://MONITOR_PROD_DOMAIN/monitor/webhook` | full HTTPS URL |
| `MONITOR_REVERSE_PROXY` | `nginx` (default), `traefik`, or `none` — controls `diagnose` / `/self` file checks | `traefik` |
| `MONITOR_TRAEFIK_CONFIG_SCAN_PATH` | File or dir grep’d for `monitor/webhook` when proxy is Traefik | `/etc/traefik/dynamic` |
| `MONITOR_WEBHOOK_STARTUP_VERIFY` | Daemon: verify `getWebhookInfo` after start (default on if webhook enabled) | `1` / `true` |
| `MONITOR_WEBHOOK_STARTUP_STRICT` | Exit daemon if startup webhook check fails | `0` or `1` |
| `MONITOR_PROD_DEPLOY_PATH` | Prod app path (backup/restart) | `/home/closlamartine/app` |
| `MONITOR_STAGING_DEPLOY_PATH` | Staging app path | `/home/clsmstaging/app` |
| `MONITOR_SCOPE` | Monitoring scope: `both`, `prod`, or `staging` | `both` (default) |
| `MONITOR_PROD_DOMAIN` | Domain label shown in production alerts | `www.mobile-i-love.com` |
| `MONITOR_STAGING_DOMAIN` | Domain label shown in staging alerts | `dlc.netsrv.be` |
| `DOCKER_SOCKET` | Docker socket path | `/var/run/docker.sock` (default) |
| `MONITOR_MIN_MEMORY_MB` | Min available RAM (MB) before alert | `100` (default) |
| `MONITOR_MIN_DISK_GB` | Min free disk (GB) before alert | `1` (default) |
| `MONITOR_CERT_WARN_DAYS` | Warn when SSL cert expires within N days | `30` (default) |
| `MONITOR_ENV_FILE` | Path to .env file (overrides default) | `/etc/clos-monitor.env` or `.env` |
| `MONITOR_PID_FILE` | Path for PID file (daemon mode). Used by `make monitor-kill` | `/tmp/clos-monitor.pid` |
| `MONITOR_PROCESS_COMM` | Comma-separated process `comm` names accepted for PID-file check (`diagnose`, `/self`) | `clos-monitor,server-monitor` |
| `MONITOR_PROD_CONTAINERS` | Comma-separated Docker names for prod checks & `/docker` | default ClosLamartine names |
| `MONITOR_STAGING_CONTAINERS` | Comma-separated Docker names for staging | default staging names |
| `MONITOR_PROD_BACKEND_URL` / `MONITOR_PROD_FRONTEND_URL` | HTTP health targets for prod | `http://127.0.0.1:8087/api/health`, `:3009/` |
| `MONITOR_STAGING_BACKEND_URL` / `MONITOR_STAGING_FRONTEND_URL` | Staging HTTP targets | `:8088/api/health`, `:3010/` |
| `MONITOR_DEV_BACKEND_URL` / `MONITOR_DEV_FRONTEND_URL` | Dev mode (`--dev`) HTTP targets | `:58081/api/health`, `:58080/` |
| `MONITOR_PROD_PG_CONTAINER` | DB container for `/prod_backup` | `closlamartine_db` |
| `MONITOR_STAGING_PG_CONTAINER` | DB container for `/staging_backup` | `clsmstaging_db` |
| `MONITOR_PROD_COMPOSE_RESTART_SH` | Shell fragment after `cd MONITOR_PROD_DEPLOY_PATH` for `/prod_restart` | `docker compose ... prod ...` |
| `MONITOR_STAGING_COMPOSE_RESTART_SH` | Same for staging restart | `docker compose ... staging ...` |
| `MONITOR_KYLIT_CONTAINERS` | Comma-separated names for `/kylit_docker` | `kylit-postgres,kylit-backend,...` |
| `MONITOR_IP_BACKEND` | `ufw` or `crowdsec` (with `MONITOR_IP_ADMIN_SECRET`) | (unset = IP commands off) |
| `MONITOR_IP_ADMIN_SECRET` | Password for `/ip_list`, `/ip_ban`, `/ip_unban` (length must match exactly) | strong random string |
| `MONITOR_CROWDSEC_BAN_DURATION` | Passed to `cscli decisions add -d` | `4h` |
| `MONITOR_CROWDSEC_BAN_REASON` | Passed to `--reason` | `monitor` |

**IP admin (UFW vs CrowdSec):** The monitor process user must be allowed to run `ufw` or `cscli` (often root or members of `crowdsec` / sudo). **Telegram is not a secret channel**; treat `MONITOR_IP_ADMIN_SECRET` as disposable and rotate if it is ever pasted in chat.

**Webhook:** For interactive commands (`/status`, `/prod_backup`, `/self`, etc.), see [TELEGRAM_WEBHOOK.md](TELEGRAM_WEBHOOK.md).

---

## 5. Deployment on Server

### 5.1 Prepare env file locally

Ensure `cmd/clos-monitor/.env` contains your Telegram credentials:

```
TELEGRAM_BOT_TOKEN=your_bot_token
TELEGRAM_CHAT_ID=your_chat_id
MONITOR_INTERVAL_SECS=300
```

### 5.2 Build and deploy (binary, .env, systemd unit)

```bash
# monitor both prod + staging (default)
make monitor-deploy

# monitor only production
make monitor-deploy-prod

# monitor only staging
make monitor-deploy-staging
```

This copies the binary to `/usr/local/bin/`, `.env` to `/etc/clos-monitor.env`, and installs the systemd unit. On first deploy, enable and start:

```bash
ssh -i ~/.ssh/id_rsa -p 2022 ubuntu@dlc.netsrv.be
sudo systemctl enable clos-monitor
sudo systemctl start clos-monitor
```

### 5.3 Docker access

Ensure the `ubuntu` user can access Docker:

```bash
sudo usermod -aG docker ubuntu
# Log out and back in, or: newgrp docker
```

### 5.4 View logs

```bash
make monitor-logs
# or
journalctl -u clos-monitor -f
```

---

## 6. Quick Reference

| Command | Description |
|---------|-------------|
| `clos-monitor check` | One-shot check, exit 1 on failure |
| `clos-monitor --scope prod check` | One-shot production-only check |
| `clos-monitor --scope staging check` | One-shot staging-only check |
| `clos-monitor --scope both check` | One-shot prod+staging check |
| `clos-monitor run` | Daemon mode, loop + Telegram alerts |
| `clos-monitor diagnose` | Print webhook chain diagnostics (env, PID, port, Telegram API, nginx) |
| `make monitor-build` | Build binary |
| `make monitor-deploy` | Build + deploy binary, .env, systemd unit |
| `make monitor-check-prod` | One-shot check for production scope |
| `make monitor-check-staging` | One-shot check for staging scope |
| `make monitor-check-both` | One-shot check for both environments |
| `make monitor-logs` | Stream systemd logs |
