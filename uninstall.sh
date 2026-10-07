#!/usr/bin/env bash
# Remove the doorbell from a Braiins Deck: the widget and the service (curl
# stays installed). Delete the doorbell scene in the Deck's web interface too.
# Usage: ./uninstall.sh <deck-ip>
set -euo pipefail
DECK="root@${1:?usage: ./uninstall.sh <deck-ip>}"
ssh -o BatchMode=yes "$DECK" '/nix/var/nix/gcroots/profiles/bmc/current/bin/bmc-nix-cli remove-packages --name widget-doorbell 2>/dev/null
/etc/init.d/doorbell stop 2>/dev/null; /etc/init.d/doorbell disable 2>/dev/null
rm -rf /usr/sbin/doorbell /etc/init.d/doorbell /www/cgi-bin/doorbell-status /www/cgi-bin/doorbell-jpg /www/cgi-bin/doorbell-unlock \
	/usr/share/doorbell /etc/doorbell /etc/config/doorbell /tmp/doorbell'
echo "Removed the doorbell widget and service."
