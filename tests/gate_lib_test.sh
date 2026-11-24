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
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
for tool in mktemp rm tail cat sleep git; do
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

initialization_cases() {
    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t
gate_step first 'exit 1'
gate_init --name other
gate_step second :
gate_run
CASE
    expect repeated-init 2 't: gate_init: called twice' 't: gate_init: called twice'

    new_case
    printf 'gate_init --name t\ngate_step first :\ngate_run\n' >> "$consumer"
    export _GATE_INITIALIZED=1
    expect inherited-init 0 't: all passed' 't: all passed'
    unset _GATE_INITIALIZED
}

load_guard_cases() {
    new_case
    printf 'gate_init --name t\ngate_step first :\ngate_run\n' >> "$consumer"
    export _GATE_LOADED=1
    expect inherited-load-guard 0 't: all passed' 't: all passed'
    unset _GATE_LOADED
}

list_stream_cases() {
    new_case
    cat >> "$consumer" <<'CASE'
gate_init --name t --strict-var TEST_STRICT -- "$@"
if gate_require invented-absent 'a check'; then exit 9; fi
gate_step first :
gate_step second :
gate_run
CASE
    export TEST_STRICT=1
    (cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer" --list) > "$case_dir/out" 2> "$case_dir/err"
    check list-missing-status test "$?" -eq 0
    check list-missing-labels test "$(cat "$case_dir/out")" = "$(printf 'first\nsecond')"
    check list-missing-stderr grep -qx 't: invented-absent is missing, skipping a check' "$case_dir/err"
    unset TEST_STRICT
}

unreadable_log_cases() {
    local kind
    for kind in test contract; do
        new_case
        cat >> "$consumer" <<CASE
gate_init --name t
gate_step --$kind erased 'rm -rf "\$TMPDIR"'
gate_run
CASE
        expect "unreadable-$kind" 2 't: gate_run: cannot read ' ''
        check "unreadable-path-$kind" test "${output##*/}" = step-1.log
    done
}

signal_cases() {
    local kind option result wanted
    for kind in plain test contract; do
        new_case
        option=''
        if [ "$kind" != plain ]; then option="--$kind"; fi
        printf "gate_init --name t\ngate_step %s signaled 'kill -9 \$\$'\ngate_run\n" "$option" >> "$consumer"
        result=1 wanted='FAIL: signaled'
        if [ "$kind" = contract ]; then result=2 wanted='ERROR: signaled (exit 137;'; fi
        expect "signal-$kind" "$result" "$wanted" ''
    done
}


# Each fixture keeps its own consumer and logs out of the change set.
touched_fixture() {
    new_case
    git init -q "$case_dir"
    mkdir -p "$case_dir/docs/deep"
    printf 'docs/* step-one\n' > "$case_dir/map"
    printf 'base\n' > "$case_dir/docs/notes.md"
    printf 'base\n' > "$case_dir/docs/deep/notes.md"
    printf 'ignored\nconsumer.sh\nlogs/\nelsewhere/\nmarker\nout\nerr\n' > "$case_dir/.gitignore"
    git -C "$case_dir" add .
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Start a fixture.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    cat >> "$consumer" <<'CASE'
gate_init --name t --map map -- "$@"
gate_step step-one 'printf "one\n" >> marker'
gate_step step-two 'printf "two\n" >> marker'
CASE
}

touched_run() {
    printf 'gate_run\n' >> "$consumer"
    expect "$1" 0 "$2" 't: all passed' --tier touched
    check "$1-marker" test "$(cat "$case_dir/marker" 2>/dev/null)" = "$3"
    check "$1-cleanup" test -z "$(ls -A "$case_dir/logs")"
}

touched_cases() {
    local change path reason init args
    for change in committed staged unstaged untracked deleted space nested; do
        touched_fixture
        case "$change" in
            untracked) printf 'new\n' > "$case_dir/docs/new.md" ;;
            deleted) rm "$case_dir/docs/notes.md" ;;
            space) printf 'new\n' > "$case_dir/docs/my notes.md" ;;
            nested) printf 'new\n' > "$case_dir/docs/deep/notes.md" ;;
            *) printf 'changed\n' > "$case_dir/docs/notes.md" ;;
        esac
        if [ "$change" = staged ] || [ "$change" = committed ]; then git -C "$case_dir" add docs; fi
        if [ "$change" = committed ]; then
            git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Change a file.'
        fi
        touched_run "$change" 't: tier touched, 1 paths changed, 1 steps' one
    done
    touched_fixture
    git -C "$case_dir" rm -q --cached docs/notes.md
    touched_run deduplicated 't: tier touched, 1 paths changed, 1 steps' one

    for change in none ignored restored; do
        touched_fixture
        case "$change" in
            ignored) printf 'ignored\n' > "$case_dir/ignored" ;;
            restored) printf 'change\n' > "$case_dir/docs/notes.md"; printf 'base\n' > "$case_dir/docs/notes.md" ;;
        esac
        touched_run "$change" 't: tier touched, 0 paths changed, 0 steps' ''
        check "$change-no-step" test "${output#*step }" = "$output"
        check "$change-no-slowest" test "${output#*slowest}" = "$output"
    done
    touched_fixture
    printf 'docs/* -\n' > "$case_dir/map"
    git -C "$case_dir" add map
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Use an empty selection.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    printf 'changed\n' > "$case_dir/docs/notes.md"
    printf 'gate_step --always scanner '\''printf "scan\\n" >> marker'\''\n' >> "$consumer"
    touched_run dash-always 't: tier touched, 1 paths changed, 1 steps' scan
    rm "$case_dir/marker"
    printf 'base\n' > "$case_dir/docs/notes.md"
    expect always-no-change 0 't: tier touched, 0 paths changed, 1 steps' 't: all passed' --tier touched
    check always-no-change-marker test "$(cat "$case_dir/marker")" = scan

    touched_fixture
    printf 'changed\n' > "$case_dir/direct.sh"
    printf 'gate_step "bash direct.sh" '\''printf "label\\n" >> marker'\''\ngate_step command '\''printf "command\\n" >> marker; : direct.sh'\''\n' >> "$consumer"
    touched_run word-matches 't: tier touched, 1 paths changed, 2 steps' "$(printf 'label\ncommand')"

    for change in unmapped base missing malformed unknown absent tab newline unicode quote backslash; do
        touched_fixture
        reason=''
        case "$change" in
            unmapped) printf x > "$case_dir/other"; reason='other has no entry in map' ;;
            base) git -C "$case_dir" update-ref -d refs/remotes/origin/main; reason='no merge base with origin/main' ;;
            missing) rm "$case_dir/map"; reason='cannot read map' ;;
            malformed) printf 'docs/*\n' > "$case_dir/map"; reason='map line 1 has no step' ;;
            unknown) printf 'docs/* absent-step\n' > "$case_dir/map"; reason='map line 1 names no registered step: absent-step' ;;
            absent) rm "$tmp/bin/git"; reason='git is unavailable' ;;
            *)
                case "$change" in
                    tab) path=$'docs/a\tb' ;; newline) path=$'docs/a\nb' ;;
                    unicode) path='docs/café.md' ;; quote) path='docs/a"b' ;; backslash) path='docs/a\b' ;;
                esac
                printf x > "$case_dir/$path"
                reason='a changed path has a character the map cannot match'
                ;;
        esac
        touched_run "fallback-$change" "t: tier touched falls back to the full tier: $reason" "$(printf 'one\ntwo')"
        case "$change" in tab|newline|unicode|quote|backslash) check "hidden-$change" test "${output#*"$path"}" = "$output" ;; esac
        if [ "$change" = absent ]; then ln -s "$(type -P git)" "$tmp/bin/git"; fi
    done

    for change in dollar backtick; do
        touched_fixture
        if [ "$change" = dollar ]; then
            cat > "$case_dir/map" <<'MAP'
