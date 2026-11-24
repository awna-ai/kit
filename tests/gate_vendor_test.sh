#!/usr/bin/env bash
set -euo pipefail
export LC_ALL=C
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0
runner=$BASH
source_repo="$tmp/source"
destination="$tmp/copy"
mkdir -p "$source_repo/gate" "$tmp/logs"
cp -R "$root/gate/." "$source_repo/gate/"
printf '0.1.0\n' > "$source_repo/gate/VERSION"
mkdir "$source_repo/gate/sample"
printf 'fixture\n' > "$source_repo/gate/sample/text"
git init -q "$source_repo"
git -C "$source_repo" config user.name Test
git -C "$source_repo" config user.email test@example.invalid
git -C "$source_repo" config commit.gpgsign false
git -C "$source_repo" config tag.gpgsign false
record() {
    git -C "$source_repo" add -A
    git -C "$source_repo" commit -qm 'Record a fixture.'
    git -C "$source_repo" tag -a "$1" -m 'Record a release.'
}
record awna-gate-v0.1.0
tree=$(git -C "$source_repo" rev-parse 'awna-gate-v0.1.0:gate')
file_mode() {
    local mode
    if mode=$(stat -c %a "$1" 2> /dev/null); then printf '%s\n' "$mode"; else stat -f %Lp "$1"; fi
}
check() {
    local name=$1
    shift
    if ! "$@"; then printf 'gate_vendor_test: FAIL %s\n' "$name"; failed=1; fi
}
check docs-git-version grep -Fq 'pin check needs git 2.29 or later' "$root/gate/README.md"
# shellcheck disable=SC2016
check docs-direct-scanner grep -Fq 'Running a vendored scanner directly (without `gate_run`) checks no pin.' "$root/gate/README.md"
check docs-rename-race grep -Fq "A concurrent writer in the destination's parent can race the final rename" "$root/gate/README.md"
expect() {
    local name=$1 wanted=$2 text=$3 actual=0
    shift 3
    output=$("$@" 2>&1) || actual=$?
    check "$name-status-$actual" test "$actual" -eq "$wanted"
    check "$name-diagnostic" test "${output#*"$text"}" != "$output"
}
vendor() {
    bash "$source_repo/gate/vendor.sh" "$@"
}
git init -q "$tmp/other"
expect inherited-common-dir 0 'vendor: copied awna-gate-v0.1.0' env GIT_COMMON_DIR="$tmp/other/.git" bash "$source_repo/gate/vendor.sh" awna-gate-v0.1.0 "$tmp/isolated-copy"
expect initial-copy 0 "vendor: copied awna-gate-v0.1.0 (tree $tree) to $destination" vendor awna-gate-v0.1.0 "$destination"
# A missing implementation cannot exercise the consumer yet.
if [ ! -f "$destination/lib.sh" ]; then exit 1; fi
printf '# Written by vendor.sh. The gate fails when this directory differs from this tag'"'"'s tree.\ntag awna-gate-v0.1.0\ntree %s\n' "$tree" > "$tmp/expected-pin"
check pin cmp -s "$tmp/expected-pin" "$destination/PIN"
git -C "$source_repo" ls-tree -r 'awna-gate-v0.1.0:gate' > "$tmp/entries"
while read -r mode kind sha path; do
    check "kind-$path" test "$kind" = blob
    git -C "$source_repo" cat-file blob "$sha" > "$tmp/blob"
    check "bytes-$path" cmp -s "$tmp/blob" "$destination/$path"
    check "mode-$path" test "$(file_mode "$destination/$path")" = "${mode#100}"
