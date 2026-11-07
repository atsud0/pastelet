#!/usr/bin/env bash
set -euo pipefail

engine=${CONTAINER_ENGINE:-docker}
image=${1:-rpaste:local}
workspace=$(mktemp -d)
name="rpaste-smoke-${workspace##*/}"
volume="$name-data"
container_started=false
volume_created=false

cleanup() {
    status=$?
    trap - EXIT
    if [[ "$container_started" == true ]]; then
        if (( status != 0 )); then "$engine" logs "$name" >&2 || true; fi
        "$engine" rm -f "$name" >/dev/null || true
    fi
    if [[ "$volume_created" == true ]]; then "$engine" volume rm "$volume" >/dev/null || true; fi
    rm -rf "$workspace"
    exit "$status"
}
trap cleanup EXIT

"$engine" volume create "$volume" >/dev/null
volume_created=true
"$engine" run -d --name "$name" --read-only --cap-drop ALL \
    --security-opt no-new-privileges:true --tmpfs /tmp:rw,noexec,nosuid,nodev,size=64m,mode=1777 \
    -v "$volume:/data" -p 127.0.0.1::3000 \
    -e API_TOKEN=smoke-token -e BASE_URL=http://localhost:3000 "$image" >/dev/null
container_started=true
address=$("$engine" port "$name" 3000/tcp)
base="http://$address"
export SMOKE_BASE="$base" SMOKE_STATE="$workspace/state.json"

python3 scripts/test-container.py create
test "$("$engine" exec "$name" id -u)" = 10001
"$engine" stop --time 30 "$name" >/dev/null
test "$("$engine" inspect --format '{{.State.ExitCode}}' "$name")" = 0
"$engine" start "$name" >/dev/null
address=$("$engine" port "$name" 3000/tcp)
export SMOKE_BASE="http://$address"
python3 scripts/test-container.py verify
"$engine" exec "$name" curl --fail --silent --max-time 4 http://127.0.0.1:3000/ >/dev/null
printf '%s\n' 'Container smoke test passed: API, password, burn, non-root, graceful shutdown and persistence.'
