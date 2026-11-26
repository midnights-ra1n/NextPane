# Configuration

Both programs are configured with environment variables. Packages load them from files in
`/etc/nextpane/`; Docker takes them from `-e`/`environment:`.

## Server — `/etc/nextpane/server.env`

| Variable | Default | Description |
|---|---|---|
| `NEXTPANE_LISTEN` | `0.0.0.0:3000` | Address the HTTP server binds to. Use `127.0.0.1:3000` behind a local reverse proxy. |
| `NEXTPANE_TLS` | `off` (`on` when `NEXTPANE_TLS_CERT` is set) | `on`: serve HTTPS directly, without a reverse proxy. Uses `NEXTPANE_TLS_CERT`/`NEXTPANE_TLS_KEY` if set, otherwise a self-signed certificate created in the data directory (agents pin it). See [Installation](installation.md#without-a-reverse-proxy-access-by-ip). |
| `NEXTPANE_TLS_CERT` / `NEXTPANE_TLS_KEY` | *(empty)* | PEM certificate chain and private key for `NEXTPANE_TLS=on`. Read at startup. |
| `NEXTPANE_DATA` | `/var/lib/nextpane-server` (`/data` in Docker) | Database `nextpane.db`, encryption key `secret.key`, update downloads. |
| `NEXTPANE_RELEASE_URL` | *(empty)* | Base URL of the releases, e.g. `https://github.com/midnights-ra1n/NextPane/releases`. Enables update checks, one-click updates and `/install.sh`. Empty disables all three. |
| `NEXTPANE_METRICS_INTERVAL` | `10` | Seconds between metric reports from agents. |
| `NEXTPANE_POLL_INTERVAL` | `60` | Seconds between Git checks for apps with auto-deploy. |
| `NEXTPANE_INSTALL` | auto-detected | `deb`, `rpm`, `binary` or `docker`. Decides how the server updates itself. Set to `docker` in the image. |

## Agent — `/etc/nextpane/agent.env`

| Variable | Default | Description |
|---|---|---|
| `NEXTPANE_SERVER` | *(required)* | WebSocket URL of the panel: `wss://panel.example.com/agent/ws` (`ws://` for a LAN without TLS). |
| `NEXTPANE_FINGERPRINT` | *(empty)* | SHA-256 of the panel's self-signed certificate. When set, the agent trusts exactly that certificate instead of public authorities. |
| `NEXTPANE_TOKEN` | *(empty)* | Enrollment key. Only used for the first connection, afterwards the host secret is used. |
| `NEXTPANE_FIREWALL` | auto-detected | Force `ufw`, `firewalld`, `nftables`, `iptables` or `none`. |
| `NEXTPANE_PATH` | `/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin` | `PATH` for builds and apps. Add the directory of your Node.js if it is not in a standard location. |

Restart after a change: `sudo systemctl restart nextpane-agent`.

## Files

### Panel host

| Path | Content |
|---|---|
| `/usr/bin/nextpane-server` | Binary (web UI embedded). |
| `/etc/nextpane/server.env` | Configuration (root:nextpane-server, 0640). |
| `/var/lib/nextpane-server/nextpane.db` | SQLite database (hosts, apps, encrypted env vars, deployments, audit log). |
| `/var/lib/nextpane-server/tls.crt`, `tls.key` | Self-signed certificate (only with `NEXTPANE_TLS=on` and no certificate files). Back up `tls.key`: a new one changes the fingerprint agents pin. |
| `/var/lib/nextpane-server/secret.key` | 32-byte key encrypting environment variables. **Losing it loses every env var.** |
| `/var/lib/nextpane-server/update/` | Packages downloaded by the self-updater. |

**Backup:** stop the service (or use `sqlite3 nextpane.db ".backup copy.db"`) and save
`nextpane.db` + `secret.key` together.

### App hosts

| Path | Content |
|---|---|
| `/usr/bin/nextpane-agent` | Binary. |
| `/etc/nextpane/agent.env` | Agent configuration (0600). |
| `/etc/nextpane/apps/<app>.env` | App environment (root, 0600). |
| `/etc/nextpane/keep-user` | If present, uninstalling keeps the `nextpane` user and its apps running. |
| `/etc/systemd/system/nextpane-app-<app>.service` | App unit (generated, do not edit). |
| `/var/lib/nextpane/apps/<app>/repo.git` | Bare Git mirror. |
| `/var/lib/nextpane/apps/<app>/releases/<time>-<sha>` | The last 3 releases. |
| `/var/lib/nextpane/apps/<app>/current` | Symlink to the running release. |
| `/var/lib/nextpane/apps/<app>/deploy_key[.pub]` | SSH deploy key for private repositories. |
| `/var/lib/nextpane/.ssh/authorized_keys` | Keys managed in *Host › SSH keys*. |
| `/var/lib/nextpane-agent/host.secret` | Host identity secret (0600). |
| `/var/lib/nextpane-agent/firewall/<app>.json` | Rules last applied for each app. |
