# Development

## Requirements

- Rust (stable), Node.js only for `node --check` (the UI has no build step).
- To test the agent for real: a Linux VM with systemd (Debian 12 or Alma 9) and Node.js.

## Run locally

```sh
# panel on http://localhost:3000, data in ./dev-data
NEXTPANE_DATA=./dev-data cargo run -p nextpane-server
```

The setup code is printed in the terminal. The UI files are embedded at compile time
(`include_bytes!`), so restart `cargo run` after editing `web/`.

Agent, on a Linux VM that can reach your machine:

```sh
cargo build --release -p nextpane-agent
sudo useradd --system --user-group --home-dir /var/lib/nextpane --shell /bin/bash nextpane
sudo install -d -o nextpane -g nextpane /var/lib/nextpane /var/lib/nextpane/apps
sudo NEXTPANE_SERVER=ws://<your-ip>:3000/agent/ws NEXTPANE_TOKEN=npk_… ./target/release/nextpane-agent
```

## Tests and checks

```sh
cargo test                                    # protocol validation, parsers, unit rendering, crypto, versions
cargo clippy --all-targets -- -D warnings
node --check web/app.js
```

CI (`.github/workflows/ci.yml`) runs the same on every commit of every branch and on pull requests.

## Packaging locally

```sh
cargo install cargo-zigbuild cargo-deb cargo-generate-rpm   # + zig (pip install ziglang)
T=x86_64-unknown-linux-musl
cargo zigbuild --release --target $T -p nextpane-agent -p nextpane-server
cargo deb --no-build --target $T -p nextpane-agent -o dist/
cargo generate-rpm --target $T -p crates/agent -o dist/
docker build -t nextpane .
```

## Releasing

Branches: work happens on `dev` (and feature branches); **`stable`** is the release branch.

| Event | Workflow | Result |
|---|---|---|
| Any commit, any branch except `stable`, pull requests | `ci.yml` | clippy, tests, UI and shell syntax checks |
| Push to `stable` | `stable.yml` | CI, then static binaries + `.deb` + `.rpm` (x86_64, aarch64) as workflow artifacts, and image `ghcr.io/<owner>/nextpane:stable` |
| Push to `stable` with a **new** version in `Cargo.toml` | `stable.yml` | all of the above, plus tag `vX.Y.Z`, a GitHub release with `latest.json`, and images `:vX.Y.Z` + `:latest` |

To ship a version:

1. Bump `version` in the root `Cargo.toml` (`[workspace.package]`) on `dev`.
2. Merge `dev` into `stable` (pull request or `git push origin dev:stable`).
3. Panels whose `NEXTPANE_RELEASE_URL` points at the repository's releases offer the update to the
   server and the agents.

A push to `stable` that keeps an already released version only rebuilds and refreshes the `:stable`
image; nothing is published to the release channel.

## Code map

| Path | What |
|---|---|
| `crates/proto/src/lib.rs` | Messages, `AppSpec`, validation (shared trust boundary) |
| `crates/agent/src/main.rs` | Connection loop, enrollment, scheduling |
| `crates/agent/src/apps.rs` | Deploy pipeline, unit files, env files, cache, SSH keys |
| `crates/agent/src/metrics.rs` | `/proc` and cgroup readers |
| `crates/agent/src/firewall.rs` | ufw / firewalld / nftables / iptables |
| `crates/agent/src/update.rs` | Agent self-update |
| `crates/server/src/main.rs` | Config, routes, static files |
| `crates/server/src/hub.rs` | Agent WebSocket, command round-trips |
| `crates/server/src/api.rs` | REST API, deployments, webhooks, env export |
| `crates/server/src/auth.rs` | Setup, login, sessions |
| `crates/server/src/crypto.rs` | Encryption at rest, OpenSSL-compatible export |
| `crates/server/src/update.rs` | Release manifest, server self-update |
| `web/app.js` | The whole UI (hash router, views) |
