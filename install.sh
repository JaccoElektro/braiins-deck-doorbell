#!/usr/bin/env bash
# Install the doorbell on a Braiins Deck: the service, and the widget from
# prebuilt/ (no Rust or SDK needed). Safe to re-run: settings and credentials
# you entered before are kept unless you choose to replace them.
#
# Usage: ./install.sh <deck-ip> [--widget-only]
set -euo pipefail

DECK_IP="${1:?usage: ./install.sh <deck-ip> [--widget-only]}"
WIDGET_ONLY=0; [ "${2:-}" = --widget-only ] && WIDGET_ONLY=1
DECK="root@$DECK_IP"
HERE="$(cd "$(dirname "$0")" && pwd)"
# -n: these never read stdin, so piped answers reach the prompts below.
SSH=(ssh -n -o BatchMode=yes -o ConnectTimeout=10 "$DECK")
SSHIN=(ssh -o BatchMode=yes -o ConnectTimeout=10 "$DECK")
FEED="https://downloads.openwrt.org/releases/22.03.4/packages/arm_cortex-a7_neon-vfpv4/packages"
sha256() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }

echo "==> Checking the Deck at $DECK_IP"
if ! "${SSH[@]}" true; then
	echo "Cannot log in to $DECK; set up key login once with: ssh-copy-id $DECK" >&2
	exit 1
