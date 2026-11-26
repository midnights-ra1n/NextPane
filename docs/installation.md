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

Data (SQLite database **and the encryption key**) lives in the `/data` volume. Back it up.

> In Docker the panel cannot replace itself: update with `docker pull` and recreate the container
> (with Compose: `docker compose pull && docker compose up -d`). The UI shows the available version.

### Docker Compose

Download [`docker-compose.yml`](../docker-compose.yml) (no need to clone the repository):

```sh
mkdir nextpane && cd nextpane
curl -fLO https://raw.githubusercontent.com/midnights-ra1n/NextPane/main/docker-compose.yml
```

Everything is set in the file itself:

| Line | Change it to |
|---|---|
| `ports: "3000:3000"` | `"127.0.0.1:3000:3000"` when a reverse proxy runs on the same host (port 3000 then stays closed to the outside), `"8080:3000"` for another port. |
| `NEXTPANE_TLS: "off"` | `"on"` to serve HTTPS directly, without a reverse proxy (setup C). |
| `NEXTPANE_RELEASE_URL` | Empty to disable update checks and `/install.sh`. |
| `--from panel.example.com` | Your domain, only with the bundled Caddy (setup B). |

Pick the setup that matches your infrastructure:

**A. Panel only, port 3000** (you already have a reverse proxy, or you test on a LAN)

```sh
docker compose up -d
docker compose logs nextpane | grep "setup code"
```

The panel is at `http://<host>:3000`. Then:

