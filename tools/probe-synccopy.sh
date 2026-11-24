#!/usr/bin/env bash
# Observability probe for the sync-copy slice (gate, not an implementation).
#
# Question: can one `ctl` round trip distinguish a copy run inline on the server
# thread at save-complete from the same copy run on the worker thread (today's
# behaviour)?  The proposed assertion is that `ctl chat "/simplebackups backup"`
# would not return until the archive exists when the copy is inline.
#
# This probe measures the CURRENT (worker-thread) package: the wall-clock time of
# the backup chat round trip, what an immediately following `list` reply names,
# and when the archive actually appears.  It writes nothing to the package; all
# scratch state lives under E2E_TMP.
set -uo pipefail
CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
cd "$CLONE"
eval "$(ports/tools/launcher-game.sh env simplebackups)"
export GAME CTL RES E2E_TMP
SOCK_DIR="$E2E_TMP"
SCRATCH=$(mktemp -d "${E2E_TMP}/simplebackups-synccopy.XXXXXX")
SOCK="$SOCK_DIR/simplebackups-synccopy.sock"
rm -f "$SOCK"
mkdir -p "$SCRATCH/config"
cat > "$SCRATCH/config/simplebackups.toml" <<'EOF'
enabled = false
interval_minutes = 0
interval_ticks = 40
keep = 3
compression_level = 1
save_before_backup = true
retry_on_save = true
broadcast = false
backups_dir = "backups"
max_disk_size = "25 GB"
EOF
export HORIZON_HOME="$SCRATCH/game"
export HORIZON_LAUNCHER_HOME="$SCRATCH/launcher"
export XDG_DATA_HOME="$SCRATCH/data"
export XDG_CONFIG_HOME="$SCRATCH/config"
export HORIZON_NO_AUDIO=1
cd "$SCRATCH"
"$CTL" --socket "$SOCK" launch --game "$GAME" --world SyncCopyProbe \
    --seed 20261124 --creative --headless --wait --resources "$RES"
ctl() { "$CTL" --socket "$SOCK" "$@"; }
ctl wait-ready 120
ctl wait 10
WORLD="$SCRATCH/saves/SyncCopyProbe"
BACKUPS="$WORLD/backups"
dd if=/dev/zero of="$WORLD/payload.bin" bs=1M count=128 status=none
sync
echo "payload-bytes: $(stat -c %s "$WORLD/payload.bin")"

sec() { python3 -c "print(f'{$2 - $1:.3f}')" "$1" "$2"; }

ctl chat "/simplebackups status" >/dev/null
ctl wait 5 >/dev/null
echo "before-status: $(ctl messages 20 | jq -r '.chat[]' | grep -m1 'saves=' || true)"

t0=$(date +%s.%N)
reply=$(ctl chat "/simplebackups backup")
t1=$(date +%s.%N)
echo "backup-chat-reply: $reply"
echo "backup-chat-seconds: $(sec "$t0" "$t1")"

t2=$(date +%s.%N)
ctl chat "/simplebackups list" >/dev/null
ctl wait 5 >/dev/null
immediate=$(ctl messages 20 | jq -r '.chat[]' | tr '\n' ' ')
t3=$(date +%s.%N)
echo "immediate-list-seconds: $(sec "$t2" "$t3")"
echo "immediate-list-replies: $immediate"
echo "archives-at-immediate-list: $(find "$BACKUPS" -maxdepth 1 -name '*.zip' 2>/dev/null | wc -l)"

t4=$(date +%s.%N)
for _ in $(seq 1 300); do
    n=$(find "$BACKUPS" -maxdepth 1 -name '*.zip' 2>/dev/null | wc -l)
    [ "$n" -ge 1 ] && break
    sleep 0.1
done
t5=$(date +%s.%N)
echo "archive-appears-after-backup-reply-seconds: $(sec "$t1" "$t5")"
echo "archive-count: $(find "$BACKUPS" -maxdepth 1 -name '*.zip' 2>/dev/null | wc -l)"
echo "archive: $(find "$BACKUPS" -maxdepth 1 -name '*.zip' 2>/dev/null | head -1)"

ctl chat "/simplebackups status" >/dev/null
ctl wait 5 >/dev/null
echo "after-status: $(ctl messages 20 | jq -r '.chat[]' | grep -m1 'saves=' || true)"
echo "PROBE-DONE"
ctl quit >/dev/null 2>&1 || true
sleep 1
rm -rf "$SCRATCH"
