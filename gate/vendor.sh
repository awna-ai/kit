#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export GIT_NO_REPLACE_OBJECTS=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES
usage() {
    printf '%s\n' 'usage: vendor.sh <tag> <destination>' >&2
    exit 2
}
fail() {
    printf 'vendor: %s\n' "$1" >&2
    exit 2
}
[ "$#" -eq 2 ] || usage
tag=$1 destination=$2
if [[ ! "$tag" =~ ^awna-gate-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
    fail 'not a release tag'
fi
library=$(cd "$(dirname "$0")" && pwd) || fail 'cannot find the source directory'
source_repo=$(git -C "$library" rev-parse --show-toplevel 2> /dev/null) || fail 'cannot find the source repository'
commit=$(git -C "$source_repo" rev-parse --verify --quiet "refs/tags/$tag^{commit}") || fail "no such tag: $tag"
tree=$(git -C "$source_repo" rev-parse --verify --quiet "$commit:gate") || fail "$tag has no gate directory"
[ "$(git -C "$source_repo" cat-file -t "$tree")" = tree ] || fail "$tag has no gate directory"
# shellcheck source=gate/lib.sh
. "$library/lib.sh"
working='' backup='' entries=''
cleanup() {
    local status=$?
    if [ -n "$backup" ] && [ -d "$backup/old" ]; then
        if [ ! -e "$destination" ] && [ ! -L "$destination" ]; then
            mv "$backup/old" "$destination" || status=2
        fi
    fi
    if [ -n "$working" ]; then rm -rf "$working" || status=2; fi
    if [ -n "$backup" ]; then rm -rf "$backup" || status=2; fi
    if [ -n "$entries" ]; then rm -f "$entries" || status=2; fi
    exit "$status"
}
trap cleanup EXIT
trap 'fail "interrupted"' HUP INT TERM
entries=$(mktemp) || fail 'cannot create a temporary file'
git -C "$source_repo" ls-tree -r -z --full-tree "$tree" > "$entries" || fail "cannot list $tag"
lib_seen=0 version_seen=0
while IFS= read -r -d '' entry; do
    header=${entry%%$'\t'*} path=${entry#*$'\t'}
    read -r mode kind sha <<< "$header"
    case "$mode:$kind" in 100644:blob|100755:blob) ;; *) fail "$tag holds an entry vendor.sh cannot copy" ;; esac
    rest=$path
    while :; do
        component=${rest%%/*}
        if [[ ! "$component" =~ ^[A-Za-z0-9._-]+$ ]] || [ "$component" = . ] || [ "$component" = .. ] || [ "$component" = .gitattributes ]; then
            fail "$tag holds an entry vendor.sh cannot copy"
        fi
        case "$rest" in */*) rest=${rest#*/} ;; *) break ;; esac
    done
    case "$path" in
        PIN|PIN/*) fail "$tag holds an entry vendor.sh cannot copy" ;;
        lib.sh) lib_seen=1 ;;
        VERSION)
            version_seen=1
            if ! git -C "$source_repo" cat-file blob "$sha" | cmp -s - <(printf '%s\n' "${tag#awna-gate-v}"); then
                fail "$tag does not match its VERSION"
            fi
            ;;
    esac
done < "$entries"
[ "$lib_seen" -eq 1 ] || fail "$tag holds an entry vendor.sh cannot copy"
[ "$version_seen" -eq 1 ] || fail "$tag does not match its VERSION"
# Strip trailing slashes so a link cannot hide behind a directory suffix.
while [ "${destination%/}" != "$destination" ]; do destination=${destination%/}; done
[ -n "$destination" ] || fail 'the destination exists and is not a vendored copy'
case "$destination" in
    */*) parent=${destination%/*}; base=${destination##*/}; [ -n "$parent" ] || parent=/ ;;
    *) parent=.; base=$destination ;;
esac
[ -d "$parent" ] || fail "the parent of $destination does not exist"
if [ -L "$destination" ] || { [ -e "$destination" ] && { [ ! -d "$destination" ] || [ ! -f "$destination/PIN" ] || [ -L "$destination/PIN" ]; }; }; then
    fail "$destination exists and is not a vendored copy"
fi
working=$(mktemp -d "$parent/.$base.XXXXXX") || fail 'cannot create a temporary directory'
while IFS= read -r -d '' entry; do
    header=${entry%%$'\t'*} path=${entry#*$'\t'}
    read -r mode kind sha <<< "$header"
    case "$path" in */*) mkdir -p "$working/${path%/*}" || fail 'cannot create the copy' ;; esac
    git -C "$source_repo" cat-file blob "$sha" > "$working/$path" || fail 'cannot create the copy'
    chmod "${mode#100}" "$working/$path" || fail 'cannot set the copy modes'
done < "$entries"
find "$working" -type d -exec chmod 755 {} + || fail 'cannot set the directory modes'
printf '# Written by vendor.sh. The gate fails when this directory differs from this tag'"'"'s tree.\ntag %s\ntree %s\n' "$tag" "$tree" > "$working/PIN" || fail 'cannot write the pin'
chmod 644 "$working/PIN" || fail 'cannot set the pin mode'
found=$(_gate_tree_hash "$working") || fail "the copy does not match $tag"
[ "$found" = "$tree" ] || fail "the copy does not match $tag"
if [ -d "$destination" ]; then
    backup=$(mktemp -d "$parent/.$base.XXXXXX") || fail 'cannot create a backup directory'
    mv "$destination" "$backup/old" || fail 'cannot back up the old copy'
fi
mv "$working" "$destination" || fail 'cannot install the copy'
working=''
if [ -n "$backup" ]; then rm -rf "$backup" || fail 'cannot remove the old copy'; backup=''; fi
printf 'vendor: copied %s (tree %s) to %s\n' "$tag" "$tree" "$destination"
