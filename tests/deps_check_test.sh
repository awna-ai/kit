#!/usr/bin/env bash
set -u -o pipefail
export LC_ALL=C
export RUSTUP_AUTO_INSTALL=0
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
root=$(cd "$(dirname "$0")/.." && pwd -P)
channel=$(sed -n 's/^channel *= *"\([^"]*\)".*/\1/p' "$root/rust-toolchain.toml")
missing() {
    if [ "${KIT_STRICT:-}" = 1 ]; then
        printf 'deps_check_test: %s is missing on this host (KIT_STRICT=1)\n' "$1" >&2
        exit 75
    fi
    printf 'deps_check_test: %s is missing, skipped\n' "$1"
    exit 0
}
for tool in cargo rustup; do
    command -v "$tool" > /dev/null 2>&1 || missing "$tool"
done
rustup toolchain list | grep -q "^$channel-" || missing "toolchain $channel"
export RUSTUP_TOOLCHAIN="$channel"
unset KIT_STRICT
tmp=$(cd "$(mktemp -d)" && pwd -P)
trap 'rm -rf "$tmp"' EXIT
export CARGO_HOME="$tmp/cargo-home"
failed=0
number=0

crate() {
    local directory=$1 name=$2
    mkdir -p "$directory/src"
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2024"\n' "$name" > "$directory/Cargo.toml"
    : > "$directory/src/lib.rs"
}

new_case() {
    number=$((number + 1))
    repo="$tmp/case-$number"
    mkdir -p "$repo/scripts"
    git init -q "$repo"
    printf '[workspace]\nmembers = ["crates/*"]\nresolver = "3"\n' > "$repo/Cargo.toml"
    local package
    for package in awna-run awna-agents awna-keystore awna-launch awna-intake; do
        crate "$repo/crates/$package" "$package"
    done
    if [ -f "$root/scripts/deps-check.sh" ]; then
        cp "$root/scripts/deps-check.sh" "$repo/scripts/"
    fi
    cd "$repo" || exit 1
    unset KIT_STRICT
}

lock() {
    if ! cargo generate-lockfile "$@" > "$tmp/lock.log" 2>&1; then
        cat "$tmp/lock.log"
        exit 1
    fi
}

expect() {
    local name=$1 expected=$2 wanted=$3 output actual
    shift 3
    output=$(bash scripts/deps-check.sh "$@" 2>&1)
    actual=$?
    if [ "$actual" -ne "$expected" ]; then
        printf 'deps_check_test: FAIL %s: expected exit %s, got %s\n' "$name" "$expected" "$actual"
        failed=1
    fi
    if ! printf '%s\n' "$output" | grep -F -q -- "$wanted"; then
        printf 'deps_check_test: FAIL %s: expected diagnostic absent: %s\n' "$name" "$wanted"
        failed=1
    fi
}

new_case
crate "$repo/crates/macro-helper" macro-helper
printf '\n[lib]\nproc-macro = true\n' >> crates/macro-helper/Cargo.toml
printf '\n[dependencies]\nmacro-helper = { path = "../macro-helper" }\n' >> crates/awna-run/Cargo.toml
lock --offline
expect clean 0 'deps-check: 5 packages clean'

for kind in normal optional dev build target renamed; do
    new_case
    crate "$repo/crates/ureq" ureq
    case "$kind" in
        normal) printf '\n[dependencies]\nureq = { path = "../ureq" }\n' ;;
        optional) printf '\n[dependencies]\nureq = { path = "../ureq", optional = true }\n' ;;
        dev) printf '\n[dev-dependencies]\nureq = { path = "../ureq" }\n' ;;
        build) printf '\n[build-dependencies]\nureq = { path = "../ureq" }\n' ;;
        target) printf '\n[target.\047cfg(windows)\047.dependencies]\nureq = { path = "../ureq" }\n' ;;
        renamed) printf '\n[dependencies]\nnet = { package = "ureq", path = "../ureq" }\n' ;;
    esac >> "$repo/crates/awna-run/Cargo.toml"
    lock --offline
    expect "$kind" 1 'deps-check: awna-run resolves ureq, a networking or plugin crate these packages must not use'
done

