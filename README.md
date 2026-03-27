# clos-monitor

Rust CLI that monitors ClosLamartine Docker containers (prod + staging) on the Ubuntu server and sends Telegram alerts on failures.

## Quick start: Telegram bot setup

```bash
./cmd/clos-monitor/scripts/setup-telegram-bot.sh
```

Or see [docs/SETUP.md](docs/SETUP.md) for the full guide (ports, deploy, config).

**Command reference (Telegram + CLI):** [docs/Help.md](docs/Help.md).

## Checks

**Server vitals:**
- **Memory**: available RAM (alert if &lt; 100 MB, config: `MONITOR_MIN_MEMORY_MB`)
- **Disk**: free space on /, /var, /home (alert if &lt; 1 GB, config: `MONITOR_MIN_DISK_GB`)
- **Disk read-only**: alerts if any monitored mount is read-only (possible crash)
- **Services**: Docker, SSH, Nginx (`systemctl is-active`)
- **SSL certs**: Certbot/Let's Encrypt expiry (warn if &lt; 30 days, config: `MONITOR_CERT_WARN_DAYS`)

**Containers:**
- **Docker**: `closlamartine_db`, `closlamartine_backend`, `closlamartine_frontend` (prod) and `clsmstaging_*` (staging)
- **Container status**: running, exited, restarting, dead
- **DB health**: Postgres healthcheck (healthy/unhealthy)
- **HTTP**: Backend `/api/health` and frontend root for both envs

## Ports (prod / staging)

| Service  | Prod | Staging |
|----------|------|---------|
| Frontend | 3009 | 3010 |
| Backend  | 8087 | 8088 |
| Postgres | 5439 | 5441 |

## Configuration

**Env file** (first existing): `MONITOR_ENV_FILE`, `/etc/clos-monitor.env`, or `.env` in cwd.

| Variable | Description |
|----------|-------------|
| `TELEGRAM_BOT_TOKEN` | Bot token from @BotFather |
| `TELEGRAM_CHAT_ID` | Chat/group ID for alerts |
| `MONITOR_INTERVAL_SECS` | Check interval in daemon mode (default: 300) |
| `MONITOR_ENV_FILE` | Path to .env file (overrides default search) |
| `DOCKER_SOCKET` | Docker socket path (default: `/var/run/docker.sock`) |

## Usage

```bash
# One-shot check (exit 1 if any failure)
clos-monitor check

# Dev mode: check local docker-compose (backend 58081, frontend 58080)
clos-monitor check --dev

# Daemon mode (loop + Telegram alerts)
clos-monitor run

# Diagnose webhook chain (env, PID, local port, Telegram getWebhookInfo, nginx)
clos-monitor diagnose
```

## Local testing (dev)

1. Start dev stack: `docker compose up -d`
2. Run monitor: `make monitor-check-dev` or `clos-monitor check --dev`

Or in one go: `make monitor-dev` (starts compose, waits, then runs check).

## Build

```bash
cd cmd/clos-monitor
cargo build --release
```

Or from project root: `make monitor-build`

Note: If you see `error: unknown proxy name: 'cursor'`, fix your rustup config (e.g. `rustup override unset` or remove the cursor proxy from `~/.rustup/settings.toml`).

## Deploy

1. Run `./cmd/clos-monitor/scripts/setup-telegram-bot.sh` to create the bot and get token/chat_id
2. Build and copy to server: `make monitor-deploy`
3. Create `/etc/clos-monitor.env` on server:
   ```
   TELEGRAM_BOT_TOKEN=your_bot_token
   TELEGRAM_CHAT_ID=your_chat_id
   ```
4. Install systemd unit:
   ```bash
   sudo cp cmd/clos-monitor/systemd/clos-monitor.service /etc/systemd/system/
   sudo systemctl daemon-reload
   sudo systemctl enable clos-monitor
   sudo systemctl start clos-monitor
   ```
5. Ensure `ubuntu` user is in `docker` group: `sudo usermod -aG docker ubuntu`

Full setup guide: [docs/SETUP.md](docs/SETUP.md)

## Logs

`make monitor-logs` or `journalctl -u clos-monitor -f`
