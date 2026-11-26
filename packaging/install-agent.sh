#!/bin/sh
# NextPane agent installer (served by the panel at /install.sh).
#
#   curl -fsSL https://panel.example.com/install.sh | sudo NEXTPANE_SERVER=wss://panel.example.com/agent/ws NEXTPANE_TOKEN=npk_... sh
#
# Panel with a self-signed certificate (NEXTPANE_TLS=on), reached by IP; both pins are shown by the panel:
#   curl -fsSLk --pinnedpubkey sha256//<spki> https://203.0.113.10:3000/install.sh | sudo NEXTPANE_SERVER=wss://203.0.113.10:3000/agent/ws NEXTPANE_FINGERPRINT=<sha256> NEXTPANE_TOKEN=npk_... sh
#
# Optional: NEXTPANE_VERSION=x.y.z (default: latest), NEXTPANE_METHOD=package|binary (default: package when apt/dnf/yum exist).
# Every download is checked against the sha256 published in the release manifest.
set -eu

RELEASE_URL="${NEXTPANE_RELEASE_URL:-__RELEASE_URL__}"
SERVER="${NEXTPANE_SERVER:-}"
TOKEN="${NEXTPANE_TOKEN:-}"
FINGERPRINT="${NEXTPANE_FINGERPRINT:-}"
VERSION="${NEXTPANE_VERSION:-}"
METHOD="${NEXTPANE_METHOD:-package}"

die() { echo "nextpane: $*" >&2; exit 1; }
[ "$(id -u)" = 0 ] || die "run as root (sudo)"
[ -n "$SERVER" ] || die "NEXTPANE_SERVER is required (wss://<panel>/agent/ws)"
command -v systemctl >/dev/null || die "systemd is required"
command -v curl >/dev/null || die "curl is required"

case "$(uname -m)" in
  x86_64|amd64) ARCH=x86_64 ;;
  aarch64|arm64) ARCH=aarch64 ;;
  *) die "unsupported architecture $(uname -m)" ;;
esac

TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT

if [ -z "$VERSION" ]; then
  VERSION=$(curl -fsSL "$RELEASE_URL/latest/download/latest.json" | sed -n 's/.*"version" *: *"\([^"]*\)".*/\1/p')
  [ -n "$VERSION" ] || die "cannot read the latest version from $RELEASE_URL"
fi
BASE="$RELEASE_URL/download/v$VERSION"
curl -fsSL -o "$TMP/latest.json" "$BASE/latest.json" || die "cannot download the manifest of $VERSION"

fetch() {
  want=$(grep -o "\"$1\" *: *\"[0-9a-f]\{64\}\"" "$TMP/latest.json" | grep -o '[0-9a-f]\{64\}' || true)
  [ -n "$want" ] || die "release $VERSION has no asset $1"
  curl -fsSL -o "$TMP/$1" "$BASE/$1" || die "download failed: $BASE/$1"
  got=$(sha256sum "$TMP/$1" | cut -d' ' -f1)
  [ "$got" = "$want" ] || die "checksum mismatch for $1"
}

# Configuration first, so the package starts the agent already connected.
mkdir -p /etc/nextpane
if ! grep -Eq '^NEXTPANE_SERVER=.+' /etc/nextpane/agent.env 2>/dev/null; then
  umask 077
  printf 'NEXTPANE_SERVER=%s\nNEXTPANE_TOKEN=%s\nNEXTPANE_FINGERPRINT=%s\n' "$SERVER" "$TOKEN" "$FINGERPRINT" > /etc/nextpane/agent.env
  umask 022
fi

if [ "$METHOD" = package ] && command -v apt-get >/dev/null; then
  fetch "nextpane-agent-$VERSION-$ARCH.deb"
  DEBIAN_FRONTEND=noninteractive apt-get install -y -o Dpkg::Options::=--force-confold "$TMP/nextpane-agent-$VERSION-$ARCH.deb"
elif [ "$METHOD" = package ] && command -v dnf >/dev/null; then
  fetch "nextpane-agent-$VERSION-$ARCH.rpm"
  dnf install -y "$TMP/nextpane-agent-$VERSION-$ARCH.rpm"
elif [ "$METHOD" = package ] && command -v yum >/dev/null; then
  fetch "nextpane-agent-$VERSION-$ARCH.rpm"
  yum install -y "$TMP/nextpane-agent-$VERSION-$ARCH.rpm"
else
  command -v git >/dev/null || die "git is required"
  fetch "nextpane-agent-$VERSION-$ARCH"
  fetch "nextpane-agent.service"
  fetch "nextpane-agent-setup.sh"
  install -m 0755 "$TMP/nextpane-agent-$VERSION-$ARCH" /usr/bin/nextpane-agent
  install -m 0644 "$TMP/nextpane-agent.service" /etc/systemd/system/nextpane-agent.service
  install -D -m 0755 "$TMP/nextpane-agent-setup.sh" /usr/lib/nextpane-agent/setup.sh
  /usr/lib/nextpane-agent/setup.sh install
fi

echo "nextpane: agent $VERSION installed. The host shows up in the panel within a few seconds."