new_case
crate "$repo/crates/helper" helper
crate "$repo/crates/hyper" hyper
printf '\n[dependencies]\nhelper = { path = "../helper" }\n' >> crates/awna-agents/Cargo.toml
printf '\n[dependencies]\nhyper = { path = "../hyper" }\n' >> crates/helper/Cargo.toml
lock --offline
expect transitive 1 'deps-check: awna-agents resolves hyper,'

for name in reqwest hyper ureq isahc surf attohttpc iroh tauri-plugin-shell iroh-relay; do
    new_case
    crate "$repo/crates/$name" "$name"
    printf '\n[dependencies]\n%s = { path = "../%s" }\n' "$name" "$name" >> crates/awna-run/Cargo.toml
    lock --offline
    expect prefix 1 "deps-check: awna-run resolves $name,"
done

for package in awna-run awna-agents awna-keystore awna-launch awna-intake; do
    new_case
    crate "$repo/crates/ureq" ureq
    printf '\n[dependencies]\nureq = { path = "../ureq" }\n' >> "crates/$package/Cargo.toml"
    lock --offline
    expect "selected-$package" 1 "deps-check: $package resolves ureq,"
done

new_case
crate "$repo/crates/helper" helper
crate "$repo/crates/ureq" ureq
printf '\n[dependencies]\nhelper = { path = "../helper" }\n' >> crates/awna-run/Cargo.toml
printf '\n[dependencies]\nureq = { path = "../ureq", optional = true }\n' >> crates/helper/Cargo.toml
lock --offline
expect transitive-optional 1 'deps-check: awna-run resolves ureq,'

for format in compact-name indented-header crlf; do
    new_case
    crate "$repo/crates/helper" helper
    crate "$repo/crates/ureq" ureq
    printf '\n[dependencies]\nhelper = { path = "../helper" }\n' >> crates/awna-run/Cargo.toml
    printf '\n[dependencies]\nureq = { path = "../ureq", optional = true }\n' >> crates/helper/Cargo.toml
    lock --offline
    wanted="deps-check: Cargo.lock is not in cargo's own format; run cargo update --workspace"
    case "$format" in
        compact-name) sed 's/^name = "/name="/' Cargo.lock ;;
        indented-header) sed 's/^\[\[package\]\]/ [[package]]/' Cargo.lock ;;
        crlf)
            wanted='deps-check: awna-run resolves ureq, a networking or plugin crate these packages must not use'
            sed 's/$/\r/' Cargo.lock
            ;;
    esac > "$tmp/rewritten.lock"
    cp "$tmp/rewritten.lock" Cargo.lock
    expect "$format" 1 "$wanted"
done

new_case
crate "$repo/crates/helper" helper
crate "$repo/crates/ureq" ureq
crate "$repo/crates/unused-helper" unused-helper
printf '\n[dependencies]\nhelper = { path = "../helper" }\n' >> crates/awna-run/Cargo.toml
printf '\n[dependencies]\nureq = { path = "../ureq", optional = true }\n' >> crates/helper/Cargo.toml
printf '\n[workspace]\n' >> crates/unused-helper/Cargo.toml
printf '[workspace]\nmembers = ["crates/awna-*", "crates/helper", "crates/ureq"]\nexclude = ["crates/unused-helper"]\nresolver = "3"\n\n[patch.crates-io]\nunused-helper = { path = "crates/unused-helper" }\n' > Cargo.toml
lock --offline
if ! grep -q '^\[\[patch.unused\]\]$' Cargo.lock; then
    echo 'deps_check_test: FAIL unused-patch-fixture: cargo did not write the unused patch table'
    exit 1
fi
expect transitive-optional-unused-patch 1 'deps-check: awna-run resolves ureq,'

new_case
crate "$tmp/symlink-helper" symlink-helper
ln -s "$tmp/symlink-helper" "$repo/crates/symlink-helper"
printf '\n[dependencies]\nsymlink-helper = { path = "../symlink-helper" }\n' >> crates/awna-run/Cargo.toml
lock --offline
expect symlink-path 1 'outside crates.io and this workspace'

