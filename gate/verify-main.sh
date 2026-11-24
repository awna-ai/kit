#!/usr/bin/env bash
set -euo pipefail
caller_lc_set=${LC_ALL+x} caller_lc=${LC_ALL-}
caller_replace_set=${GIT_NO_REPLACE_OBJECTS+x} caller_replace=${GIT_NO_REPLACE_OBJECTS-}
export LC_ALL=C GIT_NO_REPLACE_OBJECTS=1
unset GREP_OPTIONS GREP_COLOR GREP_COLORS
unset GIT_NAMESPACE GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR
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
for tool in git ps readlink mktemp; do
    if ! command -v "$tool" >/dev/null 2>&1; then
        printf 'verify-main: %s is missing\n' "$tool"
        exit 75
    fi
done
root=$(CDPATH='' cd -- "$root" 2>/dev/null && pwd -P) || usage
top=$(git -C "$root" rev-parse --show-toplevel 2>/dev/null) || usage
top=$(CDPATH='' cd -- "$top" && pwd -P) || usage
[ "$root" = "$top" ] || usage
remote_url=$(git -C "$root" remote get-url -- "$remote" 2>/dev/null) || usage
shallow=$(git -C "$root" rev-parse --is-shallow-repository) || fail 'cannot inspect history'
[ "$shallow" != true ] || fail 'a shallow clone cannot name a culprit; fetch the full history'
common=$(git -C "$root" rev-parse --git-common-dir) || fail 'cannot find the common directory'
case "$common" in /*) ;; *) common="$root/$common" ;; esac
common=$(CDPATH='' cd -- "$common" && pwd -P) || fail 'cannot find the common directory'
lock="$common/verify-main.lock"
working='' owned=0
already_running() { printf 'verify-main: already running (pid %s)\n' "$1"; exit 0; }
start=$(TZ=UTC0 ps -o lstart= -p "$$") || { printf 'verify-main: cannot inspect this process\n'; exit 75; }
identity="$$ $start"
if [ -e "$lock" ] && [ ! -L "$lock" ]; then fail "$lock is not a lock; remove it"; fi
if ! ln -sn "$identity" "$lock" 2>/dev/null; then
    [ -L "$lock" ] || fail "$lock is not a lock; remove it"
    holder=$(readlink "$lock") || fail 'cannot read the lock'
    pid=${holder%% *}
    recorded=${holder#* }
    if [[ "$pid" =~ ^[1-9][0-9]*$ ]] && [ "$pid" != "$$" ] && [ "$recorded" != "$holder" ]; then
        current=$(TZ=UTC0 ps -o lstart= -p "$pid" 2>/dev/null) || current=''
        if [ -n "$current" ] && [ "$current" = "$recorded" ]; then already_running "$pid"; fi
    fi
    if [ "$(readlink "$lock")" = "$holder" ]; then rm -f "$lock" || fail 'cannot remove a stale lock'; fi
    if ! ln -sn "$identity" "$lock" 2>/dev/null; then
        [ -L "$lock" ] || fail "$lock is not a lock; remove it"
        holder=$(readlink "$lock") || fail 'cannot read the lock'
        already_running "${holder%% *}"
    fi
fi
owned=1
cleanup() {
    local status=$?
    if [ -n "$working" ]; then
        chmod -R u+w "$working" 2>/dev/null || :
        rm -rf "$working" || printf 'verify-main: warning: cannot remove scratch directory %s\n' "$working" >&2
    fi
    if [ "$owned" -eq 1 ] && [ "$(readlink "$lock" 2>/dev/null)" = "$identity" ]; then
        rm -f "$lock" || printf 'verify-main: warning: cannot remove lock %s\n' "$lock" >&2
    fi
    exit "$status"
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP
fetch_main() {
    git -C "$root" fetch --quiet --no-tags --no-recurse-submodules -- "$remote" "+refs/heads/main:refs/remotes/$remote/main" || fail 'cannot fetch main'
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
            *)
                case "$ref" in
                    */refs/green/main) shadow=refs/green/main ;;
                    */refs/green/frozen) shadow=refs/green/frozen ;;
                    *) fail 'invalid green ref' ;;
                esac
                fail "$remote has $ref, which shadows $shadow; delete it" ;;
        esac
        if ! git -C "$root" cat-file -e "$oid" 2>/dev/null; then
            git -C "$root" fetch --quiet --no-tags --no-recurse-submodules -- "$remote" "$oid" || fail 'cannot fetch green ref'
        fi
        git -C "$root" cat-file -e "$oid^{commit}" 2>/dev/null || fail 'green ref is not a commit'
    done <<< "$refs"
}
push_failure() {
    # Only a successful atomic push may have cleared the freeze itself.
    local push_result=${1:-1}
    read_refs
    if [ "$green" = "$old_green" ] && [ "$frozen" = "$old_frozen" ]; then
        fail 'the remote refused the push; nothing changed'
    fi
    if [ "$frozen" != "$old_frozen" ] && ! {
        [ "$status" -eq 0 ] && [ -z "$frozen" ] && [ -n "$green" ] &&
            git -C "$root" merge-base --is-ancestor "$head" "$green"
    }; then
        fail 'the freeze moved during the check'
    fi
    if [ "$status" -eq 0 ] && [ -n "$green" ] && git -C "$root" merge-base --is-ancestor "$head" "$green"; then
        if [ "$clear" -eq 1 ] && [ "$push_result" -eq 0 ] && [ -z "$frozen" ]; then
            printf 'releases unfrozen (freeze at %s cleared)\n' "$old_frozen"
        fi
        printf 'covered: green is %s\n' "$green"
        exit 0
    fi
    fail 'green moved during the check'
}
fetch_main
head=$main
read_refs
if [ -n "$green" ] && [ "$green" = "$frozen" ]; then
    short=$(git -C "$root" rev-parse --short "$green")
    fail "refs/green/main and refs/green/frozen both name $short; delete one and run this again"
