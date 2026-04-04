#!/bin/bash

# Exit immediately if any command fails.
set -e

# 1. Build the Rust project
echo "📦 [1/4] Building..."
cargo build --release

# 2. Create log directory and set permissions (requires root)
echo "📂 [2/4] Creating system log directory..."
sudo mkdir -p /var/log/network-monitor
sudo chown root:wheel /var/log/network-monitor
sudo chmod 777 /var/log/network-monitor

# 3. Copy the binary to a system path
echo "🚚 [3/4] Copying binary to /usr/local/bin/..."
sudo cp target/release/network-monitor-agent /usr/local/bin/
sudo chmod +x /usr/local/bin/network-monitor-agent

# 4. Restart the background daemon (LaunchDaemon)
echo "🔄 [4/4] Restarting LaunchDaemon service..."
# Ignore errors from unload in case the service is not currently loaded.
sudo launchctl unload /Library/LaunchDaemons/com.user.network-monitor.plist 2>/dev/null || true
sudo launchctl load -w /Library/LaunchDaemons/com.user.network-monitor.plist

echo "✅ Deployment completed successfully!"
echo "----------------------------------------------------"
echo "👉 View real-time logs: tail -f /var/log/network-monitor/app.log"
echo "👉 Check process status: ps -ef | grep network-monitor-agent"
echo "----------------------------------------------------"
