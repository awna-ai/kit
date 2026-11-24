#!/usr/bin/env bash
set -u -o pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
failed=0
number=0
runner=$(type -P bash)
mode=loose
mkdir "$tmp/bin"
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1
for tool in mktemp rm tail cat git env touch tr cmp; do
    ln -s "$(type -P "$tool")" "$tmp/bin/$tool"
done
cat > "$tmp/bin/uname" <<'STUB'
#!/bin/sh
printf '%s\n' "${TEST_OS:-Darwin}"
STUB
chmod +x "$tmp/bin/uname"

new_case() {
    number=$((number + 1))
    case_dir="$tmp/case-$number"
    mkdir -p "$case_dir/logs" "$case_dir/.github/workflows"
    consumer="$case_dir/consumer.sh"
    ln -sf "$runner" "$tmp/bin/bash"
    if [ "$mode" = strict ]; then
        printf 'set -euo pipefail\ntrap '\''exit 2'\'' ERR\n' > "$consumer"
    else
        printf ':\n' > "$consumer"
    fi
    printf '. "%s/gate/lib.sh"\n' "$root" >> "$consumer"
    unset TEST_OS
}

check() {
    local name=$1
    shift
    if ! "$@"; then
        printf 'gate_workflow_test: FAIL %s\n' "$name"
        failed=1
    fi
}

expect() {
    local name=$1 expected=$2 wanted=$3 last=$4 actual
    shift 4
    if (cd "$case_dir" && PATH="$tmp/bin" TMPDIR="$case_dir/logs" "$runner" "$consumer" "$@") <<< 'real input' > "$case_dir/out" 2> "$case_dir/err"; then
        actual=0
    else
        actual=$?
    fi
    output=$(cat "$case_dir/out" "$case_dir/err")
    if [ "$actual" -ne "$expected" ] || [[ "$output" != *"$wanted"* ]] ||
        { [ -n "$last" ] && [ "${output##*$'\n'}" != "$last" ]; }; then
        printf 'gate_workflow_test: FAIL %s: exit %s (wanted %s), expected %s; got %s\n' "$name" "$actual" "$expected" "$wanted" "$output"
        failed=1
    fi
}

setup_consumer() {
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { return 0; }
if ! declare -F gate_workflow_uses > /dev/null; then
    gate_workflow_uses() { return 0; }
fi
gate_init --name t -- "$@"
gate_workflow ci.yml
gate_run
CASE
}

basic_cases() {
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
name: checks
on:
  push:
jobs:
  build:
    name: compile
    runs-on: ubuntu-latest
    steps:
      - uses: org/setup@v1
        with:
          version: 1
      - name: first
        id: first
        timeout-minutes: 1
        run: printf 'one\n' >> marker
      - run: printf 'two\n' >> marker
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: printf 'three\n' >> marker
YAML
    setup_consumer
    expect ordered-list 0 '' '' --list
    check exact-list test "$(cat "$case_dir/out")" = "$(printf '%s\n' "build: printf 'one\\n' >> marker" "build: printf 'two\\n' >> marker" "lint: printf 'three\\n' >> marker")"
    check list-no-logs test -z "$(ls -A "$case_dir/logs")"
    check list-no-marker test ! -e "$case_dir/marker"
    expect ordered-run 0 'step 3 ' 't: all passed'
    check ordered-marker test "$(cat "$case_dir/marker" 2>/dev/null)" = "$(printf 'one\ntwo\nthree')"

    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: printf 'one\n' >> marker
      - run: false
      - run: touch after
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: printf 'lint\n' >> marker
YAML
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { return 0; }
gate_init --name t
gate_workflow ci.yml
gate_step plain 'printf "plain\n" >> marker'
gate_run
CASE
    expect job-skip 1 'step 3 skipped build: touch after (an earlier step of job build failed)' ''
    check skipped-marker test ! -e "$case_dir/after"
    check other-job-and-plain test "$(cat "$case_dir/marker" 2>/dev/null)" = "$(printf 'one\nlint\nplain')"
    check skipped-no-log test ! -e "$case_dir"/logs/t.*/step-3.log
    check red-last test "${output##*$'\n'}" = "t: 1 failed (logs: $(printf '%s' "$case_dir"/logs/t.*))"
    check skip-no-slowest test "$(grep -c '  .*build: touch after' "$case_dir/out")" -eq 0
    check skip-no-count test "${output#*', 1 skipped'}" = "$output"
}