fi
if [ "$green" = "$head" ]; then
    if [ -n "$frozen" ] && [ "$frozen" != "$head" ] && git -C "$root" merge-base --is-ancestor "$frozen" "$head"; then
        old_green=$green old_frozen=$frozen status=0 clear=1
        git -C "$root" push --no-follow-tags --no-recurse-submodules --porcelain "--force-with-lease=refs/green/frozen:$frozen" -- "$remote" :refs/green/frozen >&2 || push_failure
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
case "$1" in
    /*) tree_command=0 ;;
    */*) tree_command=1 ;;
    *) tree_command=0 ;;
esac
if [ "$tree_command" -eq 0 ]; then
    check_program=$(type -P -- "$1") || check_program=''
    [ -n "$check_program" ] || fail "the check could not start ($1 not found); nothing changed"
    shift
    set -- "$check_program" "$@"
fi
working=$(mktemp -d -- "${TMPDIR:-/tmp}/verify-main.XXXXXX") || fail 'cannot create a scratch directory'
working=$(CDPATH='' cd -- "$working" && pwd -P) || fail 'cannot resolve the scratch directory'
case "$working/" in "$root/"*) fail 'the scratch directory must be outside the root' ;; esac
configuration_fingerprint() {
    local file entries
    for file in "$common/config" "$common/config.worktree"; do
        if [ -e "$file" ]; then
            cksum "$file" || return 1
        else
            printf 'missing %s\n' "$file"
        fi
    done
    if [ -d "$common/hooks" ]; then
        entries=$(find "$common/hooks" -print | sort) || return 1
        printf '%s\n' "$entries"
        while IFS= read -r file; do
            if [ -L "$file" ]; then
                readlink "$file" || return 1
            elif [ -f "$file" ]; then
                cksum "$file" || return 1
            fi
        done <<< "$entries"
    else
        printf 'missing %s/hooks\n' "$common"
    fi
}
configuration=$(configuration_fingerprint) || fail 'cannot inspect git configuration or hooks'
check_configuration() {
    local current_configuration
    current_configuration=$(configuration_fingerprint) || fail 'cannot inspect git configuration or hooks'
    [ "$current_configuration" = "$configuration" ] ||
        fail "the check changed $common's git configuration or hooks; nothing changed, inspect it"
}
attempt=0
fresh_clone() {
    attempt=$((attempt + 1))
    tree="$working/wt$attempt"
    git clone --quiet --shared --no-tags --no-checkout -- "$root" "$tree" || fail 'cannot create a scratch clone'
    git -C "$tree" remote remove origin || fail 'cannot remove the scratch remote'
    local copied_branch
    copied_branch=$(git -C "$tree" symbolic-ref HEAD) || fail 'cannot read the scratch branch'
    git -C "$tree" checkout --quiet --detach "$head" || fail 'cannot check out main'
    git -C "$tree" update-ref -d "$copied_branch" || fail 'cannot remove the scratch branch'
    git -C "$tree" remote add origin "$remote_url" || fail 'cannot set the scratch remote'
    git -C "$tree" update-ref refs/remotes/origin/main "$head" || fail 'cannot set the scratch main'
}
fresh_clone
unavailable() {
    if [ "$status" -eq 75 ]; then
        printf 'verify-main: the check cannot run on this host (exit 75); nothing changed\n'
        exit 75
    fi
}
run_check() (
    if [ "$caller_lc_set" = x ]; then export LC_ALL="$caller_lc"; else unset LC_ALL; fi
    if [ "$caller_replace_set" = x ]; then export GIT_NO_REPLACE_OBJECTS="$caller_replace"; else unset GIT_NO_REPLACE_OBJECTS; fi
    CDPATH='' cd -- "$tree" || exit 1
    command -- "$@" </dev/null
)
status=0
run_check "$@" >&2 || status=$?
check_configuration
unavailable
if [ "$status" -ne 0 ]; then
    fresh_clone
    run_check "$@" >&2 && status=0 || status=$?
    check_configuration
    unavailable
    if [ "$status" -eq 0 ]; then printf 'verify-main: the first run was red; the rerun passed\n'; fi