new_case
odd_path="$repo-other)"$'\n''[x'
crate "$odd_path" odd-helper
# TOML accepts the escaped newline in a quoted path.
escaped_path=${odd_path//$'\n'/\\n}
printf '\n[dependencies]\nodd-helper = { path = "%s" }\n' "$escaped_path" >> crates/awna-run/Cargo.toml
lock --offline
output=$(bash scripts/deps-check.sh 2>&1); actual=$?
if [ "$actual" -eq 0 ]; then
    printf 'deps_check_test: FAIL newline-path: expected a nonzero exit\n'
    failed=1
fi

for config in config config.toml; do
    new_case
    lock --offline
    mkdir -p .cargo
    printf '# A configuration file.\n' > ".cargo/$config"
    git add ".cargo/$config"
    git -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Add configuration.'
    expect "source-replacement-$config" 1 'can redirect where cargo fetches crates; remove it'
done

new_case
lock --offline
mkdir cfg
printf '# A configuration file.\n' > cfg/config.toml
ln -s cfg .cargo
git add .cargo cfg/config.toml
expect symlink-config 1 'can redirect where cargo fetches crates; remove it'

new_case
lock --offline
mkdir .CARGO
printf '# A configuration file.\n' > .CARGO/CONFIG.TOML
git add .CARGO/CONFIG.TOML
expect uppercase-config 1 'can redirect where cargo fetches crates; remove it'

# Exercise invalid output independently of cargo formatting choices.
for invalid in '[x' 'helper nonsense' 'helper v0.1.0 trailing'; do
    new_case
    lock --offline
    mkdir -p "$repo/bin"
    printf '#!/usr/bin/env bash\nprintf "%%s\\n" %q\n' "$invalid" > "$repo/bin/cargo"
    chmod +x "$repo/bin/cargo"
    saved_path=$PATH
    export PATH="$repo/bin:$PATH"
    expect "invalid-output-$invalid" 2 'unrecognised cargo tree output'
    export PATH="$saved_path"
done

new_case
lock --offline
expect extra-argument 2 'usage: deps-check.sh' extra

new_case
crate "$tmp/git-helper" git-helper
git init -q "$tmp/git-helper"
git -C "$tmp/git-helper" add .
git -C "$tmp/git-helper" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Add a helper.'
printf '\n[dependencies]\ngit-helper = { git = "file://%s/git-helper" }\n' "$tmp" >> crates/awna-run/Cargo.toml
lock
expect git-source 1 'outside crates.io and this workspace'

new_case
crate "$repo-other" external-helper
printf '\n[dependencies]\nexternal-helper = { path = "%s-other" }\n' "$repo" >> crates/awna-run/Cargo.toml
lock --offline
expect external-path 1 'outside crates.io and this workspace'

new_case
crate "$repo/crates/awna-other" awna-other
crate "$repo/crates/ureq" ureq
printf '\n[dependencies]\nureq = { path = "../ureq" }\n' >> crates/awna-other/Cargo.toml
lock --offline
expect unselected-member 0 '5 packages clean'

new_case
rm -rf crates/awna-launch
lock --offline
expect missing-package 1 'cargo tree failed for awna-launch'

new_case
lock --offline
mkdir "$tmp/bin"
for tool in bash dirname; do
    ln -s "$(command -v "$tool")" "$tmp/bin/$tool"
done
saved_path=$PATH
export PATH="$tmp/bin" KIT_STRICT=1
# Keep assertions outside the restricted search path.
output=$(bash scripts/deps-check.sh 2>&1); actual=$?
export PATH="$saved_path"
if [ "$actual" -ne 75 ]; then
    printf 'deps_check_test: FAIL strict-missing-cargo: expected exit 75, got %s\n' "$actual"
    failed=1
fi
unset KIT_STRICT
export PATH="$tmp/bin"
output=$(bash scripts/deps-check.sh 2>&1); actual=$?
export PATH="$saved_path"
if [ "$actual" -ne 0 ] || [[ "$output" != *'cargo is missing, skipped'* ]]; then
    printf 'deps_check_test: FAIL missing-cargo: expected exit 0 and skipped\n'
    failed=1
fi

if [ "$failed" -ne 0 ]; then
    exit 1
fi
printf 'deps_check_test: another-registry fixture unavailable offline, case skipped\n'
printf 'deps_check_test: all passed\n'
