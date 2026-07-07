#!/usr/bin/env bash
# Build wall.exe and install it into the Windows Startup folder (runs at logon).
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release --target x86_64-pc-windows-gnu

STARTUP="$(wslpath "$(powershell.exe -NoProfile -Command '[Environment]::GetFolderPath("Startup")' | tr -d '\r')")"
powershell.exe -NoProfile -Command 'Stop-Process -Name agent-wall -ErrorAction SilentlyContinue' >/dev/null 2>&1 || true
sleep 1
cp target/x86_64-pc-windows-gnu/release/agent-wall.exe "$STARTUP/agent-wall.exe"
echo "deployed: $STARTUP/agent-wall.exe"
