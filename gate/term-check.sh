#!/usr/bin/env bash
set -euo pipefail
# Read the exported setting before any scanner variable can replace its value.
strict_value=$(
    while [ "$#" -ge 2 ]; do
        if [ "$1" = --strict-var ]; then
            if [[ "$2" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]] &&
                [[ "$(declare -p "$2" 2> /dev/null)" =~ ^declare\ -[A-Za-z]*x ]]; then
                printf '%s\n' "${!2}"
            fi
            break
        fi
        shift 2
    done
)
export LC_ALL=C
export GIT_NO_REPLACE_OBJECTS=1
unset GREP_OPTIONS GREP_COLOR GREP_COLORS
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR

usage() {
    printf '%s\n' 'usage: term-check.sh --terms <file> [--strict-var <VAR>] (--root <dir> | --message <file>)' >&2
    exit 2
}
root='' strict_var='' terms='' message='' root_seen=0 strict_seen=0
terms_seen=0 message_seen=0
while [ "$#" -gt 0 ]; do
    [ "$#" -ge 2 ] || usage
    case "$1" in
        --root)
            [ "$root_seen" -eq 0 ] || usage
            root_seen=1
            root=$2
            ;;
        --strict-var)
            [ "$strict_seen" -eq 0 ] || usage
            strict_seen=1
            strict_var=$2
            ;;
        --terms)
            [ "$terms_seen" -eq 0 ] || usage
            terms_seen=1
            terms=$2
            ;;
        --message)
            [ "$message_seen" -eq 0 ] || usage
            message_seen=1
            message=$2
            ;;
        *) usage ;;
    esac
    shift 2
