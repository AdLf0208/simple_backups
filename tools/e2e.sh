#!/usr/bin/env bash
# In-game check for Simple Backups.
#
# Scenario A: interval_ticks=40, keep=1, enabled=true.
#   1. wait for a scheduled zip; assert level.dat is inside and no session.lock
#      or backups/ subtree is.
#   2. `/simplebackups backup` -> a second backup; keep=1 leaves one zip.
# Scenario B (counterfactual): enabled=false, interval_ticks=40.
#   3. wait 100 ticks -> zero zips (the schedule is off).
#   4. `/simplebackups backup` -> one zip (manual ignores `enabled`).
# Scenario C (differential): a save that lands while the copy is in flight. A
#   64 MiB payload makes the copy busy; `ctl pause` makes the integrated server
#   save right then. With retry_on_save=true the port throws the copy away and
#   redoes it (`retries=1`, `stale=false`); with retry_on_save=false it publishes
#   it and says so (`retries=0`, `stale=true`). The second run is the proof the
#   first assertion can fail.
#
# Since the SDK migration (2026-11-18) the mod is a flat `.hmod` the launcher
# compiles into the game, so GAME must name a game the launcher built with it:
#   horizon-launcher native-build ports/simplebackups --instance <instance> --trust-native-code
# and then `GAME=<that horizon-client> tools/e2e.sh`. The retired
# `tools/horizon.py build` output below no longer exists; the script fails with
# "missing binary" rather than running an unmodded game.
#
# SELFTEST=1 inverts scenario A's assertion — it waits for the scheduled zip and
# FAILS when one appears, which proves the assertion is not vacuous. It always
# exits non-zero.
#
# CONTROL_GAME=<unmodded horizon-client> adds the control leg: the same world
# folder, 100 ticks, and no `<world>/backups` is created. The shared unmodded
# game at ports/_shared/bin/horizon-1d63e0a5/horizon-client is such a game.
#
# CONFIG_DIR names the folder the game reads `simplebackups.toml` from. The game
# takes the instance's `config/`; for a launcher instance that is
# `<instance>/config`, and the scratch default keeps the pre-launcher behaviour.
set -euo pipefail
trap 'status=$?; echo "EXIT:$status"' EXIT

CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
PORT=$CLONE/ports/simplebackups
SHARED=$CLONE/ports/_shared/bin/horizon-1d63e0a5
CTL=${CTL:-$SHARED/horizon-ctl}
GAME=${GAME:-$PORT/target/horizon-cargo/fast/horizon-client}
CONTROL_GAME=${CONTROL_GAME:-}
RES=${RES:-${HORIZON_RES:-/home/adlf/Desktop/horizon/.local/horizon-client/26.3}}
SELFTEST=${SELFTEST:-0}

[ -x "$CTL" ] || { echo "missing horizon-ctl: $CTL" >&2; exit 1; }
[ -x "$GAME" ] || { echo "missing horizon-client: $GAME" >&2; exit 1; }
if [ -n "$CONTROL_GAME" ]; then
    [ -x "$CONTROL_GAME" ] || { echo "missing control game: $CONTROL_GAME" >&2; exit 1; }
fi
[ -d "$RES/assets" ] || { echo "missing prepared resources: $RES" >&2; exit 1; }

fail() { echo "FAIL: $*" >&2; exit 1; }

