#!/bin/sh
# The real w1a-otad in a container, listening on 127.0.0.1:7423.
#
#   sh core/tests/e2e/otad-docker.sh start    build and run it
#   sh core/tests/e2e/otad-docker.sh test     run tests/e2e_otad.rs against it
#   sh core/tests/e2e/otad-docker.sh stop
#
# Unmodified otad.c, built like linux-build/otadbuild-run.sh: armv7 Alpine, with
# OTAD_TEST so `arm` skips the key press.  The sources are mounted read-only and the
# build stays inside the container, whose /data and keys vanish with it.  A fake
# connectd state file is what makes it listen on the container's address; BLE is absent
# because there is no D-Bus, and otad carries on without it.
#
# /data is a small tmpfs, like the watch's: on armv7 op_put_begin computes free space
# in a 32-bit long, and a disk with gigabytes free can wrap it negative.
set -e
export PATH=/opt/homebrew/bin:$HOME/.cargo/bin:$PATH
HERE=$(cd "$(dirname "$0")" && pwd)
OTAD=${W1A_OTAD_DIR:-$HERE/../../../../linux-build/tools/w1a-otad}
NAME=typix-otad

case "$1" in
start)
	docker rm -f $NAME >/dev/null 2>&1 || true
	docker run -d --name $NAME --platform linux/arm/v7 \
		-p 127.0.0.1:7423:7423 -v "$OTAD:/w:ro" --tmpfs /data:size=64m \
		-e OTAD_TEST_NOKEY=1 alpine:3.21 sh -ec '
		apk add --no-cache build-base make linux-headers dbus-dev pkgconf >/dev/null
		mkdir -p /obj /data /run
		cd /w
		make BUILD=/obj test >/dev/null
		printf "wifi\ton\nbt\ton\nwlink\t%s\n" "$(hostname -i | cut -d" " -f1)" \
			>/run/w1a-connect.state
		touch /tmp/ready
		exec /obj/w1a-otad-test' >/dev/null
	i=0
	until docker exec $NAME test -e /run/w1a-otad.sock 2>/dev/null; do
		i=$((i + 1))
		[ $i -lt 240 ] || { docker logs $NAME | tail -20; exit 1; }
		sleep 1
	done
	echo "otad is up in container $NAME"
	;;
test)
	cd "$HERE/../.."
	TYPIX_OTAD_CONTAINER=$NAME cargo test --test e2e_otad -- --ignored --test-threads=1 --nocapture
	;;
stop)
	docker rm -f $NAME >/dev/null
	;;
*)
	sed -n 2,7p "$0"
	exit 2
	;;
esac
