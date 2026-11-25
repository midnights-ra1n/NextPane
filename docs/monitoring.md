# Monitoring

Each agent sends one metrics report every `NEXTPANE_METRICS_INTERVAL` seconds (10 by default). A
report is a single JSON frame of roughly 300 bytes plus ~100 bytes per app: a host with 10 apps uses
about **0.1 KB/s**. The panel keeps the last 360 reports per host in memory (one hour at the default
interval) to draw the charts.

## Host vs. apps

Every app runs in its own cgroup (`system.slice/nextpane-app-<app>.service`), so the agent can tell
exactly how much of the host the apps use. The UI always splits a resource in two:

- **apps** (white): sum over the host's NextPane apps,
- **system** (grey): everything else (OS, databases, the agent, other services, page cache excluded).

| Metric | Source | Notes |
|---|---|---|
| CPU | `/proc/stat` | % of the whole host (all cores). Load average shown next to it. |
| App CPU | cgroup `cpu.stat` | Also in % of the whole host, so it adds up with the host value. A 4-core host where one app uses one full core shows 25 %. |
| Memory | `/proc/meminfo` | Used = `MemTotal - MemAvailable` (cache and buffers are not counted). |
| App memory | cgroup `memory.current` | Includes the page cache charged to the app. |
| Disk | `statvfs` on `/var/lib/nextpane` | The filesystem holding the apps. |
| App disk | directory walk of `apps/<app>` | Mirror + releases + `node_modules` + builds. Refreshed every 5 minutes. |
| Network | `/proc/net/dev` | Bytes/s in and out, all interfaces except loopback. |
| Uptime, swap | `/proc/uptime`, `/proc/meminfo` | |

App status (running/stopped) comes from the cgroup (`cgroup.events`: `populated 1`).

## Limits on the charts

When an app has a CPU or memory limit, its page shows usage against the limit; the bar turns red above
90 % of the memory limit, where the kernel starts reclaiming and the app gets close to an OOM kill.

## History

History is in memory only: it is empty after a panel restart and fills up again within minutes. For
long-term retention, scrape the hosts with a dedicated tool (Prometheus node_exporter, Netdata…);
NextPane focuses on the live picture.

## Alerts and notifications

The **Overview** page lists what needs attention: offline hosts, failed deployments, stopped apps,
disks above 85 %, hosts above 90 % memory, apps above 90 % of their memory limit and outdated agents.

To be told without opening the panel, set a webhook in **Settings → Notifications**:

| Format | Body sent |
|---|---|
| Slack / Mattermost | `{"text": "…"}` (incoming webhook) |
| Discord | `{"content": "…"}` |
| Matrix | `m.text` message sent to a room with the Client-Server API |
| Generic JSON | `{"event": "deploy_failed", "message": "…", "ts": 1700000000}` |

Events: `deploy_failed`, `deploy_success`, `host_offline` (sent after one minute without the agent, so
agent restarts and updates stay quiet) and `host_online` (only after an offline notification). The URL
is stored encrypted, and **Send test** checks it before you save.

**Matrix:** set the homeserver URL (e.g. `https://matrix.org`), the room ID (`!abc:example.org`, in
Room settings → Advanced, aliases like `#room:server` are not accepted) and the access token of the
account that posts. Invite that account to the room first. The token is write-only: the panel never
sends it back to the browser.