environment_cases() {
    local pass
    for pass in '' '--pass EXTRA_ONE'; do
        new_case
        cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: env > marker
    env:
      APPLE: ignored
      CI: ignored
YAML
        cat >> "$consumer" <<'CASE'
export SECRET_TOKEN=leak EXTRA_ONE=one HOME=/invented LANG=C TERM=dumb
unset LOGNAME
unset USER
gate_workflow_job() { return 0; }
gate_workflow_env() {
    case "$3" in APPLE) printf 'value with spaces';; CI) printf 'override';; esac
}
gate_init --name t
CASE
        printf 'gate_workflow %s ci.yml\ngate_run\n' "$pass" >> "$consumer"
        expect scrubbed 0 '' 't: all passed'
        for item in HOME=/invented LANG=C TERM=dumb CI=override 'APPLE=value with spaces'; do
            check "env-$item" grep -Fxq "$item" "$case_dir/marker"
        done
        check env-path grep -q '^PATH=' "$case_dir/marker"
        check env-secret test "$(grep -c '^SECRET_TOKEN=' "$case_dir/marker" 2>/dev/null)" -eq 0
        check env-unset-user test "$(grep -Ec '^(LOGNAME|USER)=' "$case_dir/marker" 2>/dev/null)" -eq 0
        if [ -n "$pass" ]; then check env-pass grep -Fxq EXTRA_ONE=one "$case_dir/marker"
        else check env-no-pass test "$(grep -c '^EXTRA_ONE=' "$case_dir/marker" 2>/dev/null)" -eq 0; fi
        # The same registration with an unset base variable omits it entirely.
        sed '/export SECRET_TOKEN/a\
unset LANG' "$consumer" > "$case_dir/replaced"
        mv "$case_dir/replaced" "$consumer"
        expect unset-language 0 '' 't: all passed'
        check env-no-language test "$(grep -c '^LANG=' "$case_dir/marker" 2>/dev/null)" -eq 0
    done
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: env > marker
YAML
    setup_consumer
    expect base-ci 0 '' 't: all passed'
    check base-ci-value grep -Fxq CI=true "$case_dir/marker"
}

reusable_cases() {
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  call:
    uses: ./.github/workflows/shared.yml
  other:
    uses: ./.github/workflows/other.yml
  build:
    runs-on: ubuntu-latest
    steps:
      - run: false
YAML
    cat > "$case_dir/.github/workflows/shared.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: printf shared >> marker
  deep:
    uses: ./.github/workflows/deep.yml
  again:
    uses: ./.github/workflows/ci.yml
YAML
    cat > "$case_dir/.github/workflows/other.yml" <<'YAML'
jobs:
  apple:
    runs-on: ubuntu-latest
    steps:
      - run: printf other >> marker
YAML
    cat > "$case_dir/.github/workflows/deep.yml" <<'YAML'
jobs:
  apple:
    runs-on: ubuntu-latest
    steps:
      - run: printf deep >> marker
YAML
    setup_consumer
    expect reusable-list 0 '' '' --list
    check reusable-order test "$(cat "$case_dir/out")" = "$(printf '%s\n' 'build: false' 'build: printf shared >> marker' 'apple: printf other >> marker' 'apple: printf deep >> marker')"
    expect reusable-groups 1 'step 2 ' ''
    check reusable-marker test "$(cat "$case_dir/marker" 2>/dev/null)" = sharedotherdeep
}

shell_cases() {
    local os label
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: false; touch after
YAML
    setup_consumer
    expect shell-errexit 1 'FAIL: build: false; touch after' ''
    check shell-no-after test ! -e "$case_dir/after"
    for label in macos-latest ubuntu-latest; do
        new_case
        printf 'jobs:\n  build:\n    runs-on: %s\n    steps:\n      - run: touch marker\n' "$label" > "$case_dir/.github/workflows/ci.yml"
        setup_consumer
        for os in Linux Darwin; do
            export TEST_OS=$os
            if [ "$label/$os" = macos-latest/Linux ]; then
                expect mac-preflight 75 't: build: touch marker runs only on macOS and this host is Linux; exit 75 so another host can run the gate' ''
                check mac-no-marker test ! -e "$case_dir/marker"
            else
                expect runner-label 0 '' 't: all passed'
                check runner-marker test -f "$case_dir/marker"
            fi
        done
    done
}

# Each fixture is parsed before any command can run, including list and map modes.
refuse() {
    local name=$1 reason=$2 fixture=$3 hooks=${4:-} args=${5:-}
    new_case
    printf '%s\n' "$fixture" > "$case_dir/.github/workflows/ci.yml"
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { return 0; }
gate_init --name t --map map -- "$@"
CASE
    printf '%s\ngate_workflow ci.yml\ngate_run\n' "$hooks" >> "$consumer"
    expect "$name" 2 "$reason" "$reason" ${args:+"$args"}
    check "$name-stderr" test "$(cat "$case_dir/err")" = "$reason"
    check "$name-no-marker" test ! -e "$case_dir/marker"
    check "$name-no-logs" test -z "$(ls -A "$case_dir/logs")"
}

