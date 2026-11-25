#!/bin/sh
# Run as root by nextpane-server-update.service once the panel (unprivileged) has downloaded a
# package and verified its sha256 against the release manifest.
# shortcut: only the package name is checked here, not a signature; sign releases and verify them
# here if the panel host must resist a compromised panel process.
set -eu
D=/var/lib/nextpane-server/update
rm -f "$D/pending"
for f in "$D/package.deb" "$D/package.rpm"; do
  [ -f "$f" ] && [ ! -L "$f" ] || continue
  case "$f" in
    *.deb)
      [ "$(dpkg-deb -f "$f" Package)" = nextpane-server ] || { echo "refusing: not a nextpane-server package"; rm -f "$f"; exit 1; }
      DEBIAN_FRONTEND=noninteractive dpkg -i --force-confold "$f" ;;
    *.rpm)
      [ "$(rpm -qp --qf '%{NAME}' "$f")" = nextpane-server ] || { echo "refusing: not a nextpane-server package"; rm -f "$f"; exit 1; }
      rpm -U --replacepkgs "$f" ;;
  esac
  rm -f "$f"
done
