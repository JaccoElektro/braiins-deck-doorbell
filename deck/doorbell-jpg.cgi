#!/bin/sh
# The doorbell's picture for the widget; at most one fetch from the VTO a second.
S=/tmp/doorbell; mkdir -p $S
f=$S/snap.jpg
age=$(( $(date +%s) - $(date -r $f +%s 2>/dev/null || echo 0) ))
if [ "$age" -ge 1 ] && mkdir $S/snap.lock 2>/dev/null; then
	. /etc/doorbell/vto.conf
	[ -s $S/auth ] || /usr/sbin/doorbell auth
	curl -sk --digest -K $S/auth --max-time 8 -o $f.new \
		"${VTO_SCHEME:-https}://$VTO_HOST/cgi-bin/snapshot.cgi?channel=${VTO_CHANNEL:-1}" &&
		[ "$(head -c2 $f.new | hexdump -e '2/1 "%02x"')" = ffd8 ] && mv $f.new $f
	rm -f $f.new; rmdir $S/snap.lock
fi
[ -s $f ] || { echo 'Status: 503'; echo; exit 0; }
echo 'Content-Type: image/jpeg'
echo 'Cache-Control: no-store'
echo "Content-Length: $(wc -c < $f)"
echo
cat $f
