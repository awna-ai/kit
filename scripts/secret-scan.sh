#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
if [ "$#" -ne 0 ]; then
    printf '%s\n' 'usage: secret-scan.sh' >&2
    exit 2
fi
root=$(cd "$(dirname "$0")/.." && pwd)
exec bash "$root/gate/secret-scan.sh" --strict-var KIT_STRICT --root "$root"
