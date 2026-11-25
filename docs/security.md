# Security

## Model

| Component | Runs as | Why |
|---|---|---|
| `nextpane-server` | `nextpane-server` (deb/rpm) or `nextpane` (Docker), sandboxed | Needs no privilege. |
| `nextpane-agent` | `root` | Writes systemd units and firewall rules. It only connects outwards and only executes the fixed command set of the protocol (no shell, no arbitrary command from the panel). |
| Apps, installs, builds | `nextpane` (unprivileged) | `npm install` runs third-party code: it never runs as root. |

Inside the agent, every filesystem write under `/var/lib/nextpane` (owned by `nextpane`) is performed
**as `nextpane`**, so a compromised app cannot trick root into following a symlink it planted.

## Authentication

- **Admin**: one account, password hashed with Argon2id. Sessions are random 256-bit tokens, stored
  hashed, in an `HttpOnly; SameSite=Strict` cookie (plus `Secure` behind HTTPS), valid 7 days. Failed
  logins are slowed down. Changing the password revokes every session.
- **First setup** needs the one-time code printed in the server logs, so an exposed fresh install
  cannot be claimed by someone else.
- **Agents**: the first connection presents an **enrollment key** (`npk_…`, stored hashed). The panel
  answers with a per-host secret that the agent keeps in `/var/lib/nextpane-agent/host.secret` (0600).
  Every later connection must present that secret for its machine-id. Revoking an enrollment key
  stops new enrollments. Deleting a host in the panel disconnects its agent; it can only come back by
  re-enrolling with a still-valid key, hence: revoke keys once a rollout is done.
- **Webhooks**: one random 256-bit secret per app, in the URL. Only triggers a deploy of the
  configured branch.

## Secrets

- **Environment variables** are encrypted at rest (AES-256-GCM) with `secret.key`
  (`NEXTPANE_DATA/secret.key`, 0600). Back it up with the database, and keep the backups apart.
- On hosts they live in `/etc/nextpane/apps/<app>.env` (root, 0600), read by systemd; the app user
  only sees them as its process environment.
- **Exports** require the password again, are audit-logged, and can be encrypted with a passphrase
  (OpenSSL AES-256-CBC, PBKDF2-SHA256 600 000 iterations).
- **Deploy keys** are generated on the host; the private key never transits.
- The audit log records enrollments, deployments, env changes (keys only, never values), exports,
  SSH keys, deletions and updates.

## Transport

Run the panel behind TLS (see the reverse proxy section of [Installation](installation.md)) and use
`wss://` for agents: env vars travel to the hosts over that channel. The agent verifies the server
certificate against the Mozilla root store bundled in the binary.

## Input validation

All app settings are validated on the panel **and** again on the agent: app names
(`[a-z0-9-]{1,24}`), branches (no leading `-`, no `..`), repository URLs (no leading `-`, no spaces),
commands (no line breaks, so nothing can be injected into unit files), cache paths (relative, no
`..`), firewall ports/protocols/CIDRs, env variable names, SSH public keys. Git, systemctl and firewall
tools are always spawned with an argument vector, never through a shell. Values in unit files are
escaped for systemd (`%`, `$`, `\`, `"`).

## Updates

Downloads are verified against the sha256 published in the release manifest, fetched over HTTPS from
`NEXTPANE_RELEASE_URL`. Updates are therefore as trustworthy as that release channel; restrict who
can publish releases. The deb/rpm self-update hands the package to a root path unit, which checks the
package name before installing it.

## Known limitations

- Releases are not signed: the trust anchor is HTTPS + the release host. Add signature verification in
  `packaging/server/update.sh` and the agent if your threat model includes a compromised release
  channel or a compromised panel process.
- All apps of a host share the `nextpane` user: they can read each other's files. Put apps that must
  be isolated from each other on different hosts.
- A single admin account; no roles or SSO.
- Metrics history is not persisted.

## Hardening checklist

- [ ] Panel behind HTTPS, `NEXTPANE_LISTEN=127.0.0.1:3000`.
- [ ] Restrict access to the panel (VPN, IP allow-list in the proxy) if possible.
- [ ] Revoke enrollment keys after a rollout.
- [ ] Back up `nextpane.db` + `secret.key`, encrypted.
- [ ] Use deploy keys instead of tokens in repository URLs.
- [ ] Keep app ports closed in the firewall when a local reverse proxy serves them.
