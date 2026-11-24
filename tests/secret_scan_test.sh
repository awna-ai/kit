#!/usr/bin/env bash
set -u -o pipefail
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GITLEAKS_CONFIG GITLEAKS_CONFIG_TOML
root=$(cd "$(dirname "$0")/.." && pwd)
if ! command -v gitleaks > /dev/null 2>&1; then
    if [ "${KIT_STRICT:-}" = 1 ]; then
        echo 'secret_scan_test: gitleaks is missing on this host (KIT_STRICT=1)' >&2
        exit 75
    fi
    echo 'secret_scan_test: gitleaks is missing, skipped'
    exit 0
fi
unset KIT_STRICT
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
export CARGO_HOME="$tmp/cargo-home"
secret='gh''p_''r7K9mQ2vX8nB4sD6fG1hJ3kL5pZ0cVwY9aE2'
failed=0
number=0
saved_path=$PATH
mkdir "$tmp/plain"
printf '%s\n' "$secret" > "$tmp/plain/text"
probe_status=0
(cd "$tmp" && gitleaks dir --redact --no-banner "$tmp/plain") > "$tmp/probe" 2>&1 || probe_status=$?
if [ "$probe_status" -ne 1 ] || ! grep -q 'leaks found' "$tmp/probe"; then
    echo 'secret_scan_test: FAIL fixture: plain secret was not detected'
    exit 1
fi

new_case() {
    number=$((number + 1))
    repo="$tmp/case-$number"
    mkdir -p "$repo/scripts" "$repo/gate"
    git init -q "$repo"
    git -C "$repo" config user.name Test
    git -C "$repo" config user.email test@example.invalid
    printf 'scripts/\ngate/\n' > "$repo/.git/info/exclude"
    if [ -f "$root/scripts/secret-scan.sh" ]; then
        cp "$root/scripts/secret-scan.sh" "$repo/scripts/secret-scan.sh"
    fi
    if [ -f "$root/gate/secret-scan.sh" ]; then
        cp "$root/gate/secret-scan.sh" "$repo/gate/"
    fi
    cd "$repo" || exit 1
    printf 'hello\n' > text
    git add text
    git commit -qm initial
    unset KIT_STRICT GITLEAKS_CONFIG GITLEAKS_CONFIG_TOML
    export PATH=$saved_path
}
commit() {
    git add -A && git commit -qm "$1"
}
expect() {
    local name=$1 expected=$2 message=$3 actual=0 output
    output=$(bash scripts/secret-scan.sh 2>&1) || actual=$?
    if [ "$actual" -ne "$expected" ]; then
        printf 'secret_scan_test: FAIL %s: expected exit %s, got %s\n' "$name" "$expected" "$actual"
        failed=1
    fi
    if ! printf '%s\n' "$output" | grep -Fq -- "$message"; then
        printf 'secret_scan_test: FAIL %s: expected diagnostic absent\n' "$name"
        failed=1
    fi
}

new_case
expect clean 0 'secret-scan: clean'
new_case
printf '%s\n' "$secret" > text
commit tracked
expect tracked 1 'leaks found'
new_case
printf '%s\n' "$secret" > text
commit added
printf 'clean\n' > text
commit removed
expect removed 1 'leaks found'
new_case
printf '* -diff\n' > .gitattributes
printf '%s\n' "$secret" > text
commit added
printf 'clean\n' > text
commit removed
expect no-diff 1 'leaks found'

new_case
base=$(git branch --show-current)
git checkout -qb side
printf 'side\n' > text
commit side
git checkout -q "$base"
printf 'main\n' > text
commit changed
git merge side > "$tmp/merge" 2>&1
if [ "$?" -ne 1 ]; then
    echo 'secret_scan_test: FAIL merge-fixture: expected conflict'
    exit 1
fi
printf '%s\n' "$secret" > text
commit resolved
printf 'clean\n' > text
commit removed
expect merge-resolution 1 'leaks found'

new_case
blob=$(printf '%s\n' "$secret" | git hash-object -w --stdin)
git tag -a t "$blob" -m clean
expect tagged-blob 1 'leaks found'
new_case
git tag -a inner -m "$secret"
git -c advice.nestedTag=false tag -a outer inner -m clean
git tag -d inner > /dev/null
expect nested-tag 1 'leaks found'
new_case
git tag -a temporary -m "$secret"
git update-ref refs/custom/x "$(git rev-parse temporary)"
git tag -d temporary > /dev/null
expect custom-tag 1 'leaks found'
new_case
git commit --allow-empty -qm "$secret"
expect commit-message 1 'leaks found'

new_case
cat > .gitleaks.toml <<'CONFIG'
[extend]
useDefault = true
[allowlist]
regexes = ['.*']
CONFIG
printf '%s\n' "$secret" > text
commit configured
expect config 1 'secret-scan: .gitleaks.toml would change what gitleaks reports; remove it'
new_case
cat > .GITLEAKS.TOML <<'CONFIG'
[extend]
useDefault = true
[allowlist]
regexes = ['.*']
CONFIG
printf '%s\n' "$secret" > text
commit configured
expect uppercase-config 1 'would change what gitleaks reports; remove it'
new_case
mkdir nested
: > nested/.gitleaksignore
expect ignore 1 'secret-scan: nested/.gitleaksignore would change what gitleaks reports; remove it'
new_case
cat > "$tmp/allow.toml" <<'CONFIG'
[extend]
useDefault = true
[allowlist]
regexes = ['.*']
CONFIG
export GITLEAKS_CONFIG="$tmp/allow.toml"
printf '%s\n' "$secret" > text
commit added
expect environment-config 1 'leaks found'
new_case
GITLEAKS_CONFIG_TOML=$(cat "$tmp/allow.toml")
export GITLEAKS_CONFIG_TOML
printf '%s\n' "$secret" > text
expect environment-toml 1 'leaks found'

new_case
mkdir "$tmp/bin"
for tool in bash dirname grep git printenv; do
    ln -s "$(type -P "$tool")" "$tmp/bin/$tool"
done
export PATH="$tmp/bin" KIT_STRICT=1
expect strict-missing 75 'secret-scan: gitleaks is missing on this host (KIT_STRICT=1)'
unset KIT_STRICT
expect missing 0 'secret-scan: gitleaks is missing, skipped'
export PATH=$saved_path

if [ "$failed" -ne 0 ]; then
    exit 1
fi
echo 'secret_scan_test: all passed'
