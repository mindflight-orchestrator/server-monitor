#!/usr/bin/env bash
# Setup guide for Telegram bot used by clos-monitor
# Run this script to get step-by-step instructions and verify your setup.

set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

echo -e "${CYAN}=== ClosLamartine Monitor - Telegram Bot Setup ===${NC}"
echo ""

# Step 1: Create bot
echo -e "${YELLOW}Step 1: Create a Telegram Bot${NC}"
echo "  1. Open Telegram and search for @BotFather"
echo "  2. Send: /newbot"
echo "  3. Choose a name (e.g. ClosLamartine Monitor)"
echo "  4. Choose a username ending in 'bot' (e.g. closlamartine_monitor_bot)"
echo "  5. Copy the token you receive (format: 123456789:ABCdefGHIjklMNOpqrsTUVwxyz)"
echo ""
read -p "Paste your BOT TOKEN (or press Enter to skip): " BOT_TOKEN

if [ -n "$BOT_TOKEN" ]; then
    echo -e "${GREEN}Token received.${NC}"
else
    echo "Skipped. You can add it later to TELEGRAM_BOT_TOKEN."
fi
echo ""

# Step 2: Get Chat ID
echo -e "${YELLOW}Step 2: Get your Chat ID${NC}"
echo "  Option A - Personal chat:"
echo "    1. Send a message to your new bot (e.g. /start)"
echo "    2. Open: https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates"
echo "    3. Find \"chat\":{\"id\":123456789} - that number is your chat_id"
echo ""
echo "  Option B - Group chat:"
echo "    1. Add the bot to a group"
echo "    2. Send a message in the group"
echo "    3. Open getUpdates URL - chat id for groups is negative (e.g. -1001234567890)"
echo ""

if [ -n "$BOT_TOKEN" ]; then
    echo "  Quick link to get chat ID:"
    echo "    https://api.telegram.org/bot<YOUR_TOKEN>/getUpdates"
    echo ""
fi

read -p "Paste your CHAT ID (or press Enter to skip): " CHAT_ID

if [ -n "$CHAT_ID" ]; then
    echo -e "${GREEN}Chat ID received.${NC}"
else
    echo "Skipped. You can add it later to TELEGRAM_CHAT_ID."
fi
echo ""

# Step 3: Test
if [ -n "$BOT_TOKEN" ] && [ -n "$CHAT_ID" ]; then
    echo -e "${YELLOW}Step 3: Test the connection${NC}"
    RESP=$(curl -s -X POST "https://api.telegram.org/bot${BOT_TOKEN}/sendMessage" \
        -H "Content-Type: application/json" \
        -d "{\"chat_id\":\"${CHAT_ID}\",\"text\":\"ClosLamartine Monitor: test message from setup script\"}")
    if echo "$RESP" | grep -q '"ok":true'; then
        echo -e "${GREEN}Success! Check Telegram for the test message.${NC}"
    else
        echo "Test failed. Response: $RESP"
    fi
    echo ""
fi

# Step 4: Env file
echo -e "${YELLOW}Step 4: Create environment file${NC}"
ENV_FILE="/etc/clos-monitor.env"
echo "  For systemd on the server, create: $ENV_FILE"
echo ""
echo "  Contents:"
echo "  ---"
echo "  TELEGRAM_BOT_TOKEN=${BOT_TOKEN:-your_bot_token_here}"
echo "  TELEGRAM_CHAT_ID=${CHAT_ID:-your_chat_id_here}"
echo "  MONITOR_INTERVAL_SECS=300"
echo "  ---"
echo ""
echo "  Then: sudo chmod 600 $ENV_FILE"
echo ""

# Summary
echo -e "${GREEN}=== Setup complete ===${NC}"
echo "See cmd/clos-monitor/docs/SETUP.md for full documentation (ports, deploy, etc.)."