fi
install_widget() {
	pkg=$(ls "$HERE"/prebuilt/doorbell-widget-*.tar.gz 2>/dev/null | head -n 1)
	if [ -z "$pkg" ]; then
		echo "No widget package in prebuilt/; build one with ./package.sh" >&2
		exit 1
	fi
	( cd "$HERE/prebuilt" && sha256 -c --quiet SHA256SUMS ) || { echo "prebuilt/ does not match its checksum" >&2; exit 1; }
	version=$(basename "$pkg" .tar.gz); version=${version#doorbell-widget-}
	installed=$("${SSH[@]}" "sed -n 's/.*\"version\": *\"\([^\"]*\)\".*/\1/p' /run/current-profile/lib/bmc-widgets/doorbell/manifest.json 2>/dev/null" || true)
	if [ "$installed" = "$version" ] && [ "$WIDGET_ONLY" = 0 ]; then
		echo "==> The Doorbell widget $version is already on the Deck"
		return
	fi
	if ! "${SSH[@]}" test -x /run/current-profile/bin/bmc-wasm-thin-v0; then
		echo "This Deck's firmware has no widget runtime v0 (bmc-wasm-thin-v0); the widget needs it." >&2
		exit 1
	fi
	echo "==> Installing the Doorbell widget $version"
	"${SSHIN[@]}" 'rm -rf /tmp/bmc-widget-doorbell && tar -xzf - -C /tmp' < "$pkg"
	if ! "${SSH[@]}" "export PATH=/run/current-profile/bin:\$PATH
		path=\$(nix-store --add /tmp/bmc-widget-doorbell 2>/dev/null) && rm -rf /tmp/bmc-widget-doorbell &&
		/nix/var/nix/gcroots/profiles/bmc/current/bin/bmc-nix-cli add-packages --name widget-doorbell --version $version --store-path \$path >/dev/null 2>&1 &&
		test -f /run/current-profile/lib/bmc-widgets/doorbell/manifest.json"; then
		echo "Installing the widget on the Deck failed; see: ssh $DECK tail /var/log/bmc/bmc-nix-cli.log" >&2
		exit 1
	fi
}

if [ "$WIDGET_ONLY" = 1 ]; then
	install_widget
	echo "==> Done."
	exit 0
fi

release=$("${SSH[@]}" '. /etc/openwrt_release; echo "$DISTRIB_RELEASE $DISTRIB_ARCH"')
if [ "$release" != "22.03.4 arm_cortex-a7_neon-vfpv4" ]; then
	echo "This Deck runs OpenWrt $release; the curl packages here are for 22.03.4 arm_cortex-a7_neon-vfpv4." >&2
	exit 1
fi

if ! "${SSH[@]}" command -v curl >/dev/null; then
	echo "==> Installing curl from the official OpenWrt 22.03.4 packages"
	tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
	while read -r sum file; do
		curl -fsSL --retry 3 -o "$tmp/$file" "$FEED/$file"
		echo "$sum  $tmp/$file" | sha256 -c --quiet - || { echo "Checksum mismatch: $file" >&2; exit 1; }
	done < "$HERE/deck/SHA256SUMS"
	scp -q < /dev/null "$tmp"/*.ipk "$DECK:/tmp/"
	"${SSH[@]}" 'cd /tmp && opkg install ./libnghttp2-14_*.ipk ./libcurl4_*.ipk ./curl_*.ipk >/dev/null && rm -f /tmp/*.ipk'
fi

echo "==> Copying the service"
"${SSH[@]}" 'mkdir -p /usr/share/doorbell /etc/doorbell && chmod 700 /etc/doorbell'
scp -q < /dev/null "$HERE/deck/doorbell" "$DECK:/usr/sbin/doorbell"
scp -q < /dev/null "$HERE/deck/doorbell.init" "$DECK:/etc/init.d/doorbell"
scp -q < /dev/null "$HERE/deck/doorbell-status.cgi" "$DECK:/www/cgi-bin/doorbell-status"
scp -q < /dev/null "$HERE/deck/doorbell-jpg.cgi" "$DECK:/www/cgi-bin/doorbell-jpg"
scp -q < /dev/null "$HERE/deck/doorbell-unlock.cgi" "$DECK:/www/cgi-bin/doorbell-unlock"
scp -q < /dev/null "$HERE/deck/scene.lua" "$HERE/deck/ding_dong.mp3" "$DECK:/usr/share/doorbell/"
"${SSH[@]}" 'chmod 755 /usr/sbin/doorbell /etc/init.d/doorbell /www/cgi-bin/doorbell-status /www/cgi-bin/doorbell-jpg /www/cgi-bin/doorbell-unlock'
"${SSHIN[@]}" '[ -f /etc/config/doorbell ] || cat > /etc/config/doorbell' < "$HERE/deck/doorbell.uci"

ask_vto=1
if "${SSH[@]}" test -s /etc/doorbell/vto.conf; then
	read -rp "Keep the doorbell login already on the Deck? [Y/n] " a || a=y
	[[ "$a" =~ ^[Nn] ]] || ask_vto=0
fi
if [ "$ask_vto" = 1 ]; then
	echo "==> Your doorbell (Dahua VTO)"
	read -rp "  Address (IP or host name): " vto_host
	read -rp "  User name [admin]: " vto_user; vto_user=${vto_user:-admin}
	read -rsp "  Password: " vto_pass; echo
	q() { printf "'%s'" "$(printf %s "$1" | sed "s/'/'\\\\''/g")"; }
	printf 'VTO_HOST=%s\nVTO_SCHEME=https\nVTO_USER=%s\nVTO_PASS=%s\nVTO_CHANNEL=1\n' \
		"$(q "$vto_host")" "$(q "$vto_user")" "$(q "$vto_pass")" |
		"${SSHIN[@]}" 'umask 077; cat > /etc/doorbell/vto.conf'
	read -rsp "  The Deck's own web password (empty if it has none): " deck_pw; echo
	printf %s "$deck_pw" | "${SSHIN[@]}" 'umask 077; cat > /etc/doorbell/deck-password'
	read -rp "  Language — en or nl [en]: " lang
	[ "${lang:-en}" = nl ] && "${SSH[@]}" 'uci set doorbell.main.lang=nl; uci commit doorbell'
fi
"${SSH[@]}" 'chmod 600 /etc/doorbell/*'

echo "==> Testing the doorbell from the Deck"
code=$("${SSH[@]}" '. /etc/doorbell/vto.conf; /usr/sbin/doorbell auth; curl -sk --digest -K /tmp/doorbell/auth -o /dev/null -w "%{http_code}" --max-time 10 "$VTO_SCHEME://$VTO_HOST/cgi-bin/snapshot.cgi?channel=$VTO_CHANNEL"' || true)
case "$code" in
200) echo "  The Deck can see the doorbell." ;;
401) echo "  The doorbell refused the user name or password; run ./install.sh again." >&2; exit 1 ;;
*) echo "  The Deck cannot reach the doorbell (HTTP $code); check its address." >&2; exit 1 ;;
esac

"${SSH[@]}" '/etc/init.d/doorbell enable; /etc/init.d/doorbell restart' 2>/dev/null || true
install_widget
sleep 2
echo "==> Status: $("${SSH[@]}" /usr/sbin/doorbell status)"
echo "==> Done. In the Deck's web interface, add the \"Doorbell\" widget to a new full-screen scene"
echo "    and switch that scene off. Test with: ssh $DECK doorbell ring"
