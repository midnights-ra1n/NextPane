#!/bin/sh
# Host setup for the NextPane agent. Used by the deb/rpm scriptlets, install-agent.sh and Ansible.
#   setup.sh install    create the "nextpane" user and directories, enable and (re)start the agent
#   setup.sh uninstall  stop the agent; remove the user unless asked to keep it. Data is always kept.
set -e
USER_NAME=nextpane
HOME_DIR=/var/lib/nextpane

case "$1" in
install)
  if ! getent passwd "$USER_NAME" >/dev/null; then
    # A login shell is needed for "ssh nextpane@host" with the keys managed from the panel.
    useradd --system --user-group --home-dir "$HOME_DIR" --shell /bin/bash "$USER_NAME"
  fi
  mkdir -p "$HOME_DIR/apps"
  # After a reinstall the user may have a new uid: take back the data kept from the previous install.
  if [ "$(stat -c %U "$HOME_DIR")" != "$USER_NAME" ]; then
    chown -R "$USER_NAME:$USER_NAME" "$HOME_DIR"
  fi
  chown "$USER_NAME:$USER_NAME" "$HOME_DIR" "$HOME_DIR/apps"
  chmod 0750 "$HOME_DIR"
  install -d -m 0700 /var/lib/nextpane-agent /etc/nextpane/apps
  [ -f /etc/nextpane/agent.env ] && chmod 0600 /etc/nextpane/agent.env
  systemctl daemon-reload
  systemctl enable nextpane-agent.service >/dev/null 2>&1 || true
  if grep -Eq '^NEXTPANE_SERVER=.+' /etc/nextpane/agent.env 2>/dev/null; then
    systemctl restart nextpane-agent.service
  else
    echo "nextpane-agent: set NEXTPANE_SERVER and NEXTPANE_TOKEN in /etc/nextpane/agent.env, then run: systemctl restart nextpane-agent"
  fi
  ;;
uninstall)
  systemctl disable --now nextpane-agent.service >/dev/null 2>&1 || true
  if [ -e /etc/nextpane/keep-user ] || [ "${NEXTPANE_KEEP_USER:-0}" = 1 ]; then
    echo "nextpane-agent: keeping user '$USER_NAME'; its apps keep running without the agent"
  else
    for unit in /etc/systemd/system/nextpane-app-*.service; do
      [ -e "$unit" ] || continue
      systemctl disable --now "$(basename "$unit")" >/dev/null 2>&1 || true
      rm -f "$unit"
    done
    systemctl daemon-reload
    [ -x /usr/bin/nextpane-agent ] && /usr/bin/nextpane-agent --cleanup-firewall || true
    pkill -u "$USER_NAME" 2>/dev/null || true
    userdel "$USER_NAME" 2>/dev/null || true # no -r: the home directory (app data) is kept
    echo "nextpane-agent: user '$USER_NAME' removed, data kept in $HOME_DIR and /etc/nextpane"
  fi
  ;;
*)
  echo "usage: $0 install|uninstall" >&2
  exit 2
  ;;
esac
