#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR
root=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd -P)
runner=$BASH
suite=$(mktemp -d)
trap 'rm -rf "$suite"' EXIT
failed=0
check() {
    if ! "$@"; then printf 'FAIL %s: %s\n' "$name" "$*"; failed=1; fi
}
setup() {
    tmp=$(mktemp -d "$suite/case.XXXXXX")
    remote="$tmp/remote.git" repo="$tmp/repo"
    mkdir "$tmp/scratch"
    git init -q --bare --initial-branch=main "$remote"
    git clone -q "$remote" "$repo" 2>/dev/null
    git -C "$repo" config user.name Fixture
    git -C "$repo" config user.email fixture@example.invalid
    git -C "$repo" config commit.gpgsign false
    commit
    base=$head
    printf '0\n' > "$tmp/status"
    cat > "$tmp/check.sh" <<'CHECK'
#!/usr/bin/env bash
set -eu
work=$1
[ -z "${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}${GIT_OBJECT_DIRECTORY+x}${GIT_ALTERNATE_OBJECT_DIRECTORIES+x}${GIT_COMMON_DIR+x}${GREP_OPTIONS+x}${GREP_COLOR+x}${GREP_COLORS+x}" ] || exit 98
printf 'ran\n' >> "$work/runs"
[ "$(git rev-parse HEAD)" = "$(cat "$work/head")" ]
[ "$PWD" != "$(cat "$work/repo-path")" ]
[ "$#" -eq 2 ] && [ "$2" = 'one word; $(false)' ]
if read -r unused; then exit 99; fi
if [ -f "$work/hook" ]; then bash "$work/hook"; fi
status=$(head -1 "$work/status")
sed '1d' "$work/status" > "$work/next"
[ -s "$work/next" ] && mv "$work/next" "$work/status"
exit "$status"
CHECK
    printf '%s\n' "$repo" > "$tmp/repo-path"
    want_green='' want_frozen=''
}
commit() {
    git -C "$repo" -c core.hooksPath=/dev/null commit -q --allow-empty -m 'Record a fixture.'
    git -C "$repo" push -q origin HEAD:main
    head=$(git -C "$repo" rev-parse HEAD)
    printf '%s\n' "$head" > "$tmp/head"
}
ref() { git -C "$repo" ls-remote -- origin "$1" | cut -f1; }
seed_green() { git -C "$repo" push -q origin "$1:refs/green/main"; want_green=$1; }
seed_frozen() { git -C "$repo" push -q --force origin "$1:refs/green/frozen"; want_frozen=$1; }
invoke() {
    actual=0
    output=$(TMPDIR="$tmp/scratch" "$runner" "$root/gate/verify-main.sh" "$@" 2>"$tmp/err") || actual=$?
}
# shellcheck disable=SC2016
run() { invoke --root "$repo" -- "$runner" "$tmp/check.sh" "$tmp" 'one word; $(false)'; }
expect() {
    check test "$actual" -eq "$1"
    check test "${output##*$'\n'}" = "$2"
    check test "$(ref refs/green/main)" = "$want_green"
    check test "$(ref refs/green/frozen)" = "$want_frozen"
    check test ! -e "$repo/.git/verify-main.lock"
    check test ! -L "$repo/.git/verify-main.lock"
    check test -z "$(find "$tmp/scratch" -name 'verify-main.*' -print)"
}
case_usage() {
    for kind in no-root subdir remote dash repeat-root repeat-remote no-separator no-command unknown missing; do
        mkdir -p "$repo/sub"
        case "$kind" in
            no-root) invoke -- true ;;
            subdir) invoke --root "$repo/sub" -- true ;;
            remote) invoke --root "$repo" --remote absent -- true ;;
            dash) invoke --root "$repo" --remote -bad -- true ;;
            repeat-root) invoke --root "$repo" --root "$repo" -- true ;;
            repeat-remote) invoke --root "$repo" --remote origin --remote origin -- true ;;
            no-separator) invoke --root "$repo" true ;;
            no-command) invoke --root "$repo" -- ;;
            unknown) invoke --root "$repo" --bad -- true ;;
            missing) invoke --root ;;
        esac
        expect 2 ''
        check grep -Fxq 'usage: verify-main.sh --root <repo top> [--remote <name>] -- <check command> [<arg>...]' "$tmp/err"
    done
}
case_first_green() {
    run
    want_green=$head
    expect 0 "green $head"
    check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = 1

}
case_current() {
    seed_green "$head"
    run
    expect 0 "green is current: $head"
    check test ! -f "$tmp/runs"

}
case_current_frozen() {
    seed_green "$head"
    seed_frozen "$head"
    run
    want_frozen=''
    expect 0 "green is current: $head"
    check test "${output%%$'\n'*}" = "releases unfrozen (freeze at $head cleared)"

}
case_current_off_main() {
    seed_green "$head"
    other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
    seed_frozen "$other"
    run
    expect 0 "green is current: $head"
    short=$(git -C "$repo" rev-parse --short "$other")
    check test "${output%%$'\n'*}" = "verify-main: releases are still frozen at $short, which is not on origin/main; delete refs/green/frozen by hand"

}
case_advance() {
    seed_green "$head"
    commit
    commit
    run
    want_green=$head
    expect 0 "green $head"

}
case_red_one() {
    seed_green "$head"
    commit
    printf '1\n' > "$tmp/status"
    run
    want_frozen=$head
    expect 5 "red $head culprit $head"
    check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = 2
    short=$(git -C "$repo" rev-parse --short "$head")
    check test "${output%%$'\n'*}" = "verify-main: releases are frozen. Revert or fix $short, then run this again."

}
case_red_many() {
    seed_green "$head"
    commit
    first=$head
    commit
    commit
    printf '1\n' > "$tmp/status"
    run
    want_frozen=$head
    expect 6 "red $head across 3 commits from $first to $head"

}
case_no_green_red() {
    for n in 1 3; do
        if [ "$n" -eq 3 ]; then setup; commit; commit; fi
        printf '1\n' > "$tmp/status"
        run
        want_frozen=$head
        if [ "$n" -eq 1 ]; then expect 5 "red $head culprit $head";
        else expect 6 "red $head across 3 commits from $base to $head"; fi
    done

}
case_flaky() {
    printf '1\n0\n' > "$tmp/status"
    run
    want_green=$head
    expect 0 "green $head"
    check test "${output%%$'\n'*}" = 'verify-main: the first run was red; the rerun passed'
    check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = 2

}
case_fixed() {
    seed_green "$head"
    commit
    seed_frozen "$head"
    commit
    run
    want_green=$head want_frozen=''
    expect 0 "green $head"
    check grep -Fq "releases unfrozen (freeze at $(git -C "$repo" rev-parse HEAD~1) cleared)" <<< "$output"

}
case_unavailable() {
    seed_green "$head"
    commit
    seed_frozen "$head"
    for statuses in 75 '1 75'; do
        # Splitting the fixture's one or two numeric status words is intentional.
        # shellcheck disable=SC2086
        printf '%s\n' $statuses > "$tmp/status"
        rm -f "$tmp/runs"
        run
        expect 75 'verify-main: the check cannot run on this host (exit 75); nothing changed'
        n=1
        [ "$statuses" = 75 ] || n=2
        check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = "$n"
    done

}
case_rewritten() {
    seed_green "$head"
    other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
    git -C "$repo" push -q --force origin "$other:main"
    run
    short=$(git -C "$repo" rev-parse --short "$head")
    expect 1 "verify-main: green $short is not on origin/main; nothing changed"
    check test ! -f "$tmp/runs"

}
case_main_moved() {
    other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
    printf 'git -C "%s" push -q --force origin "%s:main"\n' "$repo" "$other" > "$tmp/hook"
    for status in 0 1; do
        printf '%s\n' "$status" > "$tmp/status"
        git -C "$repo" push -q --force origin "$head:main"
        run
        expect 1 'verify-main: main moved off the checked head; nothing changed'
    done

}
case_covered() {
    checked=$head
    commit
    descendant=$head
    git -C "$repo" push -q --force origin "$checked:main"
    head=$checked
    printf '%s\n' "$head" > "$tmp/head"
    printf 'git -C "%s" push -q origin "%s:refs/green/main"\n' "$repo" "$descendant" > "$tmp/hook"
    run
    want_green=$descendant
    expect 0 "covered: green is $descendant"

}
case_green_race() {
    for status in 0 1; do
        [ "$status" -eq 0 ] || setup
        other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
        printf 'git -C "%s" push -q origin "%s:refs/green/main"\n' "$repo" "$other" > "$tmp/hook"
        printf '%s\n' "$status" > "$tmp/status"
        run
        want_green=$other
        expect 1 'verify-main: green moved during the check'
    done

}
case_freeze_race() {
    for status in 0 1; do
        [ "$status" -eq 0 ] || setup
        other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
        printf 'git -C "%s" push -q origin "%s:refs/green/frozen"\n' "$repo" "$other" > "$tmp/hook"
        printf '%s\n' "$status" > "$tmp/status"
        run
        want_frozen=$other
        expect 1 'verify-main: the freeze moved during the check'
    done

}
case_stale_lock() {
    ln -s 99999999 "$repo/.git/verify-main.lock"
    run
    want_green=$head
    expect 0 "green $head"

}
case_live_lock() {
    cat > "$tmp/verify-main.sh" <<'LOCK'
#!/usr/bin/env bash
sleep 30 &
child=$!
trap 'kill "$child" 2>/dev/null || :' EXIT
trap 'exit 0' TERM INT HUP
wait
LOCK
    "$runner" "$tmp/verify-main.sh" &
    pid=$!
    ln -s "$pid" "$repo/.git/verify-main.lock"
    # Some sandboxes deny ps. Model only its command-line response there;
    # the lock still names a real live process, checked by kill -0.
    if ! ps -p "$pid" -o args= >/dev/null 2>&1; then
        mkdir "$tmp/bin"
        printf '#!/bin/sh\nprintf "bash verify-main.sh\\n"\n' > "$tmp/bin/ps"
        chmod +x "$tmp/bin/ps"
        saved_path=$PATH
        PATH="$tmp/bin:$PATH"
        printf 'live_lock: ps response modeled (host denies ps)\n'
    fi
    run
    check test "$actual" -eq 0
    check test "$output" = "verify-main: already running (pid $pid)"
    check test "$(ref refs/green/main)" = ''
    check test "$(ref refs/green/frozen)" = ''
    check test "$(readlink "$repo/.git/verify-main.lock")" = "$pid"
    kill "$pid"
    wait "$pid" 2>/dev/null || :
    rm -f "$repo/.git/verify-main.lock"
    if [ -n "${saved_path:-}" ]; then PATH=$saved_path; fi
    expect 0 "verify-main: already running (pid $pid)"

}
case_leftovers() {
    old="$tmp/scratch/verify-main.A1b2C3"
    mkdir "$old"
    git -C "$repo" worktree add -q --detach "$old/wt" "$head"
    mkdir "$tmp/kept"
    git -C "$repo" worktree add -q --detach "$tmp/kept/wt" "$head"
    link="$tmp/scratch/verify-main.Z9y8X7"
    ln -s "$tmp/kept" "$link"
    # Register the linked spelling without allowing git to canonicalize it.
    entry=$(git -C "$repo" rev-parse --git-path worktrees/wt1/gitdir)
    case "$entry" in /*) ;; *) entry="$repo/$entry" ;; esac
    printf '%s/wt/.git\n' "$link" > "$entry"
    run
    want_green=$head
    # The look-alike is deliberately retained, then removed by the fixture.
    check test ! -e "$old"
    check test -L "$link"
    check test -f "$tmp/kept/wt/.git"
    rm "$link"
    expect 0 "green $head"

}
case_environment() {
    for variable in GIT_DIR CDPATH GREP_OPTIONS GIT_COMMON_DIR; do
        setup
        git init -q "$tmp/other"
        case "$variable" in
            GIT_DIR) GIT_DIR="$tmp/other/.git" run ;;
            GIT_COMMON_DIR) GIT_COMMON_DIR="$tmp/other/.git" run ;;
            CDPATH) CDPATH=. run ;;
            GREP_OPTIONS) GREP_OPTIONS=-v run ;;
        esac
        want_green=$head
        expect 0 "green $head"
    done

}
case_failures() {
    real_git=$(command -v git)
    mkdir "$tmp/bin"
    cat > "$tmp/bin/git" <<'WRAP'
#!/usr/bin/env bash
set -eu
case " $* " in
    *" $FAIL_COMMAND "*)
        if [ "$FAIL_COMMAND" = ls-remote ] && [ "${BAD_REF:-0}" = 1 ]; then
            printf 'oops\trefs/green/main\n'; exit 0
        fi
        exit 1 ;;
esac
exec "$REAL_GIT" "$@"
WRAP
    chmod +x "$tmp/bin/git"
    saved_path=$PATH
    for operation in fetch ls-remote 'worktree add' push; do
        case "$operation" in
            fetch) reason='cannot fetch main' ;;
            ls-remote) reason='cannot read green refs' ;;
            'worktree add') reason='cannot create a scratch worktree' ;;
            push) reason='green moved during the check' ;;
        esac
        PATH="$tmp/bin:$saved_path" REAL_GIT="$real_git" FAIL_COMMAND="$operation" run
        expect 1 "verify-main: $reason"
    done
    PATH="$tmp/bin:$saved_path" REAL_GIT="$real_git" FAIL_COMMAND=ls-remote BAD_REF=1 run
    expect 1 'verify-main: invalid green ref'
    PATH="$tmp/empty-path" run
    expect 75 'verify-main: git is missing'

}
case_push_races() {
    seed_green "$head"
    commit
    seed_frozen "$head"
    old_frozen=$head
    commit
    other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
    mkdir "$tmp/bin"
    real_git=$(command -v git)
    cat > "$tmp/bin/git" <<'WRAP'
#!/usr/bin/env bash
set -eu
case " $* " in
    *' push --atomic '*)
        "$REAL_GIT" -C "$TEST_REPO" push -q --force origin "$RACE_OID:$RACE_REF"
        ;;
esac
exec "$REAL_GIT" "$@"
WRAP
    chmod +x "$tmp/bin/git"
    PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_OID="$other" RACE_REF=refs/green/frozen run
    want_frozen=$other
    expect 1 'verify-main: the freeze moved during the check'
    seed_frozen "$old_frozen"
    PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_OID="$head" RACE_REF=refs/green/main run
    want_green=$head want_frozen=''
    expect 0 "covered: green is $head"

}
case_red_push_failure() {
    seed_green "$head"
    commit
    printf '1\n' > "$tmp/status"
    printf '#!/bin/sh\nexit 1\n' > "$remote/hooks/pre-receive"
    chmod +x "$remote/hooks/pre-receive"
    run
    short=$(git -C "$repo" rev-parse --short "$head")
    expect 1 "verify-main: $short is red, and the freeze could not be pushed"
    check test "${output%%$'\n'*}" = "red $head culprit $head"

}
case_signals() {
    for signal in TERM HUP INT; do
        setup
        # shellcheck disable=SC2016
        printf 'kill -%s "$(readlink "%s/.git/verify-main.lock")"\n' "$signal" "$repo" > "$tmp/hook"
        run
        case "$signal" in TERM) code=143 ;; HUP) code=129 ;; INT) code=130 ;; esac
        expect "$code" ''
    done

}
case_docs() {
    check grep -Fq '## Green refs (opt-in)' "$root/gate/README.md"
    check grep -Fq 'rewind' "$root/gate/README.md"
    check grep -Fq 'refs/green/*' "$root/gate/README.md"
    check grep -Fq 'verify-main.sh' "$root/scripts/ci-touched.map"
    check test "$(cat "$root/gate/VERSION")" = 0.2.0

}
case_noncommit_ref() {
    blob=$(printf 'fixture\n' | git -C "$repo" hash-object -w --stdin)
    seed_green "$blob"
    mkdir "$tmp/bin"
    real_git=$(command -v git)
    cat > "$tmp/bin/git" <<'WRAP'
#!/usr/bin/env bash
set -eu
for arg in "$@"; do
    if [ "$arg" = "$TEST_OID" ] && [[ " $* " = *' fetch '* ]]; then
        : > "$TEST_MARKER"
    fi
done
exec "$REAL_GIT" "$@"
WRAP
    chmod +x "$tmp/bin/git"
    PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_OID="$blob" TEST_MARKER="$tmp/fetched" run
    expect 1 'verify-main: green ref is not a commit'
    check test ! -e "$tmp/fetched"

}
cases='noncommit_ref docs signals red_push_failure push_races failures environment leftovers live_lock stale_lock freeze_race green_race covered main_moved rewritten unavailable fixed flaky no_green_red red_many red_one advance current_off_main current_frozen current first_green usage'
for name in $cases; do
    if [ "$#" -gt 0 ] && [ "$1" != "$name" ]; then continue; fi
    setup
    "case_$name"
    printf '%s: checked\n' "$name"
done
[ "$failed" -eq 0 ] || exit 1
printf 'gate_verify_main_test: all passed (%s)\n' "$BASH_VERSION"
