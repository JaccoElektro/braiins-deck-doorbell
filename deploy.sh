#!/usr/bin/env bash
# Build the Doorbell widget from source and install it on a Braiins Deck — for
# when you change the widget. To install without building, use ./install.sh.
#
# Usage: ./deploy.sh <deck-ip>
set -euo pipefail
DECK_IP="${1:?usage: ./deploy.sh <deck-ip>}"
HERE="$(cd "$(dirname "$0")" && pwd)"
"$HERE/package.sh"
"$HERE/install.sh" "$DECK_IP" --widget-only
