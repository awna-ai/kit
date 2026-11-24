#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0
strict=${KIT_STRICT:-}
term='Bramble''wick'
printf '%s\n' "$term" > "$tmp/terms"
for repo in target current; do
    git init -q "$tmp/$repo"
done
cd "$tmp/current"
expect() {
    local name=$1 wanted=$2 text=$3 actual=0 output
    shift 3
    output=$("$@" 2>&1) || actual=$?
    if [ "$actual" -ne "$wanted" ] || [[ "$output" != *"$text"* ]]; then
        printf 'gate_scanners_test: FAIL %s (expected %s, got %s): %s\n' "$name" "$wanted" "$actual" "$output"
        failed=1
    fi
}
term_scan="$root/gate/term-check.sh"
secret_scan="$root/gate/secret-scan.sh"
term_usage='usage: term-check.sh --terms <file> [--strict-var <VAR>] (--root <dir> | --message <file>)'
secret_usage='usage: secret-scan.sh [--strict-var <VAR>] --root <dir>'
expect missing-terms 2 "$term_usage" bash "$term_scan" --root "$tmp/target"
expect unknown-term 2 "$term_usage" bash "$term_scan" --unknown
expect repeated-terms 2 "$term_usage" bash "$term_scan" --terms x --terms y --root "$tmp/target"
expect both 2 "$term_usage" bash "$term_scan" --terms x --root "$tmp/target" --message x
expect neither 2 "$term_usage" bash "$term_scan" --terms x
expect invalid-term-var 2 "$term_usage" bash "$term_scan" --terms x --strict-var 1bad --root "$tmp/target"
expect invalid-term-root 2 "$term_usage" bash "$term_scan" --terms x --root "$tmp/absent"
for option in --terms --strict-var --root --message; do
    expect "missing-$option" 2 "$term_usage" bash "$term_scan" "$option"
done
for option in --root --message --strict-var; do
    expect "repeated-$option" 2 "$term_usage" bash "$term_scan" --terms x "$option" x "$option" x
done
expect strict-list 75 'term-check: no term list at' env OTHER=1 bash "$term_scan" --terms "$tmp/absent" --strict-var OTHER --root "$tmp/target"
unset OTHER
export KIT_STRICT=1
expect independent-list 0 ', skipped' bash "$term_scan" --terms "$tmp/absent" --strict-var OTHER --root "$tmp/target"
expect no-strict-list 0 ', skipped' bash "$term_scan" --terms "$tmp/absent" --root "$tmp/target"
unset KIT_STRICT
printf '%s\n' "$term" > "$tmp/target/text"
expect target-dirty 1 'text:1 names a private term' bash "$term_scan" --terms "$tmp/terms" --root "$tmp/target"
rm "$tmp/target/text"
printf '%s\n' "$term" > text
expect current-dirty 0 'term-check: clean' bash "$term_scan" --terms "$tmp/terms" --root "$tmp/target"
expect empty-message 2 "term-check: can't read the message file" bash "$term_scan" --terms "$tmp/terms" --message ''
expect empty-message-missing-list 0 ', skipped' bash "$term_scan" --terms "$tmp/absent" --message ''
expect dirty-message 1 'the message names a private term' bash "$term_scan" --terms "$tmp/terms" --message text
printf 'clean\n' > "$tmp/message"
expect clean-message 0 'the message is clean' bash "$term_scan" --terms "$tmp/terms" --message "$tmp/message"
expect missing-root 2 "$secret_usage" bash "$secret_scan"
expect unknown-secret 2 "$secret_usage" bash "$secret_scan" --unknown
expect invalid-secret-var 2 "$secret_usage" bash "$secret_scan" --root "$tmp/target" --strict-var 1bad
expect repeated-secret-root 2 "$secret_usage" bash "$secret_scan" --root x --root y
expect repeated-secret-var 2 "$secret_usage" bash "$secret_scan" --strict-var OTHER --strict-var OTHER --root "$tmp/target"
expect invalid-secret-root 2 "$secret_usage" bash "$secret_scan" --root "$tmp/absent"
for option in --root --strict-var; do
    expect "missing-secret-$option" 2 "$secret_usage" bash "$secret_scan" "$option"
done
mkdir "$tmp/bin"
expect named-strict 75 'secret-scan: gitleaks is missing on this host (OTHER=1)' env PATH="$tmp/bin" OTHER=1 "$BASH" "$secret_scan" --strict-var OTHER --root "$tmp/target"
expect independent-secret 0 ', skipped' env PATH="$tmp/bin" KIT_STRICT=1 "$BASH" "$secret_scan" --strict-var OTHER --root "$tmp/target"
if command -v gitleaks > /dev/null 2>&1; then
    secret='gh''p_''r7K9mQ2vX8nB4sD6fG1hJ3kL5pZ0cVwY9aE2'
    printf '%s\n' "$secret" > "$tmp/target/text"
    expect target-secret 1 'leaks found' bash "$secret_scan" --root "$tmp/target"
    rm "$tmp/target/text"
    printf '%s\n' "$secret" > text
    expect current-secret 0 'secret-scan: clean' bash "$secret_scan" --root "$tmp/target"
elif [ "$strict" = 1 ]; then
    printf 'gate_scanners_test: gitleaks is missing on this host (KIT_STRICT=1)\n' >&2
    exit 75
else
    printf 'gate_scanners_test: gitleaks is missing, skipped\n'
fi
if [ "$failed" -ne 0 ]; then exit 1; fi
printf 'gate_scanners_test: all passed\n'
