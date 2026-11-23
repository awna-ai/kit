#!/usr/bin/env bash
set -u -o pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0
number=0
runner=$(type -P bash)
mode=loose
mkdir "$tmp/bin"
for tool in mktemp rm tail cat sleep; do
    ln -s "$(type -P "$tool")" "$tmp/bin/$tool"
done
cat > "$tmp/bin/uname" <<'STUB'
#!/bin/sh
printf '%s\n' "${TEST_OS:-Darwin}"
STUB
chmod +x "$tmp/bin/uname"

new_case() {
    number=$((number + 1))
    case_dir="$tmp/case-$number"
    mkdir -p "$case_dir/logs" "$case_dir/elsewhere"
    consumer="$case_dir/consumer.sh"
    ln -sf "$runner" "$tmp/bin/bash"
    if [ "$mode" = strict ]; then
        printf 'set -euo pipefail\ntrap '\''exit 2'\'' ERR\n' > "$consumer"
    else
        printf ':\n' > "$consumer"
    fi
    printf '. "%s/gate/lib.sh"\n' "$root" >> "$consumer"
    unset TEST_STRICT TEST_OS
}

expect() {
    local name=$1 expected=$2 wanted=$3 last=${4:-} actual
    shift 4
    output=$(cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer" "$@" 2>&1 <<< 'unused input')
    actual=$?
    if [ "$actual" -ne "$expected" ] || [[ "$output" != *"$wanted"* ]] || { [ -n "$last" ] && [[ "${output##*$'\n'}" != "$last" ]]; }; then
        printf 'gate_lib_test: FAIL %s: exit %s (wanted %s), expected %s; last %s\n' "$name" "$actual" "$expected" "$wanted" "$last"
        failed=1
    fi
}

check() {
    local name=$1
    shift
    if ! "$@"; then
        printf 'gate_lib_test: FAIL %s\n' "$name"
        failed=1
    fi
}

core_cases() {
    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t -- "$@"
gate_step step-one 'printf "one\n"'
gate_step step-two ':'
gate_run
CASE
    expect passes 0 'step 1 ' 't: all passed'
    check timing-lines test "$(printf '%s\n' "$output" | grep -Ec '^step [12] [0-9]+s step-(one|two)$')" -eq 2
    check green-cleanup test -z "$(ls -A "$case_dir/logs")"

    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t
gate_step step-one 'printf "one\n" >> marker'
gate_step step-two 'printf "two\n" >> marker; printf "detail\n"; exit 1'
gate_step step-three 'printf "three\n" >> marker'
gate_run
CASE
    expect red 1 'FAIL: step-two (whole log: ' ''
    check run-all test "$(cat "$case_dir/marker" 2>/dev/null)" = "$(printf 'one\ntwo\nthree')"
    check log-retained test -f "$case_dir"/logs/t.*/step-2.log
    check red-last test "${output##*$'\n'}" = "t: 1 failed (logs: $(printf '%s' "$case_dir"/logs/t.*))"
    check log-tail test "${output#*    detail}" != "$output"

    local kind code result wanted option
    for kind in plain test contract; do
        for code in 0 1 2 75 3; do
            new_case
            option=''
            if [ "$kind" != plain ]; then option="--$kind"; fi
            printf 'gate_init --name t\ngate_step %s step-one "exit %s"\ngate_run\n' "$option" "$code" >> "$consumer"
            result=1 wanted='FAIL: step-one'
            if [ "$code" -eq 0 ]; then result=0 wanted='t: all passed'
            elif [ "$kind" != plain ] && [ "$code" -eq 75 ]; then result=75 wanted='UNAVAILABLE: step-one'
            elif [ "$kind" = contract ] && [ "$code" -ne 1 ]; then result=2 wanted="ERROR: step-one (exit $code;"
            fi
            expect "table-$kind-$code" "$result" "$wanted" ''
            case "$result" in
                0) wanted='t: all passed' ;;
                1) wanted="t: 1 failed (logs: $(printf '%s' "$case_dir"/logs/t.*))" ;;
                2) wanted="t: internal error (logs: $(printf '%s' "$case_dir"/logs/t.*))" ;;
                75) wanted="t: this host cannot run the gate (logs: $(printf '%s' "$case_dir"/logs/t.*))" ;;
            esac
            check "table-last-$kind-$code" test "${output##*$'\n'}" = "$wanted"
        done
    done

    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t
