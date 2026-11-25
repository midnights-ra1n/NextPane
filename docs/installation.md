# Installation

NextPane has two parts:

- **`nextpane-server`**: the panel (web UI, API, agent hub). One per fleet.
- **`nextpane-agent`**: runs on every host that serves apps.

Release assets used below follow this naming (see [Updates](updates.md)):

```
nextpane-server-<version>-<arch>.deb | .rpm | (static binary)
nextpane-agent-<version>-<arch>.deb  | .rpm | (static binary)
latest.json                           # version + sha256 of every asset
```

`<arch>` is `x86_64` or `aarch64`. Binaries are static (musl), so they run on any distribution.

---

## Server

### Docker

```sh
docker run -d --name nextpane --restart unless-stopped \
  -p 3000:3000 -v nextpane-data:/data \
  -e NEXTPANE_RELEASE_URL=https://github.com/midnights-ra1n/NextPane/releases \
  ghcr.io/midnights-ra1n/nextpane:latest
docker logs nextpane | grep "setup code"
```

Or with the bundled `docker-compose.yml`, which adds Caddy for automatic HTTPS:

```sh
PANEL_DOMAIN=panel.example.com docker compose up -d
```

Data (SQLite database **and the encryption key**) lives in the `/data` volume. Back it up.

> In Docker the panel cannot replace itself: update with `docker compose pull && docker compose up -d`.
> The UI shows the available version and this command.

### Debian / Ubuntu (.deb)

```sh
curl -fLO https://github.com/midnights-ra1n/NextPane/releases/latest/download/nextpane-server-<version>-x86_64.deb
sudo apt install ./nextpane-server-<version>-x86_64.deb
sudoedit /etc/nextpane/server.env        # set NEXTPANE_RELEASE_URL to enable one-click updates
sudo systemctl restart nextpane-server
journalctl -u nextpane-server | grep "setup code"
```

### RHEL / AlmaLinux / Rocky (.rpm)

```sh
sudo dnf install https://github.com/midnights-ra1n/NextPane/releases/latest/download/nextpane-server-<version>-x86_64.rpm
sudoedit /etc/nextpane/server.env
sudo systemctl restart nextpane-server
journalctl -u nextpane-server | grep "setup code"
```

Both packages:

- create the system user `nextpane-server` (the panel never runs as root),
- install `nextpane-server.service` (sandboxed: `ProtectSystem=strict`, `NoNewPrivileges`),
- install `nextpane-server-update.path`, which lets the panel update itself from the UI
  (see [Updates](updates.md)),
- keep `/var/lib/nextpane-server` and `/etc/nextpane/server.env` on removal.

### First login

On first start the server prints a one-time **setup code** in its logs. Open the panel, enter the
code, choose a username and a password (10+ characters). The code is required so that nobody who
reaches the panel before you can claim it.

### Reverse proxy (HTTPS)

The panel listens on plain HTTP (`NEXTPANE_LISTEN`, default `0.0.0.0:3000`). Put a TLS reverse proxy
in front of it: browsers get HTTPS and agents get `wss://`. The proxy must pass WebSocket upgrades
for `/agent/ws` and set `X-Forwarded-Proto` (used to mark the session cookie `Secure`).

**Caddy**

```
panel.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

**nginx**

```nginx
server {
    listen 443 ssl http2;
    server_name panel.example.com;
    # ssl_certificate ...; ssl_certificate_key ...;

    location / {
        proxy_pass http://127.0.0.1:3000;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 120s;    # agents ping every 30 s
    }
}
```

When a proxy is in front, bind the panel to localhost: `NEXTPANE_LISTEN=127.0.0.1:3000`.

---

## Agent

### Host requirements

- Linux with **systemd** and **cgroup v2** (default on Debian 11+, Ubuntu 21.10+, RHEL 9+).
  On cgroup v1 hosts apps run, but per-app metrics read 0 and limits may be ignored.
- `git` and `curl` (pulled in by the packages).
- **Node.js** installed system-wide (e.g. NodeSource, distro package, or `/usr/local/bin`). If it lives
  elsewhere, set `NEXTPANE_PATH` (see [Configuration](configuration.md)). pnpm/yarn: enable them with
  `corepack enable`. bun: install it system-wide.

### One-line install (recommended)

In the panel: **Hosts › Add host**. It creates an enrollment key and shows:

```sh
curl -fsSL https://panel.example.com/install.sh | sudo NEXTPANE_SERVER=wss://panel.example.com/agent/ws NEXTPANE_TOKEN=npk_… sh
```

The script picks the `.deb` on apt-based systems, the `.rpm` on dnf/yum-based systems and the static
binary elsewhere (`NEXTPANE_METHOD=binary` forces it). Every file is checked against the sha256 in the
release manifest. `NEXTPANE_VERSION=x.y.z` pins a version.

### Packages

```sh
# Debian / Ubuntu
sudo apt install ./nextpane-agent-<version>-x86_64.deb
# RHEL / Alma / Rocky
sudo dnf install ./nextpane-agent-<version>-x86_64.rpm

sudoedit /etc/nextpane/agent.env    # NEXTPANE_SERVER=wss://…/agent/ws and NEXTPANE_TOKEN=npk_…
sudo systemctl restart nextpane-agent
```

### Many hosts

Use the Ansible role: see [Ansible](ansible.md).

### What the install does

| Item | Purpose |
|---|---|
| user `nextpane` (system, home `/var/lib/nextpane`) | Owns and runs every app. Login shell so you can `ssh nextpane@host` with the keys managed in the panel. |
| `/var/lib/nextpane/apps/<app>/` | Git mirror, releases, current release, deploy key. Owned by `nextpane`. |
| `/etc/nextpane/agent.env` (0600) | Agent configuration. |
| `/etc/nextpane/apps/<app>.env` (root, 0600) | Environment of each app, read by systemd, not readable by the app user. |
| `/var/lib/nextpane-agent/` (root, 0700) | Host secret, firewall state, update downloads. |
| `nextpane-agent.service` | The agent (root: it manages systemd units and the firewall). |
| `/etc/systemd/system/nextpane-app-<app>.service` | One unit per app, written by the agent. |

### Uninstall

```sh
sudo apt remove nextpane-agent      # or: sudo dnf remove nextpane-agent
```

What happens to the `nextpane` user is your choice:

| Mode | How | Result |
|---|---|---|
| **Delete the user** (default) | nothing to do | Apps are stopped and their units removed, firewall rules opened by NextPane are closed, the `nextpane` user is deleted. **Data is kept** in `/var/lib/nextpane` and `/etc/nextpane`. Reinstalling picks it up again. |
| **Keep the user** | `sudo touch /etc/nextpane/keep-user` before removing (or `NEXTPANE_KEEP_USER=1`) | Only the agent is removed. Apps keep running as `nextpane`, unmanaged. |

Remove the host in the panel too (*Host › Settings › Delete host*). To wipe everything:
`sudo rm -rf /var/lib/nextpane /var/lib/nextpane-agent /etc/nextpane`.

### Reinstalling a host

A host is identified by `/etc/machine-id` and proves it with a secret stored in
`/var/lib/nextpane-agent/host.secret`. If that file is lost (fresh install), click
*Host › Settings › Allow re-enrollment* and the next connection with a valid enrollment key takes the
host over again, keeping its apps.
