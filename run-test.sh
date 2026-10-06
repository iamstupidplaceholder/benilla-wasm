#!/usr/bin/env bash
set -Eeuo pipefail

cd "$(dirname "$0")"

export RUSTUP_HOME="../../.toolchains/benilla-rustup"
export CARGO_HOME="../../.toolchains/benilla-cargo"

"$CARGO_HOME/bin/cargo" test -p benilla-lua
"$CARGO_HOME/bin/cargo" test -p benilla-ui --lib -- --skip our_globals_stay_inside_the_1_12_surface   --skip the_exception_list_has_no_dead_entries   --skip every_1_12_widget_method_is_answered   --skip our_widget_methods_stay_inside_the_1_12_surface
CARGO_TARGET_DIR="$PWD/target" "$CARGO_HOME/bin/cargo" check --target wasm32-unknown-unknown -p benilla-lua