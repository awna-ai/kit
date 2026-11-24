#!/usr/bin/env bash
set -u -o pipefail
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset KIT_STRICT
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CARGO_HOME="$tmp/cargo-home"
gate_args=()
failed=0
number=0
saved_path=$PATH
mkdir "$tmp/base-bin"
for tool in bash git env mktemp rm mkdir tar sed grep sort cat printf dirname cp uname tail; do
    ln -s "$(type -P "$tool")" "$tmp/base-bin/$tool"
done

new_case() {
    number=$((number + 1))
    repo="$tmp/case-$number"
    mkdir -p "$repo/scripts" "$repo/bin"
    cp -P "$tmp/base-bin/"* "$repo/bin/"
    git init -q "$repo"
    git -C "$repo" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit --allow-empty -qm 'Start a fixture.'
    printf '[toolchain]\nchannel = "1.99.0"\n' > "$repo/rust-toolchain.toml"
    printf '[workspace.package]\nrust-version = "1.99.0"\n' > "$repo/Cargo.toml"
    if [ -f "$root/scripts/ci-local.sh" ]; then
        cp "$root/scripts/ci-local.sh" "$repo/scripts/"
    fi
    if [ -f "$root/gate/lib.sh" ]; then
        mkdir -p "$repo/gate"
        cp "$root/gate/lib.sh" "$repo/gate/"
    fi
    cat > "$repo/scripts/term-check.sh" <<'STUB'
echo 'term-check: clean'
exit "${TERM_STATUS:-0}"
STUB
    cat > "$repo/scripts/deps-check.sh" <<'STUB'
echo 'deps-check: 4 packages clean'
exit "${DEPS_STATUS:-0}"
STUB
    cat > "$repo/scripts/secret-scan.sh" <<'STUB'
if ! command -v gitleaks > /dev/null 2>&1; then
    if [ "${KIT_STRICT:-}" = 1 ]; then
        echo 'secret-scan: gitleaks is missing on this host (KIT_STRICT=1)' >&2
        exit 75
    fi
    echo 'secret-scan: gitleaks is missing, skipped'
    exit 0
fi
echo 'secret-scan: clean'
exit "${SECRET_STATUS:-0}"
STUB
    cat > "$repo/bin/shellcheck" <<'STUB'
#!/usr/bin/env bash
exit "${SHELL_STATUS:-0}"
STUB
    cat > "$repo/bin/gitleaks" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = stdin ]; then cat > /dev/null; fi
exit 0
STUB
    cat > "$repo/bin/cargo" <<'STUB'
#!/usr/bin/env bash
printf '%s\n' "$*" >> "$CARGO_LOG"
if [ "${RUSTUP_AUTO_INSTALL:-}" != 0 ]; then exit 99; fi
if [ "$1" = "${FAIL_COMMAND:-}" ] || { [ "$1" = +1.99.0 ] && [ "${FAIL_COMMAND:-}" = msrv ]; }; then
    exit 101
fi
exit "${CARGO_STATUS:-0}"
STUB
    cat > "$repo/bin/rustup" <<'STUB'
#!/usr/bin/env bash
if [ "$1" = toolchain ]; then
    printf '%s-aarch64-apple-darwin\n' "${STUB_VERSION:-1.99.0}"
else
    printf 'rustfmt\n%s\n' "${STUB_COMPONENT:-clippy}"
fi
STUB
    chmod +x "$repo/bin/shellcheck" "$repo/bin/gitleaks" "$repo/bin/cargo" "$repo/bin/rustup"
    unset KIT_STRICT TERM_STATUS DEPS_STATUS STUB_VERSION STUB_COMPONENT CARGO_STATUS SHELL_STATUS SECRET_STATUS FAIL_COMMAND
    export CARGO_LOG="$repo/cargo-args"
}

expect() {
    local name=$1 expected=$2 wanted=$3 last=${4:-} actual output
    # Run from outside the fixture to prove location independence.
    output=$(PATH="$repo/bin" bash "$repo/scripts/ci-local.sh" "${gate_args[@]+"${gate_args[@]}"}" 2>&1)
    actual=$?
    if [ "$actual" -ne "$expected" ]; then
        printf 'ci_local_test: FAIL %s: expected exit %s, got %s\n' "$name" "$expected" "$actual"
        failed=1
    fi
    if ! printf '%s\n' "$output" | grep -F -q -- "$wanted"; then
        printf 'ci_local_test: FAIL %s: expected diagnostic absent: %s\n' "$name" "$wanted"
        failed=1
    fi
    if [ -n "$last" ] && [ "${output##*$'\n'}" != "$last" ]; then
        printf 'ci_local_test: FAIL %s: wrong last line\n' "$name"
        failed=1
    fi
}

