#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export GIT_NO_REPLACE_OBJECTS=1
trap 'echo "secret-scan: internal error" >&2; exit 2' ERR
cd "$(dirname "$0")/.."
if [ "$#" -ne 0 ]; then
    echo 'usage: secret-scan.sh' >&2
    exit 2
fi
if ! command -v gitleaks > /dev/null 2>&1; then
    if [ "${KIT_STRICT:-}" = 1 ]; then
        echo 'secret-scan: gitleaks is missing on this host (KIT_STRICT=1)' >&2
        exit 75
    fi
    echo 'secret-scan: gitleaks is missing, skipped'
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
echo 'secret-scan: clean'
