# Telegram Webhook Setup for clos-monitor

This guide covers the secure webhook setup for interactive Telegram commands (`/status`, `/prod_backup`, etc.).

---

## 1. BotFather Setup

### 1.1 Create the bot

1. Open Telegram and search for **@BotFather**
2. Send: `/newbot`
3. Enter a display name (e.g. `ClosLamartine Monitor`)
4. Enter a username ending in `bot` (e.g. `closlamartine_monitor_bot`)
5. Copy the token (format: `123456789:ABCdefGHIjklMNOpqrsTUVwxyz`)

### 1.2 Set bot commands (optional, for menu)

1. Send `/mybots` to @BotFather and select your bot
2. Go to **Bot Settings** → **Menu** → **Edit Commands**
3. Add one command per line:

```
status - Show prod/staging/server status
help - List commands (includes /self, optional Kylit & IP admin)
self - Self-check webhook chain
myid - Get your Chat ID
status_prod - Production status only
status_staging - Staging status only
prod_backup - Backup production DB
prod_restart - Restart prod containers
staging_backup - Backup staging DB
staging_restart - Restart staging containers
```

4. Send `/done`

---

## 2. Environment Variables

Add to `cmd/clos-monitor/.env` (or `/etc/clos-monitor.env` on the server):

| Variable | Required | Description |
|----------|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | Yes | Bot token from @BotFather |
| `TELEGRAM_CHAT_ID` | Yes* | Your chat ID (or use `TELEGRAM_ALLOWED_CHAT_IDS`) |
| `TELEGRAM_ALLOWED_CHAT_IDS` | Yes* | Comma-separated chat IDs allowed to run commands |
| `MONITOR_WEBHOOK_SECRET` | Yes (webhook) | Secret token for webhook validation. Generate with `openssl rand -hex 32` |
| `MONITOR_WEBHOOK_PORT` | No | HTTP port for webhook (default: 9090) |
| `MONITOR_PROD_DEPLOY_PATH` | No | Prod app path (default: /home/closlamartine/app) |
| `MONITOR_STAGING_DEPLOY_PATH` | No | Staging app path (default: /home/clsmstaging/app) |
| `MONITOR_IP_BACKEND` | No | `ufw` or `crowdsec` — enables `/ip_*` when set with `MONITOR_IP_ADMIN_SECRET` |
| `MONITOR_IP_ADMIN_SECRET` | No | Password for `/ip_list`, `/ip_ban`, `/ip_unban` (exact length match; see SETUP.md) |

\* Either `TELEGRAM_CHAT_ID` or `TELEGRAM_ALLOWED_CHAT_IDS` must be set.

**`/help`** is built from the live config: it always lists core commands and adds Kylit and IP blocks when those features are enabled.

### Generate webhook secret

```bash
openssl rand -hex 32
```

Use the output as `MONITOR_WEBHOOK_SECRET`. Telegram allows 1–256 chars, `A-Za-z0-9_-` only. Hex output is valid.

### Reverse proxy kind (`MONITOR_REVERSE_PROXY`)

| Value | Behaviour |
|-------|-----------|
| `nginx` (default) | `clos-monitor diagnose` / `/self` grep `/etc/nginx/sites-enabled/` for `monitor/webhook` and check `/etc/nginx/telegram-webhook-secret.conf`. |
| `traefik` | Grep `MONITOR_TRAEFIK_CONFIG_SCAN_PATH` (file or directory) for `monitor/webhook`. Set this to your Traefik dynamic config dir or file (e.g. `/etc/traefik/dynamic`). |
| `none` | Skip local file checks (e.g. only cloud LB). Telegram `getWebhookInfo` still runs. |

### Public webhook URL (`MONITOR_WEBHOOK_EXPECTED_URL`)

Defaults to `https://<MONITOR_PROD_DOMAIN>/monitor/webhook` (scheme + path). **Must match exactly** the URL passed to Telegram `setWebhook`. Override if your public host differs from `MONITOR_PROD_DOMAIN`.

### Startup verification (daemon)

When `MONITOR_WEBHOOK_SECRET` is set and the HTTP listener starts:

- **`MONITOR_WEBHOOK_STARTUP_VERIFY`**: default `true` — after a short delay, the daemon calls `getWebhookInfo` and logs **OK** or a **warning** if the registered URL ≠ `MONITOR_WEBHOOK_EXPECTED_URL`.
- **`MONITOR_WEBHOOK_STARTUP_STRICT`**: set `1` or `true` to **exit the process** if that check fails (useful in systemd so a mis-registered bot fails fast).

Requires `TELEGRAM_BOT_TOKEN` to be set.

---

## 3. Register Webhook with Telegram

