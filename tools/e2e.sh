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
# Scenario G: a marker-less archive needs an operator. A "manual export" — the world
#   prefix, a 1999 name and the world's *own* files — is indistinguishable from a
#   legacy archive by shape, so it survives both passes; after
#   `/simplebackups adopt <file>` the next publish prunes it (the differential in the
#   same folder).
# Scenario H: two backups of one second. The only free suffix is a pruned one (the base
#   name) and a new archive that reused it would sort behind `-42` and be deleted by
#   the pass that follows. The new archive must be allocated above the highest suffix,
#   and `keep`/the cap must never delete the archive just written.
#   ADOPT_NEGATIVE=1 / SAME_SECOND_NEGATIVE=1 run one scene alone against a package that
#   should fail it (the previous version), and exit non-zero either way.
# Scenario I: the persisted pause pair. `interval_ticks=40`, keep=1. The schedule first makes a
#   zip (the differential, scenario A); `/simplebackups pause` then 150 ticks must make no *new*
#   zip while `status` says paused=true and the flag file beside the world says so. A manual
#   `backup` must still work while paused (upstream's createBackup ignores `paused`), and
#   `/simplebackups unpause` then ≤100 ticks must produce exactly one new zip, named by `list`.
#   SELFTEST=pause runs the same scene and FAILS when no new zip appears during the paused window,
#   so the "no zip while paused" assertion is proved able to fail; it always exits non-zero.
# Scenario J: the persisted schedule counter. interval_ticks=40, keep=3. A first scheduled zip
#   (the differential, scenario A), then a short wait so `status` reports a nonzero `ticks=`, then
#   `ctl quit`. The state file beside the world is read after the close. Relaunch the SAME world
#   from the SAME scratch home: the newest `saves=` line must report a restored `ticks=` no less
#   than the value saved at quit (a reset would report only the few ticks since relaunch). Then 20
#   ticks must make no zip and 40 more exactly one. Only the newest `status`/`list` reply is parsed
#   (a stale line in the 20-message chat buffer made a false red in 0.2.5). SELFTEST=ticks inverts
#   the restored-value assertion and exits non-zero.
# Scenario K: incremental chains (upstream's BackupChain*). With `backup_type =
# "incremental"`, a first full base and a second backup after one world file changed: the second
# archive's `simplebackups.json` must name the base and carry ONLY the changed entry; `status`
# must report `chain=2` and `list` must name both. A fresh world's first backup is full with
# `chain=1` (the differential). `save_before_backup=false` keeps the game from rewriting other
# world files between the two backups, so exactly one entry is changed. SELFTEST=chain inverts
# the manifest/base assertion and exits non-zero.
# Scenario M: the game log is appended to the archive as a flat `backup.log` entry
# (upstream's captureLatestLog, default true). The line asserted is read from the
# scratch `logs/latest.log` before the backup, so it is a concrete line the game
# really wrote. `capture_latest_log = false` is the counterfactual (no entry);
# SELFTEST=latestlog inverts the positive assertion and exits non-zero.
#
# Scenario L: the empty-server policy (noPlayerBackups / noPlayerBackupCount). The harness launches
# a single-player integrated world, so `status`'s `players=<n>` field decides whether a genuinely
# empty window is available; when the owner is online, the online branch is asserted instead and the
# limit is recorded rather than dressing a chat command up as an empty server. SELFTEST=noplayer
# inverts whichever assertion was actually made.
#
# Scenario J2 (differential): a FRESH scratch world has no state file, so its interval starts at
#   world open (scenario J's seeded counter makes one come 10 ticks sooner). It must wait a full
#   interval from its own start; the empty window is sized from the ticks it actually reports,
#   because launch itself advances the counter.
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
# exits non-zero. SELFTEST=pause does the same for scenario I's paused window:
# it FAILS when no new zip appeared during the 150 paused ticks.
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
wait_for_other_zip() { # dir exclude-name
    local dir=$1 exclude=$2 i
    for i in $(seq 1 80); do
        if find "$dir" -maxdepth 1 -name '*.zip' ! -name "$exclude" 2>/dev/null | grep -q .; then
            return 0
        fi
        sleep 0.25
    done
    return 1
}
status_text() { ctl chat "/simplebackups status" >/dev/null; ctl wait 5 >/dev/null; ctl messages 20 | jq -r '.chat[]' | tr '\n' ' '; }
# The newest `saves=` line, retried because the feedback can lag a chat round-trip. `|| true`
# keeps `set -e` from killing the leg when grep finds nothing (a silent EXIT:1 with no FAIL).
newest_status_line() {
    local i line
    for i in $(seq 1 10); do
        line=$(ctl messages 20 | jq -r '.chat[]' 2>/dev/null | grep -m1 'saves=' || true)
        if [ -n "$line" ]; then
            echo "$line"
            return 0
        fi
        ctl wait 1 >/dev/null 2>&1 || true
    done
    return 1
}
list_text() { ctl chat "/simplebackups list" >/dev/null; ctl wait 5 >/dev/null; ctl messages 20 | jq -r '.chat[]' | tr '\n' ' '; }
adopt_text() { ctl chat "/simplebackups adopt $1" >/dev/null; ctl wait 5 >/dev/null; ctl messages 20 | jq -r '.chat[]' | tr '\n' ' '; }
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
    assert all(name in ("simplebackups.json", "backup.log") or name.startswith(world + "/") for name in names), names
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

# The chain scenario's config: the interval stays at 40 ticks, but the two backups are manual so
# the single changed file is written between them with no schedule race.
write_chain_config() { # config-dir backup-type
    local dir=$1 type=$2
    mkdir -p "$dir"
    cat > "$dir/simplebackups.toml" <<EOF
enabled = false
interval_minutes = 0
interval_ticks = 40
keep = 3
compression_level = 1
save_before_backup = false
retry_on_save = true
broadcast = false
backups_dir = "backups"
max_disk_size = "25 GB"
backup_type = "$type"
full_backup_timer = 525960
ignored_paths = ["dimensions", "region"]
# A child must carry exactly the changed world entry; the flat log entry is
# proved in scenario M, not here.
capture_latest_log = false
EOF
}