count_zips() { find "$1" -maxdepth 1 -name '*.zip' 2>/dev/null | wc -l; }
first_zip() { find "$1" -maxdepth 1 -name '*.zip' 2>/dev/null | head -1; }
wait_for_zip() {
    local dir=$1 want=$2 i
    for i in $(seq 1 60); do
        if [ "$(count_zips "$dir")" -ge "$want" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}
wait_for_new_zip() {
    local dir=$1 old=$2 i
    for i in $(seq 1 80); do
        local now
        now=$(first_zip "$dir")
        if [ -n "$now" ] && [ "$now" != "$old" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}
check_zip() {
    python3 - "$1" "$2" <<'PY'
import sys, zipfile
path, world = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(path) as archive:
    assert archive.testzip() is None, "an entry's CRC failed"
    names = archive.namelist()
    assert any(name.endswith("/level.dat") for name in names), names
    assert not any(name.endswith("session.lock") for name in names), names
    assert not any("/backups/" in name for name in names), names
    assert all(name.startswith(world + "/") for name in names), names
    print(f"zip entries: {len(names)}")
PY
}

# Launch the given game in a scratch home and return with the ctl socket ready.
launch() { # sock scratch game world
    local sock=$1 scratch=$2 game=$3 world=$4
    export HORIZON_HOME=$scratch/game
    export HORIZON_LAUNCHER_HOME=$scratch/launcher
    export XDG_DATA_HOME=$scratch/data
    export XDG_CONFIG_HOME=$scratch/config
    export HORIZON_NO_AUDIO=1
    rm -f "$sock"
    cd "$scratch"
    "$CTL" --socket "$sock" launch \
        --game "$game" --world "$world" --seed 20261103 --creative --headless --wait \
        --resources "$RES"
}

write_config() { # config-dir enabled retry_on_save
    local dir=$1 enabled=$2 retry=$3
    mkdir -p "$dir"
    cat > "$dir/simplebackups.toml" <<EOF
enabled = $enabled
interval_minutes = 0
interval_ticks = 40
keep = 1
compression_level = 1
save_before_backup = true
retry_on_save = $retry
broadcast = false
backups_dir = "backups"
EOF
}

run_scenario() {
    local enabled=$1 world=$2 sock=$3
    local scratch
    scratch=$(mktemp -d /tmp/opencode/simplebackups-home.XXXXXX)
    write_config "${CONFIG_DIR:-$scratch/config}" "$enabled" true
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local backups="$scratch/saves/$world/backups"
    if [ "$enabled" = "true" ]; then
        if [ "$SELFTEST" = 1 ]; then
            if wait_for_zip "$backups" 1; then
                fail "SELFTEST: the scheduled backup appeared ($(first_zip "$backups")), so the assertion is alive"
            fi
            fail "SELFTEST inconclusive: the schedule produced no backup at all"
        fi
        wait_for_zip "$backups" 1 || fail "no scheduled backup in $backups"
        local scheduled
        scheduled=$(first_zip "$backups")
        echo "scheduled backup: $scheduled"
        check_zip "$scheduled" "$world"
    else
        ctl wait 100
        if [ "$(count_zips "$backups")" -ne 0 ]; then
            fail "enabled=false produced $(count_zips "$backups") backups"
        fi
        echo "counterfactual: no scheduled backup while disabled"
    fi
    ctl chat "/simplebackups backup"
    if [ "$enabled" = "true" ]; then
        wait_for_new_zip "$backups" "$scheduled" || fail "the manual backup did not appear"
        local count
        count=$(count_zips "$backups")
        [ "$count" -eq 1 ] || fail "keep=1 left $count backups"
        check_zip "$(first_zip "$backups")" "$world"
        echo "manual backup pruned to 1: $(first_zip "$backups")"
    else
        wait_for_zip "$backups" 1 || fail "the manual backup did not appear"
        check_zip "$(first_zip "$backups")" "$world"
        echo "manual backup while disabled: $(first_zip "$backups")"
        [ "$(count_zips "$backups")" -eq 1 ] || fail "manual backup count"
    fi
    if find "$backups" -maxdepth 1 -name '*.part' | grep -q .; then
        fail "a .part file was left behind"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario C: a save that lands while the zip is being written. A 64 MiB
# payload in the world folder keeps the copy busy for ~1 s; `ctl pause` makes
# the integrated server save right then (`IntegratedServer.tickServer` saves on
# the pause tick), so the copy overlaps a save.
run_overlap_scenario() {
    local retry=$1 world=$2 sock=$3
    local scratch
    scratch=$(mktemp -d /tmp/opencode/simplebackups-home.XXXXXX)
    write_config "${CONFIG_DIR:-$scratch/config}" false "$retry"
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"
    # The payload makes the copy long enough that a save lands inside it.
    dd if=/dev/urandom of="$world_dir/retry-payload.bin" bs=1M count=64 status=none
    ctl chat "/simplebackups backup"
    ctl wait 2
    ctl pause >/dev/null
    sleep 3
    ctl resume >/dev/null
    ctl wait 20
    wait_for_zip "$backups" 1 || fail "the overlapped backup never appeared"
    ctl chat "/simplebackups status"
    ctl wait 5
    local status
    status=$(ctl messages 12 | jq -r '.chat[]' | tr '\n' ' ')
    echo "status: $status"
    check_zip "$(first_zip "$backups")" "$world"
    if [ "$retry" = "true" ]; then
        grep -q 'retries=1' <<<"$status" || fail "the overlapped copy was not retried: $status"
        grep -q 'stale=false' <<<"$status" || fail "a retried copy is still marked stale: $status"
    else
        grep -q 'retries=0' <<<"$status" || fail "retries happened with retry_on_save=false: $status"
        grep -q 'stale=true' <<<"$status" || fail "the overlapped copy is not marked stale: $status"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# The control leg: the same scene on a game without the mod creates no backup at
# all, so "a zip appeared" is the mod's doing and not the engine's.
run_control_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d /tmp/opencode/simplebackups-control.XXXXXX)
    launch "$sock" "$scratch" "$CONTROL_GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 100
    local backups="$scratch/saves/$world/backups"
    [ "$(count_zips "$backups" 2>/dev/null || echo 0)" -eq 0 ] \
        || fail "the game without the mod wrote a backup"
    [ ! -d "$backups" ] || fail "the game without the mod made $backups"
    echo "control: no backups folder without the mod"
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

echo "== scenario A: scheduled backup + keep=1 =="
run_scenario true SimpleBackupsE2E /tmp/opencode/simplebackups-a.sock
echo "== scenario B: enabled=false counterfactual + manual =="
run_scenario false SimpleBackupsDisabled /tmp/opencode/simplebackups-b.sock
echo "== scenario C: a save overlapping the copy is retried =="
run_overlap_scenario true SimpleBackupsOverlap /tmp/opencode/simplebackups-c.sock
echo "== scenario C control: retry_on_save=false publishes the overlapped copy and marks it stale =="
run_overlap_scenario false SimpleBackupsOverlapOff /tmp/opencode/simplebackups-d.sock
if [ -n "$CONTROL_GAME" ]; then
    echo "== control: the unmodded game creates no backup =="
    run_control_scenario SimpleBackupsControl /tmp/opencode/simplebackups-e.sock
fi
echo "PASS: scheduled zip valid, keep=1 pruned, disabled schedule silent, manual works while disabled, an overlapping save is retried (and marked stale when retry is off)"
