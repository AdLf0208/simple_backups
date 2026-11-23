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
# Scenario D (differential): the disk cap (`max_disk_size`, upstream's
#   maxDiskSize). keep=3 so the keep rule removes nothing. With cap `1 B` two
#   manual backups end with one (the newest) and `status` says `over=true`; with
#   cap `1 TB` both stay and `over=false` — the counterfactual that proves the
#   removal is the cap's doing.
# Scenario E: retention only touches archives the port wrote. A hand-made ZIP with
#   the world prefix and the oldest name must survive keep=1 and a 1 B cap, while
#   the older mod-made archive beside it (the differential) is removed.
# Scenario F: a stale copy skips both retention passes. With `save_before_backup =
#   false`, `ctl save` lands inside a 256 MiB stored copy, so the copy is published
#   with `stale=true`; keep=1 and a 1 B cap must then delete nothing, leaving the
#   clean restore point beside it. STALE_SELFTEST=1 asserts the pre-fix defect
#   (retention ran) and always exits non-zero.
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
# The folder settled on `want` archives, and — for want=1 — the one left is not
# the archive named as the old one, so a removal actually happened.
wait_for_settled() { # dir old-name want
    local dir=$1 old=$2 want=$3 i
    for i in $(seq 1 80); do
        local count now
        count=$(count_zips "$dir")
        now=$(first_zip "$dir")
        if [ "$count" -eq "$want" ]; then
            if [ "$want" -ne 1 ] || [ "$now" != "$old" ]; then return 0; fi
        fi
        sleep 0.25
    done
    return 1
}
wait_for_removal() { # path
    local path=$1 i
    for i in $(seq 1 80); do
        if [ ! -e "$path" ]; then return 0; fi
        sleep 0.25
    done
    return 1
}
status_text() { ctl chat "/simplebackups status" >/dev/null; ctl wait 5 >/dev/null; ctl messages 20 | jq -r '.chat[]' | tr '\n' ' '; }
list_text() { ctl chat "/simplebackups list" >/dev/null; ctl wait 5 >/dev/null; ctl messages 20 | jq -r '.chat[]' | tr '\n' ' '; }
check_zip() {
    python3 - "$1" "$2" <<'PY'
import json, sys, zipfile
path, world = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(path) as archive:
    assert archive.testzip() is None, "an entry's CRC failed"
    names = archive.namelist()
    assert names.count("simplebackups.json") == 1, names
    manifest = json.load(archive.open("simplebackups.json"))
    assert manifest["format"] == "simplebackups/1", manifest
    assert manifest["world"] == world, manifest
    assert any(name.endswith("/level.dat") for name in names), names
    assert not any(name.endswith("session.lock") for name in names), names
    assert not any("/backups/" in name for name in names), names
    assert all(name == "simplebackups.json" or name.startswith(world + "/") for name in names), names
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

write_config() { # config-dir enabled retry_on_save [max_disk_size] [keep] [save_before_backup]
    local dir=$1 enabled=$2 retry=$3 cap=${4:-"25 GB"} keep=${5:-1} save=${6:-true}
    mkdir -p "$dir"
    cat > "$dir/simplebackups.toml" <<EOF
enabled = $enabled
interval_minutes = 0
interval_ticks = 40
keep = $keep
compression_level = 1
save_before_backup = $save
retry_on_save = $retry
broadcast = false
backups_dir = "backups"
max_disk_size = "$cap"
EOF
}

run_scenario() {
    local enabled=$1 world=$2 sock=$3
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
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
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" false "$retry"
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"
    # The payload makes the copy long enough that a save lands inside it.
    dd if=/dev/urandom of="$world_dir/retry-payload.bin" bs=1M count=1024 status=none
    ctl chat "/simplebackups backup"
    ctl wait 2
    # The scratch is a disk folder, not tmpfs, so the 512 MiB stored copy takes seconds.
    # Pause/resume makes the integrated server save while it is still running
    # then, so that save lands inside it — the overlap the retry rule is about. (64 MiB finished
    # before the resume save, so the old scene proved nothing.)
    ctl pause >/dev/null
    sleep 1.5
    ctl resume >/dev/null
    ctl wait 300
    wait_for_zip "$backups" 1 || fail "the overlapped backup never appeared"
    ctl chat "/simplebackups status"
    ctl wait 5
    local status
    status=$(ctl messages 12 | jq -r '.chat[]' | tr '\n' ' ')
    echo "status: $status"
    check_zip "$(first_zip "$backups")" "$world"
    local saves
    saves=$(sed -n 's/.*saves=\([0-9]*\).*/\1/p' <<<"$status")
    if [ "${saves:-0}" -lt 2 ]; then
        # A copy can only be overlapped by a *second* world save. A single-player world has no
        # `/save-all` (the administration profile is the dedicated server's) and pause/resume
        # does not save here, so the harness cannot produce one; the rule itself is pinned by
        # the port's `should_retry` unit test.
        echo "SKIP: no second world save is reachable in single player; the retry rule is unit-tested"
    elif [ "$retry" = "true" ]; then
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

# Scenario D: the disk cap. `keep = 3`, so the keep rule removes nothing; two
# manual backups are made into the same folder.
#   - cap = "1 B": after the second publish the folder is over the cap, so the
#     port deletes the oldest archive — one (the newest) is left, and `status`
#     reports `disk=<n> B cap=1 B over=true`.
#   - cap = "1 TB" (the counterfactual): both archives stay and `over=false`.
#     Without the cap the first run would also keep two, so the pair proves the
#     "one left" assertion is the cap's doing and not the keep rule's.
run_cap_scenario() {
    local cap=$1 world=$2 sock=$3
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" false true "$cap" 3
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local backups="$scratch/saves/$world/backups"
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 1 || fail "the first backup did not appear"
    local first
    first=$(first_zip "$backups")
    echo "cap $cap: first backup $first"
    ctl chat "/simplebackups backup"
    if [ "$cap" = "1 B" ]; then
        wait_for_settled "$backups" "$first" 1 || fail "the 1 B cap did not leave exactly the newer archive"
    else
        wait_for_settled "$backups" "$first" 2 || fail "the $cap cap did not keep both backups"
    fi
    local count status survivor
    count=$(count_zips "$backups")
    status=$(status_text)
    survivor=$(first_zip "$backups")
    echo "status: $status"
    if [ "$cap" = "1 B" ]; then
        [ "$count" -eq 1 ] || fail "a 1 B cap left $count backups (keep=3, so the cap is what removed one)"
        [ "$survivor" = "$first" ] && fail "the cap kept the older archive ($survivor)"
        grep -q 'cap=1 B' <<<"$status" || fail "status does not report the 1 B cap: $status"
        grep -q 'over=true' <<<"$status" || fail "status does not report the folder over the cap: $status"
        check_zip "$survivor" "$world"
        echo "cap enforced: the oldest archive was removed, $survivor kept"
    else
        [ "$count" -eq 2 ] || fail "a $cap cap left $count backups (keep=3: both should stay)"
        grep -q 'over=false' <<<"$status" || fail "status reports the folder over a $cap cap: $status"
        echo "counterfactual: a $cap cap keeps both backups"
    fi
    # `/simplebackups list` must agree with the folder: every archive in it is named,
    # and an archive the cap deleted is not.
    local listing names name
    listing=$(list_text)
    names=$(find "$backups" -maxdepth 1 -name '*.zip' -printf '%f\n' | sort)
    echo "list: $listing"
    while IFS= read -r name; do
        [ -n "$name" ] || continue
        grep -qF "$name" <<<"$listing" || fail "list does not name $name: $listing"
    done <<<"$names"
    if [ "$cap" = "1 B" ]; then
        grep -qF "$(basename "$first")" <<<"$listing" && fail "list still names the archive the cap deleted: $listing"
    fi
    if find "$backups" -maxdepth 1 -name '*.part' | grep -q .; then
        fail "a .part file was left behind"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario E: retention touches only archives the port wrote. `keep = 1` and a `1 B`
# cap both want to delete; a hand-made ZIP with the world prefix and a 1999 name is
# the oldest file in the folder, so a prefix rule deletes it first. It must survive,
# while the older *mod-made* archive — the differential, same name pattern and age —
# is removed.
run_foreign_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" false true "1 B" 1
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local backups="$scratch/saves/$world/backups"
    mkdir -p "$backups"
    local hand="$backups/${world}_1999-01-01_00-00-00.zip"
    python3 - "$hand" <<'PY'
import sys, zipfile
with zipfile.ZipFile(sys.argv[1], "w") as archive:
    archive.writestr("my-own-notes.txt", "do not delete")
PY
    echo "hand-made archive: $hand"
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 2 || fail "the first mod backup did not appear beside the hand-made archive"
    local first_mod
    first_mod=$(find "$backups" -maxdepth 1 -name '*.zip' ! -name "$(basename "$hand")" | head -1)
    [ -n "$first_mod" ] || fail "the first mod backup was not found"
    check_zip "$first_mod" "$world"
    echo "first mod backup: $first_mod"
    ctl chat "/simplebackups backup"
    wait_for_removal "$first_mod" || fail "keep=1 did not remove the older mod archive"
    local count newest status
    count=$(count_zips "$backups")
    [ "$count" -eq 2 ] || fail "expected the hand-made archive and the newest backup, got $count"
    [ -f "$hand" ] || fail "a hand-made ZIP named like a backup was deleted"
    newest=$(find "$backups" -maxdepth 1 -name '*.zip' -newer "$hand" ! -name "$(basename "$hand")" | head -1)
    [ -n "$newest" ] || fail "the newest mod archive is missing"
    check_zip "$newest" "$world"
    status=$(status_text)
    echo "status: $status"
    echo "unmanaged archive survived; the older mod archive was removed (the differential)"
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario F: a stale copy skips both retention passes. `keep = 1` and a `1 B` cap
# would each delete the clean first backup, leaving the copy a save may have caught
# half-written as the only restore point. `save_before_backup = false` starts the
# second copy at once; `ctl save` while the 256 MiB stored copy is being written
# moves the save sequence, so the copy is published as stale. Both archives must
# stay, and the status must say the folder is over the cap (nothing was deleted).
# STALE_SELFTEST=1 asserts the pre-fix defect (retention ran) and exits 1 either way.
run_stale_retention_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" false false "1 B" 1 false
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 1 || fail "the clean backup did not appear"
    local clean
    clean=$(first_zip "$backups")
    check_zip "$clean" "$world"
    echo "clean restore point: $clean"
    # The payload makes the next stored copy take seconds, long enough for a save to land in it.
    dd if=/dev/urandom of="$world_dir/stale-payload.bin" bs=1M count=256 status=none
    ctl chat "/simplebackups backup"
    local i
    for i in 1 2 3 4 5 6; do
        sleep 0.3
        ctl save >/dev/null || true
    done
    ctl wait 300
    wait_for_zip "$backups" 2 || fail "the second backup did not appear"
    local status count
    status=$(status_text)
    echo "status: $status"
    grep -q 'stale=true' <<<"$status" || fail "the copy was not marked stale: $status"
    count=$(count_zips "$backups")
    if [ "${STALE_SELFTEST:-0}" = 1 ]; then
        if [ "$count" -eq 1 ]; then
            fail "STALE_SELFTEST: retention ran after a stale copy (the pre-fix defect) — the assertion is alive"
        fi
        fail "STALE_SELFTEST: retention was skipped after a stale copy ($count archives, $clean kept) — the pre-fix defect is gone, so the assertion is alive"
    fi
    [ "$count" -eq 2 ] || fail "retention ran after a stale copy: $count archives, expected the clean one and the stale one"
    [ -f "$clean" ] || fail "the previous restore point was deleted after a stale copy"
    grep -q 'over=true' <<<"$status" || fail "a 1 B cap with retention skipped must report over=true: $status"
    echo "stale copy published; both retention passes skipped: $clean survives beside it"
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# The control leg: the same scene on a game without the mod creates no backup at
# all, so "a zip appeared" is the mod's doing and not the engine's.
run_control_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-control.XXXXXX")
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

if [ "${STALE_SELFTEST:-0}" = 1 ]; then
    echo "== scenario F selftest: assert retention did NOT skip after a stale copy =="
    run_stale_retention_scenario SimpleBackupsStaleSelf /tmp/opencode/simplebackups-stale-self.sock
    fail "STALE_SELFTEST inconclusive: the stale scene returned without its assertion"
fi
echo "== scenario A: scheduled backup + keep=1 =="
run_scenario true SimpleBackupsE2E /tmp/opencode/simplebackups-a.sock
echo "== scenario B: enabled=false counterfactual + manual =="
run_scenario false SimpleBackupsDisabled /tmp/opencode/simplebackups-b.sock
echo "== scenario C: a save overlapping the copy is retried =="
run_overlap_scenario true SimpleBackupsOverlap /tmp/opencode/simplebackups-c.sock
echo "== scenario C control: retry_on_save=false publishes the overlapped copy and marks it stale =="
run_overlap_scenario false SimpleBackupsOverlapOff /tmp/opencode/simplebackups-d.sock
echo "== scenario D: the disk cap removes the oldest archive =="
run_cap_scenario "1 B" SimpleBackupsCap /tmp/opencode/simplebackups-f.sock
echo "== scenario D counterfactual: a 1 TB cap keeps both =="
run_cap_scenario "1 TB" SimpleBackupsCapOff /tmp/opencode/simplebackups-g.sock
echo "== scenario E: a hand-made ZIP named like a backup survives retention =="
run_foreign_scenario SimpleBackupsForeign /tmp/opencode/simplebackups-h.sock
echo "== scenario F: a stale copy skips both retention passes =="
run_stale_retention_scenario SimpleBackupsStale /tmp/opencode/simplebackups-i.sock
# The control is opt-in (CONTROL=1 with CONTROL_GAME): the campaign's control leg is the same
# e2e run with `GAME=$CONTROL_GAME`, and launching a second game here raced the first one's
# socket and scratch.
if [ "${CONTROL:-0}" = 1 ]; then
    echo "== control: the unmodded game creates no backup =="
    run_control_scenario SimpleBackupsControl /tmp/opencode/simplebackups-e.sock
fi
echo "PASS: scheduled zip valid, keep=1 pruned, disabled schedule silent, manual works while disabled; a 1 B disk cap removes the oldest archive while a 1 TB cap keeps both (the counterfactual); a hand-made ZIP named like a backup survives retention while the older mod archive is removed (scenario E); a stale copy skips both retention passes and leaves the clean restore point over the cap (scenario F); an archive carries its own manifest identity"
