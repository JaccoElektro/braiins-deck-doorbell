#!/bin/sh
# Open the door, for the Doorbell widget on this Deck. POST only, and uhttpd
# listens on 127.0.0.1 — nothing outside the Deck can reach this. The service
# refuses unless `unlock` is switched on.
echo 'Content-Type: application/json'
echo 'Cache-Control: no-store'
echo
if [ "$REQUEST_METHOD" != POST ]; then
	echo '{"ok":false,"error":"post"}'
	exit 0
fi
/usr/sbin/doorbell unlock
