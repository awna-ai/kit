#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C GIT_NO_REPLACE_OBJECTS=1
unset GREP_OPTIONS GREP_COLOR GREP_COLORS
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR
usage() {
    printf '%s\n' 'usage: verify-main.sh --root <repo top> [--remote <name>] -- <check command> [<arg>...]' >&2
    exit 2
}
fail() { printf 'verify-main: %s\n' "$1"; exit 1; }
root='' remote=origin root_seen=0 remote_seen=0 separator=0
while [ "$#" -gt 0 ]; do
    case "$1" in
        --root)
            [ "$root_seen" -eq 0 ] && [ "$#" -ge 2 ] && [ -n "$2" ] || usage
            root=$2 root_seen=1; shift 2 ;;
        --remote)
            [ "$remote_seen" -eq 0 ] && [ "$#" -ge 2 ] && [ -n "$2" ] || usage
            remote=$2 remote_seen=1; shift 2 ;;
        --) separator=1; shift; break ;;
        *) usage ;;
    esac
done
[ "$root_seen" -eq 1 ] && [ "$separator" -eq 1 ] && [ "$#" -gt 0 ] || usage
case "$remote" in -*) usage ;; esac
if ! command -v git >/dev/null 2>&1; then printf 'verify-main: git is missing\n'; exit 75; fi
root=$(CDPATH='' cd -- "$root" 2>/dev/null && pwd -P) || usage
top=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null) || usage
top=$(CDPATH='' cd -- "$top" && pwd -P) || usage
[ "$root" = "$top" ] || usage
git -C "$root" remote get-url -- "$remote" >/dev/null 2>&1 || usage
common=$(git -C "$root" rev-parse --git-common-dir) || fail 'cannot find the common directory'
case "$common" in /*) ;; *) common="$root/$common" ;; esac
common=$(CDPATH='' cd -- "$common" && pwd -P) || fail 'cannot find the common directory'
lock="$common/verify-main.lock"
working='' owned=0
already_running() { printf 'verify-main: already running (pid %s)\n' "$1"; exit 0; }
if ! ln -s "$$" "$lock" 2>/dev/null; then
    holder=$(readlink "$lock") || fail 'cannot read the lock'
    if [[ "$holder" =~ ^[1-9][0-9]*$ ]] && kill -0 "$holder" 2>/dev/null; then
        command_line=$(ps -p "$holder" -o args=) || fail 'cannot inspect the lock owner'
        case "$command_line" in *verify-main.sh*) already_running "$holder" ;; esac
    fi
    if [ "$(readlink "$lock")" = "$holder" ]; then rm -f "$lock" || fail 'cannot remove a stale lock'; fi
    if ! ln -s "$$" "$lock" 2>/dev/null; then
        holder=$(readlink "$lock") || fail 'cannot read the lock'
        already_running "$holder"
    fi
fi
owned=1
cleanup() {
    local status=$?
    if [ -n "$working" ]; then
        git -C "$root" worktree remove --force "$working/wt" >/dev/null 2>&1 || :
        rm -rf "$working" || status=1
    fi
    if [ "$owned" -eq 1 ] && [ "$(readlink "$lock" 2>/dev/null)" = "$$" ]; then
        rm -f "$lock" || status=1
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
fetch_main() {
    git -C "$root" fetch --quiet -- "$remote" "+refs/heads/main:refs/remotes/$remote/main" || fail 'cannot fetch main'
    main=$(git -C "$root" rev-parse --verify "refs/remotes/$remote/main^{commit}") || fail 'cannot read main'
}
read_refs() {
    local refs oid ref extra
    refs=$(git -C "$root" ls-remote -- "$remote" refs/green/main refs/green/frozen) || fail 'cannot read green refs'
    green='' frozen=''
    while read -r oid ref extra; do
        [ -n "$oid" ] || continue
        [[ "$oid" =~ ^([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$ ]] && [ -z "$extra" ] || fail 'invalid green ref'
        case "$ref" in
            refs/green/main) green=$oid ;;
            refs/green/frozen) frozen=$oid ;;
            *) fail 'invalid green ref' ;;
        esac
        if ! git -C "$root" cat-file -e "$oid" 2>/dev/null; then
            git -C "$root" fetch --quiet -- "$remote" "$oid" || fail 'cannot fetch green ref'
        fi
        git -C "$root" cat-file -e "$oid^{commit}" 2>/dev/null || fail 'green ref is not a commit'
    done <<< "$refs"
}
# Only reap private scratch worktrees with the exact generated spelling.
worktrees=$(git -C "$root" -c core.quotePath=false worktree list --porcelain) || fail 'cannot list worktrees'
while IFS= read -r entry; do
    case "$entry" in
        'worktree '*)
            path=${entry#worktree }
            if [[ "$path" =~ /verify-main\.[A-Za-z0-9]{6}/wt$ ]]; then
                parent=${path%/wt}
                if [ -d "$parent" ] && [ ! -L "$parent" ] && [ -O "$parent" ]; then
                    git -C "$root" worktree remove --force "$path" >/dev/null 2>&1 || fail 'cannot remove a leftover worktree'
                    rm -rf "$parent" || fail 'cannot remove a leftover directory'
                fi
            fi
            ;;
    esac
done <<< "$worktrees"
git -C "$root" worktree prune || fail 'cannot prune worktrees'
fetch_main
head=$main
read_refs
if [ "$green" = "$head" ]; then
    if [ -n "$frozen" ] && git -C "$root" merge-base --is-ancestor "$frozen" "$head"; then
        git -C "$root" push --porcelain "--force-with-lease=refs/green/frozen:$frozen" -- "$remote" :refs/green/frozen >&2 || fail 'cannot clear the freeze'
        printf 'releases unfrozen (freeze at %s cleared)\n' "$frozen"
    elif [ -n "$frozen" ]; then
        short=$(git -C "$root" rev-parse --short "$frozen")
        printf 'verify-main: releases are still frozen at %s, which is not on %s/main; delete refs/green/frozen by hand\n' "$short" "$remote"
    fi
    printf 'green is current: %s\n' "$head"
    exit 0
fi
if [ -n "$green" ] && ! git -C "$root" merge-base --is-ancestor "$green" "$head"; then
    short=$(git -C "$root" rev-parse --short "$green")
    fail "green $short is not on $remote/main; nothing changed"
fi
working=$(mktemp -d "${TMPDIR:-/tmp}/verify-main.XXXXXX") || fail 'cannot create a scratch directory'
git -C "$root" worktree add --quiet --detach "$working/wt" "$head" || fail 'cannot create a scratch worktree'
unavailable() {
    if [ "$status" -eq 75 ]; then
        printf 'verify-main: the check cannot run on this host (exit 75); nothing changed\n'
        exit 75
    fi
}
status=0
(CDPATH='' cd -- "$working/wt" && "$@" </dev/null) >&2 || status=$?
unavailable
if [ "$status" -ne 0 ]; then
    (CDPATH='' cd -- "$working/wt" && "$@" </dev/null) >&2 && status=0 || status=$?
    unavailable
    if [ "$status" -eq 0 ]; then printf 'verify-main: the first run was red; the rerun passed\n'; fi
fi
fetch_main
git -C "$root" merge-base --is-ancestor "$head" "$main" || fail 'main moved off the checked head; nothing changed'
old_green=$green old_frozen=$frozen
read_refs
[ "$frozen" = "$old_frozen" ] || fail 'the freeze moved during the check'
if [ "$green" != "$old_green" ]; then
    if [ "$status" -eq 0 ] && [ -n "$green" ] && git -C "$root" merge-base --is-ancestor "$head" "$green"; then
        printf 'covered: green is %s\n' "$green"
        exit 0
    fi
    fail 'green moved during the check'
fi
if [ "$status" -ne 0 ]; then
    suspects=$(git -C "$root" rev-list --first-parent --reverse "${green:+$green..}$head") || fail 'cannot list suspects'
    count=0 first=''
    while IFS= read -r suspect; do
        [ -n "$suspect" ] || continue
        count=$((count + 1))
        if [ -z "$first" ]; then first=$suspect; fi
    done <<< "$suspects"
    short=$(git -C "$root" rev-parse --short "$head")
    if [ "$count" -eq 1 ]; then
        red="red $head culprit $first"
        action="Revert or fix $short, then run this again."
        result=5
    else
        red="red $head across $count commits from $first to $head"
        action="Revert or fix the commits from $first to $head, then run this again."
        result=6
    fi
    if ! git -C "$root" push --porcelain "--force-with-lease=refs/green/frozen:$frozen" -- "$remote" "$head:refs/green/frozen" >&2; then
        printf '%s\n' "$red"
        fail "$short is red, and the freeze could not be pushed"
    fi
    printf 'verify-main: releases are frozen. %s\n' "$action"
    printf '%s\n' "$red"
    exit "$result"
fi
push_args=(--atomic --porcelain "--force-with-lease=refs/green/main:$green")
clear=0
if [ -n "$frozen" ] && git -C "$root" merge-base --is-ancestor "$frozen" "$head"; then
    clear=1
    push_args+=("--force-with-lease=refs/green/frozen:$frozen")
fi
push_args+=(-- "$remote" "$head:refs/green/main")
if [ "$clear" -eq 1 ]; then push_args+=(:refs/green/frozen); fi
push_status=0
pushed=$(git -C "$root" push "${push_args[@]}" 2>&1) || push_status=$?
if [ "$push_status" -ne 0 ] || [[ "$pushed" = =* ]] || [[ "$pushed" = *$'\n='* ]]; then
    old_frozen=$frozen
    read_refs
    if [ -n "$green" ] && git -C "$root" merge-base --is-ancestor "$head" "$green"; then
        printf 'covered: green is %s\n' "$green"
        exit 0
    fi
    [ "$frozen" = "$old_frozen" ] || fail 'the freeze moved during the check'
    fail 'green moved during the check'
fi
if [ "$clear" -eq 1 ]; then printf 'releases unfrozen (freeze at %s cleared)\n' "$frozen"; fi
printf 'green %s\n' "$head"