- **Proxy on the same host** (nginx, Caddy, HAProxy, Apache installed on the system): use
  `"127.0.0.1:3000:3000"` in `ports` and point the proxy to `http://127.0.0.1:3000`
  (configurations in [Reverse proxy](#reverse-proxy-https)).
- **Proxy on another host**: keep `"3000:3000"`, point the proxy to `http://<panel-ip>:3000`
  and allow port 3000 only from the proxy's IP in the panel host's firewall. Traffic between the
  proxy and the panel is plain HTTP: keep it on a private network or a VPN.
- **Proxy running in Docker** (Traefik, Nginx Proxy Manager, Caddy…): either target
  `http://<host-ip>:3000`, or remove `ports`, attach the panel to the proxy's network and target
  `http://nextpane:3000`:

  ```yaml
  services:
    nextpane:
      # ...
      networks: [default, proxy]
  networks:
    proxy:
      external: true          # name of your proxy's network (docker network ls)
  ```

Generate the certificate in your proxy as usual (Let's Encrypt, Nginx Proxy Manager, Traefik
`certresolver`…). The proxy only needs to forward WebSockets and `X-Forwarded-Proto`, see below.

**B. Panel + bundled Caddy** (nothing listens on 80/443 on this host)

Replace `panel.example.com` with your domain, use `"127.0.0.1:3000:3000"` in `ports`, then:

```sh
docker compose --profile caddy up -d
docker compose logs nextpane | grep "setup code"
```

Point the DNS of the domain to the host and open ports 80 and 443: Caddy obtains and renews the
Let's Encrypt certificate itself. Always pass `--profile caddy` (or `export COMPOSE_PROFILES=caddy`),
including for `pull`, `up` and `down`.

**C. Panel alone with its own HTTPS, reached by IP** (no proxy, no domain)

Set `NEXTPANE_TLS: "on"`, then:

```sh
docker compose up -d
docker compose logs nextpane | grep -E "setup code|FINGERPRINT"
```

The panel serves `https://<ip>:3000` with a self-signed certificate, see
[Without a reverse proxy](#without-a-reverse-proxy-access-by-ip). Do not combine it with
`--profile caddy` (Caddy talks plain HTTP to the panel).

**Agents** connect to `wss://<domain>/agent/ws` behind a proxy, `wss://<ip>:3000/agent/ws` in
setup C, or `ws://<ip>:3000/agent/ws` on a trusted network without TLS.

**Update:** `docker compose pull && docker compose up -d` (add `--profile caddy` for setup B).
**Backup:** `docker compose stop nextpane`, then copy the `nextpane-data` volume
(`docker run --rm -v nextpane_nextpane-data:/data -v "$PWD":/backup alpine tar czf /backup/nextpane-data.tgz -C /data .`).
The volume name is prefixed with the Compose project name (the directory name by default).

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

### Without a reverse proxy (access by IP)

A reverse proxy is optional. Browsers and agents can reach the panel directly at
`<ip>:3000`, in one of three ways:

| `NEXTPANE_TLS` | Panel URL | Agent URL | Use |
|---|---|---|---|
| `on`, no certificate files | `https://<ip>:3000` | `wss://<ip>:3000/agent/ws` + `NEXTPANE_FINGERPRINT` | **Recommended without a proxy.** The panel creates a self-signed certificate (`tls.crt`/`tls.key` in the data directory) once. |
| `on` + `NEXTPANE_TLS_CERT` / `NEXTPANE_TLS_KEY` | `https://<name>:3000` | `wss://<name>:3000/agent/ws` | You already have a certificate (certbot, internal CA, IP certificate). |
| `off` (default) | `http://<ip>:3000` | `ws://<ip>:3000/agent/ws` | Trusted private network or VPN only: passwords, enrollment keys and environment variables travel **unencrypted**. |

**Self-signed certificate.** Nothing is signed by a public authority, so trust works by
**pinning**: at startup the panel logs the certificate fingerprints

```
Self-signed certificate. Agents: NEXTPANE_FINGERPRINT=19073b31…  curl: --pinnedpubkey sha256//tHAy0Bg…=
```

- The **browser** shows a warning the first time: compare the certificate's SHA-256 shown by the
  browser with `NEXTPANE_FINGERPRINT` from the logs, then accept it.
- **Hosts › Add host** then generates commands that already contain both pins:

  ```sh
  curl -fsSLk --pinnedpubkey sha256//<spki> https://203.0.113.10:3000/install.sh \
    | sudo NEXTPANE_SERVER=wss://203.0.113.10:3000/agent/ws NEXTPANE_FINGERPRINT=<sha256> NEXTPANE_TOKEN=npk_… sh
  ```

  `-k` only skips the authority check: `--pinnedpubkey` still makes curl refuse any other key, and
  the agent refuses any certificate whose SHA-256 differs from `NEXTPANE_FINGERPRINT`. With Ansible,
  set `nextpane_server_fingerprint` (see [Ansible](ansible.md)).
- Keep `tls.key` and back it up with the data directory: a new certificate means a new fingerprint
  to update in `/etc/nextpane/agent.env` on every host. Deleting `tls.crt`/`tls.key` generates a new one.

**Own certificate.** Point `NEXTPANE_TLS_CERT` (full chain) and `NEXTPANE_TLS_KEY` to PEM files
readable by the `nextpane-server` user (in Docker: mount them, e.g. `- /etc/letsencrypt:/etc/letsencrypt:ro`).
No fingerprint is needed: agents check it against public authorities. The certificate is read at
startup, so restart the panel after a renewal (certbot: `--deploy-hook "systemctl restart nextpane-server"`).

### Reverse proxy (HTTPS)

With `NEXTPANE_TLS=off`, the panel listens on plain HTTP (`NEXTPANE_LISTEN`, default `0.0.0.0:3000`)
and a TLS reverse proxy can sit in front of it: browsers get HTTPS and agents get `wss://`. The proxy must pass WebSocket upgrades
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
In Docker, publish the port on localhost instead: `"127.0.0.1:3000:3000"` in the Compose `ports`, or
`-p 127.0.0.1:3000:3000`.

**Nginx Proxy Manager:** create a *Proxy Host* to `http://<host-ip>:3000` (or `nextpane:3000` on a
shared network), tick *Websockets Support*, and request a Let's Encrypt certificate in the *SSL* tab.

**Traefik:** WebSockets and `X-Forwarded-Proto` work out of the box. With the panel on Traefik's
network, add to the `nextpane` service:

```yaml
    labels:
      - traefik.enable=true
      - traefik.http.routers.nextpane.rule=Host(`panel.example.com`)
      - traefik.http.routers.nextpane.entrypoints=websecure
      - traefik.http.routers.nextpane.tls.certresolver=letsencrypt   # your resolver's name
      - traefik.http.services.nextpane.loadbalancer.server.port=3000
```

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