refusal_cases() {
    local prefix='t: ci.yml, job build:' head=$'jobs:\n  build:' steps=$'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:'
    refuse no-hook "t: ci.yml: job 'build' is not mirrored; define gate_workflow_job to allow it" "$steps" 'unset -f gate_workflow_job'
    refuse cr 't: ci.yml: line 1: a carriage return or a tab' $'jobs:\r'
    refuse tab 't: ci.yml: line 2: a carriage return or a tab' $'jobs:\n\tbuild:'
    refuse top-env "t: ci.yml: line 1: top-level key 'env' is not mirrored" 'env:'
    refuse top-defaults "t: ci.yml: line 1: top-level key 'defaults' is not mirrored" 'defaults:' '' --list
    refuse top-shape 't: ci.yml: line 1: cannot be mirrored' 'jobs:bad'
    refuse orphan-indent 't: ci.yml: line 1: cannot be mirrored' $'  stray\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: touch marker'
    refuse no-jobs 't: ci.yml: no jobs: block' 'on: push'
    refuse inline-jobs 't: ci.yml: no jobs: block' 'jobs: {}'
    refuse odd-indent "$prefix line 3: cannot be mirrored" "$head"$'\n   steps:'
    refuse no-job 't: ci.yml: line 2: cannot be mirrored' $'jobs:\n    steps:'
    refuse job-id 't: ci.yml: line 2: cannot be mirrored' $'jobs:\n  bad.id:'
    refuse job-value 't: ci.yml: line 2: cannot be mirrored' $'jobs:\n  build: value'
    refuse job-key "$prefix job key 'strategy' is not mirrored" "$head"$'\n    strategy:' '' --check-map
    refuse inline-env "$prefix env must be a block" "$head"$'\n    env: {A: 1}'
    refuse inline-services "$prefix services must be a block" "$head"$'\n    services: {}'
    refuse inline-steps "$prefix steps must be a block" "$head"$'\n    steps: []'
    refuse runner-flow "$prefix runs-on '[self-hosted, linux]' is not a plain runner label" "$head"$'\n    runs-on: [self-hosted, linux]'
    refuse runner-expression "$prefix runs-on '\${{ matrix.os }}' is not a plain runner label" "$head"$'\n    runs-on: ${{ matrix.os }}'
    refuse no-runner "$prefix steps without runs-on" "$head"$'\n    steps:\n      - run: touch marker'
    refuse remote-call "$prefix uses 'org/repo/.github/workflows/x.yml@v1' is not a local reusable workflow" "$head"$'\n    uses: org/repo/.github/workflows/x.yml@v1'
    refuse both-job "$prefix both steps and uses" "$steps"$'\n      - run: touch marker\n    uses: ./.github/workflows/shared.yml'
    refuse empty-job "$prefix no run: steps to mirror" "$head"$'\n    needs: other'
    refuse env-denied "$prefix env variable APPLE is not mirrored" "$head"$'\n    env:\n      APPLE: ignored'
    refuse env-newline "$prefix env variable APPLE is not mirrored" "$head"$'\n    env:\n      APPLE: ignored' 'gate_workflow_env() { printf "one\ntwo\n"; }'
    refuse env-name "$prefix line 4: env must be single-line NAME: value pairs" "$head"$'\n    env:\n      BAD-NAME: value'
    refuse env-nested "$prefix line 5: env must be single-line NAME: value pairs" "$head"$'\n    env:\n      APPLE: value\n        nested: value' 'gate_workflow_env() { :; }'
    refuse service "$prefix service 'apple' is not mirrored" "$head"$'\n    services:\n      apple:'
    refuse flow-step "$prefix line 5: write the step in block style" "$steps"$'\n      - { run: x }'
    refuse bare-step "$prefix line 5: write the step in block style" "$steps"$'\n      -'
    refuse step-if "$prefix step key 'if' is not mirrored" "$steps"$'\n      - if: always()'
    refuse step-shell "$prefix step key 'shell' is not mirrored" "$steps"$'\n      - shell: bash'
    refuse run-block "$prefix line 5: a multi-line run: step" "$steps"$'\n      - run: |'
    refuse run-folded "$prefix line 5: a multi-line run: step" "$steps"$'\n      - run: >-'
    refuse run-empty "$prefix line 5: a multi-line run: step" "$steps"$'\n      - run:'
    refuse run-quoted "$prefix line 5: run: is not a plain one-line value" "$steps"$'\n      - run: '\''quoted'\'
    refuse run-colon "$prefix line 5: run: is not a plain one-line value" "$steps"$'\n      - run: echo a: b'
    refuse run-expression "$prefix line 5: run: uses an expression" "$steps"$'\n      - run: echo ${{ inputs.x }}'
    refuse step-uses "$prefix step uses 'org/setup@v1' is not mirrored" "$steps"$'\n      - uses: org/setup@v1'
    refuse step-none "$prefix line 5: a step needs exactly one of run: and uses:" "$steps"$'\n      - name: first'
    refuse step-both "$prefix line 5: a step needs exactly one of run: and uses:" "$steps"$'\n      - run: touch marker\n        uses: org/setup@v1' 'gate_workflow_uses() { :; }'
    refuse no-runs 't: ci.yml: no jobs: block' 'jobs:'
    new_case
    setup_consumer
    expect missing-file 2 't: ci.yml: workflow not found in .github/workflows' 't: ci.yml: workflow not found in .github/workflows'
}

