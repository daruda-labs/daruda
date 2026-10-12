#!/usr/bin/env bash
# Run `<binary> --smoke` under a display server: Xvfb for gpui's X11 client,
# a headless sway for its Wayland one. sway rather than weston: gpui unwraps a
# `wl_seat`, which weston's headless backend does not advertise.
set -euo pipefail

[ $# -eq 2 ] || { echo "usage: $0 {x11|wayland} <binary>" >&2; exit 2; }
backend=$1
binary=$2

case "$backend" in
    x11)
        exec env -u WAYLAND_DISPLAY \
            xvfb-run --auto-servernum --server-args="-screen 0 1280x800x24" "$binary" --smoke
        ;;
    wayland)
        runtime=$(mktemp -d)
        chmod 700 "$runtime"
        : >"$runtime/sway.conf"
        XDG_RUNTIME_DIR=$runtime WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
            WLR_RENDERER=pixman sway -c "$runtime/sway.conf" >"$runtime/sway.log" 2>&1 &
        sway_pid=$!
        trap 'kill "$sway_pid" 2>/dev/null || true; rm -rf "$runtime"' EXIT

        socket=""
        for _ in $(seq 1 50); do
            socket=$(find "$runtime" -maxdepth 1 -type s -name 'wayland-[0-9]*' -printf '%f\n' | head -1)
            [ -n "$socket" ] && break
            sleep 0.2
        done
        if [ -z "$socket" ]; then
            echo "sway did not open a Wayland socket" >&2
            cat "$runtime/sway.log" >&2
            exit 1
        fi
        status=0
        env -u DISPLAY XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$socket" \
            "$binary" --smoke || status=$?
        if [ "$status" -ne 0 ]; then
            echo "--- sway log" >&2
            cat "$runtime/sway.log" >&2
        fi
        exit "$status"
        ;;
    *)
        echo "unknown backend: $backend" >&2
        exit 2
        ;;
esac
