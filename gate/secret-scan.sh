#!/usr/bin/env bash
set -euo pipefail
# Read the exported setting before any scanner variable can replace its value.
strict_value=$(
    while [ "$#" -ge 2 ]; do
        if [ "$1" = --strict-var ]; then
            printenv -- "$2" || true
            break
        fi
        shift 2
    done
)
export LC_ALL=C
export GIT_NO_REPLACE_OBJECTS=1
unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES GIT_COMMON_DIR
trap 'printf "%s\n" "secret-scan: internal error" >&2; exit 2' ERR
usage() {
    printf '%s\n' 'usage: secret-scan.sh [--strict-var <VAR>] --root <dir>' >&2
    exit 2
}
root='' strict_var='' root_seen=0 strict_seen=0
while [ "$#" -gt 0 ]; do
    [ "$#" -ge 2 ] && [ -n "$2" ] || usage
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
        *) usage ;;
    esac
    shift 2
done
if [ "$strict_seen" -eq 1 ] && [[ ! "$strict_var" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then usage; fi
strict=0
if [ -n "$strict_var" ] && [ "$strict_value" = 1 ]; then strict=1; fi
[ "$root_seen" -eq 1 ] || usage
[ -d "$root" ] || usage
root=$(CDPATH='' cd -- "$root" && pwd -P) || usage
if ! top=$(git -C "$root" rev-parse --show-toplevel 2> /dev/null) ||
    ! top=$(CDPATH='' cd -- "$top" && pwd -P) || [ "$top" != "$root" ]; then
    printf 'secret-scan: %s is not the top of a git repository\n' "$root" >&2
    exit 2
fi
CDPATH='' cd -- "$root" || usage
if ! command -v gitleaks > /dev/null 2>&1; then
    if [ "$strict" -eq 1 ]; then
        printf 'secret-scan: gitleaks is missing on this host (%s=1)\n' "$strict_var" >&2
        exit 75
    fi
    printf '%s\n' 'secret-scan: gitleaks is missing, skipped'
    exit 0
fi
unset GITLEAKS_CONFIG GITLEAKS_CONFIG_TOML
for config in .gitleaks.toml .gitleaksignore; do
    if [ -e "$config" ]; then
        printf 'secret-scan: %s would change what gitleaks reports; remove it\n' "$config" >&2
        exit 1
    fi
done
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

git ls-files -z --cached --others --exclude-standard > "$tmp/paths" || exit 2
while IFS= read -r -d '' path; do
    case "$(printf '%s' "${path##*/}" | tr '[:upper:]' '[:lower:]')" in
        .gitleaks.toml|.gitleaksignore)
            printf 'secret-scan: %s would change what gitleaks reports; remove it\n' "$path" >&2
            exit 1
            ;;
    esac
done < "$tmp/paths"

scan() {
    local status=0
    "$@" || status=$?
    case "$status" in
        0) ;;
        1) exit 1 ;;
        *) exit 2 ;;
    esac
}

# An empty repository still has a working tree to scan.
git rev-list --objects --all --no-object-names > "$tmp/objects" || exit 2
if [ -s "$tmp/objects" ]; then
    scan gitleaks git --redact --no-banner --log-opts=--all .
fi
: > "$tmp/contents"
while IFS= read -r object; do
    git cat-file -p "$object" >> "$tmp/contents" || exit 2
    printf '\n' >> "$tmp/contents"
done < "$tmp/objects"
# Run outside the repository so repository configuration cannot affect stdin.
(cd "$tmp" && scan gitleaks stdin --redact --no-banner < "$tmp/contents") || exit "$?"

mkdir "$tmp/tree"
while IFS= read -r -d '' path; do
    if [ -f "$path" ] && [ ! -L "$path" ]; then
        printf '%s\0' "$path"
    fi
done < "$tmp/paths" > "$tmp/files"
tar --null -T "$tmp/files" -cf - | tar -xf - -C "$tmp/tree"
scan gitleaks dir --redact --no-banner "$tmp/tree"
printf '%s\n' 'secret-scan: clean'