done < "$tmp/entries"
check directory-mode test "$(file_mode "$destination")" = 755
consumer="$tmp/consumer.sh"
make_consumer() {
    : > "$consumer"
    if [ "$style" = strict ]; then printf 'set -euo pipefail\ntrap '\''exit 2'\'' ERR\n' >> "$consumer"; fi
    printf '. "%s/lib.sh"\ngate_init --name t --strict-var CHECK_STRICT --map "%s/map" -- "$@"\ngate_step one '\''printf ran > "%s/marker"'\''\ngate_run\n' "$destination" "$tmp" "$tmp" >> "$consumer"
}
reset_copy() {
    rm -rf "$destination" "$tmp/marker"
    vendor awna-gate-v0.1.0 "$destination" > /dev/null
}
run_consumer() {
    TMPDIR="$tmp/logs" "$runner" "$consumer" "$@"
}
hash_runtime_cases() {
    reset_copy
    mkdir -p "$tmp/hash-bin" "$tmp/hash-logs"
    TREE_TEST_GIT=$(type -P git)
    export TREE_TEST_GIT
    cat > "$tmp/hash-bin/git" <<'SH'
#!/usr/bin/env bash
if [ "${1:-}" = init ]; then
    GIT_DEFAULT_HASH=sha256 "$TREE_TEST_GIT" "$@"
else
    exec "$TREE_TEST_GIT" "$@"
fi
SH
    chmod +x "$tmp/hash-bin/git"
    # shellcheck disable=SC2016
    expect sha1-default 0 "$tree" env PATH="$tmp/hash-bin:$PATH" "$runner" -c '. "$1/lib.sh"; _gate_tree_hash "$2"' bash "$root/gate" "$destination"
    # Exit the hashing subshell during init; its EXIT trap must still clean up.
    # shellcheck disable=SC2016
    expect hash-exit-cleanup 1 'hash-status=1' env TMPDIR="$tmp/hash-logs" "$runner" -c '. "$1/lib.sh"; git() { exit 1; }; _gate_tree_hash "$2"; status=$?; printf "hash-status=%s\n" "$status"; exit "$status"' bash "$root/gate" "$destination"
    check hash-no-temporary test -z "$(ls -A "$tmp/hash-logs")"
}
owner_mode_cases() {
    mkdir -p "$tmp/modes"
    printf 'fixture\n' > "$tmp/modes/run"
    chmod 744 "$tmp/modes/run"
    blob=$(git -C "$source_repo" hash-object -w "$tmp/modes/run")
    mode_tree=$(printf '100755 blob %s\trun\n' "$blob" | git -C "$source_repo" mktree)
    # Simulate execute access denied independently of the recorded owner bit.
    # shellcheck disable=SC2016
    expect owner-mode 0 "$mode_tree" "$runner" -c '. "$1/lib.sh"; function [() { if builtin [ "$#" -eq 3 ] && builtin [ "$1" = -x ]; then return 1; fi; builtin [ "$@"; }; _gate_tree_hash "$2"' bash "$root/gate" "$tmp/modes"
}
directory_cases() {
    local style=strict
    make_consumer
    reset_copy
    printf '\n# drift\n' >> "$destination/lib.sh"
    # Source a relative path so an inherited CDPATH could pollute the saved path.
    # shellcheck disable=SC2016
    expect cdpath-drift 1 'differs from its pin awna-gate-v0.1.0' env CDPATH=. TMPDIR="$tmp/logs" "$runner" -c 'cd -- "$1"; . copy/lib.sh; gate_init --name t --; gate_step one '\''printf ran > marker'\''; gate_run' bash "$tmp"
    check cdpath-no-marker test ! -e "$tmp/marker"
    # shellcheck disable=SC2016
    expect newline-library-source 2 "t: gate_run: cannot find the library's directory" "$runner" -c '. "$1/lib.sh"; _GATE_LIBRARY_SOURCE=$'\''bad\npath'\''; gate_init --name t --; gate_step one :; gate_run' bash "$destination"
}
consumer_cases() {
    local style change
    for style in loose strict; do
        make_consumer
        reset_copy
        expect "$style-green" 0 't: library pinned at awna-gate-v0.1.0' run_consumer
        check marker test -f "$tmp/marker"
        expect subdirectory-consumer 0 'library pinned at awna-gate-v0.1.0' run_inside_copy
        reset_copy
        expect "$style-glob-clean" 0 't: library pinned at awna-gate-v0.1.0' run_glob_consumer
        check glob-marker test -f "$tmp/marker"
        reset_copy
        printf x >> "$destination/README.md"
        expect "$style-glob-drift" 1 'differs from its pin awna-gate-v0.1.0' run_glob_consumer
        check glob-no-marker test ! -e "$tmp/marker"
        check glob-no-logs test -z "$(ls -A "$tmp/logs")"
        reset_copy
        expect "$style-failglob-clean" 0 't: library pinned at awna-gate-v0.1.0' run_glob_consumer -O failglob
        check failglob-marker test -f "$tmp/marker"
        for change in bytes extra hidden double-dot git removed mode link tree nested empty; do
            reset_copy
            case "$change" in
                bytes) printf '\n# changed\n' >> "$destination/lib.sh" ;;
                extra) printf x > "$destination/extra" ;;
                hidden) printf x > "$destination/.extra" ;;
                double-dot) printf x > "$destination/..extra" ;;
                git) git init -q "$destination" ;;
                removed) rm "$destination/README.md" ;;
                mode) chmod +x "$destination/README.md" ;;
                link) cp "$destination/README.md" "$tmp/identical"; rm "$destination/README.md"; ln -s "$tmp/identical" "$destination/README.md" ;;
                tree) sed 's/^tree .*/tree 0000000000000000000000000000000000000000/' "$destination/PIN" > "$tmp/pin"; mv "$tmp/pin" "$destination/PIN" ;;
                nested) mkdir "$destination/nested"; git init -q "$destination/nested" ;;
                empty) mkdir "$destination/empty" ;;
            esac
            expect "$style-drift-$change" 1 'differs from its pin awna-gate-v0.1.0' run_consumer
            check no-marker test ! -e "$tmp/marker"
            check no-logs test -z "$(ls -A "$tmp/logs")"
        done
        reset_copy
        mkfifo "$destination/pipe"
        expect "$style-pipe" 2 't: gate_run: cannot hash the library' run_consumer
        check pipe-no-marker test ! -e "$tmp/marker"
        check pipe-no-logs test -z "$(ls -A "$tmp/logs")"
        for change in link third tag short duplicate nul directory; do
            reset_copy
            case "$change" in
                link) mv "$destination/PIN" "$tmp/pin"; ln -s "$tmp/pin" "$destination/PIN" ;;
                directory) rm "$destination/PIN"; mkdir "$destination/PIN" ;;
                nul) printf '\0' >> "$destination/PIN" ;;
                third) printf 'extra data\n' >> "$destination/PIN" ;;
                tag) printf 'tag v0.1.0\ntree %s\n' "$tree" > "$destination/PIN" ;;
                short) printf 'tag awna-gate-v0.1.0\ntree %s\n' "${tree:0:39}" > "$destination/PIN" ;;
                duplicate) printf 'tag awna-gate-v0.1.0\n' >> "$destination/PIN" ;;
            esac
            expect "$style-pin-$change" 1 "t: the library's pin cannot be read:" run_consumer
            check no-marker test ! -e "$tmp/marker"
        done
        reset_copy
        printf x > "$destination/extra"
        expect drift-list 0 one run_consumer --list
        check list-no-pin test "${output#*pinned}" = "$output"
        git init -q "$tmp/consumer-repo"
        printf '* one\n' > "$tmp/map"
        printf x > "$tmp/consumer-repo/text"
        git -C "$tmp/consumer-repo" add text
        expect drift-map 0 't: map ok' run_map
        check map-no-pin test "${output#*pinned}" = "$output"
        printf '* -\n' > "$tmp/map"
        if ! git -C "$tmp/consumer-repo" rev-parse --verify HEAD > /dev/null 2>&1; then
            git -C "$tmp/consumer-repo" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record a fixture.'
        fi
        git -C "$tmp/consumer-repo" update-ref refs/remotes/origin/main HEAD
        expect empty-touched-drift 1 'differs from its pin' run_touched
        check empty-touched-no-marker test ! -e "$tmp/marker"
        check empty-touched-no-logs test -z "$(ls -A "$tmp/logs")"
        reset_copy
        # Configuration, inherited index and object settings cannot hide drift.
        printf '[core]\n autocrlf = true\n filemode = false\n' > "$tmp/hostile-config"
        expect hostile-config 0 'library pinned at awna-gate-v0.1.0' env GIT_CONFIG_GLOBAL="$tmp/hostile-config" GIT_INDEX_FILE="$tmp/unused-index" GIT_OBJECT_DIRECTORY="$tmp/absent-objects" GIT_CONFIG_COUNT=1 GIT_CONFIG_KEY_0=core.filemode GIT_CONFIG_VALUE_0=false TMPDIR="$tmp/logs" "$runner" "$consumer"
        check isolated-index test ! -e "$tmp/unused-index"
        chmod +x "$destination/README.md"
        expect hostile-mode-drift 1 'differs from its pin' env GIT_CONFIG_GLOBAL="$tmp/hostile-config" TMPDIR="$tmp/logs" "$runner" "$consumer"
        reset_copy
        # Comments and either field order are valid pin syntax.
        printf '# comment\n\ntree %s\n# another comment\ntag awna-gate-v0.1.0\n' "$tree" > "$destination/PIN"
        expect reordered-pin 0 'library pinned at awna-gate-v0.1.0' run_consumer
        reset_copy
        rm "$destination/PIN"
        expect unpinned 0 't: all passed' run_consumer
        check no-pin-line test "${output#*pinned}" = "$output"
        reset_copy
        expect missing-git 0 't: all passed, 1 skipped' env PATH="$tmp/bin" TMPDIR="$tmp/logs" "$runner" "$consumer"
        check missing-git-line test "${output#*git is missing, skipping the library pin check}" != "$output"
        expect strict-missing-git 75 'git is missing on this host (CHECK_STRICT=1)' env CHECK_STRICT=1 PATH="$tmp/bin" "$runner" "$consumer"
    done
}
run_inside_copy() { (cd "$destination/sample" && run_consumer); }
run_glob_consumer() {
    # shellcheck disable=SC2016
    TMPDIR="$tmp/logs" "$runner" "$@" -c 'set -f; shopt -s dotglob nocaseglob; GLOBIGNORE="*.md"; script=$1; shift; . "$script"' bash "$consumer"
}
run_map() { (cd "$tmp/consumer-repo" && run_consumer --check-map); }
run_touched() { (cd "$tmp/consumer-repo" && run_consumer --tier touched); }
mkdir "$tmp/bin"
for tool in bash mktemp rm tail; do ln -s "$(type -P "$tool")" "$tmp/bin/$tool"; done
hash_runtime_cases
owner_mode_cases
directory_cases
consumer_cases
# shellcheck disable=SC2016
if [ -x /bin/bash ] && [ "$(/bin/bash -c 'printf %s "${BASH_VERSINFO[0]}"')" != "$("$runner" -c 'printf %s "${BASH_VERSINFO[0]}"')" ]; then
    runner=/bin/bash
    hash_runtime_cases
    owner_mode_cases
    directory_cases
    consumer_cases