allowed_cases() {
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
---
run-name: checks # ignored
permissions:
  contents: read
concurrency: checks
jobs:
  build:
    strategy:
      matrix:
         apple: [1, 2]
    services:
      apple:
         image: invented
    runs-on: ubuntu-latest # label
    steps:
      - run: printf ok > marker # note
YAML
    cat >> "$consumer" <<'CASE'
gate_workflow_job_key() { [ "$3" = strategy ]; }
gate_workflow_service() { [ "$3" = apple ]; }
CASE
    setup_consumer
    expect allowed-blocks 0 '' 't: all passed'
    check allowed-marker test "$(cat "$case_dir/marker" 2>/dev/null)" = ok

    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: invented-tool --flag # a note
      - run: echo "a # b"
YAML
    setup_consumer
    expect inline-comments 0 '' '' --list
    check comment-labels test "$(cat "$case_dir/out")" = "$(printf '%s\n' 'build: invented-tool --flag' 'build: echo "a')"
    # Keep only the second command to observe its shell parse failure.
    sed '/invented-tool/d' "$case_dir/.github/workflows/ci.yml" > "$case_dir/replaced"
    mv "$case_dir/replaced" "$case_dir/.github/workflows/ci.yml"
    expect comment-shell 1 'FAIL: build: echo "a' ''
}

api_cases() {
    local call reason
    while IFS='|' read -r call reason; do
        new_case
        printf 'gate_init --name t\ngate_workflow %s\n' "$call" >> "$consumer"
        expect "api-$call" 2 "t: gate_workflow: $reason" "t: gate_workflow: $reason"
    done <<'CASES'
--dir '' ci.yml|invalid --dir
--dir one --dir two ci.yml|missing or repeated --dir
--dir|missing or repeated --dir
--pass 1BAD ci.yml|invalid --pass
--pass|invalid --pass
|expected one workflow file
ci.yml shared.yml|expected one workflow file
--unknown ci.yml|unknown option: --unknown
../x.yml|invalid workflow file name
x.txt|invalid workflow file name
CASES
    new_case
    printf 'gate_workflow ci.yml\n' >> "$consumer"
    expect before-init 2 'gate: gate_workflow: gate_init has not run' 'gate: gate_workflow: gate_init has not run'
    new_case
    mkdir "$case_dir/custom"
    printf 'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: touch marker' > "$case_dir/custom/ci.yaml"
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t
directory=$(pwd -P)
cd custom
gate_workflow --dir custom -- ci.yaml
gate_step after :
gate_workflow --dir "$directory/custom" ci.yaml
gate_run
CASE
    expect custom-directory 0 'step 3 ' 't: all passed'
    check custom-root test -f "$case_dir/marker"
}

data_cases() {
    local head=$'jobs:\n  build:' steps=$'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:' attack
    # shellcheck disable=SC2016 # These strings must remain workflow data.
    for attack in '$(touch pwned)' '`touch pwned`'; do
        refuse data-job 't: ci.yml: line 2: cannot be mirrored' "jobs:"$'\n  '"$attack:"
        refuse data-service 't: ci.yml, job build: line 4: cannot be mirrored' "$head"$'\n    services:\n      '"$attack:"
        refuse data-env 't: ci.yml, job build: line 4: env must be single-line NAME: value pairs' "$head"$'\n    env:\n      '"$attack: value"
        refuse data-key 't: ci.yml, job build: line 3: cannot be mirrored' "$head"$'\n    '"$attack: value"
        refuse data-uses "t: ci.yml, job build: step uses '$attack' is not mirrored" "$steps"$'\n      - uses: '"$attack" 'gate_workflow_uses() { printf "%s\n" "$@" >> arguments; return 0; }'
        check data-no-execution test ! -e "$case_dir/pwned"
    done
    refuse printable "t: ci.yml: line 5: a byte outside printable ASCII" "$steps"$'\n      - uses: org/\033@v1'
    new_case
    printf '%s\n' "$steps" '      - uses: org/setup@v1' '      - run: touch marker' > "$case_dir/.github/workflows/ci.yml"
    cat >> "$consumer" <<'CASE'
gate_workflow_uses() { printf '%s\n' "$@" >> arguments; }
CASE
    setup_consumer
    expect hook-arguments 0 '' 't: all passed'
    check hook-literal test "$(cat "$case_dir/arguments" 2>/dev/null)" = "$(printf '%s\n' ci.yml build org/setup@v1)"
    check all-data-no-execution test -z "$(find "$tmp" -name pwned -print)"
}