gate_missing invented-check 'a check'
gate_step --contract step-one 'printf "a, skipped\nb, skipped\nignored\n"'
gate_step plain 'printf "c, skipped\n"'
gate_run
CASE
    expect skips 0 '    a, skipped' 't: all passed, 3 skipped'
    check second-skip test "${output#*    b, skipped}" != "$output"
    check plain-hidden test "${output#*c, skipped}" = "$output"

    local args
    for args in unknown --tier '--tier other' '--list --list' '--tier full --tier land'; do
        new_case
        printf 'gate_init --name t -- "$@"\ngate_step step-one :\ngate_run\n' >> "$consumer"
        # Deliberate splitting supplies the argument combinations above.
        read -r -a arguments <<< "$args"
        expect "usage-$args" 2 'usage: consumer.sh [--tier full|land] [--list]' '' "${arguments[@]+"${arguments[@]}"}"
    done
    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t -- "$@"
printf 'tier=%s\n' "$GATE_TIER"
gate_step step-one :
gate_run
CASE
    expect tier-land 0 'tier=land' 't: all passed' --tier land
    expect tier-full 0 'tier=full' 't: all passed' --tier full
}

missing_cases() {
    new_case
    printf 'gate_init --name t --strict-var TEST_STRICT -- "$@"\ngate_missing invented-check "a check"\nprintf "returned\\n"\n' >> "$consumer"
    export TEST_STRICT=1
    expect strict-missing 75 't: invented-check is missing on this host (TEST_STRICT=1)' 't: invented-check is missing on this host (TEST_STRICT=1)'
    expect list-missing 0 't: invented-check is missing, skipping a check' returned --list
    unset TEST_STRICT
    expect unset-missing 0 't: invented-check is missing, skipping a check' returned
    export TEST_STRICT=yes
    expect non-strict-missing 0 't: invented-check is missing, skipping a check' returned
}

core_cases
missing_cases
mode=strict
core_cases
missing_cases
mode=loose

for before in 0 1; do
    for stop in 75 2; do
        new_case
        printf 'gate_init --name t\ngate_step first "exit %s"\ngate_step --contract second "exit %s"\ngate_step last "printf ran > later"\ngate_run\n' "$before" "$stop" >> "$consumer"
        result=$stop
        wanted='this host cannot run the gate'
        if [ "$stop" -eq 2 ]; then wanted='internal error'
        elif [ "$before" -eq 1 ]; then result=1 wanted='1 failed'
        fi
        expect "stop-$before-$stop" "$result" "$wanted" ''
        check stop-count test "${output#*stopped after step 2; 1 steps not run}" != "$output"
        check stopped test ! -e "$case_dir/later"
    done
done

new_case
printf 'gate_init --name t\ngate_step first "exit 1"\ngate_step second "exit 1"\ngate_run\n' >> "$consumer"
expect two-reds 1 't: 2 failed' ''

new_case
cat >> "$consumer" <<'CASE'
gate_init --name t
if gate_require cat 'a check'; then printf 'present\n'; else exit 9; fi
if gate_require invented-absent 'a check'; then exit 9; else printf 'absent=%s\n' "$?"; fi
CASE
expect require 0 't: invented-absent is missing, skipping a check' absent=1
check require-present test "${output#*present}" != "$output"

new_case
cat >> "$consumer" <<'CASE'
gate_init --name t -- "$@"
gate_step first 'printf "ran\n" > marker'
gate_step --mac-only second 'printf "ran\n" > marker'
gate_run
CASE
export TEST_OS=Linux
expect list 0 "$(printf 'first\nsecond')" second --list
check list-no-run test ! -e "$case_dir/marker"
check list-no-logs test -z "$(ls -A "$case_dir/logs")"
expect wrong-os 75 't: second runs only on macOS and this host is Linux; exit 75 so another host can run the gate' ''
check preflight test ! -e "$case_dir/marker"
export TEST_OS=Darwin
expect right-os 0 'step 2 ' 't: all passed'
check mac-ran test -f "$case_dir/marker"

for init in '' '--name' '--name t --strict-var' '--name "Bad Name"' '--name t --strict-var 1X' '--name t --unknown'; do
    new_case
    printf 'gate_init %s\n' "$init" >> "$consumer"
    expect "init-$init" 2 'gate_init:' ''
done
for step in '' '""' 'label ""' '$'"'bad\nlabel'" 'label $'"'bad\ncommand'" '--test --contract label' '--unknown label'; do
    new_case
    printf 'gate_init --name t\ngate_step %s\n' "$step" >> "$consumer"
    expect "step-$step" 2 't: gate_step:' ''