fi
reset_copy
printf 'old\n' > "$destination/old-only"
cp -R "$destination" "$tmp/before"
refuse() {
    local name=$1 reason=$2
    shift 2
    expect "$name" 2 "$reason" vendor "$@"
    check "$name-unchanged" diff -r "$tmp/before" "$destination"
}
refuse arguments 'usage: vendor.sh <tag> <destination>'
for tag in v0.1.0 awna-gate-v01.0.0; do refuse invalid-tag 'vendor: not a release tag' "$tag" "$destination"; done
refuse unknown 'vendor: no such tag: awna-gate-v9.0.0' awna-gate-v9.0.0 "$destination"
git -C "$source_repo" branch refs/tags/awna-gate-v9.0.0
refuse branch-not-tag 'vendor: no such tag: awna-gate-v9.0.0' awna-gate-v9.0.0 "$destination"
for change in absent link pin attributes version nul-version missing-lib bad-path; do
    rm -rf "$source_repo/gate"
    git -C "$source_repo" restore --source=awna-gate-v0.1.0 -- gate
    case "$change" in
        absent) rm -rf "$source_repo/gate"; reason='has no gate directory' ;;
        link) ln -s VERSION "$source_repo/gate/link"; reason='holds an entry vendor.sh cannot copy' ;;
        pin) printf x > "$source_repo/gate/PIN"; reason='holds an entry vendor.sh cannot copy' ;;
        attributes) mkdir "$source_repo/gate/sub"; printf x > "$source_repo/gate/sub/.gitattributes"; reason='holds an entry vendor.sh cannot copy' ;;
        nul-version) printf '0.3.0\0\n' > "$source_repo/gate/VERSION"; reason='does not match its VERSION' ;;
        version) printf '0.9.0\n' > "$source_repo/gate/VERSION"; reason='does not match its VERSION' ;;
        missing-lib) rm "$source_repo/gate/lib.sh"; reason='holds an entry vendor.sh cannot copy' ;;
        bad-path) printf x > "$source_repo/gate/bad name"; reason='holds an entry vendor.sh cannot copy' ;;
    esac
    # Keep the running vendor script outside the fixture being changed.
    cp "$root/gate/vendor.sh" "$source_repo/vendor.sh"
    cp "$root/gate/lib.sh" "$source_repo/lib.sh"
    if [ "$change" = nul-version ]; then
        tag=awna-gate-v0.3.0
    else
        tag="awna-gate-v0.2.$(( ${count:-0} + 1 ))"
    fi
    count=$(( ${count:-0} + 1 ))
    if [ "$change" != absent ] && [ "$change" != version ] && [ "$change" != nul-version ]; then printf '%s\n' "${tag#awna-gate-v}" > "$source_repo/gate/VERSION"; fi
    record "$tag"
    expect "refuse-$change" 2 "$reason" bash "$source_repo/vendor.sh" "$tag" "$destination"
    check "$change-unchanged" diff -r "$tmp/before" "$destination"