for fixture in metacharacters prefix space; do
    new_case
    mkdir "$repo/tests"
    case "$fixture" in
        metacharacters) file='x_test.sh;true #_test.sh' ;;
        prefix)
            printf 'exit 0\n' > "$repo/tests/a_test.sh"
            file='a_test.sh b_test.sh'
            ;;
        space) file='my ok_test.sh' ;;
    esac
    if [ "$fixture" = space ]; then
        printf 'exit 0\n' > "$repo/tests/$file"
        expect test-space 0 'ci-local: all passed' 'ci-local: all passed'
    else
        printf 'printf "fixture-failed\\n"; exit 1\n' > "$repo/tests/$file"
        expect "test-$fixture" 1 '    fixture-failed'
    fi
done

new_case
# Both versions are available; the MSRV must be one literal cargo argument.
printf '[workspace.package]\nrust-version = "1.99.0;true #"\n' > "$repo/Cargo.toml"
cat > "$repo/bin/rustup" <<'STUB'
#!/bin/sh
if [ "$1" = toolchain ]; then
    printf '1.99.0-host\n1.99.0;true #-host\n'
else
    printf 'rustfmt\nclippy\n'
fi
STUB
expect msrv-quoted 0 'ci-local: all passed' 'ci-local: all passed'
if ! grep -q '^+1.99.0;true # check ' "$CARGO_LOG"; then
    printf 'ci_local_test: FAIL msrv-quoted: literal version argument absent\n'
    failed=1
fi

new_case
export KIT_STRICT=1
expect strict-clean 0 'ci-local: all passed' 'ci-local: all passed'

new_case
rm "$repo/bin/gitleaks"
export KIT_STRICT=1
expect strict-missing-scan 75 'secret-scan: gitleaks is missing on this host (KIT_STRICT=1)'
unset KIT_STRICT
expect missing-scan 0 ', skipped' 'ci-local: all passed, 1 skipped'

new_case
rm "$repo/bin/shellcheck"
export KIT_STRICT=1
expect strict-missing-shellcheck 75 'shellcheck is missing on this host'

new_case
export STUB_VERSION=1.98.0 KIT_STRICT=1
expect strict-missing-toolchain 75 '1.99.0'
unset KIT_STRICT
export DEPS_STATUS=1
expect missing-toolchain 0 'skipping the Rust checks' 'ci-local: all passed, 1 skipped'

new_case
export TERM_STATUS=75
expect term-unavailable 75 'term-check: clean'
export TERM_STATUS=1
expect term-failed 1 'term-check: clean'

new_case
export DEPS_STATUS=1
expect deps-failed 1 'deps-check: 4 packages clean'

new_case
export STUB_COMPONENT=other KIT_STRICT=1
expect strict-missing-component 75 'clippy'
unset KIT_STRICT
expect missing-component 0 'skipping the Rust checks' 'ci-local: all passed, 1 skipped'

new_case
rm "$repo/bin/rustup"
export KIT_STRICT=1
expect strict-missing-rustup 75 'rustup is missing on this host'
unset KIT_STRICT
expect missing-rustup 0 'skipping the Rust checks' 'ci-local: all passed, 1 skipped'

new_case
export CARGO_STATUS=101
expect rust-failed 1 'cargo fmt --all --check'

new_case
export SHELL_STATUS=2
expect shell-failed 1 'shellcheck scripts/*.sh tests/*.sh'

new_case
printf 'echo "term-check: no term list, skipped"\n' > "$repo/scripts/term-check.sh"
expect term-skipped 0 ', skipped' 'ci-local: all passed, 1 skipped'

for status in 1 2 75; do
    new_case
    export SECRET_STATUS=$status
    expect "secret-$status" "$status" 'bash scripts/secret-scan.sh'
done

for command in fmt clippy test msrv; do
    new_case
    export FAIL_COMMAND=$command
    expect "cargo-$command" 1 'cargo '
    if [ "$command" = msrv ] && ! grep -q '^+1.99.0 check ' "$CARGO_LOG"; then
        echo 'ci_local_test: FAIL cargo-msrv: version argument absent'
        failed=1
    fi
done

for status in 1 75; do
    new_case
    mkdir "$repo/tests"
    printf 'exit %s\n' "$status" > "$repo/tests/fail_test.sh"
    expect "test-$status" "$status" 'bash tests/fail_test.sh'
done

new_case
mkdir "$repo/tests"
printf 'echo "x, skipped"\n' > "$repo/tests/fail_test.sh"
expect test-skipped 0 'ci-local: all passed, 1 skipped' 'ci-local: all passed, 1 skipped'