docs/* $(touch pwned)
MAP
        else
            cat > "$case_dir/map" <<'MAP'
docs/* `touch pwned2`
MAP
        fi
        printf 'gate_run\n' >> "$consumer"
        expect "data-$change" 0 'names no registered step:' 't: all passed' --tier touched
        check "data-$change-safe" test ! -e "$case_dir/pwned"
        check "data-$change-safe-two" test ! -e "$case_dir/pwned2"
    done

    touched_fixture
    printf 'changed\n' > "$case_dir/docs/notes.md"
    printf 'gate_run\n' >> "$consumer"
    (cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer" --list --tier touched) > "$case_dir/out" 2> "$case_dir/err"
    check touched-list-status test "$?" -eq 0
    check touched-list-stdout test "$(cat "$case_dir/out")" = step-one
    check touched-list-stderr grep -qx 't: tier touched, 1 paths changed, 1 steps' "$case_dir/err"
    check touched-list-no-run test ! -e "$case_dir/marker"
    check touched-list-no-logs test -z "$(ls -A "$case_dir/logs")"
    printf x > "$case_dir/other"
    (cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer" --tier touched --list) > "$case_dir/out" 2> "$case_dir/err"
    check fallback-list test "$(cat "$case_dir/out")" = "$(printf 'step-one\nstep-two')"
    check fallback-list-stderr grep -qx 't: tier touched falls back to the full tier: other has no entry in map' "$case_dir/err"

    touched_fixture
    printf 'gate_step --mac-only mac '\''printf "mac\\n" >> marker'\''\ngate_run\n' >> "$consumer"
    printf 'changed\n' > "$case_dir/docs/notes.md"
    export TEST_OS=Linux
    expect unselected-mac 0 'step 1 ' 't: all passed' --tier touched
    rm "$case_dir/marker"
    printf 'docs/* mac\nmap -\n' > "$case_dir/map"
    git -C "$case_dir" add map
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Select a platform step.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    expect selected-mac 75 'runs only on macOS' '' --tier touched
    check selected-mac-preflight test ! -e "$case_dir/marker"


    # Every matching row contributes, with each selected step run once.
    touched_fixture
    printf 'docs/* step-one\ndocs/* step-*\nmap -\n' > "$case_dir/map"
    record_fixture
    printf 'changed\n' > "$case_dir/docs/new.md"
    printf 'changed\n' > "$case_dir/docs/notes.md"
    touched_run multi-match 't: tier touched, 2 paths changed, 2 steps' "$(printf 'one\ntwo')"

    # Map whitespace, comments, a step pattern with spaces and no final newline.
    touched_fixture
    printf ' \t# comment\n\t \n docs/* \tstep with spaces \t' > "$case_dir/map"
    printf 'gate_step "step with spaces" :\ngate_run\n' >> "$consumer"
    expect map-whitespace 0 't: map ok' 't: map ok' --check-map

    # Change-listing errors and temporary-file failures must run the full tier.
    local fault real_git
    real_git=$(type -P git)
    for fault in diff ls-files mktemp; do
        touched_fixture
        printf 'changed\n' > "$case_dir/docs/notes.md"
        if [ "$fault" = mktemp ]; then
            rm "$tmp/bin/mktemp"
            cat > "$tmp/bin/mktemp" <<'STUB'
#!/bin/sh
if [ "$1" != -d ]; then exit 1; fi
STUB
            printf 'runner="%s"\n' "$(type -P mktemp)" >> "$tmp/bin/mktemp"
            cat >> "$tmp/bin/mktemp" <<'STUB'
exec "$runner" "$@"
STUB
            chmod +x "$tmp/bin/mktemp"
        else
            rm "$tmp/bin/git"
            {
                printf '#!/bin/sh\n'
                printf 'fault=%s\n' "$fault"
                cat <<'STUB'
if [ "$3" = "$fault" ]; then exit 1; fi
STUB
                printf 'runner="%s"\n' "$real_git"
                cat <<'STUB'
exec "$runner" "$@"
STUB
            } > "$tmp/bin/git"
            chmod +x "$tmp/bin/git"
        fi
        touched_run "listing-$fault" 't: tier touched falls back to the full tier: could not list the change' "$(printf 'one\ntwo')"
        if [ "$fault" = ls-files ] || [ "$fault" = mktemp ]; then
            expect "check-listing-$fault" 2 't: gate_run: could not list tracked files' '' --check-map
            check "check-listing-$fault-cleanup" test -z "$(ls -A "$case_dir/logs")"
        fi
        if [ "$fault" = mktemp ]; then
            rm "$tmp/bin/mktemp"
            ln -s "$(type -P mktemp)" "$tmp/bin/mktemp"
        else
            rm "$tmp/bin/git"
            ln -s "$real_git" "$tmp/bin/git"
        fi
    done

    touched_fixture
    git -C "$case_dir" branch -f main HEAD
    printf 'changed\n' > "$case_dir/docs/notes.md"
    git -C "$case_dir" add docs
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Change the base.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    # Only the local main branch sees the committed change.
    sed 's/--map map/--map map --base main/' "$consumer" > "$case_dir/replaced"
    mv "$case_dir/replaced" "$consumer"
    touched_run local-base 't: tier touched, 1 paths changed, 1 steps' one

    for init in '--map ""' '--base -x' '--base "a b"' '--map map --map map' '--base main --base main' $'--map \'bad\nmap\'' '--name t' '--strict-var X --strict-var X'; do
        new_case
        printf 'gate_init --name t %s\n' "$init" >> "$consumer"
        expect "invalid-$init" 2 'gate_init:' ''
    done
    new_case
    printf 'gate_init --name t\ngate_step --always --always first :\n' >> "$consumer"
    expect repeated-always 2 'gate_step: repeated --always' ''
    for args in '--tier touched' '--check-map'; do
        new_case
        printf 'gate_init --name t -- "$@"\n' >> "$consumer"
        read -r -a arguments <<< "$args"
        expect "no-map-$args" 2 'usage: consumer.sh [--tier full|land] [--list]' '' "${arguments[@]+"${arguments[@]}"}"
    done
    for args in '--check-map --list' '--check-map --tier full'; do
        touched_fixture
        read -r -a arguments <<< "$args"
        expect "map-usage-$args" 2 'usage: consumer.sh [--tier full|land|touched] [--list] | --check-map' '' "${arguments[@]+"${arguments[@]}"}"
    done
    touched_fixture
    cat >> "$consumer" <<'CASE'
printf 'tier=%s\n' "$GATE_TIER"
gate_run
CASE
    expect touched-global 0 'tier=touched' 't: all passed' --tier touched

    touched_fixture
    printf 'gate_run\n' >> "$consumer"
    expect good-map 0 't: map ok' 't: map ok' --check-map
    check map-no-run test ! -e "$case_dir/marker"
    check map-cleanup test -z "$(ls -A "$case_dir/logs")"
    printf '# comment\n docs/* \n docs/* absent-step\n nowhere/* step-one\n' > "$case_dir/map"
    expect bad-map 1 't: map has 3 problems' 't: map has 3 problems' --check-map
    check malformed-line test "${output#*FAIL: map line 2 has no step}" != "$output"
    check unknown-line test "${output#*FAIL: map line 3 names no registered step: absent-step}" != "$output"
    check unmatched-line test "${output#*FAIL: map line 4 matches no tracked file: nowhere/*}" != "$output"
    check map-problem-count test "$(printf '%s\n' "$output" | grep -c '^FAIL: ')" -eq 3
    printf 'nowhere/*\n' > "$case_dir/map"
    expect malformed-unmatched-map 1 'FAIL: map line 1 matches no tracked file: nowhere/*' 't: map has 2 problems' --check-map
    rm "$case_dir/map"
    expect unreadable-map 1 'FAIL: cannot read map' 't: map has 1 problems' --check-map
    rm "$tmp/bin/git"
    expect check-without-git 2 't: gate_run: git is unavailable' '' --check-map
    ln -s "$(type -P git)" "$tmp/bin/git"
    touched_fixture
    printf 'gate_missing invented-check "a check"\ngate_run\n' >> "$consumer"
    # Strict mode is selected during initialization, before the missing probe.
    sed 's/--map map/--strict-var TEST_STRICT --map map/' "$consumer" > "$case_dir/replaced"
    mv "$case_dir/replaced" "$consumer"
    export TEST_STRICT=1
    expect strict-map-missing 75 't: cannot check the map on this host: a tool is missing' '' --check-map
    unset TEST_STRICT
    expect loose-map-missing 75 't: cannot check the map on this host: a tool is missing' '' --check-map
}

# Commit setup separately so each case measures only the intended change.
record_fixture() {
    git -C "$case_dir" add .
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record fixture setup.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
}

touched_fixup_cases() {
    local change path first second
    touched_fixture
    git -C "$case_dir" tag origin/main HEAD
    touched_run ambiguous-base 'the base origin/main is ambiguous' "$(printf 'one\ntwo')"

    touched_fixture
    git -C "$case_dir" branch -f main HEAD
    sed 's/--map map/--map map --base refs\/heads\/main/' "$consumer" > "$case_dir/replaced"
    mv "$case_dir/replaced" "$consumer"
    printf x >> "$case_dir/docs/notes.md"
    touched_run full-base-ref 't: tier touched, 1 paths changed, 1 steps' one

    touched_fixture
    first=$(git -C "$case_dir" rev-parse HEAD)
    printf '[submodule "s"]\n path = vendor/s\n url = ./unused\n ignore = all\n' > "$case_dir/.gitmodules"
    git -C "$case_dir" update-index --add --cacheinfo "160000,$first,vendor/s"
    git -C "$case_dir" add .gitmodules
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record a gitlink.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    second=$(git -C "$case_dir" rev-parse HEAD)
    git -C "$case_dir" update-index --cacheinfo "160000,$second,vendor/s"
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Bump a gitlink.'
    touched_run submodule-ignore 'vendor/s has no entry in map' "$(printf 'one\ntwo')"

    touched_fixture
    sed '/^gate_init/i\
cd docs' "$consumer" > "$case_dir/replaced"
    mv "$case_dir/replaced" "$consumer"
    cp "$case_dir/map" "$case_dir/docs/map"
    printf 'gate_run\n' >> "$consumer"
    expect subdirectory-root 0 'the gate root is not the top of the repository' 't: all passed' --tier touched
    check subdirectory-root-marker test "$(cat "$case_dir/docs/marker")" = "$(printf 'one\ntwo')"
    expect subdirectory-check-map 2 't: gate_run: the gate root is not the top of the repository' '' --check-map

    touched_fixture
    mkdir "$case_dir/tests"
    printf x > "$case_dir/tests/my ok_test.sh"
    printf 'gate_step "bash tests/my ok_test.sh" '\''printf "space\\n" >> marker'\''\n' >> "$consumer"
    touched_run space-label 't: tier touched, 1 paths changed, 1 steps' space

    touched_fixture
    printf x > "$case_dir/warnings"
    printf 'gate_step warnings '\''printf "warnings\\n" >> marker'\''\n' >> "$consumer"
    touched_run bare-word 'warnings has no entry in map' "$(printf 'one\ntwo\nwarnings')"

    for change in map consumer library; do
        touched_fixture
        printf '* -\n' > "$case_dir/map"
        # Use an in-repository library and a tracked consumer for these cases.
        mkdir "$case_dir/gate"
        cp "$root/gate/lib.sh" "$case_dir/gate/lib.sh"
        sed "s|$root/gate/lib.sh|$case_dir/gate/lib.sh|" "$consumer" > "$case_dir/replaced"
        mv "$case_dir/replaced" "$consumer"
        printf 'gate_run\n' >> "$consumer"
        git -C "$case_dir" add -f consumer.sh
        record_fixture
        case "$change" in
            map) path=map ;;
            consumer) path=consumer.sh ;;
            library) path=gate/lib.sh ;;
        esac
        printf '# changed\n' >> "$case_dir/$path"
        expect "own-$change" 0 "$path is one of the gate's own files" 't: all passed' --tier touched
        check "own-$change-marker" test "$(cat "$case_dir/marker")" = "$(printf 'one\ntwo')"
    done

    touched_fixture
    printf 'docs/* absent\033step\nmissing\033/* step-one\n' > "$case_dir/map"
    printf 'gate_run\n' >> "$consumer"
    expect escaped-map-fallback 0 'names no registered step: absent?step' 't: all passed' --tier touched
    check escaped-map-fallback-no-escape test "${output#*$'\033'}" = "$output"
    expect escaped-map-check 1 'matches no tracked file: missing?/*' 't: map has 2 problems' --check-map
    check escaped-map-check-step test "${output#*names no registered step: absent?step}" != "$output"
    check escaped-map-check-no-escape test "${output#*$'\033'}" = "$output"

    touched_fixture
    printf 'shopt -s nocasematch\ngate_run\n' >> "$consumer"
    expect nocasematch-touched 0 'nocasematch is set' 't: all passed' --tier touched
    expect nocasematch-check 1 'FAIL: nocasematch is set' 't: map has 1 problems' --check-map
}

touched_contract_cases() {
    local path
    touched_fixture
    printf 'docs/notes.md step-one\ndocs/renamed.md -\n' > "$case_dir/map"
    record_fixture
    git -C "$case_dir" mv docs/notes.md docs/renamed.md
    touched_run no-renames 't: tier touched, 2 paths changed, 1 steps' one

    touched_fixture
    printf 'docs/notes.md step-one\ndocs/base.md step-two\n' > "$case_dir/map"
    record_fixture
    path=$(git -C "$case_dir" rev-parse HEAD)
    printf x > "$case_dir/docs/base.md"
    git -C "$case_dir" add docs
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Advance the base.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    git -C "$case_dir" switch -q -c topic "$path"
    printf x >> "$case_dir/docs/notes.md"
    touched_run merge-base-not-tip 't: tier touched, 1 paths changed, 1 steps' one

    touched_fixture
    printf 'docs/*\t-\ndocs/*\tstep-one' > "$case_dir/map"
    record_fixture
    printf x >> "$case_dir/docs/notes.md"
    touched_run tab-final-dash 't: tier touched, 1 paths changed, 1 steps' one

    touched_fixture
    path=$'docs/a\177b'
    printf x > "$case_dir/$path"
    touched_run del-path 'a changed path has a character the map cannot match' "$(printf 'one\ntwo')"
    check del-hidden test "${output#*"$path"}" = "$output"

    touched_fixture
    printf 'docs/* absent-first\ndocs/* absent-second\n' > "$case_dir/map"
    git -C "$case_dir" update-ref -d refs/remotes/origin/main
    touched_run map-problem-precedence 'full tier: map line 1 names no registered step: absent-first' "$(printf 'one\ntwo')"
    check first-map-problem test "${output#*absent-second}" = "$output"

    touched_fixture
    printf 'docs/* stop\ndocs/* after\n' > "$case_dir/map"
    record_fixture
    printf x >> "$case_dir/docs/notes.md"
    printf 'gate_step --contract stop "exit 2"\ngate_step after :\ngate_run\n' >> "$consumer"
    expect selected-stop-count 2 'stopped after step 1; 1 steps not run' '' --tier touched

    touched_fixture
    printf 'docs/* command-only\n' > "$case_dir/map"
    printf 'gate_step label-only command-only\ngate_run\n' >> "$consumer"
    expect patterns-use-labels 0 'map line 1 names no registered step: command-only' '' --tier touched --list

    touched_fixture
    # Isolate selection's file I/O so a missing TMPDIR tests only log creation.
    cat >> "$consumer" <<'CASE'
TMPDIR="$TMPDIR/absent"
gate_run
CASE
    rm "$tmp/bin/mktemp"
    cat > "$tmp/bin/mktemp" <<'STUB'
#!/bin/sh
if [ "$1" = -d ]; then exit 1; fi
STUB
    printf 'exec "%s" "%s/selection.XXXXXX"\n' "$(type -P mktemp)" "$case_dir/logs" >> "$tmp/bin/mktemp"
    chmod +x "$tmp/bin/mktemp"
    expect empty-no-log-directory 0 '0 paths changed, 0 steps' 't: all passed' --tier touched
    check empty-no-log-path test ! -e "$case_dir/logs/absent"
    rm "$tmp/bin/mktemp"
    ln -s "$(type -P mktemp)" "$tmp/bin/mktemp"
}

touched_safety_cases() {
    touched_fixture
    printf 'changed\n' > "$case_dir/docs/notes.md"
    git -C "$case_dir" add docs
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Change a file.'
    touched_run strict-committed-selection 't: tier touched, 1 paths changed, 1 steps' one
    touched_fixture
    printf x > "$case_dir/other"
    touched_run strict-fallback 't: tier touched falls back to the full tier: other has no entry in map' "$(printf 'one\ntwo')"
    touched_fixture
    printf 'gate_run\n' >> "$consumer"
    expect strict-good-map 0 't: map ok' 't: map ok' --check-map
}

consumer_cases() {
    touched_fixup_cases
    touched_contract_cases
    touched_cases
    mode=strict
    touched_safety_cases
    mode=loose
    # The command runs in the child shell whose version is being inspected.
    # shellcheck disable=SC2016
    printf 'gate_lib_test: consumer bash %s\n' "$("$runner" -c 'printf %s "${BASH_VERSINFO[0]}"')"
initialization_cases
load_guard_cases
list_stream_cases
signal_cases
unreadable_log_cases
mode=strict
unreadable_log_cases
mode=loose
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
gate_step first 'sleep 1'
gate_step second 'sleep 2'
gate_step third :
gate_step fourth :
gate_step fifth :
gate_step sixth :
# Start just after a clock tick to keep command overhead away from the next boundary.
tick=$SECONDS
while [ "$SECONDS" -eq "$tick" ]; do sleep 0.01; done
gate_run
CASE
expect slowest 0 't: slowest steps' 't: all passed'
ranking=${output#*t: slowest steps$'\n'}
check slow-count test "$(printf '%s\n' "$ranking" | grep -c '^  ')" -eq 5
check slow-order test "$(printf '%s\n' "$ranking" | sed -n '1s/.*s //p')" = second
check next-slow test "$(printf '%s\n' "$ranking" | sed -n '2s/.*s //p')" = first
check remaining-slow test "$(printf '%s\n' "$ranking" | sed -n '3,5p' | grep -Ec '^  [0-9]+s (third|fourth|fifth|sixth)$')" -eq 3
# Only compare registration order when the measured durations actually tie.
previous_time=-1 previous_position=0
while read -r duration label; do
    case "$label" in
        first) position=1 ;; second) position=2 ;; third) position=3 ;;
        fourth) position=4 ;; fifth) position=5 ;; sixth) position=6 ;;
        *) continue ;;
    esac
    if [ "$duration" = "$previous_time" ]; then
        check tie-order test "$position" -gt "$previous_position"
    fi
    previous_time=$duration previous_position=$position
done <<< "$ranking"

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

}

consumer_cases

# Compare with the consumer runner, independently of this file's interpreter.
# shellcheck disable=SC2016
if [ -x /bin/bash ] && [ "$(/bin/bash -c 'printf %s "${BASH_VERSINFO[0]}"')" != "$("$runner" -c 'printf %s "${BASH_VERSINFO[0]}"')" ]; then
    runner=/bin/bash
    mode=loose
    consumer_cases
else
    printf 'gate_lib_test: alternate shell unavailable, skipped\n'
fi
if [ "$failed" -ne 0 ]; then exit 1; fi
printf 'gate_lib_test: all passed\n'
