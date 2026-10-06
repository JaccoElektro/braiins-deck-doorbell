#!/usr/bin/env bash
# Remove the doorbell service from a Braiins Deck (curl stays installed).
# Usage: ./uninstall.sh <deck-ip>
set -euo pipefail
DECK="root@${1:?usage: ./uninstall.sh <deck-ip>}"
ssh -o BatchMode=yes "$DECK" '/etc/init.d/doorbell stop 2>/dev/null; /etc/init.d/doorbell disable 2>/dev/null
rm -rf /usr/sbin/doorbell /etc/init.d/doorbell /www/cgi-bin/doorbell-status /www/cgi-bin/doorbell-jpg /www/cgi-bin/doorbell-unlock \
	/usr/share/doorbell /etc/doorbell /etc/config/doorbell /tmp/doorbell'
echo "Removed the doorbell service. Remove the widget with: ./undeploy.sh ${1}"