for status in 2 75; do
    new_case
    export DEPS_STATUS=$status
    expect "deps-$status" "$status" 'bash scripts/deps-check.sh'
done

new_case
export TERM_STATUS=2
expect term-internal 2 'bash scripts/term-check.sh'

new_case
rm "$repo/scripts/term-check.sh"
expect term-absent 2 'bash scripts/term-check.sh'

new_case
gate_args=(extra)
expect extra-argument 2 'usage: ci-local.sh'
gate_args=()

new_case
export RUSTUP_AUTO_INSTALL=1
expect no-install 0 'ci-local: all passed' 'ci-local: all passed'

new_case
rm "$repo/bin/cargo"
export KIT_STRICT=1
expect strict-missing-cargo 75 'cargo is missing on this host'

new_case
mkdir "$repo/tests"
printf 'exit 1\n' > "$repo/tests/fail_test.sh"
rm "$repo/bin/cargo"
export KIT_STRICT=1
expect probe-before-tests 75 'cargo is missing on this host'

for step in term secret deps; do
    new_case
    case "$step" in
        term) export TERM_STATUS=127 ;;
        secret) export SECRET_STATUS=127 ;;
        deps) export DEPS_STATUS=127 ;;
    esac
    expect "$step-unexpected" 2 'bash scripts/'
done

new_case
gate_args=(--list)
expect list 0 'shellcheck scripts/*.sh tests/*.sh gate/*.sh' 'bash scripts/deps-check.sh'
if [ -s "$CARGO_LOG" ]; then
    printf 'ci_local_test: FAIL list: commands ran\n'
    failed=1
fi

for tier in full land; do
    new_case
    gate_args=(--tier "$tier")
    expect "tier-$tier" 0 'ci-local: all passed' 'ci-local: all passed'
done
for arguments in '--tier bogus' '--list --list'; do
    new_case
    read -r -a gate_args <<< "$arguments"
    expect "usage-$arguments" 2 'usage: ci-local.sh [--tier full|land|touched] [--list] | --check-map'
done
gate_args=()

new_case
export FAIL_COMMAND=fmt
expect continue-after-red 1 'cargo fmt --all --check'
for command in '^fmt ' '^clippy ' '^test ' '^+1.99.0 check '; do
    if ! grep -q "$command" "$CARGO_LOG"; then
        printf 'ci_local_test: FAIL continue-after-red: command absent: %s\n' "$command"
        failed=1
    fi
done


new_case
map_output=$(PATH="$repo/bin" bash "$root/scripts/ci-local.sh" --check-map 2>&1)
map_status=$?
if [ "$map_status" -ne 0 ] || [ "$map_output" != 'ci-local: map ok' ]; then
    printf 'ci_local_test: FAIL repository-map: exit %s: %s\n' "$map_status" "$map_output"
    failed=1
fi