touched_cases() {
    new_case
    mkdir "$case_dir/src"
    printf x > "$case_dir/src/apple.sh"
    printf 'src/* build: *\n* -\n' > "$case_dir/map"
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: printf build >> marker
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: printf lint >> marker
YAML
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t --map map -- "$@"
gate_workflow ci.yml
gate_run
CASE
    printf 'logs/\nout\nerr\nmarker\n' > "$case_dir/.gitignore"
    git -C "$case_dir" init -q
    git -C "$case_dir" add .
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record fixture setup.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    printf '# change\n' >> "$case_dir/.github/workflows/ci.yml"
    expect workflow-own 0 "t: tier touched falls back to the full tier: .github/workflows/ci.yml is one of the gate's own files" 't: all passed' --tier touched
    check own-full test "$(cat "$case_dir/marker" 2>/dev/null)" = buildlint
    git -C "$case_dir" restore .github/workflows/ci.yml
    rm -f "$case_dir/marker"
    printf x >> "$case_dir/src/apple.sh"
    expect touched-job 0 't: tier touched, 1 paths changed, 1 steps' 't: all passed' --tier touched
    check touched-marker test "$(cat "$case_dir/marker" 2>/dev/null)" = build
    new_case
    mkdir "$case_dir/tools"
    printf 'tools/* -\n* -\n' > "$case_dir/map"
    printf ':\n' > "$case_dir/tools/check.sh"
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: make generate
      - run: bash tools/check.sh
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: touch unrelated
YAML
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t --map map -- "$@"
gate_workflow ci.yml
gate_run
CASE
    printf 'logs/\nout\nerr\n' > "$case_dir/.gitignore"
    git -C "$case_dir" init -q
    git -C "$case_dir" add .
    git -C "$case_dir" -c user.name=Test -c user.email=test@example.invalid -c commit.gpgsign=false commit -qm 'Record fixture setup.'
    git -C "$case_dir" update-ref refs/remotes/origin/main HEAD
    printf '# change\n' >> "$case_dir/tools/check.sh"
    expect touched-whole-job 0 't: tier touched, 1 paths changed, 2 steps' '' --tier touched --list
    check touched-whole-job-list test "$(cat "$case_dir/out")" = "$(printf '%s\n' 'build: make generate' 'build: bash tools/check.sh')"
    check touched-list-no-run test ! -e "$case_dir/unrelated"
}

boundary_cases() {
    local value indent
    local steps=$'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:'
    # shellcheck disable=SC2016 # Scalar prefixes are literal workflow text.
    for value in "'x'" '"x"' '[x]' ']x' '{x}' '}x' '&x' '*x' '!x' '%x' '@x' '`x`' ',x' '?x' ':x' '- x' 'x:'; do
        refuse scalar-start 't: ci.yml, job build: line 5: run: is not a plain one-line value' "$steps"$'\n      - run: '"$value"
    done
    for indent in '       ' '         ' '          '; do
        refuse step-indent 't: ci.yml, job build: line 6: cannot be mirrored' "$steps"$'\n      - run: touch marker\n'"${indent}name: invalid"
    done
    refuse step-shape 't: ci.yml, job build: line 5: cannot be mirrored' "$steps"$'\n      - run touch marker'
    refuse service-shape 't: ci.yml, job build: line 4: cannot be mirrored' $'jobs:\n  build:\n    services:\n      bad.name:'
    refuse env-shape 't: ci.yml, job build: line 4: env must be single-line NAME: value pairs' $'jobs:\n  build:\n    env:\n      APPLE'
    refuse only-setup 't: ci.yml, job build: no run: steps to mirror' "$steps"$'\n      - uses: org/setup@v1' 'gate_workflow_uses() { :; }'
    new_case
    printf '%s\n' "$steps" '      - run: env > marker' > "$case_dir/.github/workflows/ci.yml"
    cat >> "$consumer" <<'CASE'
export total=external index=outside name=original EXTRA_ONE=first EMPTY_ONE=''
export job=consumer previous=caller skip=unchanged
unset ABSENT_ONE
gate_workflow_job() { printf ignored; }
gate_init --name t -- "$@"
gate_workflow --pass total --pass index --pass name --pass EXTRA_ONE --pass EMPTY_ONE --pass ABSENT_ONE ci.yml
EXTRA_ONE=second
gate_step plain 'printf "%s" "$EXTRA_ONE" > plain-marker; printf "%s|%s|%s" "$job" "$previous" "$skip" > plain-context'
gate_run
CASE
    expect pass-caller-values 0 '' 't: all passed'
    for value in total=external index=outside name=original EXTRA_ONE=second EMPTY_ONE=; do
        check "caller-$value" grep -Fxq "$value" "$case_dir/marker"
    done
    check absent-pass test "$(grep -c '^ABSENT_ONE=' "$case_dir/marker" 2>/dev/null)" -eq 0
    check plain-environment test "$(cat "$case_dir/plain-marker" 2>/dev/null)" = second
    check plain-caller-context test "$(cat "$case_dir/plain-context" 2>/dev/null)" = 'consumer|caller|unchanged'
    rm -f "$case_dir/marker" "$case_dir/plain-marker"
    expect silent-hook 0 '' '' --list
    check hook-output-ignored test "$(cat "$case_dir/out")" = "$(printf '%s\n' 'build: env > marker' plain)"
    check boundary-list-no-marker test ! -e "$case_dir/marker"
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  --build:
    runs-on: ubuntu-latest
    steps:
      - run: touch marker
YAML
    setup_consumer
    expect option-like-job 0 '' 't: all passed'
    check option-like-job-marker test -e "$case_dir/marker"
}

