# Ansible

The role `ansible/roles/nextpane_agent` installs, configures, upgrades and removes the agent. It works
on one host or hundreds, and is idempotent.

| OS family | Install method |
|---|---|
| Debian, Ubuntu | `.deb` through `apt` |
| RHEL, AlmaLinux, Rocky, Fedora | `.rpm` through `dnf` |
| anything else | static binary + systemd unit |

Requires Ansible 2.15+ on the control node; hosts need Python (standard for Ansible) and systemd.

## 1. Create an enrollment key

In the panel: *Settings › Enrollment keys › New key* (or *Hosts › Add host*). One key can enroll any
number of hosts. Each host then gets its own secret, so you can revoke the key once the rollout is done.

## 2. Inventory and variables

`ansible/inventory.ini`:

```ini
[nextpane]
web-1.example.com
web-2.example.com
10.0.0.12 ansible_user=debian

[nextpane:vars]
nextpane_server_url=wss://panel.example.com/agent/ws
nextpane_release_url=https://github.com/midnights-ra1n/NextPane/releases
```

Keep the enrollment key in a vault:

```sh
mkdir -p ansible/group_vars/nextpane
echo 'nextpane_enroll_token: "npk_…"' > ansible/group_vars/nextpane/vault.yml
ansible-vault encrypt ansible/group_vars/nextpane/vault.yml
```

## 3. Run

```sh
cd ansible
ansible-playbook playbooks/agent.yml --ask-vault-pass
```

`playbooks/agent.yml` targets the `nextpane` group, 20 hosts at a time (`serial: 20`). Hosts appear in
the panel as soon as their agent connects.

## Variables

| Variable | Default | Description |
|---|---|---|
| `nextpane_state` | `present` | `present` installs/upgrades, `absent` uninstalls. |
| `nextpane_server_url` | `""` | Panel WebSocket URL, `wss://<panel>/agent/ws`. Required. |
| `nextpane_enroll_token` | `""` | Enrollment key (`npk_…`). Vault it. |
| `nextpane_release_url` | `""` | Release base URL. Required. |
| `nextpane_version` | `latest` | `latest` or a pinned version (`"0.2.0"`). |
| `nextpane_install_method` | `auto` | `auto`, `package` or `binary`. |
| `nextpane_firewall` | `""` | Force a firewall backend (`ufw`, `firewalld`, `nftables`, `iptables`, `none`). |
| `nextpane_path` | `""` | `PATH` for builds and apps (e.g. a custom Node.js location). |
| `nextpane_keep_user` | `false` | On uninstall, keep the `nextpane` user and let its apps run. |

## Common tasks

**Upgrade every agent** to the latest release (agents can also be updated one by one from the UI):

```sh
ansible-playbook playbooks/agent.yml --ask-vault-pass
```

**Pin a version** on a group: `nextpane_version: "0.2.0"` in its group_vars, then run the playbook.

**Canary rollout**:

```sh
ansible-playbook playbooks/agent.yml -l web-1.example.com -e nextpane_version=0.3.0
```

**Install Node.js first**, in the same play:

```yaml
- hosts: nextpane
  become: true
  pre_tasks:
    - name: Node.js 22 (NodeSource)
      ansible.builtin.shell: curl -fsSL https://deb.nodesource.com/setup_22.x | bash - && apt-get install -y nodejs
      args:
        creates: /usr/bin/node
      when: ansible_os_family == 'Debian'
  roles:
    - nextpane_agent
```

**Uninstall**, deleting the `nextpane` user (data kept in `/var/lib/nextpane`):

```sh
ansible-playbook playbooks/agent-uninstall.yml
```

**Uninstall but keep the user** and its apps running:

```sh
ansible-playbook playbooks/agent-uninstall.yml -e nextpane_keep_user=true
```

## Using the role from another repository

The role uses two files from `packaging/agent/` through symlinks (`files/setup.sh`,
`files/nextpane-agent.service`). When copying the role elsewhere, copy the targets, not the links
(`cp -rL ansible/roles/nextpane_agent …`).

```yaml
# requirements.yml
roles:
  - name: nextpane_agent
    src: https://github.com/midnights-ra1n/NextPane
    version: stable
```

## What the role does

1. Reads `latest.json` from the release to resolve the version and the asset checksums.
2. Installs `git`, writes `/etc/nextpane/agent.env` (0600) from the variables.
3. Downloads the asset for the host (`get_url` with `checksum: sha256:…`).
4. Installs it with `apt`/`dnf`, or for the binary method copies the binary, the systemd unit and
   `setup.sh`, then runs `setup.sh install` (creates the `nextpane` user and directories).
5. Enables and starts `nextpane-agent`; configuration changes trigger a restart.
