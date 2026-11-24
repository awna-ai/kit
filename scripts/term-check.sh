#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
root=$(cd "$(dirname "$0")/.." && pwd)
if [ "$#" -ne 0 ] && { [ "$#" -ne 2 ] || [ "$1" != --message ]; }; then
    printf '%s\n' 'usage: term-check.sh [--message <file>]' >&2
    exit 2
fi
terms=${KIT_TERMS_FILE:-${XDG_CONFIG_HOME:-$HOME/.config}/awna/terms.txt}
if [ "$#" -eq 2 ]; then
    exec bash "$root/gate/term-check.sh" --terms "$terms" --strict-var KIT_STRICT --message "$2"
fi
exec bash "$root/gate/term-check.sh" --terms "$terms" --strict-var KIT_STRICT --root "$root"
