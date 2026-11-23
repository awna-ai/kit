#!/usr/bin/env bash
# Exit contract: 0 green; 1 a step failed; 2 usage or an internal error;
# 75 this host cannot run a step (strict only: missing tool, toolchain or term list).
set -euo pipefail
export LC_ALL=C
export RUSTUP_AUTO_INSTALL=0
trap 'exit 2' ERR
cd "$(dirname "$0")/.."
if [ "$#" -ne 0 ]; then
    echo 'usage: ci-local.sh' >&2
    exit 2
fi

skipped=0
missing() {
    if [ "${KIT_STRICT:-}" = 1 ]; then
        printf 'ci-local: %s is missing on this host (KIT_STRICT=1)\n' "$1" >&2
        exit 75
    fi
    printf 'ci-local: %s is missing, skipping %s\n' "$1" "$2"
    skipped=$((skipped + 1))
}

run() {
    printf '%s\n' "$*"
    if ! "$@"; then
        exit 1
    fi
}

shopt -s nullglob
if command -v shellcheck > /dev/null 2>&1; then
    echo 'shellcheck scripts/*.sh tests/*.sh'
    shell_files=(scripts/*.sh tests/*.sh)
    shellcheck "${shell_files[@]}" || exit 1
else
    missing shellcheck 'the shell checks'
fi

rust_ready=1
if ! command -v cargo > /dev/null 2>&1; then
    missing cargo 'the Rust checks'
    rust_ready=0
elif ! command -v rustup > /dev/null 2>&1; then
    missing rustup 'the Rust checks'
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
            missing "Rust toolchain $version" 'the Rust checks'
            rust_ready=0
            break
        fi
    done
    if [ "$rust_ready" -eq 1 ]; then
        components=$(rustup component list --toolchain "$channel" --installed)
        for component in rustfmt clippy; do
            if ! printf '%s\n' "$components" | grep -Eq "^$component(-|$)"; then
                missing "$component for $channel" 'the Rust checks'
                rust_ready=0
                break
            fi
        done
    fi
fi
# Shell tests preserve host-unavailable results and count their skips.
# Globs are sorted by name in the fixed C locale.
checked_script() {
    local kind=$1 script=$2 status=0 output
    printf 'bash %s\n' "$script"
    output=$(bash "$script") || status=$?
    printf '%s\n' "$output"
    if [ "$kind" = test ]; then
        case "$status" in
            0) ;;
            75) exit 75 ;;
            *) exit 1 ;;
        esac
    else
        case "$status" in
            0) ;;
            1|2|75) exit "$status" ;;
            *) exit 2 ;;
        esac
    fi
    if printf '%s\n' "$output" | grep -q ', skipped$'; then
        skipped=$((skipped + 1))
    fi
}
for test_file in tests/*_test.sh; do
    checked_script test "$test_file"
done
checked_script step scripts/term-check.sh
checked_script step scripts/secret-scan.sh

if [ "$rust_ready" -eq 1 ]; then
    run cargo fmt --all --check
    run cargo clippy --workspace --all-targets --all-features --locked -- -D warnings
    run cargo test --workspace --all-features --locked
    run cargo "+$msrv" check --workspace --all-targets --all-features --locked
fi

if [ "$rust_ready" -eq 1 ]; then
    checked_script step scripts/deps-check.sh
fi

if [ "$skipped" -eq 0 ]; then
    echo 'ci-local: all passed'
else
    printf 'ci-local: all passed, %s skipped\n' "$skipped"
fi
