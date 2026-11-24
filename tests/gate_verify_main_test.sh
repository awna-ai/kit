#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR
root=$(CDPATH='' cd -- "$(dirname "$0")/.." && pwd -P)
runner=$BASH
suite=$(mktemp -d)
suite=$(CDPATH='' cd -- "$suite" && pwd -P)
trap 'rm -rf "$suite"' EXIT
failed=0
if ! ps -o lstart= -p "$$" >/dev/null 2>&1; then
    mkdir "$suite/bin"
    cat > "$suite/bin/ps" <<'PS'
#!/bin/sh
[ "$1" = -o ] && [ "$2" = lstart= ] && [ "$3" = -p ] || exit 1
kill -0 "$4" 2>/dev/null || exit 1
printf 'Mon Jan  1 00:00:00 2024\n'
PS
    chmod +x "$suite/bin/ps"
    export PATH="$suite/bin:$PATH"
    printf 'process start times modeled: host denies ps\n'
fi
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
[ -z "${GIT_DIR+x}${GIT_WORK_TREE+x}${GIT_INDEX_FILE+x}${GIT_OBJECT_DIRECTORY+x}${GIT_ALTERNATE_OBJECT_DIRECTORIES+x}${GIT_COMMON_DIR+x}${GIT_NAMESPACE+x}${GREP_OPTIONS+x}${GREP_COLOR+x}${GREP_COLORS+x}" ] || exit 98
printf 'ran\n' >> "$work/runs"
printf 'fixture check output\n'
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
ref() { git -C "$repo" ls-remote -- origin "$1" | awk -v ref="$1" '$2 == ref {print $1}'; }
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
    check grep -Fxq 'fixture check output' "$tmp/err"

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
    short=$(git -C "$repo" rev-parse --short "$head")
    expect 1 "verify-main: refs/green/main and refs/green/frozen both name $short; delete one and run this again"
    check test ! -e "$tmp/runs"
}
case_current_ancestor_frozen() {
    seed_frozen "$head"
    frozen_head=$head
    commit
    seed_green "$head"
    run
    want_frozen=''
    expect 0 "green is current: $head"
    check test "${output%%$'\n'*}" = "releases unfrozen (freeze at $frozen_head cleared)"
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
    ln -s '99999999 Mon Jan  1 00:00:00 2024' "$repo/.git/verify-main.lock"
    run
    want_green=$head
    expect 0 "green $head"

}
case_live_lock() {
    sleep 30 &
    pid=$!
    start=$(TZ=UTC0 ps -o lstart= -p "$pid")
    ln -s "$pid $start" "$repo/.git/verify-main.lock"
    run
    check test "$actual" -eq 0
    check test "$output" = "verify-main: already running (pid $pid)"
    check test "$(ref refs/green/main)" = ''
    check test "$(ref refs/green/frozen)" = ''
    check test "$(readlink "$repo/.git/verify-main.lock")" = "$pid $start"
    kill "$pid"
    wait "$pid" 2>/dev/null || :
    rm -f "$repo/.git/verify-main.lock"
    expect 0 "verify-main: already running (pid $pid)"
}
case_lock_identity() {
    # The child shell supplies its own pid, exactly matching the script after exec.
    actual=0
    # shellcheck disable=SC2016
    output=$(TMPDIR="$tmp/scratch" "$runner" -c '
        ln -s "$$ $(TZ=UTC0 ps -o lstart= -p "$$")" "$1/.git/verify-main.lock"
        exec "$2" "$3" --root "$1" -- true
    ' fixture "$repo" "$runner" "$root/gate/verify-main.sh" 2>"$tmp/err") || actual=$?
    want_green=$head
    expect 0 "green $head"
    setup
    sleep 30 &
    pid=$!
    ln -s "$pid wrong start" "$repo/.git/verify-main.lock"
    run
    want_green=$head
    expect 0 "green $head"
    kill "$pid"
    wait "$pid" 2>/dev/null || :
}
case_bad_lock() {
    for type in directory file; do
        if [ "$type" = directory ]; then mkdir "$repo/.git/verify-main.lock";
        else : > "$repo/.git/verify-main.lock"; fi
        run
        check test "$actual" -eq 1
        check test "$output" = "verify-main: $repo/.git/verify-main.lock is not a lock; remove it"
        check test "$(ref refs/green/main)$(ref refs/green/frozen)" = ''
        rm -rf "$repo/.git/verify-main.lock"
    done
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
    for operation in fetch ls-remote clone push; do
        case "$operation" in
            fetch) reason='cannot fetch main' ;;
            ls-remote) reason='cannot read green refs' ;;
            clone) reason='cannot create a scratch clone' ;;
            push) reason='the remote refused the push; nothing changed' ;;
        esac
        PATH="$tmp/bin:$saved_path" REAL_GIT="$real_git" FAIL_COMMAND="$operation" run
        expect 1 "verify-main: $reason"
    done
    PATH="$tmp/bin:$saved_path" REAL_GIT="$real_git" FAIL_COMMAND=ls-remote BAD_REF=1 run
    expect 1 'verify-main: invalid green ref'
    PATH="$tmp/empty-path" run
    expect 75 'verify-main: git is missing'

}
case_deleted_freeze() {
    for mode in current advance; do
        setup
        seed_frozen "$head"
        commit
        seed_green "$head"
        if [ "$mode" = advance ]; then
            commit
        fi
        mkdir "$tmp/bin"
        real_git=$(command -v git)
        cat > "$tmp/bin/git" <<'WRAP'
#!/bin/sh
case " $* " in
    *' push '*)
        "$REAL_GIT" -C "$TEST_REPO" push -q origin :refs/green/frozen
        # In the advance case the other writer also publishes the checked head.
        if [ "$RACE_MODE" = advance ]; then
            "$REAL_GIT" -C "$TEST_REPO" push -q origin "$RACE_HEAD:refs/green/main"
        fi ;;
