#!/usr/bin/env bash
# Exit contract: 0 green; 1 a step failed; 2 usage or an internal error;
# 75 this host cannot run a step (strict only: missing tool, toolchain or term list).
set -euo pipefail
export LC_ALL=C
export RUSTUP_AUTO_INSTALL=0
trap 'exit 2' ERR
cd "$(dirname "$0")/.."
# shellcheck source=gate/lib.sh
. gate/lib.sh
gate_init --name ci-local --strict-var KIT_STRICT -- "$@"

shopt -s nullglob
if gate_require shellcheck 'the shell checks'; then
    gate_step 'shellcheck scripts/*.sh tests/*.sh gate/*.sh'
fi

rust_ready=1
if ! command -v cargo > /dev/null 2>&1; then
    gate_missing cargo 'the Rust checks'
    rust_ready=0
elif ! command -v rustup > /dev/null 2>&1; then
    gate_missing rustup 'the Rust checks'
    rust_ready=0
else
    channel=$(sed -nE 's/^[[:space:]]*channel[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' rust-toolchain.toml)
    msrv=$(sed -nE 's/^[[:space:]]*rust-version[[:space:]]*=[[:space:]]*"([^"]+)".*/\1/p' Cargo.toml)
    if [ -z "$channel" ] || [ -z "$msrv" ] || [[ "$channel$msrv" == *$'\n'* ]]; then
        echo 'ci-local: cannot read the Rust versions' >&2
        exit 2
    fi
    installed=$(rustup toolchain list)
    for version in "$channel" "$msrv"; do
        found=0
        while IFS= read -r toolchain; do
            case "$toolchain" in
                "$version"-*) found=1 ;;
            esac
        done <<< "$installed"
        if [ "$found" -eq 0 ]; then
            gate_missing "Rust toolchain $version" 'the Rust checks'
            rust_ready=0
            break
        fi
    done
    if [ "$rust_ready" -eq 1 ]; then
        components=$(rustup component list --toolchain "$channel" --installed)
        for component in rustfmt clippy; do
            if ! printf '%s\n' "$components" | grep -Eq "^$component(-|$)"; then
                gate_missing "$component for $channel" 'the Rust checks'
                rust_ready=0
                break
            fi
        done
    fi
fi
# Globs are sorted by name in the fixed C locale.
for test_file in tests/*_test.sh; do
    printf -v quoted '%q' "$test_file"
    gate_step --test "bash $test_file" "bash $quoted"
done
gate_step --contract 'bash scripts/term-check.sh'
gate_step --contract 'bash scripts/secret-scan.sh'

if [ "$rust_ready" -eq 1 ]; then
    gate_step 'cargo fmt --all --check'
    gate_step 'cargo clippy --workspace --all-targets --all-features --locked -- -D warnings'
    gate_step 'cargo test --workspace --all-features --locked'
    printf -v quoted '%q' "+$msrv"
    gate_step "cargo +$msrv check --workspace --all-targets --all-features --locked" "cargo $quoted check --workspace --all-targets --all-features --locked"
    gate_step --contract 'bash scripts/deps-check.sh'
fi

gate_run