byte_cases() {
    local bytes
    local steps=$'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:'
    for bytes in $'\342\200\250' $'\302\205' $'\342\200\251'; do
        refuse unicode-name 't: ci.yml: line 6: a byte outside printable ASCII' "$steps"$'\n      - run: touch marker\n        name: first'"$bytes"'      - run: false'
        refuse unicode-comment 't: ci.yml: line 6: a byte outside printable ASCII' "$steps"$'\n      - run: touch marker\n# hidden'"$bytes"'      - run: false'
    done
    refuse escape-byte 't: ci.yml: line 5: a byte outside printable ASCII' "$steps"$'\n      - run: echo \033'
    refuse value-tab 't: ci.yml: line 5: a carriage return or a tab' "$steps"$'\n      - run: touch marker\t'
    refuse comment-letter 't: ci.yml: line 6: a byte outside printable ASCII' "$steps"$'\n      - run: touch marker\n# caf\303\251'
    new_case
    printf '%s\n      - run: touch marker\000\n' "$steps" > "$case_dir/.github/workflows/ci.yml"
    setup_consumer
    expect nul-byte 2 't: ci.yml: a NUL byte' 't: ci.yml: a NUL byte'
    check nul-no-execution test ! -e "$case_dir/marker"
}

inline_cases() {
    local value key
    local head=$'jobs:\n  build:\n    runs-on: ubuntu-latest'
    local steps="$head"$'\n    steps:'
    local reason='a quoted or flow value must close on its own line'
    refuse continued-name "t: ci.yml, job build: line 6: $reason" "$steps"$'\n      - run: true\n        name: "x\n    needs: y"\n      - run: false'
    refuse continued-flow "t: ci.yml, job build: line 7: $reason" "$steps"$'\n      - run: true\n      - uses: org/setup@v1\n        with: {a: 1,\n    needs: 2}\n      - run: false' 'gate_workflow_uses() { :; }'
    refuse continued-top "t: ci.yml, job build: line 6: $reason" "$steps"$'\n      - run: true\n        name: "x\non: y"\n      - run: false'
    for value in '"x\"' "'it''s'" '"a # b"' '[a,' '{a: 1,'; do
        refuse quote-close "t: ci.yml, job build: line 6: $reason" "$steps"$'\n      - run: touch marker\n        name: '"$value"
    done
    for value in '&a x' '*a' '!tag x'; do
        refuse scalar-decoration 't: ci.yml, job build: line 6: anchors, aliases and tags are not mirrored' "$steps"$'\n      - run: touch marker\n        name: '"$value"
    done
    for key in name run-name on permissions concurrency; do
        refuse top-inline "t: ci.yml: line 1: $reason" "$key: \"x"$'\n'"$steps"$'\n      - run: touch marker'
    done
    for key in name needs permissions timeout-minutes; do
        refuse job-inline "t: ci.yml, job build: line 4: $reason" "$head"$'\n    '"$key: \"x"$'\n    steps:\n      - run: touch marker'
    done
    for key in id with timeout-minutes; do
        refuse step-inline "t: ci.yml, job build: line 6: $reason" "$steps"$'\n      - run: touch marker\n        '"$key: \"x"
    done
    refuse ignored-top-inline "t: ci.yml: line 2: $reason" $'on:\n  push: "x\n'"$steps"$'\n      - run: touch marker'
    refuse ignored-with-inline "t: ci.yml, job build: line 7: $reason" "$steps"$'\n      - run: touch marker\n        with:\n          option: "x'
    refuse ignored-service-inline "t: ci.yml, job build: line 6: $reason" "$head"$'\n    services:\n      apple:\n        image: "x' 'gate_workflow_service() { :; }'
    refuse ignored-job-inline "t: ci.yml, job build: line 5: $reason" "$head"$'\n    strategy:\n      option: "x' 'gate_workflow_job_key() { :; }'
    refuse env-inline "t: ci.yml, job build: line 5: $reason" "$head"$'\n    env:\n      APPLE: "x' 'gate_workflow_env() { :; }'
    refuse nested-runner 't: ci.yml, job build: line 4: cannot be mirrored' "$head"$'\n      more\n    steps:\n      - run: touch marker'
    refuse nested-call 't: ci.yml, job build: line 4: cannot be mirrored' $'jobs:\n  build:\n    uses: ./.github/workflows/shared.yml\n      more'
    new_case
    printf '%s\n' "$head" '    needs: [a, b]' '    name: "Build one"' '    steps:' '      - run: touch marker' '        with: {a: 1}' > "$case_dir/.github/workflows/ci.yml"
    setup_consumer
    expect closed-values 0 '' 't: all passed'
    check closed-values-ran test -e "$case_dir/marker"
}