esac
exec "$REAL_GIT" "$@"
WRAP
        chmod +x "$tmp/bin/git"
        PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_MODE="$mode" RACE_HEAD="$head" run
        want_green=$head want_frozen=''
        expect 0 "covered: green is $head"
        check test "$output" = "covered: green is $head"
    done
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
    PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_OID='' RACE_REF=refs/green/frozen run
    want_frozen=''
    expect 1 'verify-main: the freeze moved during the check'
    seed_frozen "$old_frozen"
    PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_OID="$head" RACE_REF=refs/green/main run
    want_green=$head want_frozen=''
    expect 0 "covered: green is $head"
    check grep -Fq "releases unfrozen (freeze at $old_frozen cleared)" <<< "$output"

}
case_green_push_failure() {
    printf '#!/bin/sh\nprintf "fixture refusal\\n" >&2\nexit 1\n' > "$remote/hooks/pre-receive"
    chmod +x "$remote/hooks/pre-receive"
    run
    expect 1 'verify-main: the remote refused the push; nothing changed'
    check grep -Fq 'fixture refusal' "$tmp/err"
}
case_red_push_failure() {
    seed_green "$head"
    commit
    printf '1\n' > "$tmp/status"
    printf '#!/bin/sh\nexit 1\n' > "$remote/hooks/pre-receive"
    chmod +x "$remote/hooks/pre-receive"
    run
    expect 1 'verify-main: the remote refused the push; nothing changed'
    check test "${output%%$'\n'*}" = "red $head culprit $head"

}
case_signals() {
    for signal in TERM HUP INT; do
        setup
        # shellcheck disable=SC2016
        printf 'holder=$(readlink "%s/.git/verify-main.lock"); kill -%s "${holder%%%% *}"\n' "$repo" "$signal" > "$tmp/hook"
        run
        case "$signal" in TERM) code=143 ;; HUP) code=129 ;; INT) code=130 ;; esac
        expect "$code" ''
    done

}
case_ps_failure() {
    mkdir "$tmp/bin"
    printf '#!/bin/sh\nexit 1\n' > "$tmp/bin/ps"
    chmod +x "$tmp/bin/ps"
    PATH="$tmp/bin:$PATH" run
    expect 75 'verify-main: cannot inspect this process'
    check test ! -e "$tmp/runs"
}
case_docs() {
    check grep -Fq '## Green refs (opt-in)' "$root/gate/README.md"
    check grep -Fq 'rewind' "$root/gate/README.md"
    check grep -Fq 'refs/green/*' "$root/gate/README.md"
    check grep -Fq 'verify-main.sh' "$root/scripts/ci-touched.map"
    version=$(cat "$root/gate/VERSION")
    check grep -Fq "## $version" "$root/gate/CHANGELOG.md"
    for phrase in 'scratch clone' 'not a sandbox' 'env -i PATH=' 'best effort' 'exit 0 does not mean' 'shadows refs/green/' 'off-main freeze' 'two clones'; do
        check grep -Fq "$phrase" "$root/gate/README.md"
    done
    check grep -Fq "the check runs the remote's code with the caller's environment" "$root/.ai/SECURITY.md"
    check grep -Fq 'best effort' "$root/.ai/SECURITY.md"
    # shellcheck disable=SC2016
    for phrase in 'git commands inside the check' "root's configuration, refs or hooks" 'can read and write anything the caller can' 'every object' 'alternates' 'Only run it on a' 'INT, TERM and HUP are handled once the running check exits' 'SIGKILL leaves the check running' 'hung check delays cleanup' 'must require `refs/green/main` to equal the commit it ships' "caller's \`GIT_CONFIG_*\`, \`GIT_SSH_COMMAND\` and \`GIT_EXEC_PATH\`" 'credential helper may need more' 'cannot inspect this process' 'config.worktree' 'not found' "\`origin\` names the remote's URL"; do
        check grep -Fq "$phrase" "$root/gate/README.md"
    done
    # shellcheck disable=SC2016
    check grep -Fq 'env -i PATH="$PATH" HOME="$HOME" SSH_AUTH_SOCK="$SSH_AUTH_SOCK" bash gate/verify-main.sh' "$root/gate/README.md"
    for phrase in 'git commands inside the check' "root's configuration, refs or hooks" 'can read and write anything the caller can' 'every object' 'alternates' 'Only run it on a'; do
        check grep -Fq "$phrase" "$root/.ai/SECURITY.md"
    done
    for phrase in 'fresh scratch clone per attempt' 'best-effort lock per local clone' 'configuration or hooks' 'missing host commands' 'shallow roots' 'shadowing refs'; do
        check grep -Fq "$phrase" "$root/gate/CHANGELOG.md"
    done
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
case_dash_tmp() {
    mkdir -p "$tmp/elsewhere/-x"
    actual=0
    output=$(cd "$tmp/elsewhere"; TMPDIR=-x "$runner" "$root/gate/verify-main.sh" --root "$repo" -- true 2>"$tmp/err") || actual=$?
    want_green=$head
    expect 0 "green $head"
    check test -z "$(find "$tmp/elsewhere/-x" -mindepth 1 -print)"
}
case_fetch_tags() {
    git -C "$repo" tag -a remote-fixture -m 'Record a remote fixture.'
    git -C "$repo" push -q origin refs/tags/remote-fixture
    git -C "$repo" tag -d remote-fixture >/dev/null
    run
    want_green=$head
    expect 0 "green $head"
    check test -z "$(git -C "$repo" tag --list)"
}
case_lock_directory_link() {
    mkdir "$tmp/lock-target"
    ln -s "$tmp/lock-target" "$repo/.git/verify-main.lock"
    run
    want_green=$head
    expect 0 "green $head"
    check test -z "$(find "$tmp/lock-target" -mindepth 1 -print)"
}
case_lock_timezone() {
    mkdir "$tmp/bin"
    model_ps=$(command -v ps)
    cat > "$tmp/bin/ps" <<'PS'
#!/bin/sh
if [ "${TZ-}" != UTC0 ]; then printf 'another local start time\n'; exit 0; fi
exec "$MODEL_PS" "$@"
PS
    chmod +x "$tmp/bin/ps"
    sleep 30 &
    pid=$!
    start=$(TZ=UTC0 ps -o lstart= -p "$pid")
    ln -s "$pid $start" "$repo/.git/verify-main.lock"
    PATH="$tmp/bin:$PATH" MODEL_PS="$model_ps" TZ=HST10 run
    check test "$actual" -eq 0
    check test "$output" = "verify-main: already running (pid $pid)"
    check test "$(readlink "$repo/.git/verify-main.lock" 2>/dev/null || :)" = "$pid $start"
    kill "$pid"
    wait "$pid" 2>/dev/null || :
    rm -f "$repo/.git/verify-main.lock"
    # The check still receives the caller's timezone.
    # shellcheck disable=SC2016
    printf 'test "$TZ" = HST10\n' > "$tmp/hook"
    TZ=HST10 run
    want_green=$head
    expect 0 "green $head"
}
case_scratch_refs() {
    commit
    checked=$head
    git -C "$repo" reset -q --hard "$base"
    git -C "$repo" checkout -qb private-fixture
    git -C "$repo" commit -q --allow-empty -m 'Record a local fixture.'
    git -C "$repo" tag local-fixture
    printf 'git remote > "%s/remotes"\ngit remote get-url origin > "%s/url"\ngit rev-parse origin/main > "%s/checked"\ngit for-each-ref --format="%%(refname)" > "%s/refs"\n' "$tmp" "$tmp" "$tmp" "$tmp" > "$tmp/hook"
    run
    want_green=$checked
    expect 0 "green $checked"
    check test "$(cat "$tmp/remotes")" = origin
    check test "$(cat "$tmp/url")" = "$remote"
    check test "$(cat "$tmp/checked")" = "$checked"
    check test "$(cat "$tmp/refs")" = refs/remotes/origin/main
}
case_scratch_config() {
    printf 'git config core.hooksPath /nonexistent-x\ngit remote set-url origin /nonexistent-y\n' > "$tmp/hook"
    before=$(git -C "$repo" config --local --list)
    run
    want_green=$head
    expect 0 "green $head"
    check test "$(git -C "$repo" config --local --list)" = "$before"
}
case_fresh_rerun() {
    invoke --root "$repo" -- sh -c '[ -f marker ] || { touch marker; exit 1; }'
    want_frozen=$head
    expect 5 "red $head culprit $head"
}
case_follow_tags() {
    for status in 0 1; do
        setup
        git -C "$repo" config push.followTags true
        git -C "$repo" tag -a fixture -m 'Record a fixture tag.'
        printf '%s\n' "$status" > "$tmp/status"
        run
        if [ "$status" -eq 0 ]; then want_green=$head; expect 0 "green $head";
        else want_frozen=$head; expect 5 "red $head culprit $head"; fi
        check test -z "$(git -C "$repo" ls-remote --tags origin)"
    done
}
case_shadow() {
    for target in main frozen; do
        setup
        shadow="refs/heads/refs/green/$target"
        git -C "$repo" push -q origin "$head:$shadow"
        run
        expect 1 "verify-main: origin has $shadow, which shadows refs/green/$target; delete it"
        check test "$(ref "$shadow")" = "$head"
    done
}
case_cannot_start() {
    for program in no-such-command-xyz usage; do
        mkdir -p "$tmp/bin"
        real_git=$(command -v git)
        cat > "$tmp/bin/git" <<'WRAP'
#!/bin/sh
if [ "$1" = clone ]; then : > "$ATTEMPT_MARKER"; fi
exec "$REAL_GIT" "$@"
WRAP
        chmod +x "$tmp/bin/git"
        PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" ATTEMPT_MARKER="$tmp/attempt" invoke --root "$repo" -- "$program"
        expect 1 "verify-main: the check could not start ($program not found); nothing changed"
        check test ! -e "$tmp/attempt"
        check test ! -e "$tmp/runs"
    done
}
case_check_exit_codes() {
    for status in 126 127; do
        setup
        printf '%s\n' "$status" > "$tmp/status"
        run
        want_frozen=$head
        expect 5 "red $head culprit $head"
        check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = 2
    done
    setup
    printf '#!/bin/sh\nno-such-command-xyz\n' > "$repo/ci.sh"
    chmod +x "$repo/ci.sh"
    git -C "$repo" add ci.sh
    commit
    seed_green "$base"
    invoke --root "$repo" -- ./ci.sh
    want_frozen=$head
    expect 5 "red $head culprit $head"
}
case_tree_check() {
    printf '#!/bin/sh\nexit 0\n' > "$repo/ci.sh"
    chmod +x "$repo/ci.sh"
    git -C "$repo" add ci.sh
    commit
    invoke --root "$repo" -- ./ci.sh
    want_green=$head
    expect 0 "green $head"
    git -C "$repo" rm -q ci.sh
    commit
    invoke --root "$repo" -- ./ci.sh
    want_frozen=$head
    expect 5 "red $head culprit $head"
    setup
    printf '#!/bin/sh\nexit 0\n' > "$repo/ci.sh"
    git -C "$repo" add ci.sh
    commit
    seed_green "$base"
    invoke --root "$repo" -- ./ci.sh
    want_frozen=$head
    expect 5 "red $head culprit $head"
}
case_relative_tmp() {
    mkdir -p "$tmp/elsewhere/rel" "$tmp/bin"
    real_git=$(command -v git)
    cat > "$tmp/bin/git" <<'WRAP'
#!/usr/bin/env bash
if [ "$1" = clone ]; then
    for last in "$@"; do :; done
    case "$last" in /*) ;; *) printf 'relative clone path\n' >&2; exit 1 ;; esac
fi
exec "$REAL_GIT" "$@"
WRAP
    chmod +x "$tmp/bin/git"
    actual=0
    output=$(CDPATH=rel; export CDPATH; cd "$tmp/elsewhere"; PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TMPDIR=rel "$runner" "$root/gate/verify-main.sh" --root "$repo" -- true 2>"$tmp/err") || actual=$?
    want_green=$head
    expect 0 "green $head"
    check test -z "$(find "$tmp/elsewhere/rel" "$repo" -name 'verify-main.*' -print)"
}
case_inside_tmp() {
    mkdir "$repo/scratch"
    actual=0
    output=$(TMPDIR="$repo/scratch" "$runner" "$root/gate/verify-main.sh" --root "$repo" -- true 2>"$tmp/err") || actual=$?
    expect 1 'verify-main: the scratch directory must be outside the root'
    check test -z "$(find "$repo/scratch" -mindepth 1 -print)"
}
case_check_environment() {
    for value in C POSIX; do
        setup
        # shellcheck disable=SC2016
        printf 'printf "%%s %%s\\n" "${LC_ALL-unset}" "${GIT_NO_REPLACE_OBJECTS-unset}" > "%s/environment"\n' "$tmp" > "$tmp/hook"
        LC_ALL="$value" GIT_NO_REPLACE_OBJECTS=0 GIT_NAMESPACE=fixture run
        want_green=$head
        expect 0 "green $head"
        check test "$(cat "$tmp/environment")" = "$value 0"
    done
    setup
    # shellcheck disable=SC2016
    printf 'test -z "${LC_ALL+x}${GIT_NO_REPLACE_OBJECTS+x}"\n' > "$tmp/hook"
    (unset LC_ALL GIT_NO_REPLACE_OBJECTS; run; want_green=$head; expect 0 "green $head")
}
case_root_tripwire() {
    for mutation in config worktree hook deleted-hook symlink-hook retargeted-hook; do
        for attempt in 1 2; do
            setup
            printf '#!/bin/sh\ntouch "%s/payload-ran"\n' "$tmp" > "$tmp/payload"
            chmod +x "$tmp/payload"
            if [ "$mutation" = deleted-hook ]; then
                printf 'fixture\n' > "$repo/.git/hooks/fixture"
            fi
            if [ "$mutation" = retargeted-hook ]; then
                cp "$tmp/payload" "$tmp/original-payload"
                ln -s "$tmp/original-payload" "$repo/.git/hooks/post-checkout"
            fi
            before=$(cat "$repo/.git/config")
            cat > "$tmp/check-tripwire" <<'CHECK'
#!/bin/sh
printf 'ran\n' >> "$1/attempts"
[ "$(wc -l < "$1/attempts" | tr -d ' ')" = "$4" ] || exit 1
case "$3" in
    config) git -C "$2" config core.fsmonitor "$1/payload" ;;
    worktree) printf 'fixture\n' > "$2/.git/config.worktree" ;;
    hook) printf 'fixture\n' > "$2/.git/hooks/new-hook" ;;
    deleted-hook) rm "$2/.git/hooks/fixture" ;;
    symlink-hook) ln -s "$1/payload" "$2/.git/hooks/post-checkout" ;;
    retargeted-hook)
        rm "$2/.git/hooks/post-checkout"
        ln -s "$1/payload" "$2/.git/hooks/post-checkout"
        ;;
esac
: > "$1/changed"
CHECK
            mkdir "$tmp/bin"
            real_git=$(command -v git)
            cat > "$tmp/bin/git" <<'WRAP'
#!/bin/sh
if [ -e "$TRIPWIRE_CHANGED" ]; then : > "$LATE_GIT"; fi
exec "$REAL_GIT" "$@"
WRAP
            chmod +x "$tmp/bin/git"
            PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TRIPWIRE_CHANGED="$tmp/changed" LATE_GIT="$tmp/late-git" invoke --root "$repo" -- sh "$tmp/check-tripwire" "$tmp" "$repo" "$mutation" "$attempt"
            check test "$actual" -eq 1
            check test "$output" = "verify-main: the check changed $repo/.git's git configuration or hooks; nothing changed, inspect it"
            check test ! -e "$tmp/payload-ran"
            check test ! -e "$tmp/late-git"
            check test "$(wc -l < "$tmp/attempts" | tr -d ' ')" = "$attempt"
            # Inspect refs only after restoring the fixture's safe configuration.
            printf '%s\n' "$before" > "$repo/.git/config"
            expect 1 "verify-main: the check changed $repo/.git's git configuration or hooks; nothing changed, inspect it"
        done
    done
}
case_readonly_red() {
    for target in nested root; do
        setup
        if [ "$target" = nested ]; then
            invoke --root "$repo" -- sh -c 'mkdir -p fx/ro && touch fx/ro/f && chmod 555 fx/ro; exit 1'
        else
            invoke --root "$repo" -- sh -c 'chmod 555 .; exit 1'
        fi
        want_frozen=$head
        expect 5 "red $head culprit $head"
        # Restore fixture permissions even on red, so the suite can clean up.
        chmod -R u+w "$tmp/scratch"
    done
}
case_cleanup_status() {
    for result in 0 5 6 75; do
        setup
        mkdir "$tmp/bin"
        real_rm=$(command -v rm)
        cat > "$tmp/bin/rm" <<'WRAP'
#!/bin/sh
case "${2##*/}" in verify-main.lock) ;; verify-main.*) exit 1 ;; esac
exec "$REAL_RM" "$@"
WRAP
        chmod +x "$tmp/bin/rm"
        status=1
        case "$result" in
            0) status=0; want_green=$head; last="green $head" ;;
            5) want_frozen=$head; last="red $head culprit $head" ;;
            6) commit; commit; want_frozen=$head; last="red $head across 3 commits from $base to $head" ;;
            75) status=75; last='verify-main: the check cannot run on this host (exit 75); nothing changed' ;;
        esac
        # shellcheck disable=SC2016
        PATH="$tmp/bin:$PATH" REAL_RM="$real_rm" invoke --root "$repo" -- sh -c 'if [ "$1" = 0 ]; then chmod 555 .; fi; exit "$1"' fixture "$status"
        check test "$actual" -eq "$result"
        check test "${output##*$'\n'}" = "$last"
        check grep -Fq 'verify-main: warning: cannot remove scratch directory' "$tmp/err"
        chmod -R u+w "$tmp/scratch"
        "$real_rm" -rf "$tmp/scratch/"*
        expect "$result" "$last"
    done
}
case_shallow() {
    git clone -q --depth=1 "file://$remote" "$tmp/shallow"
    repo="$tmp/shallow"
    run
    expect 1 'verify-main: a shallow clone cannot name a culprit; fetch the full history'
}
case_missing_tools() {
    saved_path=$PATH
    mkdir "$tmp/tools"
    real_git=$(command -v git)
    real_ps=$(command -v ps)
    real_readlink=$(command -v readlink)
    real_mktemp=$(command -v mktemp)
    for missing in ps readlink mktemp; do
        for tool in git ps readlink mktemp; do
            rm -f "$tmp/tools/$tool"
            [ "$tool" = "$missing" ] && continue
            case "$tool" in git) executable=$real_git ;; ps) executable=$real_ps ;; readlink) executable=$real_readlink ;; mktemp) executable=$real_mktemp ;; esac
            ln -s "$executable" "$tmp/tools/$tool"
        done
        PATH="$tmp/tools" invoke --root "$repo" -- true
        expect 75 "verify-main: $missing is missing"
    done
}
case_rerun_unavailable_empty() {
    printf '1\n75\n' > "$tmp/status"
    run
    expect 75 'verify-main: the check cannot run on this host (exit 75); nothing changed'
    check test "$(wc -l < "$tmp/runs" | tr -d ' ')" = 2
}
case_red_covered() {
    checked=$head
    commit
    descendant=$head
    git -C "$repo" push -q --force origin "$checked:main"
    head=$checked
    printf '%s\n' "$head" > "$tmp/head"
    printf 'git -C "%s" push -q origin "%s:refs/green/main"\n' "$repo" "$descendant" > "$tmp/hook"
    printf '1\n' > "$tmp/status"
    run
    want_green=$descendant
    expect 1 'verify-main: green moved during the check'
}
case_off_main_advance() {
    seed_green "$head"
    other=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
    seed_frozen "$other"
    commit
    run
    want_green=$head
    expect 0 "green $head"
}
case_stolen_lock() {
    printf 'rm "%s/.git/verify-main.lock"\nln -s "99999999 another start" "%s/.git/verify-main.lock"\n' "$repo" "$repo" > "$tmp/hook"
    run
    check test "$(readlink "$repo/.git/verify-main.lock")" = '99999999 another start'
    rm -f "$repo/.git/verify-main.lock"
    want_green=$head
    expect 0 "green $head"
}
case_check_stdout() {
    printf 'printf "fixture stdout\\n"\n' > "$tmp/hook"
    run
    want_green=$head
    expect 0 "green $head"
    check test "$output" = "green $head"
    check grep -Fxq 'fixture stdout' "$tmp/err"
}
case_merge_suspects() {
    seed_green "$head"
    git -C "$repo" checkout -qb side
    printf 'fixture\n' > "$repo/side-file"
    git -C "$repo" add side-file
    commit
    side=$head
    git -C "$repo" checkout -q main
    # Restore main: commit() pushed the side commit to remote main.
    git -C "$repo" push -q --force origin HEAD:main
    commit
    first=$head
    git -C "$repo" merge -q --no-ff -m 'Merge a fixture.' "$side"
    head=$(git -C "$repo" rev-parse HEAD)
    git -C "$repo" push -q origin HEAD:main
    printf '%s\n' "$head" > "$tmp/head"
    printf '1\n' > "$tmp/status"
    run
    want_frozen=$head
    expect 6 "red $head across 2 commits from $first to $head"
}
case_remote_green_object() {
    git clone -q "$remote" "$tmp/writer"
    other=$(printf 'Record another fixture.\n' | git -C "$tmp/writer" -c user.name=Fixture -c user.email=fixture@example.invalid commit-tree 'HEAD^{tree}')
    git -C "$tmp/writer" push -q origin "$other:refs/green/main"
    check test "$(git -C "$repo" cat-file -t "$other" 2>/dev/null || :)" = ''
    want_green=$other
    run
    short=$(git -C "$repo" rev-parse --short "$other")
    expect 1 "verify-main: green $short is not on origin/main; nothing changed"
    check git -C "$repo" cat-file -e "$other^{commit}"
}
case_lease_races() {
    for mode in green red red-same red-fast; do
        setup
        seed_green "$head"
        commit
        race_head=$head
        seed_frozen "$head"
        commit
        if [ "$mode" != green ]; then printf '1\n' > "$tmp/status"; fi
        mkdir "$tmp/bin"
        real_git=$(command -v git)
        cat > "$tmp/bin/git" <<'WRAP'
#!/usr/bin/env bash
set -eu
case " $* " in
    *' push '*)
        if [ ! -e "$RACE_MARKER" ]; then
            : > "$RACE_MARKER"
            "$REAL_GIT" -C "$TEST_REPO" push -q --force origin "$RACE_OID:$RACE_REF"
        fi ;;