fi
fetch_main
git -C "$root" merge-base --is-ancestor "$head" "$main" || fail 'main moved off the checked head; nothing changed'
old_green=$green old_frozen=$frozen clear=0
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
    push_status=0
    pushed=$(git -C "$root" push --no-follow-tags --no-recurse-submodules --porcelain "--force-with-lease=refs/green/frozen:$frozen" -- "$remote" "$head:refs/green/frozen" 2>&1) || push_status=$?
    printf '%s\n' "$pushed" >&2
    if [ "$push_status" -ne 0 ] || [[ "$pushed" = =* ]] || [[ "$pushed" = *$'\n='* ]]; then
        # An unchanged, existing freeze at this head is a legitimate no-op.
        if [ "$push_status" -eq 0 ] && [ "$old_frozen" = "$head" ]; then
            read_refs
            if [ "$green" != "$old_green" ] || [ "$frozen" != "$old_frozen" ]; then
                printf '%s\n' "$red"
                push_failure
            fi
        else
            printf '%s\n' "$red"
            push_failure
        fi
    fi
    printf 'verify-main: releases are frozen. %s\n' "$action"
    printf '%s\n' "$red"
    exit "$result"
fi
push_args=(--atomic --no-follow-tags --no-recurse-submodules --porcelain "--force-with-lease=refs/green/main:$green")
clear=0
if [ -n "$frozen" ] && [ "$frozen" != "$head" ] && git -C "$root" merge-base --is-ancestor "$frozen" "$head"; then
    clear=1
    push_args+=("--force-with-lease=refs/green/frozen:$frozen")
fi
push_args+=(-- "$remote" "$head:refs/green/main")
if [ "$clear" -eq 1 ]; then push_args+=(:refs/green/frozen); fi
push_status=0
pushed=$(git -C "$root" push "${push_args[@]}" 2>&1) || push_status=$?
printf '%s\n' "$pushed" >&2
if [ "$push_status" -ne 0 ] || [[ "$pushed" = =* ]] || [[ "$pushed" = *$'\n='* ]]; then
    push_failure "$push_status"
fi
if [ "$clear" -eq 1 ]; then printf 'releases unfrozen (freeze at %s cleared)\n' "$frozen"; fi
printf 'green %s\n' "$head"
