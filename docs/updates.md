# Updates

Both the server and the agents update from the web UI, with no SSH session.

## Release channel

Updates come from `NEXTPANE_RELEASE_URL` (server setting), a GitHub-style release URL such as
`https://github.com/midnights-ra1n/NextPane/releases`. NextPane reads:

```
<release_url>/latest/download/latest.json          # what is the latest version?
<release_url>/download/v<version>/<asset>           # the files themselves
```

`latest.json` lists every asset with its sha256:

```json
{
  "version": "0.2.0",
  "files": {
    "nextpane-agent-0.2.0-x86_64.deb": "9f2c…",
    "nextpane-agent-0.2.0-x86_64.rpm": "41ab…",
    "nextpane-agent-0.2.0-x86_64": "c07d…",
    "nextpane-server-0.2.0-aarch64.deb": "…"
  }
}
```

**Every download is verified against this checksum before it is installed.** The release workflow
(`.github/workflows/stable.yml`) produces all of this when a new version lands on the `stable` branch. Any static file
server with the same layout works too (useful for air-gapped networks: mirror the release folder and
point `NEXTPANE_RELEASE_URL` at it).

## Updating the server

*Settings › NextPane server* shows the current and latest version and an **Update** button. What it
does depends on how the server was installed (detected automatically):

| Install | Update mechanism |
|---|---|
| **deb / rpm** | The panel (unprivileged) downloads and verifies the package into `/var/lib/nextpane-server/update/` and drops a `pending` marker. The root-owned `nextpane-server-update.path` unit sees it and runs `/usr/lib/nextpane-server/update.sh`, which checks the package name and installs it with `dpkg -i` / `rpm -U`. The package's post-install restarts the panel. The package database stays consistent. |
| **binary** | The new binary replaces the running one (atomic rename) and the process exits; systemd restarts it. The binary must be writable by the service user. |
| **Docker** | Not possible from inside a container. The UI shows the command: `docker compose pull && docker compose up -d`. |

The panel is unavailable for a few seconds; agents reconnect on their own and the page reloads.

## Updating agents

Each host page shows the agent version and, when a newer release exists, **Update agent to x.y.z**.
The agent:

1. picks the asset for its architecture and install method (deb, rpm or binary),
2. downloads it with `curl` and verifies the sha256,
3. installs it:
   - **deb / rpm**: `dpkg -i` / `rpm -U` launched in a separate transient unit (`systemd-run`), because
     the package restarts the agent and must not be killed with it;
   - **binary**: atomic replacement of `/usr/bin/nextpane-agent`, then exit; systemd restarts it.
4. reconnects with the new version a few seconds later. Apps keep running during the whole process.

To update a whole fleet at once, re-run the Ansible playbook (see [Ansible](ansible.md)), which
installs `nextpane_version: latest` everywhere.

## Versioning

Versions follow semver. The agent protocol is additive: new fields have defaults, so an older agent
keeps working with a newer panel. Update the panel first, then the agents.
