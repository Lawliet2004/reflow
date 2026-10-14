#!/usr/bin/env bash
set -eu
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_dir=$(CDPATH= cd -- "$script_dir/.." && pwd)
cd "$repo_dir/src-tauri"
if test -z "${REFLOW_KGLOBALACCELD:-}"; then
  for candidate in /usr/libexec/kglobalacceld /usr/lib/*/libexec/kglobalacceld; do
    if test -x "$candidate"; then REFLOW_KGLOBALACCELD="$candidate"; break; fi
  done
fi
: "${REFLOW_KGLOBALACCELD:?Set REFLOW_KGLOBALACCELD to the KDE 6 kglobalacceld executable}"
export REFLOW_KGLOBALACCELD
if test "${REFLOW_PRIVATE_BUS_INNER:-}" != 1; then
  scratch=$(mktemp -d /tmp/reflow-kde-shortcut-private.XXXXXX)
  mkdir -m 700 "$scratch/runtime" "$scratch/config" "$scratch/cache" "$scratch/data"
  export XDG_RUNTIME_DIR="$scratch/runtime" XDG_CONFIG_HOME="$scratch/config" XDG_CACHE_HOME="$scratch/cache" XDG_DATA_HOME="$scratch/data"
  export XDG_SESSION_TYPE=x11 QT_QPA_PLATFORM=offscreen DISPLAY= WAYLAND_DISPLAY= XDG_CURRENT_DESKTOP=
  export REFLOW_PRIVATE_BUS_INNER=1 REFLOW_ISOLATED_KDE_TEST=1 REFLOW_PRIVATE_TEST_DIR="$scratch"
  exec dbus-run-session -- bash "$script_dir/test_kde_shortcuts.sh"
fi
"$REFLOW_KGLOBALACCELD" > "$REFLOW_PRIVATE_TEST_DIR/daemon.log" 2>&1 &
daemon_pid=$!
trap 'kill "$daemon_pid" 2>/dev/null || true; wait "$daemon_pid" 2>/dev/null || true' EXIT
for _ in {1..50}; do
  if busctl --address="$DBUS_SESSION_BUS_ADDRESS" --no-pager list | grep -Eq "^org.kde.kglobalaccel[[:space:]]+$daemon_pid[[:space:]]"; then
    printf 'Private KF6 daemon ready: %s PID %s\n' "$REFLOW_PRIVATE_TEST_DIR" "$daemon_pid"
    exec_path=$(readlink "/proc/$daemon_pid/exe")
    test "$exec_path" = "$(readlink -f "$REFLOW_KGLOBALACCELD")"
    cargo test --lib platform::kde_shortcuts::tests::kde_assignment_round_trip_and_conflicts -- --ignored --nocapture
    exit $?
  fi
  if ! kill -0 "$daemon_pid" 2>/dev/null; then
    cat "$REFLOW_PRIVATE_TEST_DIR/daemon.log"
    wait "$daemon_pid"
    exit 1
  fi
  sleep 0.1
done
cat "$REFLOW_PRIVATE_TEST_DIR/daemon.log"
exit 1
