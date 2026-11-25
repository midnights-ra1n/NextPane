#!/bin/sh
# Host setup for the NextPane server package (deb/rpm scriptlets).
set -e
if ! getent passwd nextpane-server >/dev/null; then
  useradd --system --user-group --home-dir /var/lib/nextpane-server --shell /usr/sbin/nologin nextpane-server
fi
install -d -m 0750 -o nextpane-server -g nextpane-server /var/lib/nextpane-server /var/lib/nextpane-server/update
chown root:nextpane-server /etc/nextpane/server.env
chmod 0640 /etc/nextpane/server.env
systemctl daemon-reload
systemctl enable nextpane-server.service nextpane-server-update.path >/dev/null 2>&1 || true
systemctl start nextpane-server-update.path || true
systemctl restart nextpane-server.service
echo "nextpane-server: running on port 3000. First start? Get the setup code with: journalctl -u nextpane-server | grep 'setup code'"