# Scenario M config: log capture on or off. Manual backups keep the scene short;
# save_before_backup=false avoids a save cycle between the log read and the zip.
write_log_config() { # config-dir capture
    local dir=$1 capture=$2
    mkdir -p "$dir"
    cat > "$dir/simplebackups.toml" <<EOF
enabled = false
interval_minutes = 0
interval_ticks = 40
keep = 3
compression_level = 1
save_before_backup = false
retry_on_save = true
broadcast = false
backups_dir = "backups"
max_disk_size = "25 GB"
capture_latest_log = $capture
EOF
}

# Scenario M: pick a concrete line from the scratch latest.log, take a manual
# backup, then assert the newest archive carries a flat `backup.log` entry with
# that exact line. `off` asserts there is no such entry, `selftest` inverts the
# positive assertion (and always exits non-zero).
run_game_log_scenario() { # world sock mode
    local world=$1 sock=$2 mode=${3:-on}
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog.XXXXXX")
    local capture=true
    [ "$mode" != off ] || capture=false
    write_log_config "${CONFIG_DIR:-$scratch/config}" "$capture"
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10

    # Measured at this pin: the headless launcher writes <scratch>/game/logs/latest.log
    # (HORIZON_HOME=<scratch>/game) while saves live in <scratch>/saves.
    local log="$scratch/game/logs/latest.log"
    [ -f "$log" ] || fail "the launcher wrote no $log (log-path measurement changed)"
    local line
    line=$(grep -F -m1 'Starting integrated minecraft server' "$log" || true)
    [ -n "$line" ] || fail "no concrete server-start line in $log: $(head -3 "$log")"
    echo "game log ($log) line: $line"

    local backups="$scratch/saves/$world/backups"
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 1 || fail "the manual backup did not appear"
    local archive
    archive=$(first_zip "$backups")
    check_zip "$archive" "$world"
    python3 - "$archive" "$line" "$mode" <<'PY'
import sys, zipfile
path, line, mode = sys.argv[1:]
with zipfile.ZipFile(path) as archive:
    names = archive.namelist()
    has = "backup.log" in names
    if mode == "selftest":
        if has and line in archive.read("backup.log").decode("utf-8", "replace"):
            raise SystemExit("FAIL: GAMELOG SELFTEST: backup.log held the chosen line — the assertion is alive")
        raise SystemExit("FAIL: GAMELOG SELFTEST inconclusive: no backup.log with that line either")
    if mode == "off":
        if has:
            raise SystemExit("FAIL: capture_latest_log=false still packed a backup.log")
    else:
        if not has:
            raise SystemExit(f"FAIL: no flat backup.log entry in {names}")
        text = archive.read("backup.log").decode("utf-8", "replace")
        if line not in text:
            raise SystemExit("FAIL: backup.log does not contain the exact chosen line")
PY
    if [ "${mode:-on}" = on ]; then
        echo "backup.log holds the exact line chosen from the scratch latest.log"
    elif [ "${mode:-on}" = off ]; then
        echo "counterfactual: capture_latest_log=false packed no backup.log"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario L config: the empty-server policy. The two keys are upstream's
# `noPlayerBackups` / `noPlayerBackupCount`; `save_before_backup=false` keeps the
# scheduled window deterministic (no save cycle between the two status reads).
write_no_player_config() { # config-dir flag count
    local dir=$1 flag=$2 count=$3
    mkdir -p "$dir"
    cat > "$dir/simplebackups.toml" <<EOF
enabled = true
interval_minutes = 0
interval_ticks = 40
keep = 3
compression_level = 1
save_before_backup = false
retry_on_save = true
broadcast = false
backups_dir = "backups"
max_disk_size = "25 GB"
no_player_backups = $flag
no_player_backup_count = $count
EOF
}

# Scenario L: the empty-server policy. `default` is upstream's noPlayerBackups=false;
# `optin` turns it on with a count of one. A horizon-ctl launch opens a single-player
# integrated world whose owner is a placed player, so `status`'s new `players=<n>` field
# decides which assertion can honestly be made: with nobody online the empty window is
# asserted, otherwise the online branch is asserted and the harness limit is recorded.
# `selftest` inverts whichever assertion was actually available.
run_no_player_scenario() { # world sock mode
    local world=$1 sock=$2 mode=$3
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-noplayer.XXXXXX")
    local flag=false count=1
    [ "$mode" != optin ] || flag=true
    write_no_player_config "${CONFIG_DIR:-$scratch/config}" "$flag" "$count"
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local backups="$scratch/saves/$world/backups"
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 5 >/dev/null
    local status players
    status=$(newest_status_line) || fail "no status reply in the no-player scene"
    echo "no-player status (before): $status"
    players=$(sed -n 's/.*players=\([0-9][0-9]*\).*/\1/p' <<<"$status")
    [ -n "$players" ] || fail "status did not report players=<n>: $status"
    if [ "$players" -eq 0 ]; then
        echo "harness: nobody is online — the genuinely playerless window is available"
        if [ "$mode" = selftest ]; then
            if wait_for_zip "$backups" 1; then
                fail "NOPLAYER SELFTEST: an empty server made a backup ($(first_zip "$backups")) — the empty-window assertion is alive"
            fi
            fail "NOPLAYER SELFTEST inconclusive: the empty window produced no backup either way"
        fi
        if [ "$mode" = optin ]; then
            wait_for_zip "$backups" 1 || fail "noPlayerBackups=true made no empty-server backup"
            local first
            first=$(first_zip "$backups")
            echo "empty server with the opt-in: $first"
            ctl wait 60
            [ "$(count_zips "$backups")" -eq 1 ] || fail "the count=1 limit made a second empty-server backup"
            [ "$(first_zip "$backups")" = "$first" ] || fail "the first empty-server backup was replaced"
            echo "empty period: exactly one backup until a player joins"
            check_zip "$first" "$world"
        else
            ctl wait 60
            [ "$(count_zips "$backups")" -eq 0 ] || fail "an empty server made $(count_zips "$backups") backups with noPlayerBackups=false"
            echo "empty window: no new archive across 60 ticks"
            # A join is only meaningful if the harness can actually place a player.
            ctl lan open 25570 true >/dev/null 2>&1 || true
            ctl wait 10
            ctl chat "/simplebackups status" >/dev/null
            ctl wait 5 >/dev/null
            local joined
            joined=$(newest_status_line) || fail "no status reply after lan open"
            echo "no-player status (after lan open): $joined"
            if [[ "$joined" =~ players=0 ]]; then
                echo "SKIP: lan open placed no player, so the join assertion cannot be made here"
            else
                wait_for_zip "$backups" 1 || fail "no backup after a player joined"
                [ "$(count_zips "$backups")" -eq 1 ] || fail "expected exactly one archive after the join"
                echo "player joined: exactly one archive: $(first_zip "$backups")"
                check_zip "$(first_zip "$backups")" "$world"
            fi
        fi
    else
        echo "harness: the owner is online (players=$players); a genuinely playerless interval is not producible by ctl"
        if [ "$mode" = selftest ]; then
            if wait_for_zip "$backups" 1; then
                fail "NOPLAYER SELFTEST: a scheduled backup appeared with a player online — the online-window assertion is alive (no empty window was available to invert)"
            fi
            fail "NOPLAYER SELFTEST inconclusive: no backup appeared with a player online either"
        fi
        wait_for_zip "$backups" 1 || fail "no scheduled backup with a player online under noPlayerBackups=$flag"
        echo "online-window assertion: a scheduled backup appeared with players=$players"
        check_zip "$(first_zip "$backups")" "$world"
    fi
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 5 >/dev/null
    local after
    after=$(newest_status_line) || fail "no final status reply in the no-player scene"
    echo "no-player status (after): $after"
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
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

# Scenario G: retention deletes a marker-less archive only after an operator adopts it.
# The export is what a player makes by hand: the world prefix, an old name, and the world's
# own files, so the world-zip shape cannot tell it apart. It must survive both passes until
# `/simplebackups adopt <file>` — and then the next publish deletes it (the differential).
run_adoption_scenario() {
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
    local export="$backups/${world}_1999-01-01_00-00-00.zip"
    python3 - "$export" "$world" <<'PY'
import sys, zipfile
path, world = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(path, "w") as archive:
    archive.writestr(f"{world}/level.dat", "a world")
    archive.writestr(f"{world}/region/r.0.0.mca", "chunk")
    archive.writestr(f"{world}/data/raids.dat", "\x00\x00")
PY
    echo "manual export: $export"

    ctl chat "/simplebackups backup"
    wait_for_other_zip "$backups" "$(basename "$export")" || fail "the first mod backup did not appear beside the manual export"
    local first_mod
    first_mod=$(find "$backups" -maxdepth 1 -name '*.zip' ! -name "$(basename "$export")" | head -1)
    [ -n "$first_mod" ] || fail "the first mod backup was not found"
    check_zip "$first_mod" "$world"
    if [ ! -f "$export" ]; then
        if [ "${ADOPT_NEGATIVE:-0}" = 1 ]; then
            fail "ADOPT_NEGATIVE: the package deleted a manual export without an operator adoption — the scene fails on it, so it is alive"
        fi
        fail "a manual export was deleted by the first publish, before any operator adopted it"
    fi

    # keep=1 and a 1 B cap both run after this publish too; the export is the oldest file in
    # the folder, so a shape rule (or a name rule) deletes it here as well.
    ctl chat "/simplebackups backup"
    wait_for_removal "$first_mod" || fail "the second backup did not publish (the older mod archive stayed)"
    if [ ! -f "$export" ]; then
        if [ "${ADOPT_NEGATIVE:-0}" = 1 ]; then
            fail "ADOPT_NEGATIVE: the package deleted a manual export without an operator adoption — the scene fails on it, so it is alive"
        fi
        fail "a manual export was deleted without an operator adopting it"
    fi
    echo "manual export survived both passes: $export"

    local reply status
    reply=$(adopt_text "$(basename "$export")")
    echo "adopt reply: $reply"
    grep -qi 'adopted' <<<"$reply" || fail "adopt did not confirm the adoption: $reply"
    status=$(status_text)
    grep -q 'adopted=1' <<<"$status" || fail "status does not report one adopted archive: $status"

    # The same folder, the same rules: the next publish prunes the adopted archive because it
    # is now this world's.
    ctl chat "/simplebackups backup"
    if ! wait_for_removal "$export"; then
        fail "an adopted archive was not pruned by the next publish"
    fi
    echo "adopted export pruned by the next publish (the differential)"
    if [ "${ADOPT_NEGATIVE:-0}" = 1 ]; then
        fail "ADOPT_NEGATIVE: the adoption scene passed on a package that should not have it"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario H: two archives of the same second. `keep = 2` (H1) leaves both the seeded `-42`
# and the new archive; the cap answers the same rule with `keep = 3` and a `1 B` cap (H2):
# the older generations go and the archive just written stays.
#
# The seeds are copies of a real archive (they carry the marker), named for the second the
# new backup is about to land in; the base name is deliberately free, which is the state a
# pruned suffix leaves behind. The new archive must be `-43`, never the base name: as the
# oldest of its second it would be deleted by the pass that runs immediately after.
run_same_second_scenario() { # world sock keep cap
    local world=$1 sock=$2 keep=$3 cap=$4
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-home.XXXXXX")
    # No world save before the copy: the backup starts at the next tick, so seeding the
    # current second still lands inside it.
    write_config "${CONFIG_DIR:-$scratch/config}" false true "$cap" "$keep" false
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local backups="$scratch/saves/$world/backups"
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 1 || fail "the source backup did not appear"
    local source
    source=$(first_zip "$backups")
    check_zip "$source" "$world"
    # The bytes are kept outside the backup folder: retention deletes the archive itself, so a
    # retry cannot copy from it.
    cp -p "$source" "$scratch/seed-source.zip"

    local attempt=0 stem="" new_name=""
    while [ "$attempt" -lt 6 ]; do
        attempt=$((attempt + 1))
        stem=$(date -u +"${world}_%Y-%m-%d_%H-%M-%S")
        find "$backups" -maxdepth 1 -name '*.zip' -delete
        cp -p "$scratch/seed-source.zip" "$backups/${stem}-41.zip"
        cp -p "$scratch/seed-source.zip" "$backups/${stem}-42.zip"
        find "$backups" -maxdepth 1 -name '*.zip' -printf '%f\n' | sort > "$scratch/before.txt"
        ctl chat "/simplebackups backup"
        new_name=""
        local i
        for i in $(seq 1 120); do
            find "$backups" -maxdepth 1 -name '*.zip' -printf '%f\n' | sort > "$scratch/now.txt"
            new_name=$(comm -13 "$scratch/before.txt" "$scratch/now.txt" | head -1)
            [ -n "$new_name" ] && break
            sleep 0.25
        done
        case "$new_name" in
            "$stem"*) break ;;   # landed in the seeded second
            *) new_name="" ;;    # another second: seed again and retry
        esac
    done
    if [ -z "$new_name" ]; then
        # No new name survived. If the port's own `status` names an archive of the seeded second,
        # the publish wrote it and deleted it in the same pass — the suffix reuse.
        local last
        last=$(status_text | sed -n 's/.*last=\([^ ][^ ]*\).*/\1/p')
        if [ -n "$last" ] && [ "${last#"$stem"}" != "$last" ]; then
            if [ "${SAME_SECOND_NEGATIVE:-0}" = 1 ]; then
                fail "SAME_SECOND_NEGATIVE: the publish wrote $last and deleted it in the same pass (the reused base suffix) — the scene fails on it, so it is alive"
            fi
            fail "the publish wrote $last and then deleted it: the new archive reused a pruned suffix"
        fi
        fail "no backup landed in the seeded second in six attempts"
    fi
    echo "seeded second: ${stem} (attempts: $attempt)"
    echo "new archive: $new_name"

    local status
    status=$(status_text)
    echo "status: $status"
    local expected="${stem}-43.zip"
    if [ "$new_name" != "$expected" ]; then
        if [ "${SAME_SECOND_NEGATIVE:-0}" = 1 ]; then
            fail "SAME_SECOND_NEGATIVE: the package reused a pruned suffix ($new_name instead of $expected) — the scene fails on it, so it is alive"
        fi
        fail "the new archive reused a pruned suffix ($new_name), so it sorts behind an older one of the same second"
    fi
    [ -f "$backups/$expected" ] || fail "the archive just written ($expected) does not exist"
    grep -q "last=$expected" <<<"$status" || fail "status does not name the newest archive: $status"

    if [ "$cap" = "1 B" ]; then
        [ -f "$backups/${stem}-42.zip" ] && fail "the cap kept the older generation -42"
        [ -f "$backups/${stem}-41.zip" ] && fail "the cap kept the older generation -41"
        grep -q 'over=true' <<<"$status" || fail "a 1 B cap that kept one archive must report over=true: $status"
        echo "the cap removed the older generations and never the archive just written"
    else
        [ -f "$backups/${stem}-42.zip" ] || fail "keep=2 deleted the second archive of the same second"
        [ -f "$backups/${stem}-41.zip" ] && fail "keep=2 kept three archives of the same second"
        echo "both archives of the same second survive, -42 then -43"
    fi
    if [ "${SAME_SECOND_NEGATIVE:-0}" = 1 ]; then
        fail "SAME_SECOND_NEGATIVE: the same-second scene passed on a package that should not have it"
    fi
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario I: pause/unpause. The 40-tick schedule makes a zip before the pause (the differential,
# scenario A); a 150-tick paused window must not make another while status and the persisted flag
# both say paused; a manual backup still works while paused; unpause makes exactly one new zip and
# `list` names it. SELFTEST=pause inverts the paused-window assertion and exits non-zero.
run_pause_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-pause.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" true true
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"

    # Differential: the same unpaused scene makes a zip in the first window (scenario A).
    wait_for_zip "$backups" 1 || fail "no scheduled backup before pausing"
    local before
    before=$(first_zip "$backups")
    echo "before pause: $before"
    check_zip "$before" "$world"

    # The short spelling is this port's assertion; the nested one is upstream's.
    # `ctl messages` is newest-first, so `grep -m1` takes this status line, not a
    # stale one still in the 20-message buffer (the last-N-lines false-proof trap).
    ctl chat "/simplebackups pause"
    ctl wait 5
    local status status_line
    status=$(status_text)
    status_line=$(newest_status_line) || fail "no status reply after pause"
    echo "status after pause: $status"
    echo "paused line: $status_line"
    grep -q 'paused=true' <<<"$status_line" || fail "pause did not report paused=true: $status_line"
    [ -f "$world_dir/simplebackups-paused.txt" ] \
        || fail "the pause flag was not persisted beside the world"
    grep -q 'paused = true' "$world_dir/simplebackups-paused.txt" \
        || fail "the persisted pause flag has the wrong value"

    # 150 ticks at a 40-tick interval would make ~3 backups; paused, none may appear.
    ctl wait 150
    local during
    during=$(first_zip "$backups")
    if [ "$during" != "$before" ]; then
        fail "a scheduled backup appeared while paused: $during"
    fi
    if [ "${SELFTEST:-0}" = "pause" ]; then
        fail "PAUSE SELFTEST: no new zip appeared during 150 paused ticks (the 40-tick interval would have made ~3) — the assertion is alive"
    fi
    echo "paused window: no new backup across 150 ticks"

    # Upstream's manual `createBackup` ignores `paused`; so does this port's queued request.
    ctl chat "/simplebackups backup"
    wait_for_new_zip "$backups" "$before" || fail "a manual backup did not run while paused"
    local manual
    manual=$(first_zip "$backups")
    check_zip "$manual" "$world"
    echo "manual backup while paused: $manual"

    # Upstream's nested spelling unpauses; the schedule must make exactly one fresh zip.
    ctl chat "/simplebackups backup unpause"
    ctl wait 5
    status=$(status_text)
    status_line=$(newest_status_line) || fail "no status reply after unpause"
    echo "status after unpause: $status"
    echo "paused line: $status_line"
    grep -q 'paused=false' <<<"$status_line" || fail "unpause did not report paused=false: $status_line"
    grep -q 'paused = false' "$world_dir/simplebackups-paused.txt" \
        || fail "the cleared pause flag was not persisted"
    wait_for_new_zip "$backups" "$manual" || fail "no scheduled backup within 100 ticks of unpause"
    local resumed
    resumed=$(first_zip "$backups")
    [ "$(count_zips "$backups")" -eq 1 ] || fail "unpause left $(count_zips "$backups") archives, not exactly one"
    check_zip "$resumed" "$world"
    echo "resumed backup: $resumed"

    # Same trap: take only the newest actual `list` line (an archive is printed as
    # "name (size)"), never the older `last=<name>` fragments in the chat buffer.
    local listing
    listing=$(ctl chat "/simplebackups list" >/dev/null; ctl wait 5 >/dev/null; \
        ctl messages 20 | jq -r '.chat[]' | grep -m1 '\.zip (' || true)
    echo "list: $listing"
    grep -qF "$(basename "$resumed")" <<<"$listing" || fail "list does not name the resumed archive: $listing"
    if grep -qF "$(basename "$manual")" <<<"$listing"; then
        fail "list still names the pruned archive: $listing"
    fi

    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario J: the persisted schedule counter. The SAME scratch home and world are reused across
# `ctl quit` and the relaunch, so the state file beside the world is the only thing that can carry
# the counter over. Only the newest `saves=` chat line is parsed.
run_schedule_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-schedule.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" true true "25 GB" 3
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"

    # The differential: a fresh world (no state file) makes a scheduled zip at 40 ticks.
    wait_for_zip "$backups" 1 || fail "no scheduled backup before quitting"
    local before
    before=$(first_zip "$backups")
    echo "fresh-world backup: $before"
    check_zip "$before" "$world"

    # Let the counter advance past zero, then read the NEWEST status line only. On a busy box a
    # single ctl round-trip can advance the headless counter by a whole interval, so a sample can
    # land back on 0; retry until it is mid-interval instead of false-redding on the drift.
    local status_line ticks_before attempt
    for attempt in $(seq 1 20); do
        ctl wait 2 >/dev/null
        ctl chat "/simplebackups status" >/dev/null
        ctl wait 3 >/dev/null
        status_line=$(newest_status_line) || continue
        ticks_before=$(sed -n 's/.*ticks=\([0-9]*\).*/\1/p' <<<"$status_line")
        if [ -n "$ticks_before" ] && [ "$ticks_before" -gt 0 ] && [ "$ticks_before" -lt 20 ]; then
            break
        fi
    done
    echo "counter before quit: $ticks_before ($status_line)"
    [ -n "$ticks_before" ] || fail "status did not report ticks=: $status_line"
    [ "$ticks_before" -gt 0 ] && [ "$ticks_before" -lt 20 ] \
        || fail "counter before quit is $ticks_before, expected 1..19"

    # Graceful quit writes the progress beside the world with a staged rename.
    ctl quit >/dev/null 2>&1 || true
    sleep 2
    local state="$world_dir/simplebackups-schedule.txt"
    [ -f "$state" ] || fail "the schedule counter was not persisted beside the world"
    local saved
    saved=$(sed -n 's/^ticks = \([0-9]*\)$/\1/p' "$state")
    echo "state file after quit: $(cat "$state")"
    [ -n "$saved" ] || fail "the state file has no parseable ticks line: $(cat "$state")"
    [ "$saved" -ge "$ticks_before" ] \
        || fail "quit did not snapshot the advanced counter ($saved < $ticks_before)"
    [ ! -e "$world_dir/simplebackups-schedule.txt.tmp" ] \
        || fail "the staging file was left behind"

    # Seed a known persisted value, the way scenarios E/H seed archives. The quit snapshot above
    # proved the mod writes the file; seeding makes the restored value a known quantity instead of
    # one drifting with headless ticks during the ctl round-trips.
    local seeded=10
    printf 'ticks = %s\n' "$seeded" > "$state"
    echo "seeded state file: $(cat "$state")"

    # Relaunch the SAME world from the SAME scratch home. `launch --wait` already waited for the
    # world, so no extra wait-ready: every ctl round-trip lets the headless game tick on.
    launch "$sock" "$scratch" "$GAME" "$world"
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 1 >/dev/null
    local restored_line ticks_restored
    restored_line=$(newest_status_line) || fail "no status reply after relaunch"
    ticks_restored=$(sed -n 's/.*ticks=\([0-9]*\).*/\1/p' <<<"$restored_line")
    echo "counter after relaunch: $ticks_restored ($restored_line)"
    [ -n "$ticks_restored" ] || fail "status did not report ticks= after relaunch: $restored_line"
    [ "$ticks_restored" -ge "$seeded" ] \
        || fail "the counter reset on restart ($ticks_restored < $seeded) — status proves no resume"
    [ "$ticks_restored" -lt 40 ] \
        || fail "the resumed counter already passed the interval ($ticks_restored); harness drift"
    if [ "${SELFTEST:-0}" = "ticks" ]; then
        fail "TICKS SELFTEST: the counter resumed ($ticks_restored >= $seeded) — the pre-fix reset is gone, so the assertion is alive"
    fi

    # A window shorter than the ticks still left in the interval must stay empty. The headless
    # harness advances the counter during ctl round-trips, so size the window from the parsed
    # restored value rather than assuming a fixed 20 ticks (which could false-red on drift).
    local count_before no_zip
    count_before=$(count_zips "$backups")
    no_zip=$((40 - ticks_restored - 3))
    if [ "$no_zip" -ge 3 ]; then
        ctl wait "$no_zip"
        [ "$(count_zips "$backups")" -eq "$count_before" ] \
            || fail "a backup appeared within $no_zip ticks of relaunch (ticks=$ticks_restored)"
        echo "resumed window: no new zip across $no_zip ticks (ticks=$ticks_restored at relaunch)"
    else
        echo "resumed window: the restored counter ($ticks_restored) is within 3 ticks of due; the parsed status is the resume proof"
    fi

    ctl wait 40
    # Diagnostics (also the transcript for a genuine failure): the newest status line tells whether
    # the resumed counter actually reached the interval, and the folder tells what was published.
    local after_line after_names
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 3 >/dev/null
    after_line=$(newest_status_line) || fail "no status reply after the resumed window"
    after_names=$(find "$backups" -maxdepth 1 -name '*.zip' -printf '%f\n' | sort | tr '\n' ' ')
    echo "status after the resumed window: $after_line"
    echo "archives after the resumed window: $after_names"
    wait_for_other_zip "$backups" "$(basename "$before")" \
        || fail "expected a fresh zip after the resumed window, got $(count_zips "$backups")"
    local resumed
    resumed=$(find "$backups" -maxdepth 1 -name '*.zip' ! -name "$(basename "$before")" | head -1)
    check_zip "$resumed" "$world"
    echo "fresh zip after resume: $resumed"

    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario J2: a fresh world has no state file, so it starts the interval at zero and needs the
# full 40 ticks — the differential that makes scenario J's resumed counter meaningful.
run_fresh_schedule_scenario() {
    local world=$1 sock=$2
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-fresh.XXXXXX")
    write_config "${CONFIG_DIR:-$scratch/config}" true true "25 GB" 3
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 3 >/dev/null
    local status_line ticks_fresh
    status_line=$(newest_status_line) || fail "no status reply on the fresh world"
    ticks_fresh=$(sed -n 's/.*ticks=\([0-9]*\).*/\1/p' <<<"$status_line")
    echo "fresh world: $status_line"
    [ -n "$ticks_fresh" ] || fail "status did not report ticks= on a fresh world: $status_line"
    [ ! -f "$scratch/saves/$world/simplebackups-schedule.txt" ] \
        || fail "a fresh world already has a persisted schedule counter"

    # With no state file the interval starts at world open, so the first backup is a full 40 ticks
    # from launch (scenario J's seeded counter made one come 10 ticks sooner). Size the empty
    # window from the value the fresh world actually reports: launch itself advances the counter.
    local backups="$scratch/saves/$world/backups"
    local empty=$((40 - ticks_fresh - 3))
    if [ "$empty" -ge 3 ]; then
        ctl wait "$empty"
        [ "$(count_zips "$backups")" -eq 0 ] \
            || fail "a fresh world made a backup before the full interval (ticks=$ticks_fresh)"
        echo "fresh world: no zip across $empty ticks (ticks=$ticks_fresh at read)"
    fi
    wait_for_zip "$backups" 1 || fail "a fresh world did not make a backup after the interval"
    echo "fresh-world differential: no state file, ticks=$ticks_fresh at read, one zip after the full interval"
    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"
}

# Scenario K: incremental chains. The base is a full backup; the child, taken after one world
# file changed, must name the base and carry only that entry. `status` reports chain=2, `list`
# names both, and `merge` folds the chain into one standalone archive. A fresh world's first
# backup is full with chain=1 (the differential). SELFTEST=chain inverts the base assertion.
run_chain_scenario() { # world sock [selftest]
    local world=$1 sock=$2 mode=${3:-0}
    local scratch
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-chain.XXXXXX")
    write_chain_config "${CONFIG_DIR:-$scratch/config}" incremental
    launch "$sock" "$scratch" "$GAME" "$world"
    local ctl
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local world_dir="$scratch/saves/$world"
    local backups="$world_dir/backups"

    # The first backup is the full base of the chain.
    ctl chat "/simplebackups backup"
    wait_for_zip "$backups" 1 || fail "the base backup did not appear"
    local base base_name
    base=$(first_zip "$backups")
    base_name=$(basename "$base")
    echo "full base: $base"
    python3 - "$base" "$world" "$base_name" <<'PY'
import json, sys, zipfile
path, world, name = sys.argv[1:]
with zipfile.ZipFile(path) as z:
    assert z.testzip() is None
    m = json.load(z.open("simplebackups.json"))
    assert m["format"] == "simplebackups/1", m
    assert m["world"] == world, m
    assert m["kind"] == "full", m
    assert m["len"] == 1, m
    assert m["chain"] == name, m
    print(f"base manifest: kind={m['kind']} chain={m['chain']} len={m['len']}")
PY

    # Change exactly one world file, then take the second (child) backup.
    printf 'changed by scenario K\n' > "$world_dir/changed-marker.txt"
    ctl chat "/simplebackups backup"
    wait_for_other_zip "$backups" "$base_name" || fail "the child backup did not appear"
    local child child_name
    child=$(find "$backups" -maxdepth 1 -name '*.zip' ! -name "$base_name" | head -1)
    child_name=$(basename "$child")
    echo "child backup: $child"

    local manifest_base manifest_kind manifest_changed manifest_len
    manifest_base=$(python3 - "$child" <<'PY'
import json, sys, zipfile
with zipfile.ZipFile(sys.argv[1]) as z:
    m = json.load(z.open("simplebackups.json"))
    print(m.get("chain", ""), m.get("kind", ""), m.get("len", ""), ",".join(m.get("changed", [])))
PY
)
    manifest_kind=$(awk '{print $2}' <<<"$manifest_base")
    manifest_len=$(awk '{print $3}' <<<"$manifest_base")
    manifest_changed=$(awk '{print $4}' <<<"$manifest_base")
    local named_base
    named_base=$(awk '{print $1}' <<<"$manifest_base")
    echo "child manifest: chain=$named_base kind=$manifest_kind len=$manifest_len changed=$manifest_changed"

    if [ "${mode:-0}" = "chain" ]; then
        fail "CHAIN SELFTEST: the child manifest named the base ($named_base) — the assertion is alive"
    fi
    [ "$named_base" = "$base_name" ] \
        || fail "the child manifest does not name the base: $named_base (wanted $base_name)"
    [ "$manifest_kind" = "child" ] || fail "the child is not marked as a child: $manifest_kind"
    [ "$manifest_len" = "2" ] || fail "the chain length is not 2: $manifest_len"
    [ "$manifest_changed" = "$world/changed-marker.txt" ] \
        || fail "the child did not carry only the changed entry: $manifest_changed"
    # A child must not re-carry unchanged world files: its only entries are the manifest and
    # the one changed file (upstream's changed-only partial backup).
    python3 - "$child" "$world" <<'PY'
import sys, zipfile
path, world = sys.argv[1:]
with zipfile.ZipFile(path) as z:
    names = sorted(z.namelist())
    assert set(names) == {
        "simplebackups.json",
        f"{world}/changed-marker.txt",
    }, f"a child carried more than the changed entry: {names}"
PY
    echo "the child carries only the changed entry; the base and the change are in the manifest"

    # The parsed newest status line reports chain=2; list names both archives.
    local status_line listing
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 3 >/dev/null
    status_line=$(newest_status_line) || fail "no status reply after the child"
    echo "status: $status_line"
    grep -q 'chain=2' <<<"$status_line" || fail "status did not report chain=2: $status_line"
    listing=$(ctl chat "/simplebackups list" >/dev/null; ctl wait 5 >/dev/null; \
        ctl messages 20 | jq -r '.chat[]' | grep '\.zip (' || true)
    echo "list: $listing"
    grep -qF "$base_name" <<<"$listing" || fail "list does not name the base: $listing"
    grep -qF "$child_name" <<<"$listing" || fail "list does not name the child: $listing"

    # Merge folds the chain into one full, standalone archive (newest entry wins).
    ctl chat "/simplebackups merge" >/dev/null
    local merged="" i
    for i in $(seq 1 60); do
        merged=$(find "$backups" -maxdepth 1 -name 'merged_backup-*.zip' 2>/dev/null | head -1)
        [ -n "$merged" ] && break
        sleep 0.25
    done
    [ -n "$merged" ] || fail "merge did not produce a standalone archive"
    python3 - "$merged" "$world" <<'PY'
import json, sys, zipfile
path, world = sys.argv[1:]
with zipfile.ZipFile(path) as z:
    assert z.testzip() is None
    names = z.namelist()
    m = json.load(z.open("simplebackups.json"))
    assert m["kind"] == "full", m
    assert f"{world}/changed-marker.txt" in names, names
    print(f"merged archive: {len(names)} entries, kind={m['kind']}")
PY
    echo "merge folded the chain into one standalone archive"

    ctl quit >/dev/null 2>&1 || true
    sleep 1
    rm -rf "$scratch"

    # Differential: a FRESH world with no chain state makes a full first backup, chain=1.
    local fresh_world="${world}Fresh"
    scratch=$(mktemp -d "${E2E_TMP:-/tmp/opencode}/simplebackups-chain-fresh.XXXXXX")
    write_chain_config "${CONFIG_DIR:-$scratch/config}" incremental
    launch "$sock" "$scratch" "$GAME" "$fresh_world"
    ctl() { "$CTL" --socket "$sock" "$@"; }
    ctl wait-ready 120
    ctl wait 10
    local fresh_backups="$scratch/saves/$fresh_world/backups"
    ctl chat "/simplebackups backup"
    wait_for_zip "$fresh_backups" 1 || fail "the fresh world's first backup did not appear"
    local fresh fresh_name
    fresh=$(first_zip "$fresh_backups")
    fresh_name=$(basename "$fresh")
    echo "fresh-world first backup: $fresh"
    # Ask for status explicitly: the 20-message buffer still holds the *previous* world's
    # `saves=… chain=2` line, and only the newest reply may be parsed (the 0.2.5 trap).
    ctl chat "/simplebackups status" >/dev/null
    ctl wait 3 >/dev/null
    python3 - "$fresh" "$fresh_name" <<'PY'
import json, sys, zipfile
path, name = sys.argv[1:]
with zipfile.ZipFile(path) as z:
    m = json.load(z.open("simplebackups.json"))
    assert m["kind"] == "full", m
    assert m["len"] == 1, m
    assert m["chain"] == name, m
    print(f"fresh-world manifest: kind={m['kind']} chain={m['chain']} len={m['len']}")
PY
    local fresh_status
    fresh_status=$(newest_status_line) || fail "no status reply on the fresh world"
    echo "fresh-world status: $fresh_status"
    grep -q 'chain=1' <<<"$fresh_status" || fail "the fresh world did not report chain=1: $fresh_status"
    echo "fresh-world differential: the first backup is full with chain=1"
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

if [ "${SELFTEST:-0}" = "pause" ]; then
    echo "== scenario I selftest: invert the pause flag's effect =="
    run_pause_scenario SimpleBackupsPauseSelf /tmp/opencode/simplebackups-pause-self.sock
    fail "PAUSE SELFTEST inconclusive: the pause scene returned without its assertion"
fi
if [ "${SELFTEST:-0}" = "ticks" ]; then
    echo "== scenario J selftest: invert the restored tick counter =="
    run_schedule_scenario SimpleBackupsTicksSelf "${E2E_TMP:-/tmp/opencode}/simplebackups-ticks-self.sock"
    fail "TICKS SELFTEST inconclusive: the schedule scene returned without its assertion"
fi
if [ "${SELFTEST:-0}" = "chain" ]; then
    echo "== scenario K selftest: invert the chain manifest/base assertion =="
    run_chain_scenario SimpleBackupsChainSelf "${E2E_TMP:-/tmp/opencode}/simplebackups-chain-self.sock" chain
    fail "CHAIN SELFTEST inconclusive: the chain scene returned without its assertion"
fi
if [ "${SELFTEST:-0}" = "noplayer" ]; then
    echo "== scenario L selftest: invert the assertion the harness can actually make =="
    run_no_player_scenario SimpleBackupsNoPlayerSelf "${E2E_TMP:-/tmp/opencode}/simplebackups-noplayer-self.sock" selftest
    fail "NOPLAYER SELFTEST inconclusive: the no-player scene returned without its assertion"
fi
if [ "${SELFTEST:-0}" = "latestlog" ]; then
    echo "== scenario M selftest: invert the backup.log assertion =="
    run_game_log_scenario SimpleBackupsGameLogSelf "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog-self.sock" selftest
    fail "GAMELOG SELFTEST inconclusive: the log scene returned without its assertion"
fi
if [ "${ONLY:-}" = "M" ]; then
    echo "== scenario M alone (diagnostic) =="
    run_game_log_scenario SimpleBackupsGameLogDiag "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog-diag.sock" on
    echo "PASS: scenario M alone"
    exit 0
fi
if [ "${ONLY:-}" = "Mo" ]; then
    echo "== scenario M counterfactual alone (diagnostic): capture_latest_log=false =="
    run_game_log_scenario SimpleBackupsGameLogOffDiag "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog-off-diag.sock" off
    echo "PASS: scenario M counterfactual alone"
    exit 0
fi
if [ "${ONLY:-}" = "L" ]; then
    echo "== scenario L alone (diagnostic): report the observed player count =="
    run_no_player_scenario SimpleBackupsNoPlayerDiag "${E2E_TMP:-/tmp/opencode}/simplebackups-noplayer-diag.sock" default
    echo "PASS: scenario L alone"
    exit 0
fi
if [ "${ONLY:-}" = "J" ]; then
    echo "== scenario J alone (diagnostic) =="
    run_schedule_scenario SimpleBackupsScheduleDiag "${E2E_TMP:-/tmp/opencode}/simplebackups-j-diag.sock"
    echo "PASS: scenario J alone"
    exit 0
fi
if [ "${ONLY:-}" = "K" ]; then
    echo "== scenario K alone (diagnostic) =="
    run_chain_scenario SimpleBackupsChainDiag "${E2E_TMP:-/tmp/opencode}/simplebackups-k-diag.sock"
    echo "PASS: scenario K alone"
    exit 0
fi
if [ "${STALE_SELFTEST:-0}" = 1 ]; then
    echo "== scenario F selftest: assert retention did NOT skip after a stale copy =="
    run_stale_retention_scenario SimpleBackupsStaleSelf /tmp/opencode/simplebackups-stale-self.sock
    fail "STALE_SELFTEST inconclusive: the stale scene returned without its assertion"
fi
if [ "${ADOPT_NEGATIVE:-0}" = 1 ]; then
    echo "== scenario G negative: a manual export must need an operator (the previous package fails this) =="
    run_adoption_scenario SimpleBackupsAdoptNeg /tmp/opencode/simplebackups-adopt-neg.sock
    fail "ADOPT_NEGATIVE inconclusive: the adoption scene passed on a package that should fail it"
fi
if [ "${SAME_SECOND_NEGATIVE:-0}" = 1 ]; then
    echo "== scenario H negative: the same-second allocation (the previous package fails this) =="
    run_same_second_scenario SimpleBackupsSameSecondNeg /tmp/opencode/simplebackups-h-neg.sock 2 "1 TB"
    fail "SAME_SECOND_NEGATIVE inconclusive: the same-second scene passed on a package that should fail it"
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
echo "== scenario G: a manual export is deletable only after an operator adopts it =="
run_adoption_scenario SimpleBackupsAdopt /tmp/opencode/simplebackups-j.sock
echo "== scenario H: two archives of the same second, allocated above the highest suffix =="
run_same_second_scenario SimpleBackupsSameSecond /tmp/opencode/simplebackups-k.sock 2 "1 TB"
echo "== scenario H2: the cap never deletes the archive just written =="
run_same_second_scenario SimpleBackupsSameSecondCap /tmp/opencode/simplebackups-l.sock 3 "1 B"
echo "== scenario I: pause suppresses the schedule, manual still runs, unpause resumes =="
run_pause_scenario SimpleBackupsPause /tmp/opencode/simplebackups-m.sock
echo "== scenario J: the persisted schedule counter survives quit and relaunch =="
run_schedule_scenario SimpleBackupsSchedule "${E2E_TMP:-/tmp/opencode}/simplebackups-schedule.sock"
echo "== scenario J2: a fresh world waits the full interval =="
run_fresh_schedule_scenario SimpleBackupsFresh "${E2E_TMP:-/tmp/opencode}/simplebackups-fresh.sock"
echo "== scenario K: incremental chains (base + changed-only child, merge, fresh-world differential) =="
run_chain_scenario SimpleBackupsChain "${E2E_TMP:-/tmp/opencode}/simplebackups-chain.sock"
echo "== scenario L: empty-server policy (the assertion the harness can make) =="
run_no_player_scenario SimpleBackupsNoPlayer "${E2E_TMP:-/tmp/opencode}/simplebackups-noplayer.sock" default
echo "== scenario L2: the noPlayerBackups opt-in and its per-period count =="
run_no_player_scenario SimpleBackupsNoPlayerOptIn "${E2E_TMP:-/tmp/opencode}/simplebackups-noplayer-optin.sock" optin
echo "== scenario M: the scratch latest.log is captured as a flat backup.log =="
run_game_log_scenario SimpleBackupsGameLog "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog.sock" on
echo "== scenario M counterfactual: capture_latest_log=false packs no backup.log =="
run_game_log_scenario SimpleBackupsGameLogOff "${E2E_TMP:-/tmp/opencode}/simplebackups-gamelog-off.sock" off
# The control is opt-in (CONTROL=1 with CONTROL_GAME): the campaign's control leg is the same
# e2e run with `GAME=$CONTROL_GAME`, and launching a second game here raced the first one's
# socket and scratch.
if [ "${CONTROL:-0}" = 1 ]; then
    echo "== control: the unmodded game creates no backup =="
    run_control_scenario SimpleBackupsControl /tmp/opencode/simplebackups-e.sock
fi
echo "PASS: scheduled zip valid, keep=1 pruned, disabled schedule silent, manual works while disabled; a 1 B disk cap removes the oldest archive while a 1 TB cap keeps both (the counterfactual); a hand-made ZIP named like a backup survives retention while the older mod archive is removed (scenario E); a stale copy skips both retention passes and leaves the clean restore point over the cap (scenario F); a manual export with the world's own shape survives until an operator adopts it and is pruned after (scenario G); a backup allocated in a second that already has generations becomes the highest suffix and survives keep and the cap (scenario H, H2); pause suppresses only the schedule and persists beside the world, a manual backup still works while paused, and unpause resumes with exactly one new zip named by list (scenario I); the tick counter resumes from the persisted state beside the world across quit and relaunch, while a fresh world with no state file waits the full interval (scenarios J, J2); an incremental chain's child names its full base and carries only the changed file, status reports chain=2 and list names both, merge folds the chain into one standalone archive, and a fresh world's first backup is full with chain=1 (scenario K); the empty-server policy's online branch still schedules with players online while status reports the observed player count and the persisted empty-period counter (scenarios L, L2); a concrete line from the scratch latest.log is captured as a flat backup.log entry and capture_latest_log=false packs no such entry (scenario M)"