hook_state_cases() {
    local globals='name=build key=strategy value=org/setup@v1 file=x job=y command=z runner=macos-1 text=t number=1 index=0'
    # shellcheck disable=SC2016 # Hook bodies run in the consumer.
    local hooks='gate_workflow_job() { [ "$2" = "$name" ]; }
gate_workflow_job_key() { [ "$3" = "$key" ]; }
gate_workflow_uses() { [ "$3" = "$value" ]; }'
    refuse consumer-job "t: ci.yml: job 'deploy' is not mirrored; define gate_workflow_job to allow it" $'jobs:\n  deploy:\n    if: false\n    runs-on: ubuntu-latest\n    steps:\n      - uses: other/thing@v9\n      - run: echo deploy' "$globals"$'\n'"$hooks"
    refuse consumer-key "t: ci.yml, job build: job key 'container' is not mirrored" $'jobs:\n  build:\n    container: ignored' "$globals"$'\n'"$hooks"
    refuse consumer-uses "t: ci.yml, job build: step uses 'other/thing@v9' is not mirrored" $'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: other/thing@v9\n      - run: touch marker' "$globals"$'\n'"$hooks"
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    strategy:
    runs-on: ubuntu-latest
    steps:
      - uses: org/setup@v1
      - run: touch marker
YAML
    printf '%s\n%s\n' "$globals" "$hooks" >> "$consumer"
    printf 'gate_init --name t\ngate_workflow ci.yml\ngate_run\n' >> "$consumer"
    expect consumer-globals 0 '' 't: all passed'
    check consumer-globals-ran test -e "$case_dir/marker"
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    runs-on: ubuntu-latest
    steps:
      - run: false
      - run: touch after
    strategy:
  lint:
    runs-on: ubuntu-latest
    steps:
      - run: touch marker
YAML
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_workflow_job_key() { name=other commands=() job= runner=; _GATE_WF_JOB=other _GATE_WF_COMMANDS=(); }
gate_init --name t -- "$@"
gate_workflow ci.yml
gate_run
CASE
    expect mutation-list 0 '' '' --list
    check mutation-labels test "$(cat "$case_dir/out")" = "$(printf '%s\n' 'build: false' 'build: touch after' 'lint: touch marker')"
    expect mutation-run 1 'step 2 skipped build: touch after (an earlier step of job build failed)' ''
    check mutation-skipped test ! -e "$case_dir/after"
    check mutation-other-job test -e "$case_dir/marker"
    new_case
    cat > "$case_dir/.github/workflows/ci.yml" <<'YAML'
jobs:
  build:
    strategy:
    env:
      APPLE: ignored
    services:
      apple:
    runs-on: ubuntu-latest
    steps:
      - uses: org/setup@v1
      - run: env > marker
YAML
    cat >> "$consumer" <<'CASE'
export APPLE=passed
gate_workflow_job() { if read -r _; then return 1; fi; printf job >> hooks; }
gate_workflow_job_key() { if read -r _; then return 1; fi; printf key >> hooks; }
gate_workflow_env() { if read -r _; then return 1; fi; printf env >> hooks; printf supplied; }
gate_workflow_service() { if read -r _; then return 1; fi; printf service >> hooks; }
gate_workflow_uses() { if read -r _; then return 1; fi; printf uses >> hooks; }
gate_init --name t
gate_workflow --pass APPLE ci.yml
gate_run
CASE
    expect hook-stdin 0 '' 't: all passed'
    check every-hook-called test "$(cat "$case_dir/hooks" 2>/dev/null)" = jobkeyenvserviceuses
    check assignment-order grep -Fxq APPLE=supplied "$case_dir/marker"
}

