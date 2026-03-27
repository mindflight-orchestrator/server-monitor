# clos-monitor — Deployment Checklist

This doc explains how the webhook works and what you must do for commands (`/status`, `/help`, etc.) to work in Telegram.

---

## How it works (flow)

```
Telegram user sends /status
        ↓
Telegram servers POST to https://www.mobile-i-love.com/monitor/webhook
        ↓
Nginx (on server) receives request, forwards to 127.0.0.1:9090
        ↓
clos-monitor (on server) receives request, validates secret, runs command, replies
```

**Important:** clos-monitor does NOT register itself with Telegram. You must do it manually (or via `make monitor-webhook-set`).

---

## One-time setup (if not done)

### 1. Nginx — webhook route

Nginx must forward `/monitor/webhook` to clos-monitor (port 9090). The block is in `nginx-closlamartine.conf`.

```bash
make setup-nginx-prod
# Then reload nginx on server if config was already there:
# ssh ... 'sudo systemctl reload nginx'
```

### 2. Telegram — register webhook

Tell Telegram where to send updates. Run **from your machine** (uses local `.env`):

```bash
make monitor-webhook-set
```

This calls Telegram's `setWebhook` with:
- URL: `https://www.mobile-i-love.com/monitor/webhook`
- Secret: `MONITOR_WEBHOOK_SECRET` from `cmd/clos-monitor/.env`

### 3. BotFather (optional) — menu commands

In Telegram, `/mybots` → your bot → Bot Settings → Menu → Edit Commands. Add:

```
status - Full status
help - List commands
status_prod - Production only
status_staging - Staging only
```

---

## Deploy / update clos-monitor

```bash
make monitor-deploy
make monitor-webhook-set   # Re-register webhook (idempotent, safe to run)
```

---

## Verify

| Check | Command |
|-------|---------|
| Webhook URL | `make monitor-webhook-info` |
| Monitor logs | `make monitor-logs` |
| Nginx has webhook block | `ssh ... 'grep -A5 monitor/webhook /etc/nginx/sites-enabled/*'` |
| Full chain (on server, with env loaded) | `clos-monitor diagnose` or Telegram `/self` (allowed chats) |

---

## Troubleshooting

| Symptom | Cause | Fix |
|---------|-------|-----|
| Commands don't respond | Webhook not registered | `make monitor-webhook-set` |
| Commands don't respond | Nginx missing /monitor/webhook | `make setup-nginx-prod` then reload nginx |
| 403 Forbidden | Secret mismatch or Cloudflare strips header | `make monitor-deploy` injects secret via `/etc/nginx/telegram-webhook-secret.conf`. Run `make setup-nginx-prod` if nginx doesn't have the webhook block. |
| Commands ignored | Chat ID not allowed | Add your chat ID to `TELEGRAM_ALLOWED_CHAT_IDS` or `TELEGRAM_CHAT_ID` |
