# Validation

> **Historical — the retired `.hnmod` format (0.1.0).** Everything below was recorded against the
> pre-SDK bundle (`compatibility.json`, `tools/horizon.py`, the mixin plan), which no longer loads on
> launcher ≥ 0.2.70.

## Current: SDK package 0.2.3 (2026-11-23, pin `0a875da`)

The ladder and its logs live in [`notes.md`](notes.md) under *Slice 3 (0.2.3)* and
`../_shared/evidence/simplebackups/`:

| Check | Result | Log |
|---|---|---|
| `cargo test --offline` | 46 passed, 0 failed (28 `runtime`, 7 `support`, 11 `zip`) | `sdk-test.log` |
| red run before the fix | 36 passed, 4 failed (stale skip, unmanaged ZIP, two same-second orders) | `sdk-test-red.log` |
| `cargo clippy --offline --all-targets -- -D warnings` | exit 0, 0 warnings | `sdk-clippy.log` |
| `cargo fmt --check` | clean | `sdk-fmt.log` |
| `rm -f Cargo.lock && cargo horizon-mod build` | `Built ./target/horizon-mod/simplebackups.hmod`, exit 0 | `sdk-build.log` |
| second build + `cmp` | byte-identical, 45,307 B, sha256 `4c9061c0…592a` | `sdk-sha256-second.txt` |
| `cargo horizon-mod check` | format `native-1/0a875da…/e59da90…`, for 26.3, 45,307 bytes, exit 0 | `sdk-check.log` |
| `fits --build 0a875da…` | `0a875da  made for`, exit 0 | `sdk-fits.log` |
| `fits --build 0b14f8a…` | `0b14f8a  fits`, exit 0 (the real gate) | `sdk-fits-other.log` |
| `verify-sdk-package.py simplebackups` | 1/1, `package matches the tree` | — |
| `unzip -l` | 10 entries, no `Cargo.lock` | `sdk-entries.log` |
| in game | `PASS` + `EXIT:0`; control `EXIT:1`; `SELFTEST=1` `EXIT:1`; `STALE_SELFTEST=1` `EXIT:1` | `../simplebackups-e2e/e2e-launcher*.log` |
| e2e log gate | `logs=4 ok=4 missing-exit=0 bad-exit=0` (`--strict`) | — |

The 0.2.2 package was validated the same way at pin `0a875da`; its record is the *Slice 2* section
of `notes.md`. The 0.2.1 package was validated at pin `0b14f8a`; its e2e logs are in
`../_shared/evidence/simplebackups-e2e/retired-0.2.1/`.

## Historical — 0.1.0 `.hnmod`

Validated on Linux with Rust 1.95.0 on 2026-10-30, using the exact revisions in
`compatibility.json` and the real bundle committed here.

| Check | Result |
|---|---|
| `cargo fmt --all --check` | Pass |
| `cargo clippy --workspace --all-targets --locked --offline -- -D warnings` | Pass |
| `cargo test --workspace --locked --offline` | Runtime unit test passes |
| `python3 tools/test_template.py` | Actual bundle compiles in the tiny fixture; original method calls/result, shared runtime, AST audit, unchanged bytes/mtimes and incompatible revision/target rejection pass |
| `python3 tools/test_stage.py` | Complete pinned Horizon export stages deterministically; original checkout unchanged; exact HEAD/module audit passes |
| `python3 tools/horizon.py check` | Actual `horizon-client` binary checks successfully with `--no-default-features` |
| `python3 tools/horizon.py test` | The runtime's embedded test passes inside the actual Horizon game test binary |

The native check reported 69.55 seconds in Cargo, and the native test passed.
Both reused an existing external Cargo target directory with cached
Horizon dependencies. These are compilation/test checks, not a fresh-build
benchmark. Staging the full export took about 24 seconds per command on this
machine; a repeated application preserved the stage.

No game was launched and no Minecraft resources or player data were read or
changed. Interactive logging/gameplay remains a manual check with the compatible
prepared resources. The source audit verifies only the declared HEAD statement
and support module differ from pristine Rust ASTs; it does not claim compatibility
with other Horizon revisions or arbitrary third-party mods.

CI reproduces the formatting, clippy, standalone runtime and tiny-fixture tests.
The full native checks additionally need a Horizon Git history containing the
pin and its cached dependencies; use the commands in README.md to reproduce them.