done
rm -rf "$source_repo/gate"
git -C "$source_repo" restore --source=awna-gate-v0.1.0 -- gate
refuse missing-parent 'does not exist' awna-gate-v0.1.0 "$tmp/absent/copy"
ln -s "$destination" "$tmp/link"
refuse destination-link 'exists and is not a vendored copy' awna-gate-v0.1.0 "$tmp/link"
mkdir "$tmp/plain"
printf keep > "$tmp/plain/file"
refuse destination-plain 'exists and is not a vendored copy' awna-gate-v0.1.0 "$tmp/plain"
check preserved test "$(cat "$tmp/plain/file")" = keep
# A PIN filename alone does not authorize replacing a directory.
mkdir -p "$tmp/plain/src"
printf 'keep\n' > "$tmp/plain/src/keep.txt"
printf 'x\n' > "$tmp/plain/PIN"
expect destination-fake-pin 2 "$tmp/plain exists and is not a vendored copy" vendor awna-gate-v0.1.0 "$tmp/plain"
check fake-pin-preserved test -f "$tmp/plain/src/keep.txt"
printf 'tag awna-gate-v0.1.0\ntree %s\n' "$tree" > "$tmp/plain/PIN"
expect destination-unmarked-pin 2 "$tmp/plain exists and is not a vendored copy" vendor awna-gate-v0.1.0 "$tmp/plain"
check unmarked-pin-preserved test -f "$tmp/plain/src/keep.txt"
printf '0.1.1\n' > "$source_repo/gate/VERSION"
printf ':\n' > "$source_repo/gate/executable.sh"
chmod +x "$source_repo/gate/executable.sh"
record awna-gate-v0.1.1
printf '\n# uncommitted\n' >> "$source_repo/gate/lib.sh"
expect replacement 0 'vendor: copied awna-gate-v0.1.1' vendor awna-gate-v0.1.1 "$destination"
check removed-old test ! -e "$destination/old-only"
check executable test -x "$destination/executable.sh"
git -C "$source_repo" show 'awna-gate-v0.1.1:gate/lib.sh' > "$tmp/blob"
check tag-not-working-tree cmp -s "$tmp/blob" "$destination/lib.sh"
style=strict
make_consumer
expect replacement-consumer 0 'library pinned at awna-gate-v0.1.1' run_consumer
if [ "$failed" -ne 0 ]; then exit 1; fi
printf 'gate_vendor_test: all passed\n'
