#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export RUSTUP_AUTO_INSTALL=0

packages=(awna-run awna-agents awna-keystore awna-launch awna-intake)
root=$(cd "$(dirname "$0")/.." && pwd -P)
cd "$root"
if [ "$#" -ne 0 ]; then
    echo 'usage: deps-check.sh' >&2
    exit 2
fi
if ! command -v cargo > /dev/null 2>&1; then
    if [ "${KIT_STRICT:-}" = 1 ]; then
        echo 'deps-check: cargo is missing on this host (KIT_STRICT=1)' >&2
        exit 75
    fi
    echo 'deps-check: cargo is missing, skipped'
    exit 0
fi

if [ -e .cargo/config ] || [ -e .cargo/config.toml ]; then
    echo 'deps-check: .cargo/config or .cargo/config.toml can redirect where cargo fetches crates; remove it' >&2
    exit 1
fi
files=$(git ls-files --cached --others --exclude-standard -z | tr '\0' '\n') || exit 2
while IFS= read -r file; do
    case "$(printf '/%s' "$file" | tr '[:upper:]' '[:lower:]')" in
        */.cargo/config|*/.cargo/config.toml)
            printf 'deps-check: %s can redirect where cargo fetches crates; remove it\n' "$file" >&2
            exit 1
            ;;
    esac
done <<< "$files"

failed=0
forbidden() {
    case "$2" in
        reqwest|hyper|ureq|isahc|surf|attohttpc|tauri-plugin-*|iroh*)
            printf 'deps-check: %s resolves %s, a networking or plugin crate these packages must not use\n' "$1" "$2"
            failed=1
            ;;
    esac
}
for package in "${packages[@]}"; do
    if ! tree=$(cargo tree --locked -p "$package" --all-features --target all -e normal,build,dev --prefix none --format '{p}'); then
        printf 'deps-check: cargo tree failed for %s\n' "$package" >&2
        failed=1
        continue
    fi
    while IFS= read -r line; do
        shape='^([A-Za-z0-9_-]+) v[0-9][A-Za-z0-9.+_-]*( \(proc-macro\))?( \(([^()]+)\))?( \(\*\))?$'
        if [[ ! "$line" =~ $shape ]]; then
            echo 'deps-check: unrecognised cargo tree output' >&2
            exit 2
        fi
        name=${BASH_REMATCH[1]}
        source=${BASH_REMATCH[4]}
        # The duplicate marker without a source is not a path.
        [ "$source" != '*' ] || source=''
        forbidden "$package" "$name"
        [ -n "$source" ] || continue
        physical=''
        if [[ "$source" = /* ]] && [ -d "$source" ]; then
            physical=$(cd -P "$source" && pwd -P) || physical=''
        fi
        case "$physical" in
            "$root"|"$root"/*) ;;
            *)
                printf 'deps-check: %s resolves %s from %s, outside crates.io and this workspace\n' "$package" "$name" "$source"
                failed=1
                ;;
        esac
    done <<< "$tree"
done
[ "$failed" -eq 0 ] || exit 1
# Require cargo's canonical lockfile before parsing its layout.
tmp=$(mktemp -d) || exit 2
trap 'rm -rf "$tmp"' EXIT
mkdir "$tmp/tree" || exit 2
while IFS= read -r file; do
    if [ -f "$file" ] && [ ! -L "$file" ]; then
        printf '%s\0' "$file"
    fi
done <<< "$files" > "$tmp/files"
tar --null -T "$tmp/files" -cf - | tar -xf - -C "$tmp/tree" || exit 2
if ! (cd "$tmp/tree" && cargo update --workspace --offline --quiet); then
    echo 'deps-check: could not regenerate Cargo.lock' >&2
    exit 2
fi
if ! cmp -s Cargo.lock "$tmp/tree/Cargo.lock"; then
    echo "deps-check: Cargo.lock is not in cargo's own format; run cargo update --workspace" >&2
    exit 1
fi
# Lockfiles include optional edges that cargo tree omits for dependencies.
if ! locked=$(awk -v roots="${packages[*]}" '
    { sub(/\r$/, "") }
    /^\[\[/ {
        in_package = ($0 == "[[package]]")
        if (in_package) count++
        in_dependencies = 0
        next
    }
    !in_package { next }
    /^name = / { names[count] = $3; gsub(/"/, "", names[count]); next }
    /^version = / { versions[count] = $3; gsub(/"/, "", versions[count]); next }
    /^dependencies = \[/ { in_dependencies = 1; next }
    in_dependencies && /^\]/ { in_dependencies = 0; next }
    in_dependencies && /^[[:space:]]*"/ {
        edge = $0
        sub(/^[[:space:]]*"/, "", edge)
        sub(/",?[[:space:]]*$/, "", edge)
        edges[count, ++edge_count[count]] = edge
    }
    END {
        root_count = split(roots, selected, " ")
        for (r = 1; r <= root_count; r++) {
            for (i = 1; i <= count; i++) reached[i] = (names[i] == selected[r])
            changed = 1
            while (changed) {
                changed = 0
                for (i = 1; i <= count; i++) if (reached[i]) {
                    for (e = 1; e <= edge_count[i]; e++) {
                        fields = split(edges[i, e], dependency, " ")
                        for (j = 1; j <= count; j++) {
                            if (names[j] == dependency[1] &&
                                (fields == 1 || versions[j] == dependency[2]) && !reached[j]) {
                                reached[j] = 1
                                changed = 1
                            }
                        }
                    }
                }
            }
            for (i = 1; i <= count; i++) if (reached[i]) print selected[r], names[i]
        }
    }
' Cargo.lock); then
    echo 'deps-check: could not read Cargo.lock' >&2
    exit 2
fi
while read -r package name; do
    [ -n "$package" ] || continue
    forbidden "$package" "$name"
done <<< "$locked"
if [ "$failed" -ne 0 ]; then
    exit 1
fi
printf 'deps-check: %s packages clean\n' "${#packages[@]}"