for changed in gate/README.md Cargo.lock tests/a_test.sh gate/.cargo/config.toml .ai/.cargo/config.toml docs/settings.md ignored/.cargo/config.toml; do
    new_case
    mkdir "$repo/tests"
    printf 'exit 0\n' > "$repo/tests/a_test.sh"
    # Register every test label named by the map, without running those tests.
    for test_name in gate_lib gate_workflow gate_scanners gate_vendor ci_local term_check secret_scan deps_check; do
        printf 'exit 0\n' > "$repo/tests/${test_name}_test.sh"
    done
    if [ -f "$root/scripts/ci-touched.map" ]; then cp "$root/scripts/ci-touched.map" "$repo/scripts/"; fi
    git -C "$repo" add .
    git -C "$repo" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record a fixture.'
    git -C "$repo" update-ref refs/remotes/origin/main HEAD
    if [ "$changed" = ignored/.cargo/config.toml ]; then
        printf 'ignored/\n' >> "$repo/.git/info/exclude"
    fi
    mkdir -p "$(dirname "$repo/$changed")"
    printf '# changed\n' >> "$repo/$changed"
    selected=$(PATH="$repo/bin" bash "$repo/scripts/ci-local.sh" --tier touched --list 2> "$tmp/touched-notice")
    selected_status=$?
    if [ "$changed" = Cargo.lock ]; then
        wanted=$(printf '%s\n' 'bash scripts/term-check.sh' 'bash scripts/secret-scan.sh' \
            'cargo fmt --all --check' 'cargo clippy --workspace --all-targets --all-features --locked -- -D warnings' \
            'cargo test --workspace --all-features --locked' 'cargo +1.99.0 check --workspace --all-targets --all-features --locked' \
            'bash scripts/deps-check.sh')
    elif [ "$changed" = gate/README.md ]; then
        wanted=$(printf '%s\n' 'bash tests/gate_scanners_test.sh' 'bash tests/gate_vendor_test.sh' \
            'bash scripts/term-check.sh' 'bash scripts/secret-scan.sh' 'bash scripts/deps-check.sh')
    elif [ "$changed" = tests/a_test.sh ]; then
        wanted=$(printf '%s\n' 'shellcheck scripts/*.sh tests/*.sh gate/*.sh' 'bash tests/a_test.sh' \
            'bash scripts/term-check.sh' 'bash scripts/secret-scan.sh' 'bash scripts/deps-check.sh')
    fi
    if [ "$changed" = docs/settings.md ] || [ "$changed" = ignored/.cargo/config.toml ]; then
        wanted=$(printf '%s\n' 'bash scripts/term-check.sh' 'bash scripts/secret-scan.sh' 'bash scripts/deps-check.sh')
    elif [[ "$changed" == */.cargo/* ]]; then
        wanted=$(PATH="$repo/bin" bash "$repo/scripts/ci-local.sh" --list)
        if ! grep -F -q 'falls back to the full tier:' "$tmp/touched-notice"; then
            printf 'ci_local_test: FAIL touched-config-fallback: %s\n' "$changed"
            failed=1
        fi
    fi
    if [ "$selected_status" -ne 0 ] || [ "$selected" != "$wanted" ]; then
        printf 'ci_local_test: FAIL touched-%s: exit %s: %s\n' "$changed" "$selected_status" "$selected"
        failed=1
    fi
    if [ -s "$CARGO_LOG" ]; then
        printf 'ci_local_test: FAIL touched-list-ran\n'
        failed=1
    fi
done

new_case
mkdir "$repo/tests"
for test_name in gate_lib gate_workflow gate_scanners gate_vendor ci_local term_check secret_scan deps_check; do
    printf 'exit 0\n' > "$repo/tests/${test_name}_test.sh"
done
cp "$root/scripts/ci-touched.map" "$repo/scripts/"
git -C "$repo" add .
git -C "$repo" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record a fixture.'
git -C "$repo" update-ref refs/remotes/origin/main HEAD
printf '# changed\n' >> "$repo/scripts/ci-local.sh"
selected=$(cd "$repo/scripts" && PATH="$repo/bin" bash ci-local.sh --tier touched --list 2> "$tmp/touched-notice")
selected_status=$?
wanted=$(PATH="$repo/bin" bash "$repo/scripts/ci-local.sh" --list)
if [ "$selected_status" -ne 0 ] || [ "$selected" != "$wanted" ] ||
    ! grep -F -qx "ci-local: tier touched falls back to the full tier: scripts/ci-local.sh is one of the gate's own files" "$tmp/touched-notice"; then
    printf 'ci_local_test: FAIL touched-own-relative\n'
    failed=1
fi

# shellcheck disable=SC2016
if [ -x /bin/bash ] && [ "$(/bin/bash -c 'printf %s "${BASH_VERSINFO[0]}"')" != "$(bash -c 'printf %s "${BASH_VERSINFO[0]}"')" ]; then
    for fixture in metacharacters space msrv-quoted; do
        new_case
        rm "$repo/bin/bash"
        ln -s /bin/bash "$repo/bin/bash"
        case "$fixture" in
            metacharacters|space)
                mkdir "$repo/tests"
                if [ "$fixture" = space ]; then
                    printf 'exit 0\n' > "$repo/tests/my ok_test.sh"
                    expect alternate-space 0 'ci-local: all passed' 'ci-local: all passed'
                else
                    printf 'printf "fixture-failed\\n"; exit 1\n' > "$repo/tests/x_test.sh;true #_test.sh"
                    expect alternate-metacharacters 1 '    fixture-failed'
                fi
                ;;
            msrv-quoted)
                printf '[workspace.package]\nrust-version = "1.99.0;true #"\n' > "$repo/Cargo.toml"
                cat > "$repo/bin/rustup" <<'STUB'
#!/bin/sh
if [ "$1" = toolchain ]; then
    printf '1.99.0-host\n1.99.0;true #-host\n'
else
    printf 'rustfmt\nclippy\n'
fi
STUB
                expect alternate-msrv 0 'ci-local: all passed' 'ci-local: all passed'
                if ! grep -q '^+1.99.0;true # check ' "$CARGO_LOG"; then
                    printf 'ci_local_test: FAIL alternate-msrv-argument\n'
                    failed=1
                fi
                ;;
        esac
    done
else
    printf 'ci_local_test: alternate shell unavailable, skipped\n'
fi

export PATH="$saved_path"
if [ "$failed" -ne 0 ]; then
    exit 1
fi
printf 'ci_local_test: all passed\n'
