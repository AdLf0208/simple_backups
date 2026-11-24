#!/usr/bin/env bash
# Run the 0.2.7 e2e legs in a few slot leases. Each batch is one `build-slot.sh run`
# invocation; the game legs stay local (the runner caller sets FORWARDED=1).
#   run-legs.sh diag   -> scenario K alone (quick validation)
#   run-legs.sh main   -> the full A-K leg -> e2e-launcher.log
#   run-legs.sh aux    -> control + the four SELFTEST legs
#   run-legs.sh neg    -> the three negative/counterfactual legs
set -uo pipefail
CLONE=/home/adlf/Desktop/all_the_mods_10_rust_rewrite
cd "$CLONE"
eval "$(ports/tools/launcher-game.sh env simplebackups)"
export GAME CTL CONTROL_GAME RES HORIZON_RES E2E_TMP
E=$CLONE/ports/_shared/evidence/simplebackups-e2e
P=ports/simplebackups/tools/e2e.sh

cleanup_games() {
    pkill -f 'horizon-client --world SimpleBackups' 2>/dev/null || true
    rm -f /tmp/opencode/simplebackups-*.sock "$E2E_TMP"/*.sock 2>/dev/null || true
    sleep 1
}

leg() { # name then VAR=val... (the script gets them via env); each leg holds its own slot lease
    local name=$1; shift
    echo "== LEG $name =="
    ( "$@" "$CLONE/ports/tools/build-slot.sh" run --job W1-simplebackups \
        --port ports/simplebackups --step e2e -- bash "$P" ) > "$E/$name.log" 2>&1
    local rc=$?
    echo "== LEG $name rc=$rc last=$(grep -m1 '^EXIT:' "$E/$name.log" | tail -1) =="
    cleanup_games
}

case "${1:-}" in
    diag)
        leg e2e-launcher-chain-diag env ONLY=K GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        ;;
    main)
        leg e2e-launcher env GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        ;;
    aux)
        leg e2e-launcher-control env GAME="$CONTROL_GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-selftest env SELFTEST=1 GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-pause-selftest env SELFTEST=pause GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-ticks-selftest env SELFTEST=ticks GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-chain-selftest env SELFTEST=chain GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-adopt-digest-selftest env SELFTEST=adopt-digest GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        ;;
    neg)
        leg e2e-launcher-stale-selftest env STALE_SELFTEST=1 GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-adopt-negative env ADOPT_NEGATIVE=1 GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        leg e2e-launcher-same-second-negative env SAME_SECOND_NEGATIVE=1 GAME="$GAME" CTL="$CTL" E2E_TMP="$E2E_TMP"
        ;;
    *) echo "usage: run-legs.sh diag|main|aux|neg" >&2; exit 64 ;;
esac