edge_cases() {
    local label exported
    local steps=$'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: touch marker'
    refuse duplicate-top "t: ci.yml: line 2: duplicate key 'name'" $'name: one\nname: two\n'"$steps"
    refuse duplicate-jobs "t: ci.yml: line 6: duplicate key 'jobs'" "$steps"$'\njobs:'
    refuse duplicate-id "t: ci.yml: line 6: duplicate key 'build'" "$steps"$'\n  build:\n    uses: ./.github/workflows/shared.yml'
    refuse duplicate-job-key "t: ci.yml, job build: line 6: duplicate key 'runs-on'" "$steps"$'\n    runs-on: macos-latest'
    refuse duplicate-step-key "t: ci.yml, job build: line 7: duplicate key 'name'" "$steps"$'\n        name: first\n        name: second'
    refuse duplicate-run "t: ci.yml, job build: line 6: duplicate key 'run'" "$steps"$'\n        run: false'
    refuse duplicate-env "t: ci.yml, job build: line 8: duplicate key 'APPLE'" "$steps"$'\n    env:\n      APPLE: one\n      APPLE: two' 'gate_workflow_env() { :; }'
    refuse duplicate-service "t: ci.yml, job build: line 8: duplicate key 'apple'" "$steps"$'\n    services:\n      apple:\n      apple:' 'gate_workflow_service() { :; }'
    refuse second-document 't: ci.yml: line 6: a second YAML document' "$steps"$'\n---'
    refuse repeated-document 't: ci.yml: line 2: a second YAML document' $'---\n---\n'"$steps"
    refuse document-after-name 't: ci.yml: line 2: a second YAML document' $'name: first\n---\n'"$steps"
    refuse no-run-cycle 't: ci.yml: no run: steps found' $'jobs:\n  call:\n    uses: ./.github/workflows/ci.yml'
    refuse missing-step-start 't: ci.yml, job build: line 5: cannot be mirrored' $'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n        run: touch marker'
    new_case
    printf '# comment\n\n---\n%s\n' "$steps" > "$case_dir/.github/workflows/ci.yml"
    setup_consumer
    expect first-document 0 '' 't: all passed'
    for label in macOS-latest MacOS-14 macos-14; do
        new_case
        printf 'jobs:\n  build:\n    runs-on: %s\n    steps:\n      - run: touch marker\n' "$label" > "$case_dir/.github/workflows/ci.yml"
        setup_consumer
        export TEST_OS=Linux
        expect mixed-mac 75 'runs only on macOS' ''
        check mixed-mac-no-run test ! -e "$case_dir/marker"
    done
    new_case
    printf '%s\n' "$steps" '  call:' '    uses: ./.github/workflows/shared.yml' > "$case_dir/.github/workflows/ci.yml"
    printf 'jobs:\n' > "$case_dir/.github/workflows/shared.yml"
    setup_consumer
    expect empty-called 2 't: shared.yml: no jobs: block' 't: shared.yml: no jobs: block'
    check empty-called-no-run test ! -e "$case_dir/marker"
    for exported in no yes; do
        new_case
        printf 'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: env > marker\n' > "$case_dir/.github/workflows/ci.yml"
        printf 'unset MY_SHELL_ONLY\nMY_SHELL_ONLY=1\nunset LANG\nLANG=C\n' >> "$consumer"
        if [ "$exported" = yes ]; then printf 'export MY_SHELL_ONLY LANG\n' >> "$consumer"; fi
        cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t
gate_workflow --pass MY_SHELL_ONLY ci.yml
gate_run
CASE
        expect exported-only 0 '' 't: all passed'
        if [ "$exported" = yes ]; then
            check exported-pass grep -Fxq MY_SHELL_ONLY=1 "$case_dir/marker"
            check exported-base grep -Fxq LANG=C "$case_dir/marker"
        else
            check shell-only-pass test "$(grep -c '^MY_SHELL_ONLY=' "$case_dir/marker")" -eq 0
            check shell-only-base test "$(grep -c '^LANG=' "$case_dir/marker")" -eq 0
        fi
    done
    new_case
    mkdir "$case_dir/other"
    printf 'jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - run: false\n      - run: touch after\n' > "$case_dir/.github/workflows/ci.yml"
    printf '%s\n' "$steps" > "$case_dir/other/ci.yml"
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t
gate_workflow ci.yml
gate_workflow --dir other ci.yml
gate_run
CASE
    expect separate-calls 1 'step 2 skipped build: touch after (an earlier step of job build failed)' ''
    check separate-calls-ran test -e "$case_dir/marker"
    check separate-calls-skipped test ! -e "$case_dir/after"
    new_case
    printf '%s\n' "$steps" > "$case_dir/.github/workflows/ci.yml"
    cat >> "$consumer" <<'CASE'
gate_workflow_job() { :; }
gate_init --name t
shopt -s nocasematch
trap 'if shopt -q nocasematch; then printf unchanged > option; fi' EXIT
gate_workflow ci.yml
gate_run
CASE
    expect nocasematch 2 't: gate_workflow: nocasematch is set' 't: gate_workflow: nocasematch is set'
    check option-preserved test "$(cat "$case_dir/option" 2>/dev/null)" = unchanged
    check nocasematch-no-run test ! -e "$case_dir/marker"
    new_case
    printf '%s\n' "$steps" > "$case_dir/real.yml"
    ln -s ../../real.yml "$case_dir/.github/workflows/ci.yml"
    setup_consumer
    expect linked-workflow 2 't: ci.yml: workflow is a symbolic link' 't: ci.yml: workflow is a symbolic link'
    check linked-no-run test ! -e "$case_dir/marker"
}

consumer_cases() {
    # shellcheck disable=SC2016 # Inspect the actual consumer interpreter.
    printf 'gate_workflow_test: consumer bash %s\n' "$("$runner" -c 'printf %s "${BASH_VERSINFO[0]}"')"
    byte_cases
    inline_cases
    hook_state_cases
    edge_cases
    basic_cases
    environment_cases
    reusable_cases
    shell_cases
    refusal_cases
    allowed_cases
    api_cases
    data_cases
    touched_cases
    boundary_cases
    mode=strict
    byte_cases
    inline_cases
    hook_state_cases
    edge_cases
    basic_cases
    environment_cases
    refuse strict-hook "t: ci.yml: job 'build' is not mirrored; define gate_workflow_job to allow it" $'jobs:\n  build:' 'gate_workflow_job() { return 1; }'
    mode=loose
}
consumer_cases
if [ -x /bin/bash ] && [ "$runner" != /bin/bash ]; then
    runner=/bin/bash
    consumer_cases
else
    printf 'gate_workflow_test: alternate shell unavailable, skipped\n'
fi
if [ "$failed" -ne 0 ]; then exit 1; fi
printf 'gate_workflow_test: all passed\n'