esac
exec "$REAL_GIT" "$@"
WRAP
        chmod +x "$tmp/bin/git"
        if [ "$mode" = green ]; then
            race_ref=refs/green/main
            reason='green moved during the check'
        else
            race_head=$(printf 'Record another fixture.\n' | git -C "$repo" commit-tree 'HEAD^{tree}')
            if [ "$mode" = red-same ]; then race_head=$head; fi
            if [ "$mode" = red-fast ]; then race_head=$base; fi
            race_ref=refs/green/frozen
            reason='the freeze moved during the check'
        fi
        PATH="$tmp/bin:$PATH" REAL_GIT="$real_git" TEST_REPO="$repo" RACE_OID="$race_head" RACE_REF="$race_ref" RACE_MARKER="$tmp/raced" run
        if [ "$mode" = green ]; then want_green=$race_head; else want_frozen=$race_head; fi
        expect 1 "verify-main: $reason"
    done
}
case_equal_freeze_advance() {
    seed_green "$head"
    commit
    seed_frozen "$head"
    run
    want_green=$head
    expect 0 "green $head"
    check test "$output" = "green $head"
    run
    short=$(git -C "$repo" rev-parse --short "$head")
    expect 1 "verify-main: refs/green/main and refs/green/frozen both name $short; delete one and run this again"
}
case_repeat_red() {
    seed_frozen "$head"
    printf '1\n' > "$tmp/status"
    run
    expect 5 "red $head culprit $head"
}
cases='repeat_red equal_freeze_advance rerun_unavailable_empty red_covered off_main_advance stolen_lock check_stdout merge_suspects remote_green_object lease_races missing_tools cleanup_status shallow green_push_failure lock_identity bad_lock check_environment relative_tmp inside_tmp cannot_start shadow follow_tags fresh_rerun noncommit_ref docs signals red_push_failure push_races failures environment scratch_config live_lock stale_lock freeze_race green_race covered main_moved rewritten unavailable fixed flaky no_green_red red_many red_one advance current_off_main current_ancestor_frozen current_frozen current first_green usage'
cases="ps_failure deleted_freeze tree_check check_exit_codes root_tripwire readonly_red dash_tmp fetch_tags lock_directory_link lock_timezone scratch_refs $cases"
selected=0
for name in $cases; do
    if [ "$#" -gt 0 ] && [ "$1" != "$name" ]; then continue; fi
    selected=$((selected + 1))
    declare -F "case_$name" >/dev/null || { printf 'missing case: %s\n' "$name"; exit 1; }
    setup
    "case_$name"
    printf '%s: checked\n' "$name"
done
[ "$selected" -gt 0 ] || { printf 'no case selected\n'; exit 1; }
[ "$failed" -eq 0 ] || exit 1
printf 'gate_verify_main_test: all passed (%s)\n' "$BASH_VERSION"