done
new_case
printf 'gate_init --name t\ngate_run\n' >> "$consumer"
expect no-steps 2 't: no steps registered' ''

new_case
cat >> "$consumer" <<'CASE'
gate_init --name t
gate_step first :
gate_step second 'sleep 1'
gate_step third :
gate_step fourth 'sleep 2'
gate_step fifth :
gate_step sixth :
gate_run
CASE
expect slowest 0 't: slowest steps' 't: all passed'
ranking=${output#*t: slowest steps$'\n'}
check slow-count test "$(printf '%s\n' "$ranking" | grep -c '^  ')" -eq 5
check slow-order test "$(printf '%s\n' "$ranking" | sed -n '1s/.*s //p')" = fourth
check next-slow test "$(printf '%s\n' "$ranking" | sed -n '2s/.*s //p')" = second
check tie-order test "$(printf '%s\n' "$ranking" | sed -n '3,5s/.*s //p')" = "$(printf 'first\nthird\nfifth')"

new_case
printf 'gate_init --name t\ngate_step input "cat > input"\ncd elsewhere\ngate_step root "pwd > location"\ngate_run\n' >> "$consumer"
expect root-and-stdin 0 't: all passed' 't: all passed'
check empty-input test -f "$case_dir/input"
check no-input test ! -s "$case_dir/input"
check root-location test "$(cat "$case_dir/location" 2>/dev/null)" = "$(cd "$case_dir" && pwd -P)"

new_case
expect source-alone 0 '' ''
check silent-first-source test -z "$output"
printf '. "%s/gate/lib.sh"\n' "$root" >> "$consumer"
expect source-twice 0 '' ''
check silent-source test -z "$output"

new_case
cat >> "$consumer" <<'CASE'
gate_init --name t
gate_step --test step-one 'printf "a, skipped\nb, skipped"'
gate_run
CASE
expect test-skips 0 '    b, skipped' 't: all passed, 2 skipped'

new_case
cat >> "$consumer" <<'CASE'
gate_init --name t
gate_step step-one 'for ((i=1; i<=17; i++)); do printf "line-%s\n" "$i"; done; printf "error-detail\n" >&2; exit 1'
gate_run
CASE
expect tail-limit 1 '    error-detail' ''
check tail-count test "$(printf '%s\n' "$output" | grep -c '^    ')" -eq 15
check tail-start test "${output#*    line-4}" != "$output"
check tail-excludes test "${output#*    line-3}" = "$output"
# Inspect streams independently of the merged diagnostic assertions.
(cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer") > "$case_dir/out" 2> "$case_dir/err"
check red-status test "$?" -eq 1
check red-stderr grep -q '^FAIL: step-one ' "$case_dir/err"
check timing-stdout grep -Eq '^step 1 [0-9]+s step-one$' "$case_dir/out"
check summary-stderr grep -q '^t: 1 failed ' "$case_dir/err"

new_case
cat > "$consumer" <<'CASE'
set -euo pipefail
trap 'exit 2' ERR
before_options=$(set +o)
before_traps=$(trap -p)
before_shopt=$(shopt -p)
CASE
printf '. "%s/gate/lib.sh"\n' "$root" >> "$consumer"
cat >> "$consumer" <<'CASE'
if [ "$before_options" != "$(set +o)" ] || [ "$before_traps" != "$(trap -p)" ] || [ "$before_shopt" != "$(shopt -p)" ]; then
    printf 'changed shell state\n'
    exit 9
fi
gate_init --name t
gate_step step-one :
gate_run
CASE
expect caller-state 0 't: all passed' 't: all passed'

new_case
printf 'gate_init --name t\ngate_step step-one :\ngate_run\n' >> "$consumer"
rm "$tmp/bin/mktemp"
cat > "$tmp/bin/mktemp" <<'STUB'
#!/bin/sh
exit 1
STUB
chmod +x "$tmp/bin/mktemp"
expect log-creation 2 't: gate_run: cannot create a log directory' 't: gate_run: cannot create a log directory'
rm "$tmp/bin/mktemp"
ln -s "$(type -P mktemp)" "$tmp/bin/mktemp"

if [ -x /bin/bash ] && [ "$(/bin/bash -c 'printf %s "${BASH_VERSINFO[0]}"')" != "${BASH_VERSINFO[0]}" ]; then
    runner=/bin/bash
    mode=strict
    core_cases
else
    printf 'gate_lib_test: alternate shell unavailable, skipped\n'
fi
if [ "$failed" -ne 0 ]; then exit 1; fi
printf 'gate_lib_test: all passed\n'
