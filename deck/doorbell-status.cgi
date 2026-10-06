#!/bin/sh
# For the Doorbell widget on this Deck (uhttpd listens on 127.0.0.1:8000 only).
echo 'Content-Type: application/json'
echo 'Cache-Control: no-store'
echo
/usr/sbin/doorbell status