done
if [ "$strict_seen" -eq 1 ] && [[ ! "$strict_var" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then usage; fi
strict=0
if [ -n "$strict_var" ] && [ "$strict_value" = 1 ]; then strict=1; fi
[ "$terms_seen" -eq 1 ] && [ -n "$terms" ] && [ "$((root_seen + message_seen))" -eq 1 ] || usage
if [ "$root_seen" -eq 1 ]; then
    [ -d "$root" ] || usage
    root=$(CDPATH='' cd -- "$root" && pwd -P) || usage
    if ! top=$(git -C "$root" rev-parse --show-toplevel 2> /dev/null) ||
        ! top=$(CDPATH='' cd -- "$top" && pwd -P) || [ "$top" != "$root" ]; then
        printf 'term-check: %s is not the top of a git repository\n' "$root" >&2
        exit 2
    fi
fi
if [ ! -f "$terms" ] || [ ! -r "$terms" ]; then
    if [ "$strict" -eq 1 ]; then
        printf 'term-check: no term list at %s\n' "$terms" >&2
        exit 75
    fi
    printf 'term-check: no term list at %s, skipped\n' "$terms"
    exit 0
fi

list=$(mktemp)
paths=''
scan=$(mktemp)
normalized=$(mktemp)
objects=''
history=''
messages=''
trap 'rm -f "$list" "$paths" "$scan" "$objects" "$history" "$normalized" "$messages"' EXIT
sed -e 's/\r$//' -e 's/[[:space:]]*$//' -e '/^[[:space:]]*#/d' -e '/^$/d' "$terms" > "$list"
if [ ! -s "$list" ]; then
    if [ "$strict" -eq 1 ]; then
        printf 'term-check: the term list at %s is empty\n' "$terms" >&2
        exit 75
    fi
    printf 'term-check: the term list at %s is empty, skipped\n' "$terms"
    exit 0
fi

normalize() {
    sed -E -e 's/_/ /g; s/([a-z0-9])([A-Z])/\1 \2/g; s/([A-Z])([A-Z][a-z])/\1 \2/g; s/([A-Za-z])([0-9])/\1 \2/g; s/([0-9])([A-Za-z])/\1 \2/g' -- "$@"
}

# hits <file>: 0 on a hit, 1 on none; any grep error exits 2.
hits() {
    local status
    if grep -a -F -i -w -f "$list" -- "$1" > /dev/null; then
        return 0
    else
        status=$?
        [ "$status" -eq 1 ] || exit 2
    fi
    normalize "$1" > "$normalized" || exit 2
    if grep -a -F -i -w -f "$list" -- "$normalized" > /dev/null; then
        return 0
    else
        status=$?
        [ "$status" -eq 1 ] || exit 2
    fi
    return 1
}

if [ "$message_seen" -eq 1 ]; then
    if [ ! -f "$message" ] || [ ! -r "$message" ]; then
        printf '%s\n' "term-check: can't read the message file" >&2
        exit 2
    fi
    if hits "$message"; then
        printf '%s\n' 'term-check: the message names a private term'
        exit 1
    fi
    printf '%s\n' 'term-check: the message is clean'
    exit 0
fi

paths=$(mktemp)
git -C "$root" ls-files -z --cached --others --exclude-standard > "$paths"
files=0
commits=0
hit=0
while IFS= read -r -d '' path; do
    if [ ! -L "$root/$path" ] && [ ! -f "$root/$path" ]; then
        continue
    fi
    files=$((files + 1))
    if [ -L "$root/$path" ]; then
        readlink "$root/$path" > "$scan" || exit 2
        if hits "$scan"; then
            printf 'term-check: the link %s points at a private term\n' "$path"
            hit=1
        fi
    else
        text_status=0
        grep -I -e '' -- "$root/$path" > /dev/null || text_status=$?
        if [ "$text_status" -gt 1 ]; then
            exit 2
        fi
        if hits "$root/$path"; then
            if [ "$text_status" -eq 0 ]; then
                lines=$({
                    grep -a -F -i -w -I -n -f "$list" -- "$root/$path" || { [ "$?" -eq 1 ] || exit 2; }
                    normalize "$root/$path" > "$normalized" || exit 2
                    grep -a -F -i -w -n -f "$list" -- "$normalized" || { [ "$?" -eq 1 ] || exit 2; }
                } | cut -d: -f1 | sort -n -u) || exit 2
                while IFS= read -r line; do
                    printf 'term-check: %s:%s names a private term\n' "$path" "$line"
                done <<< "$lines"
            else
                printf 'term-check: %s (binary) names a private term\n' "$path"
            fi
            hit=1
        fi
    fi
    printf '%s\n' "$path" > "$scan"
    if hits "$scan"; then
        printf 'term-check: the path %s names a private term\n' "$path"
        hit=1
    fi
done < "$paths"

grafts=$(git -C "$root" rev-parse --git-path info/grafts) || exit 2
case "$grafts" in
    /*) ;;
    *) grafts="$root/$grafts" ;;
esac
if [ -f "$grafts" ]; then
    printf '%s\n' 'term-check: a grafts file rewrites the history; remove it first' >&2
    exit 2
fi
git -C "$root" for-each-ref --format='%(refname)' > "$scan" || exit 2
if hits "$scan"; then
    printf '%s\n' 'term-check: a ref name names a private term'
    hit=1
fi

objects=$(mktemp)
if ! git -C "$root" rev-list --all --objects --no-object-names \
    | git -C "$root" cat-file --batch-check='%(objecttype) %(objectname)' > "$objects"; then
    printf '%s\n' 'term-check: git could not list the history' >&2
    exit 2
fi
if ! awk '$1 != "commit" && $1 != "tree" && $1 != "blob" && $1 != "tag" { exit 1 }' "$objects"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
history=$(mktemp)
if ! awk '$1 == "blob" { print $2 }' "$objects" \
    | git -C "$root" cat-file --batch='' >> "$history"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
messages=$(mktemp)
if ! awk '$1 == "commit" || $1 == "tag" { print $2 }' "$objects" \
    | git -C "$root" cat-file --batch='%(objectsize)' > "$messages"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
if [ "$(wc -c < "$messages")" -ne "$(tr -d '\000' < "$messages" | wc -c)" ]; then
    printf '%s\n' 'term-check: a commit or tag holds a NUL byte; git refuses those, so this check does too' >&2
    exit 2
fi
if ! awk '
        {
            if ($0 !~ /^[0-9]+$/) exit 1
            n = $0 + 0
            consumed = 0
            header = 1
            while (consumed < n) {
                if ((getline) <= 0) exit 1
                consumed += length($0) + 1
                if (consumed > n + 1) exit 1
                if ($0 == "") header = 0
                if (!header || $0 !~ /^(author|committer|tagger) .* <[^>]*> [0-9]+ [+-][0-9]{4}$/)
                    print
            }
            if (consumed == n && ((getline) <= 0 || $0 != "")) exit 1
        }
    ' "$messages" >> "$history"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
if ! git -C "$root" log --all -m --no-renames --name-only -z --format= >> "$history"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
if ! git -C "$root" rev-list --all --objects >> "$history"; then
    printf '%s\n' 'term-check: git could not read the history' >&2
    exit 2
fi
tr '\0' '\n' < "$history" > "$scan" || exit 2
if hits "$scan"; then
    history_hit=$hit
    hit=0
    while read -r kind sha; do
        case "$kind" in
            commit|tag)
                if ! git -C "$root" cat-file "$kind" "$sha" | sed '1,/^$/d' > "$scan"; then
                    printf 'term-check: git could not read %s %s\n' "$kind" "${sha:0:7}" >&2
                    exit 2
                fi
                if [ "$kind" = commit ]; then
                    commits=$((commits + 1))
                fi
                diagnostic="$kind ${sha:0:7} names a private term in its message"
                ;;
            tree)
                if ! git -C "$root" ls-tree -z --name-only "$sha" | tr '\0' '\n' > "$scan"; then
                    printf 'term-check: git could not read %s %s\n' "$kind" "${sha:0:7}" >&2
                    exit 2
                fi
                diagnostic="the history holds a path naming a private term (tree ${sha:0:7})"
                ;;
            blob)
                if ! git -C "$root" cat-file blob "$sha" > "$scan"; then
                    printf 'term-check: git could not read %s %s\n' "$kind" "${sha:0:7}" >&2
                    exit 2
                fi
                diagnostic="the history holds a file naming a private term (blob ${sha:0:7})"
                ;;
            *)
                printf '%s\n' 'term-check: git could not list the history' >&2
                exit 2
                ;;
        esac
        if hits "$scan"; then
            printf 'term-check: %s\n' "$diagnostic"
            hit=1
        fi
    done < "$objects"
    if [ "$hit" -eq 0 ]; then
        printf '%s\n' 'term-check: the history names a private term'
        hit=1
    fi
    if [ "$history_hit" -ne 0 ]; then
        hit=1
    fi
else
    commits=$(awk '$1 == "commit"' "$objects" | wc -l | tr -d '[:space:]')
fi

if [ "$hit" -ne 0 ]; then
    exit 1
fi
printf 'term-check: clean (%s files, %s commits)\n' "$files" "$commits"