After deploying the monitor and your **nginx** or **Traefik** route to `127.0.0.1:<MONITOR_WEBHOOK_PORT>`, register the webhook URL with Telegram.

### Option A: Make target (recommended)

```bash
# From project root, with .env loaded (or export vars)
make monitor-webhook-set
```

### Option B: Manual curl

```bash
curl -X POST "https://api.telegram.org/bot<YOUR_TOKEN>/setWebhook" \
  -H "Content-Type: application/json" \
  -d '{"url":"https://www.mobile-i-love.com/monitor/webhook","secret_token":"<YOUR_MONITOR_WEBHOOK_SECRET>"}'
```

Replace `<YOUR_TOKEN>` and `<YOUR_MONITOR_WEBHOOK_SECRET>` with your values.

### Verify webhook

```bash
curl "https://api.telegram.org/bot<YOUR_TOKEN>/getWebhookInfo"
```

You should see your URL and `has_custom_certificate: false`.

### Traefik (example)

Point a router at the monitor’s local port (`9090` by default). The path must be `/monitor/webhook`. Forward the secret header (case-sensitive): **`X-Telegram-Bot-Api-Secret-Token`**.

**File provider (YAML fragment):**

```yaml
http:
  routers:
    clos_monitor_wh:
      rule: PathPrefix(`/monitor/webhook`)
      service: clos_monitor_wh_svc
      entryPoints:
        - websecure
      tls: {}
  services:
    clos_monitor_wh_svc:
      loadBalancer:
        servers:
          - url: http://127.0.0.1:9090
```

Adjust `entryPoints` / TLS to match your setup. Ensure Traefik can reach the host where the monitor listens (here `127.0.0.1` on the same machine as Traefik).

Set `MONITOR_REVERSE_PROXY=traefik` and `MONITOR_TRAEFIK_CONFIG_SCAN_PATH` to the directory or file that contains this rule so `diagnose` can grep it.

---

## 4. Unset Webhook (switch back to getUpdates)

```bash
make monitor-webhook-unset
# or
curl "https://api.telegram.org/bot<YOUR_TOKEN>/deleteWebhook"
```

---

## 5. Available Commands

| Command | Action |
|---------|--------|
| `/status` | Full status (prod, staging, server) |
| `/status_prod` | Production only |
| `/status_staging` | Staging only |
| `/status_server` | Server vitals only |
| `/space_left` | Disk space for /, /var, /home |
| `/uptime_stats` | Uptime + load average |
| `/memory` | RAM usage |
| `/certs` | SSL cert expiry |
| `/docker` | Container list (prod + staging) |
| `/help` | List commands |
| `/myid` | Get your Chat ID (works even before access is granted) |
| `/prod_backup` | Backup production DB (saved to /tmp/clos-monitor-backups/) |
| `/prod_restart` | Restart prod containers |
| `/staging_backup` | Backup staging DB |
| `/staging_restart` | Restart staging containers |
| `/kylit_backup_db` | Kylit: Postgres dump (needs `MONITOR_KYLIT_WEBHOOK=1`, see `env.kylit.example`) |
| `/kylit_backup_minio` | Kylit: `mc mirror` via `scripts/minio-mirror-backup.sh` |
| `/kylit_backup_all` | Kylit: `scripts/kylit-prod-backup.sh` (DB + MinIO + optional rclone) |
| `/kylit_docker` | Kylit: `kylit-*` container status |

---

## 6. Troubleshooting

### Webhook not receiving updates

- **Check nginx**: Ensure `/monitor/webhook` location is configured and nginx reloaded
- **Check secret**: `X-Telegram-Bot-Api-Secret-Token` must be forwarded by nginx (see nginx-closlamartine.conf)
- **Check monitor**: `journalctl -u clos-monitor -f` — webhook server starts only if `MONITOR_WEBHOOK_SECRET` is set

### 403 Forbidden

- Secret token mismatch: ensure `MONITOR_WEBHOOK_SECRET` matches the `secret_token` used in `setWebhook`
- Nginx must forward the header: `proxy_set_header X-Telegram-Bot-Api-Secret-Token $http_x_telegram_bot_api_secret_token;`

### Commands ignored

- Chat ID not in allow list: add your chat ID to `TELEGRAM_CHAT_ID` or `TELEGRAM_ALLOWED_CHAT_IDS`
- **Get your chat ID**: send `/myid` or `/start` to the bot — it replies with your Chat ID. Add it to `TELEGRAM_ALLOWED_CHAT_IDS` and redeploy.

### Backup/restart fails

- Ensure `ubuntu` user is in `docker` group: `sudo usermod -aG docker ubuntu`
- Check deploy paths exist: `/home/closlamartine/app`, `/home/clsmstaging/app`
- Ensure `.env.prod` and `.env.staging` exist in those paths
