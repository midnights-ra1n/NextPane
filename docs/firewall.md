# Firewall

Each app can declare **inbound allow rules**: a port, a protocol (TCP/UDP) and an optional source
(IP or CIDR, IPv4 or IPv6). Edit them in *App › Firewall*. NextPane manages **only its own rules**:
it never changes your default policy or rules you wrote yourself.

> Typical setups: an app behind a reverse proxy on the same host needs **no** rule (the proxy talks to
> it over localhost). Open the app port only if clients reach it directly, ideally restricted to a
> source such as your load balancer's address.

## Backend detection

The agent picks the first backend that is active, and shows it on the host page:

| Order | Backend | Detected when | Persistence |
|---|---|---|---|
| 1 | **ufw** | `ufw status` reports `Status: active` | ufw's own (survives reboots) |
| 2 | **firewalld** | `firewall-cmd --state` is `running` | `--permanent` rules + reload |
| 3 | **nftables** | `nft list tables` works | re-applied by the agent at every start |
| 4 | **iptables** | `iptables -S INPUT` works (legacy or nft) | re-applied by the agent at every start |
| — | none | nothing found | rules are refused with an error |

Force a backend with `NEXTPANE_FIREWALL=ufw|firewalld|nftables|iptables|none` in
`/etc/nextpane/agent.env`.

## What is created

For a rule `3000/tcp from 10.0.0.0/8` on app `web`:

**ufw**

```sh
ufw allow proto tcp from 10.0.0.0/8 to any port 3000 comment 'nextpane:web'
```

**firewalld** (default zone, permanent, then `--reload`)

```sh
firewall-cmd --permanent --add-rich-rule='rule family="ipv4" source address="10.0.0.0/8" port port="3000" protocol="tcp" accept'
```

**nftables**: a chain per app inside the standard `inet filter` table, jumped to from its `input`
chain (created with policy *accept* if the table does not exist):

```
table inet filter {
  chain input { ...; jump np-web; ... }
  chain np-web { ip saddr 10.0.0.0/8 tcp dport 3000 accept }
}
```

**iptables / ip6tables**: a chain per app, jumped to from the top of `INPUT`:

```sh
iptables -N NP-web
iptables -A NP-web -p tcp -s 10.0.0.0/8 --dport 3000 -j ACCEPT
iptables -I INPUT 1 -j NP-web
```

IPv4 sources go to `iptables`, IPv6 sources to `ip6tables`, rules without a source to both.

## Lifecycle

- **Save** in the UI applies the rules immediately (and on every deploy).
- On every agent (re)connection all rules are re-applied: this restores nftables/iptables rules after a
  reboot. For ufw/firewalld nothing happens when the rules did not change.
- The last applied rules are stored in `/var/lib/nextpane-agent/firewall/<app>.json`, which is how
  the agent removes exactly what it added when rules change.
- **Deleting an app** removes its rules. **Uninstalling the agent** (default mode) removes the rules of
  every app (`nextpane-agent --cleanup-firewall`).

## Caveats

- **Docker on the same host** bypasses `INPUT` for published container ports; that does not affect
  NextPane apps (they are not containers), but keep it in mind when reading the rules.
- With **nftables**, if your ruleset lives in a table other than `inet filter` with a *drop* policy,
  the accept in `inet filter` is not enough (each table's base chains are evaluated). Use firewalld or
  ufw, or set `NEXTPANE_FIREWALL=iptables` if your tables are managed by iptables-nft.
- Validation is strict on the panel and again on the agent: ports 1–65535, `tcp`/`udp` only, sources
  must parse as an IP or CIDR. Commands are executed without a shell.
