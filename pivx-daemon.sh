#!/bin/bash
# =============================================================================
# PIVX Daemon Control Script for macOS
# -----------------------------------------------------------------------------
# Controls the PIVX daemon: start | stop | restart | status | logs
# - Portable: uses $HOME dynamically
# - Tested for macOS (Intel & Apple Silicon)
# - Relies on pivxd and pivx-cli from ~/pivx/pivx-5.6.1/bin
# =============================================================================

set -e

# --- Configuration ---
BIN_DIR="$HOME/pivx/pivx-5.6.1/bin"
DATA_DIR="$HOME/.pivx"
CONF_FILE="$DATA_DIR/pivx.conf"
PID_FILE="$DATA_DIR/pivx.pid"
LOG_FILE="$DATA_DIR/debug.log"

PIVXD_BIN="$BIN_DIR/pivxd"
PIVXCLI_BIN="$BIN_DIR/pivx-cli"

# --- Sanity checks ---
if [ ! -x "$PIVXD_BIN" ]; then
    echo "❌ Error: pivxd binary not found or not executable at $PIVXD_BIN"
    exit 1
fi

if [ ! -x "$PIVXCLI_BIN" ]; then
    echo "❌ Error: pivx-cli binary not found or not executable at $PIVXCLI_BIN"
    exit 1
fi

# --- Helper: check running process ---
is_running() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE" 2>/dev/null)
        if [ -n "$PID" ] && ps -p "$PID" >/dev/null 2>&1; then
            return 0
        fi
    fi
    # Fallback: search process by name if PID file missing
    pgrep -f "$PIVXD_BIN" >/dev/null 2>&1 && return 0
    return 1
}

# --- Start daemon ---
start_pivx() {
    if is_running; then
        echo "⚠️  PIVX is already running (PID: $(cat "$PID_FILE" 2>/dev/null))"
        exit 0
    fi

    echo "🚀 Starting PIVX daemon..."
    "$PIVXD_BIN" \
        -shrinkdebugfile \
        -daemon \
        -pid="$PID_FILE" \
        -conf="$CONF_FILE" \
        -datadir="$DATA_DIR"

    sleep 2
    if is_running; then
        echo "✅ PIVX started successfully (PID: $(cat "$PID_FILE" 2>/dev/null))"
    else
        echo "❌ Failed to start PIVX. Check $LOG_FILE for details."
        exit 1
    fi
}

# --- Stop daemon ---
stop_pivx() {
    if ! is_running; then
        echo "⚠️  PIVX is not running."
        exit 0
    fi

    echo "🛑 Stopping PIVX daemon..."
    "$PIVXCLI_BIN" -conf="$CONF_FILE" -datadir="$DATA_DIR" stop >/dev/null 2>&1 || true

    # Graceful wait
    for i in {1..10}; do
        if ! is_running; then
            echo "✅ PIVX stopped successfully."
            rm -f "$PID_FILE" >/dev/null 2>&1 || true
            return
        fi
        sleep 1
    done

    # Force kill if stuck
    PID=$(pgrep -f "$PIVXD_BIN" || true)
    if [ -n "$PID" ]; then
        echo "⚠️ Forcing kill of PIVX daemon (PID: $PID)"
        kill -9 "$PID" || true
    fi
    rm -f "$PID_FILE" >/dev/null 2>&1 || true
    echo "✅ PIVX fully stopped."
}

# --- Show status ---
status_pivx() {
    if is_running; then
        PID=$(cat "$PID_FILE" 2>/dev/null || pgrep -f "$PIVXD_BIN")
        echo "✅ PIVX is running (PID: $PID)"
        "$PIVXCLI_BIN" -conf="$CONF_FILE" -datadir="$DATA_DIR" getblockcount 2>/dev/null || true
    else
        echo "❌ PIVX is not running."
        exit 1
    fi
}

# --- Restart ---
restart_pivx() {
    echo "🔄 Restarting PIVX..."
    stop_pivx
    sleep 2
    start_pivx
}

# --- Show live logs ---
logs_pivx() {
    if [ ! -f "$LOG_FILE" ]; then
        echo "⚠️ No log file found at $LOG_FILE"
        exit 0
    fi
    echo "📜 Showing live logs (Ctrl+C to exit)..."
    tail -f "$LOG_FILE"
}

# --- Main dispatcher ---
case "$1" in
    start)   start_pivx ;;
    stop)    stop_pivx ;;
    restart) restart_pivx ;;
    status)  status_pivx ;;
    logs)    logs_pivx ;;
    *)
        echo "Usage: $0 {start|stop|restart|status|logs}"
        exit 1
        ;;
esac

