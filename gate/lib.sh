#!/usr/bin/env bash
# A sourced library: leave the caller's options and traps alone.
if [ "${_GATE_LOADED:-0}" = 1 ] && declare -F gate_run > /dev/null; then
    return 0
fi
_GATE_LOADED=1
_GATE_INITIALIZED=0

# Resolve the directory while sourcing, before the consumer can change directory.
_gate_physical_file() {
    local path=$1 directory
    case "$path" in
        */*) directory=${path%/*}; if [ -z "$directory" ]; then directory=/; fi ;;
        *) directory=. ;;
    esac
    if ! directory=$(cd "$directory" 2> /dev/null && pwd -P); then return 1; fi
    printf '%s/%s\n' "${directory%/}" "${path##*/}"
}
if ! _GATE_LIBRARY_SOURCE=$(_gate_physical_file "${BASH_SOURCE[0]}"); then
    _GATE_LIBRARY_SOURCE=''
fi

_gate_error() {
    printf '%s: %s: %s\n' "${GATE_NAME:-gate}" "$1" "$2" >&2
    exit 2
}

_gate_usage() {
    if [ -n "${_GATE_MAP:-}" ]; then
        printf 'usage: %s [--tier full|land|touched] [--list] | --check-map\n' "${0##*/}" >&2
    else
        printf 'usage: %s [--tier full|land] [--list]\n' "${0##*/}" >&2
    fi
    exit 2
}

gate_init() {
    if [ "$_GATE_INITIALIZED" -eq 1 ]; then _gate_error gate_init 'called twice'; fi
    _GATE_INITIALIZED=1
    local name='' strict_var='' tier_seen=0 map_seen=0 base_seen=0 argument_count
    local own_files=()
    _GATE_MAP=''
    _GATE_BASE=origin/main
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --name)
                if [ -n "$name" ] || [ "$#" -lt 2 ]; then _gate_error gate_init 'missing or repeated --name'; fi
                name=$2
                if [[ ! "$name" =~ ^[a-z0-9][a-z0-9._-]*$ ]]; then
                    _gate_error gate_init 'invalid --name'
                fi
                GATE_NAME=$name
                shift 2
                ;;
            --strict-var)
                if [ -n "$strict_var" ] || [ "$#" -lt 2 ]; then _gate_error gate_init 'missing or repeated --strict-var'; fi
                strict_var=$2
                if [[ ! "$strict_var" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
                    _gate_error gate_init 'invalid --strict-var'
                fi
                shift 2
                ;;
            --map)
                if [ "$map_seen" -ne 0 ] || [ "$#" -lt 2 ]; then _gate_error gate_init 'missing or repeated --map'; fi
                _GATE_MAP=$2
                if [ -z "$_GATE_MAP" ] || [[ "$_GATE_MAP" == *$'\n'* ]]; then _gate_error gate_init 'invalid --map'; fi
                map_seen=1
                shift 2
                ;;
            --own)
                if [ "$#" -lt 2 ]; then _gate_error gate_init 'missing --own'; fi
                if [ -z "$2" ] || [[ "$2" == *$'\n'* ]]; then _gate_error gate_init 'invalid --own'; fi
                own_files[${#own_files[@]}]=$2
                shift 2
                ;;
            --base)
                if [ "$base_seen" -ne 0 ] || [ "$#" -lt 2 ]; then _gate_error gate_init 'missing or repeated --base'; fi
                _GATE_BASE=$2
                if [[ ! "$_GATE_BASE" =~ ^[A-Za-z0-9][A-Za-z0-9._/-]*$ ]]; then _gate_error gate_init 'invalid --base'; fi
                base_seen=1
                shift 2
                ;;
            --) shift; break ;;
            *) _gate_error gate_init "unknown option: $1" ;;
        esac
    done
    if [ -z "$name" ]; then _gate_error gate_init 'missing --name'; fi
    GATE_TIER=full
    GATE_LIST=0
    GATE_CHECK_MAP=0
    argument_count=$#
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --tier)
                if [ "$tier_seen" -ne 0 ] || [ "$#" -lt 2 ]; then _gate_usage; fi
                GATE_TIER=$2
                case "$GATE_TIER" in
                    full|land) ;;
                    touched) if [ -z "$_GATE_MAP" ]; then _gate_usage; fi ;;
                    *) _gate_usage ;;
                esac
                tier_seen=1
                shift 2
                ;;
            --check-map)
                if [ -z "$_GATE_MAP" ] || [ "$argument_count" -ne 1 ]; then _gate_usage; fi
                GATE_CHECK_MAP=1
                shift
                ;;
            --list)
                if [ "$GATE_LIST" -ne 0 ]; then _gate_usage; fi
                GATE_LIST=1
                shift
                ;;
            *) _gate_usage ;;
        esac
    done
    _GATE_STRICT_VAR=$strict_var
    _GATE_STRICT=0
    if [ -n "$strict_var" ] && [ "${!strict_var:-}" = 1 ]; then _GATE_STRICT=1; fi
    if ! _GATE_ROOT=$(pwd -P); then _gate_error gate_init 'cannot record the repository root'; fi
    _GATE_OWN_PATHS=()
    local own_file resolved
    for own_file in "$_GATE_MAP" "${own_files[@]+"${own_files[@]}"}" "$_GATE_LIBRARY_SOURCE"; do
        if [ -n "$own_file" ] && resolved=$(_gate_physical_file "$own_file"); then
            case "$resolved" in
                "${_GATE_ROOT%/}"/*)
                    _GATE_OWN_PATHS[${#_GATE_OWN_PATHS[@]}]=${resolved#"${_GATE_ROOT%/}/"}
                    ;;
            esac
        fi
    done
    _GATE_SKIPPED=0
    _GATE_LABELS=()
    _GATE_COMMANDS=()
    _GATE_KINDS=()
    _GATE_MAC=()
    _GATE_ALWAYS=()
    _GATE_STEP_JOBS=()
    _GATE_STEP_ENV_START=()
    _GATE_STEP_ENV_COUNT=()
    _GATE_STEP_PASS_START=()
    _GATE_STEP_PASS_COUNT=()
    _GATE_ENV=()
    _GATE_PASSES=()
    return 0
}

gate_missing() {
    if [ "$_GATE_STRICT" -eq 1 ] && [ "$GATE_LIST" -eq 0 ] && [ "$GATE_CHECK_MAP" -eq 0 ]; then
        printf '%s: %s is missing on this host (%s=1)\n' "$GATE_NAME" "$1" "$_GATE_STRICT_VAR" >&2
        exit 75
    fi
    if [ "$GATE_LIST" -eq 1 ] || [ "$GATE_CHECK_MAP" -eq 1 ]; then
        printf '%s: %s is missing, skipping %s\n' "$GATE_NAME" "$1" "$2" >&2
    else
        printf '%s: %s is missing, skipping %s\n' "$GATE_NAME" "$1" "$2"
    fi
    _GATE_SKIPPED=$((_GATE_SKIPPED + 1))
    return 0
}

gate_require() {
    if command -v "$1" > /dev/null 2>&1; then return 0; fi
    gate_missing "$1" "$2"
    return 1
}

gate_step() {
    local kind=plain mac=0 always=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --test|--contract)
                if [ "$kind" != plain ]; then _gate_error gate_step 'repeated kind'; fi
                kind=${1#--}
                shift
                ;;
            --mac-only)
                if [ "$mac" -ne 0 ]; then _gate_error gate_step 'repeated --mac-only'; fi
                mac=1
                shift
                ;;
            --always)
                if [ "$always" -ne 0 ]; then _gate_error gate_step 'repeated --always'; fi
                always=1
                shift
                ;;
            --*) _gate_error gate_step "unknown option: $1" ;;
            *) break ;;
        esac
    done
    if [ "$#" -lt 1 ] || [ "$#" -gt 2 ]; then _gate_error gate_step 'expected a label and optional command'; fi
    _gate_register_step "$kind" "$mac" "$always" "$1" "${2-$1}"
}

# Share validation and storage without treating a workflow label as an option.
_gate_register_step() {
    local kind=$1 mac=$2 always=$3 label=$4 command=$5
    if [ -z "$label" ] || [ -z "$command" ] || [[ "$label$command" == *$'\n'* ]]; then
        _gate_error gate_step 'label and command must be non-empty single lines'
    fi
    _GATE_LABELS[${#_GATE_LABELS[@]}]=$label
    _GATE_COMMANDS[${#_GATE_COMMANDS[@]}]=$command
    _GATE_KINDS[${#_GATE_KINDS[@]}]=$kind
    _GATE_MAC[${#_GATE_MAC[@]}]=$mac
    _GATE_ALWAYS[${#_GATE_ALWAYS[@]}]=$always
    _GATE_STEP_JOBS[${#_GATE_STEP_JOBS[@]}]=''
    _GATE_STEP_ENV_START[${#_GATE_STEP_ENV_START[@]}]=0
    _GATE_STEP_ENV_COUNT[${#_GATE_STEP_ENV_COUNT[@]}]=0
    _GATE_STEP_PASS_START[${#_GATE_STEP_PASS_START[@]}]=0
    _GATE_STEP_PASS_COUNT[${#_GATE_STEP_PASS_COUNT[@]}]=0
    return 0
}


_gate_printable() {
    local LC_ALL=C
    local text=$1
    printf '%s' "${text//[!\ -\~]/?}"
}

_gate_workflow_error() {
    local context=$1 reason
    if [ -n "$2" ]; then context="$context, job $2"; fi
    context=$(_gate_printable "$context")
    reason=$(_gate_printable "$3")
    _gate_error "$context" "$reason"
}

_gate_workflow_hook() {
    local hook=$1
    shift
    if declare -F "$hook" > /dev/null; then
        if "$hook" "$@" < /dev/null; then return 0; fi
    fi
    return 1
}

# The parser helpers use the file reader's dynamically scoped job and step state.
_gate_workflow_finish_step() {
    if [ "$step_line" -eq 0 ]; then return 0; fi
    if [ "$((step_run + step_uses))" -ne 1 ]; then
        _gate_workflow_error "$file" "$job" "line $step_line: a step needs exactly one of run: and uses:"
    fi
    if [ "$step_run" -eq 1 ]; then commands[${#commands[@]}]=$command; fi
    step_line=0 step_run=0 step_uses=0 step_key='' command=''
    return 0
}

_gate_workflow_finish_job() {
    if [ -z "$job" ]; then return 0; fi
    _gate_workflow_finish_step
    if [ "$has_steps" -eq 1 ] && [ "$has_call" -eq 1 ]; then
        _gate_workflow_error "$file" "$job" 'both steps and uses'
    fi
    if [ "$has_steps" -eq 1 ] && [ -z "$runner" ]; then
        _gate_workflow_error "$file" "$job" 'steps without runs-on'
    fi
    if [ "${#commands[@]}" -eq 0 ] && [ "$has_call" -eq 0 ]; then
        _gate_workflow_error "$file" "$job" 'no run: steps to mirror'
    fi
    local item index mac=0
    case "$runner" in macos-*) mac=1 ;; esac
    for item in "${commands[@]+"${commands[@]}"}"; do
        index=${#_GATE_LABELS[@]}
        _gate_register_step plain "$mac" 0 "$job: $item" "$item"
        _GATE_STEP_JOBS[index]="$file/$job"
        _GATE_STEP_ENV_START[index]=${#_GATE_ENV[@]}
        _GATE_STEP_ENV_COUNT[index]=${#job_env[@]}
        _GATE_STEP_PASS_START[index]=${#_GATE_PASSES[@]}
        _GATE_STEP_PASS_COUNT[index]=${#passes[@]}
        _GATE_ENV=("${_GATE_ENV[@]+"${_GATE_ENV[@]}"}" "${job_env[@]+"${job_env[@]}"}")
        _GATE_PASSES=("${_GATE_PASSES[@]+"${_GATE_PASSES[@]}"}" "${passes[@]+"${passes[@]}"}")
    done
    job='' job_key='' runner='' has_steps=0 has_call=0
    commands=() job_env=()
    return 0
}

_gate_workflow_value() {
    local value=$1
    value=${value%%[$' \t']#*}
    value=${value#"${value%%[! ]*}"}
    value=${value%"${value##*[! ]}"}
    printf '%s' "$value"
}

_gate_workflow_file() {
    local file=$1 directory=$2 given_dir=$3 path resolved
    local line text leading indent number=0 in_jobs=0 saw_jobs=0 top_key='' job='' job_key='' runner=''
    local has_steps=0 has_call=0 step_line=0 step_run=0 step_uses=0 step_key='' command=''
    local key value name item duplicate
    local commands=() job_env=()
    path="$directory/$file"
    if [ ! -f "$path" ] || [ ! -r "$path" ]; then
        _gate_workflow_error "$file" '' "workflow not found in $given_dir"
    fi
    if resolved=$(_gate_physical_file "$path"); then
        case "$resolved" in
            "${_GATE_ROOT%/}"/*)
                _GATE_OWN_PATHS[${#_GATE_OWN_PATHS[@]}]=${resolved#"${_GATE_ROOT%/}/"}
                ;;
        esac
    fi
    while IFS= read -r line || [ -n "$line" ]; do
        number=$((number + 1))
        leading=${line%%[!$' \t']*}
        if [[ "$line" == *$'\r'* || "$leading" == *$'\t'* ]]; then
            _gate_workflow_error "$file" '' "line $number: a carriage return or a tab"
        fi
        text=${line#"$leading"}
        case "$text" in ''|\#*) continue ;; esac
        indent=${#leading}
        if [ "$indent" -eq 0 ]; then
            _gate_workflow_finish_job
            in_jobs=0 top_key=''
            if [ "$text" = --- ]; then continue; fi
            if [[ ! "$text" =~ ^([A-Za-z_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$file" '' "line $number: cannot be mirrored"
            fi
            key=${BASH_REMATCH[1]}
            top_key=$key
            value=$(_gate_workflow_value "${text#*:}")
            case "$key" in
                jobs)
                    if [ -n "$value" ]; then _gate_workflow_error "$file" '' 'no jobs: block'; fi
                    in_jobs=1 saw_jobs=1
                    ;;
                name|run-name|on|permissions|concurrency) ;;
                *) _gate_workflow_error "$file" '' "line $number: top-level key '$key' is not mirrored" ;;
            esac
            continue
        fi
        if [ "$in_jobs" -eq 0 ]; then
            if [ -z "$top_key" ]; then _gate_workflow_error "$file" '' "line $number: cannot be mirrored"; fi
            continue
        fi
        if [ "$indent" -eq 2 ]; then
            _gate_workflow_finish_job
            if [[ ! "$text" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$file" '' "line $number: cannot be mirrored"
            fi
            name=${BASH_REMATCH[1]}
            value=$(_gate_workflow_value "${text#*:}")
            if [ -n "$value" ]; then _gate_workflow_error "$file" '' "line $number: cannot be mirrored"; fi
            if ! _gate_workflow_hook gate_workflow_job "$file" "$name" > /dev/null; then
                _gate_workflow_error "$file" '' "job '$name' is not mirrored; define gate_workflow_job to allow it"
            fi
            job=$name
            continue
        fi
        if [ -z "$job" ]; then _gate_workflow_error "$file" '' "line $number: cannot be mirrored"; fi
        if [ "$indent" -eq 4 ]; then
            _gate_workflow_finish_step
            if [[ ! "$text" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"
            fi
            key=${BASH_REMATCH[1]}
            value=$(_gate_workflow_value "${text#*:}")
            job_key=$key
            case "$key" in
                name|needs|permissions|timeout-minutes) ;;
                runs-on)
                    if [[ ! "$value" =~ ^[A-Za-z0-9._-]+$ ]]; then
                        _gate_workflow_error "$file" "$job" "runs-on '$value' is not a plain runner label"
                    fi
                    runner=$value
                    ;;
                env|services|steps)
                    if [ -n "$value" ]; then _gate_workflow_error "$file" "$job" "$key must be a block"; fi
                    if [ "$key" = steps ]; then has_steps=1; fi
                    ;;
                uses)
                    name=${value#./.github/workflows/}
                    if [ "$name" = "$value" ] || [[ ! "$name" =~ ^[A-Za-z0-9._-]+\.ya?ml$ ]]; then
                        _gate_workflow_error "$file" "$job" "uses '$value' is not a local reusable workflow"
                    fi
                    has_call=1 duplicate=0
                    for item in "${_GATE_WORKFLOW_QUEUE[@]+"${_GATE_WORKFLOW_QUEUE[@]}"}"; do
                        if [ "$name" = "$item" ]; then duplicate=1; break; fi
                    done
                    if [ "$duplicate" -eq 0 ]; then _GATE_WORKFLOW_QUEUE[${#_GATE_WORKFLOW_QUEUE[@]}]=$name; fi
                    ;;
                *)
                    if ! _gate_workflow_hook gate_workflow_job_key "$file" "$job" "$key" > /dev/null; then
                        _gate_workflow_error "$file" "$job" "job key '$key' is not mirrored"
                    fi
                    ;;
            esac
            continue
        fi
        # Only the nested blocks below can ignore otherwise invalid indentation.
        if [ "$indent" -gt 4 ]; then
            case "$job_key" in
                env)
                    if [ "$indent" -ne 6 ] || [[ ! "$text" =~ ^([A-Za-z_][A-Za-z0-9_]*):( .*)?$ ]]; then
                        _gate_workflow_error "$file" "$job" "line $number: env must be single-line NAME: value pairs"
                    fi
                    name=${BASH_REMATCH[1]}
                    if ! value=$(_gate_workflow_hook gate_workflow_env "$file" "$job" "$name") || [[ "$value" == *$'\n'* ]]; then
                        _gate_workflow_error "$file" "$job" "env variable $name is not mirrored"
                    fi
                    job_env[${#job_env[@]}]="$name=$value"
                    continue
                    ;;
                services)
                    if [ "$indent" -gt 6 ]; then continue; fi
                    if [ "$indent" -ne 6 ] || [[ ! "$text" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                        _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"
                    fi
                    name=${BASH_REMATCH[1]}
                    value=$(_gate_workflow_value "${text#*:}")
                    if [ -n "$value" ]; then _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"; fi
                    if ! _gate_workflow_hook gate_workflow_service "$file" "$job" "$name" > /dev/null; then
                        _gate_workflow_error "$file" "$job" "service '$name' is not mirrored"
                    fi
                    continue
                    ;;
                steps) ;;
                '') ;;
                *) continue ;;
            esac
        fi
        if [ "$job_key" != steps ]; then _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"; fi
        if [ "$indent" -ge 10 ] && [ "$step_key" = with ]; then continue; fi
        if [ "$indent" -eq 6 ]; then
            _gate_workflow_finish_step
            if [[ "$text" == '- {'* ]] || [ "$text" = - ]; then
                _gate_workflow_error "$file" "$job" "line $number: write the step in block style"
            fi
            if [[ "$text" != '- '* ]]; then _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"; fi
            text=${text#- }
            step_line=$number
        elif [ "$indent" -ne 8 ] || [ "$step_line" -eq 0 ]; then
            _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"
        fi
        if [[ ! "$text" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
            _gate_workflow_error "$file" "$job" "line $number: cannot be mirrored"
        fi
        key=${BASH_REMATCH[1]}
        value=$(_gate_workflow_value "${text#*:}")
        step_key=$key
        case "$key" in
            name|id|with|timeout-minutes) ;;
            run)
                case "$value" in
                    ''|\|*|\>*) _gate_workflow_error "$file" "$job" "line $number: a multi-line run: step" ;;
                    [\'\"\[\]\{\}\&\*\!%\@\`\#\,\?:]*|'- '*|*': '*|*:)
                        _gate_workflow_error "$file" "$job" "line $number: run: is not a plain one-line value"
                        ;;
                esac
                # shellcheck disable=SC2016 # The expression marker is data.
                if [[ "$value" == *'${{'* ]]; then
                    _gate_workflow_error "$file" "$job" "line $number: run: uses an expression"
                fi
                step_run=1 command=$value
                ;;
            uses)
                if [[ ! "$value" =~ ^[A-Za-z0-9._/@:-]+$ ]] ||
                    ! _gate_workflow_hook gate_workflow_uses "$file" "$job" "$value" > /dev/null; then
                    _gate_workflow_error "$file" "$job" "step uses '$value' is not mirrored"
                fi
                step_uses=1
                ;;
            *) _gate_workflow_error "$file" "$job" "step key '$key' is not mirrored" ;;
        esac
    done < "$path"
    _gate_workflow_finish_job
    if [ "$saw_jobs" -eq 0 ]; then _gate_workflow_error "$file" '' 'no jobs: block'; fi
    return 0
}

gate_workflow() {
    if [ "$_GATE_INITIALIZED" -eq 0 ]; then _gate_error gate_workflow 'gate_init has not run'; fi
    local directory=.github/workflows given_dir dir_seen=0 file index before=${#_GATE_LABELS[@]}
    local passes=()
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --dir)
                if [ "$dir_seen" -eq 1 ] || [ "$#" -lt 2 ]; then _gate_error gate_workflow 'missing or repeated --dir'; fi
                directory=$2 dir_seen=1
                if [ -z "$directory" ] || [[ "$directory" == *$'\n'* ]]; then _gate_error gate_workflow 'invalid --dir'; fi
                shift 2
                ;;
            --pass)
                if [ "$#" -lt 2 ] || [[ ! "$2" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then _gate_error gate_workflow 'invalid --pass'; fi
                passes[${#passes[@]}]=$2
                shift 2
                ;;
            --) shift; break ;;
            -*) _gate_error gate_workflow "unknown option: $1" ;;
            *) break ;;
        esac
    done
    if [ "$#" -ne 1 ]; then _gate_error gate_workflow 'expected one workflow file'; fi
    file=$1
    if [[ ! "$file" =~ ^[A-Za-z0-9._-]+\.ya?ml$ ]]; then _gate_error gate_workflow 'invalid workflow file name'; fi
    given_dir=$directory
    case "$directory" in /*) ;; *) directory="$_GATE_ROOT/$directory" ;; esac
    _GATE_WORKFLOW_QUEUE=("$file")
    for ((index = 0; index < ${#_GATE_WORKFLOW_QUEUE[@]}; index++)); do
        _gate_workflow_file "${_GATE_WORKFLOW_QUEUE[index]}" "$directory" "$given_dir"
    done
    if [ "${#_GATE_LABELS[@]}" -eq "$before" ]; then _gate_workflow_error "$file" '' 'no run: steps found'; fi
    return 0
}

# Snapshot the caller's environment before gate_run introduces local variables.
_gate_workflow_environment() {
    local _GATE_EXEC_NAME
    _GATE_BASE_ENV=()
    _GATE_PASS_VALUES=()
    _GATE_PASS_SET=()
    for _GATE_EXEC_NAME in HOME USER LOGNAME PATH TMPDIR LANG TERM; do
        if [ "${!_GATE_EXEC_NAME+set}" = set ]; then
            _GATE_BASE_ENV[${#_GATE_BASE_ENV[@]}]="$_GATE_EXEC_NAME=${!_GATE_EXEC_NAME}"
        fi
    done
    _GATE_BASE_ENV[${#_GATE_BASE_ENV[@]}]=CI=true
    for _GATE_EXEC_NAME in "${_GATE_PASSES[@]+"${_GATE_PASSES[@]}"}"; do
        _GATE_PASS_SET[${#_GATE_PASS_SET[@]}]=${!_GATE_EXEC_NAME+set}
        _GATE_PASS_VALUES[${#_GATE_PASS_VALUES[@]}]="$_GATE_EXEC_NAME=${!_GATE_EXEC_NAME-}"
    done
    return 0
}

_gate_command() {
    if [ -z "${_GATE_STEP_JOBS[$1]}" ]; then
        bash -c "${_GATE_COMMANDS[$1]}"
        return "$?"
    fi
    local index=$1 start count offset
    local assignments=("${_GATE_BASE_ENV[@]+"${_GATE_BASE_ENV[@]}"}")
    start=${_GATE_STEP_PASS_START[index]} count=${_GATE_STEP_PASS_COUNT[index]}
    for ((offset = 0; offset < count; offset++)); do
        if [ "${_GATE_PASS_SET[start + offset]}" = set ]; then
            assignments[${#assignments[@]}]=${_GATE_PASS_VALUES[start + offset]}
        fi
    done
    start=${_GATE_STEP_ENV_START[index]} count=${_GATE_STEP_ENV_COUNT[index]}
    for ((offset = 0; offset < count; offset++)); do
        assignments[${#assignments[@]}]=${_GATE_ENV[start + offset]}
    done
    env -i "${assignments[@]+"${assignments[@]}"}" "$BASH" -e -c "${_GATE_COMMANDS[index]}"
}

_gate_root_is_top() {
    local top
    if ! top=$(git -C "$_GATE_ROOT" rev-parse --show-toplevel 2> /dev/null) ||
        ! top=$(cd "$top" && pwd -P); then return 1; fi
    [ "$top" = "$_GATE_ROOT" ]
}

# Both map users share one parser; patterns remain data throughout.
_gate_map_problem() {
    local problem
    problem=$(_gate_printable "$1")
    if [ "$GATE_CHECK_MAP" -eq 1 ]; then
        printf 'FAIL: %s\n' "$problem" >&2
        _GATE_PROBLEMS=$((_GATE_PROBLEMS + 1))
    elif [ -z "$_GATE_REASON" ]; then
        _GATE_REASON=$problem
    fi
    return 0
}

_gate_read_map() {
    local file line path_pattern step_pattern label path matched number=0
    _GATE_PATH_PATTERNS=()
    _GATE_STEP_PATTERNS=()
    if shopt -q nocasematch; then
        _gate_map_problem 'nocasematch is set'
        return 0
    fi
    file=$_GATE_MAP
    case "$file" in /*) ;; *) file="$_GATE_ROOT/$file" ;; esac
    if [ ! -f "$file" ] || [ ! -r "$file" ]; then
        _gate_map_problem "cannot read $_GATE_MAP"
        return 0
    fi
    if ! while IFS= read -r line || [ -n "$line" ]; do
        number=$((number + 1))
        line=${line#"${line%%[!$' \t']*}"}
        line=${line%"${line##*[!$' \t']}"}
        case "$line" in ''|\#*) continue ;; esac
        path_pattern=${line%%[$' \t']*}
        step_pattern=${line#"$path_pattern"}
        step_pattern=${step_pattern#"${step_pattern%%[!$' \t']*}"}
        if [ -z "$step_pattern" ]; then
            _gate_map_problem "$_GATE_MAP line $number has no step"
        fi
        _GATE_PATH_PATTERNS[${#_GATE_PATH_PATTERNS[@]}]=$path_pattern
        _GATE_STEP_PATTERNS[${#_GATE_STEP_PATTERNS[@]}]=$step_pattern
        if [ -n "$step_pattern" ] && [ "$step_pattern" != - ]; then
            matched=0
            for label in "${_GATE_LABELS[@]+"${_GATE_LABELS[@]}"}"; do
                # shellcheck disable=SC2053 # The map's right-hand side is a pattern, not code.
                if [[ "$label" == $step_pattern ]]; then matched=1; break; fi
            done
            if [ "$matched" -eq 0 ]; then
                _gate_map_problem "$_GATE_MAP line $number names no registered step: $step_pattern"
            fi
        fi
        if [ "$GATE_CHECK_MAP" -eq 1 ]; then
            matched=0
            for path in "${_GATE_TRACKED[@]+"${_GATE_TRACKED[@]}"}"; do
                # shellcheck disable=SC2053 # The map's right-hand side is a pattern, not code.
                if [[ "$path" == $path_pattern ]]; then matched=1; break; fi
            done
            if [ "$matched" -eq 0 ]; then
                _gate_map_problem "$_GATE_MAP line $number matches no tracked file: $path_pattern"
            fi
        fi
    done < "$file"; then
        _gate_map_problem "cannot read $_GATE_MAP"
    fi
    return 0
}

_gate_check_map() {
    local file path
    if ! command -v git > /dev/null 2>&1; then _gate_error gate_run 'git is unavailable'; fi
    if ! _gate_root_is_top; then _gate_error gate_run 'the gate root is not the top of the repository'; fi
    if ! file=$(mktemp "${TMPDIR:-/tmp}/$GATE_NAME.XXXXXX"); then
        _gate_error gate_run 'could not list tracked files'
    fi
    if ! git -C "$_GATE_ROOT" ls-files -z > "$file"; then
        if ! rm -f "$file"; then _gate_error gate_run 'cannot remove temporary files'; fi
        _gate_error gate_run 'could not list tracked files'
    fi
    _GATE_TRACKED=()
    while IFS= read -r -d '' path; do
        _GATE_TRACKED[${#_GATE_TRACKED[@]}]=$path
    done < "$file"
    if ! rm -f "$file"; then _gate_error gate_run 'cannot remove temporary files'; fi
    _GATE_PROBLEMS=0
    _gate_read_map
    if [ "$_GATE_PROBLEMS" -gt 0 ]; then
        printf '%s: map has %s problems\n' "$GATE_NAME" "$_GATE_PROBLEMS" >&2
        exit 1
    fi
    printf '%s: map ok\n' "$GATE_NAME"
    exit 0
}

_gate_select_touched() {
    local base file path previous duplicate index row mapped word text candidate commit count=0 own_file
    # This host's locales cannot distinguish this byte guard; keep it defensive.
    local LC_ALL=C
    local paths=() chosen=() words=() candidates=()
    _GATE_REASON=''
    if ! command -v git > /dev/null 2>&1; then
        _GATE_REASON='git is unavailable'
        return 0
    fi
    _gate_read_map
    if [ -n "$_GATE_REASON" ]; then return 0; fi
    if ! _gate_root_is_top; then
        _GATE_REASON='the gate root is not the top of the repository'
        return 0
    fi
    case "$_GATE_BASE" in
        refs/*) candidates=("$_GATE_BASE") ;;
        *) candidates=("refs/$_GATE_BASE" "refs/tags/$_GATE_BASE" "refs/heads/$_GATE_BASE" "refs/remotes/$_GATE_BASE") ;;
    esac
    base=''
    for candidate in "${candidates[@]+"${candidates[@]}"}"; do
        if commit=$(git -C "$_GATE_ROOT" rev-parse --verify --quiet "$candidate^{commit}" 2> /dev/null); then
            count=$((count + 1))
            base=$commit
        fi
    done
    if [ "$count" -gt 1 ]; then
        _GATE_REASON="the base $_GATE_BASE is ambiguous"
        return 0
    fi
    if [ "$count" -eq 0 ] || ! base=$(git -C "$_GATE_ROOT" merge-base HEAD "$base" 2> /dev/null); then
        _GATE_REASON="no merge base with $_GATE_BASE"
        return 0
    fi
    if ! file=$(mktemp "${TMPDIR:-/tmp}/$GATE_NAME.XXXXXX"); then
        _GATE_REASON='could not list the change'
        return 0
    fi
    if ! git -C "$_GATE_ROOT" diff -z --name-only --no-renames --no-ext-diff --no-textconv --ignore-submodules=none --no-relative "$base" -- > "$file" 2> /dev/null ||
        ! git -C "$_GATE_ROOT" ls-files -z --others --exclude-standard >> "$file" 2> /dev/null; then
        _GATE_REASON='could not list the change'
    else
        while IFS= read -r -d '' path; do
            case "$path" in
                *[!\ -\~]*|*\"*|*\\*) _GATE_REASON='a changed path has a character the map cannot match'; break ;;
            esac
            duplicate=0
            for previous in "${paths[@]+"${paths[@]}"}"; do
                if [ "$previous" = "$path" ]; then duplicate=1; break; fi
            done
            if [ "$duplicate" -eq 0 ]; then paths[${#paths[@]}]=$path; fi
        done < "$file"
    fi
    if ! rm -f "$file"; then _gate_error gate_run 'cannot remove temporary files'; fi
    if [ -n "$_GATE_REASON" ]; then return 0; fi
    for path in "${paths[@]+"${paths[@]}"}"; do
        for own_file in "${_GATE_OWN_PATHS[@]+"${_GATE_OWN_PATHS[@]}"}"; do
            if [ "$path" = "$own_file" ]; then
                _GATE_REASON="$path is one of the gate's own files"
                return 0
            fi
        done
        mapped=0
        for ((row = 0; row < ${#_GATE_PATH_PATTERNS[@]}; row++)); do
            # shellcheck disable=SC2053 # The map's right-hand side is a pattern, not code.
            if [[ "$path" == ${_GATE_PATH_PATTERNS[row]} ]]; then
                mapped=1
                if [ "${_GATE_STEP_PATTERNS[row]}" != - ]; then
                    for ((index = 0; index < ${#_GATE_LABELS[@]}; index++)); do
                        # shellcheck disable=SC2053 # The map's right-hand side is a pattern, not code.
                        if [[ "${_GATE_LABELS[index]}" == ${_GATE_STEP_PATTERNS[row]} ]]; then chosen[index]=1; fi
                    done
                fi
            fi
        done
        for ((index = 0; index < ${#_GATE_LABELS[@]}; index++)); do
            if [[ "$path" == *' '* ]] && [[ "$path" == */* || "$path" == *.[A-Za-z]* ]] &&
                [[ " ${_GATE_LABELS[index]} " == *" $path "* ]]; then
                chosen[index]=1
                mapped=1
            fi
            for text in "${_GATE_LABELS[index]}" "${_GATE_COMMANDS[index]}"; do
                IFS=$' \t\n' read -r -a words <<< "$text"
                for word in "${words[@]+"${words[@]}"}"; do
                    case "$word" in
                        */*|*.[A-Za-z]*) if [ "$word" = "$path" ]; then chosen[index]=1; mapped=1; fi ;;
                    esac
                done
            done
        done
        if [ "$mapped" -eq 0 ]; then
            _GATE_REASON="$path has no entry in $_GATE_MAP"
            return 0
        fi
    done
    _GATE_SELECTED=()
    for ((index = 0; index < ${#_GATE_LABELS[@]}; index++)); do
        if [ "${chosen[index]:-0}" -eq 1 ] || [ "${_GATE_ALWAYS[index]}" -eq 1 ]; then
            _GATE_SELECTED[${#_GATE_SELECTED[@]}]=$index
        fi
    done
    _GATE_PATH_COUNT=${#paths[@]}
    return 0
}

_gate_selection() {
    local index notice
    _GATE_SELECTED=()
    for ((index = 0; index < ${#_GATE_LABELS[@]}; index++)); do
        _GATE_SELECTED[index]=$index
    done
    if [ "$GATE_TIER" != touched ]; then return 0; fi
    _gate_select_touched
    if [ -n "$_GATE_REASON" ]; then
        notice="$GATE_NAME: tier touched falls back to the full tier: $_GATE_REASON"
    else
        notice="$GATE_NAME: tier touched, $_GATE_PATH_COUNT paths changed, ${#_GATE_SELECTED[@]} steps"
    fi
    if [ "$GATE_LIST" -eq 1 ]; then printf '%s\n' "$notice" >&2
    else printf '%s\n' "$notice"; fi
    return 0
}

_gate_tail() {
    local line
    while IFS= read -r line || [ -n "$line" ]; do
        printf '    %s\n' "$line" >&2
    done < <(tail -n 15 "$1")
}

_gate_skips() {
    if [ ! -r "$1" ]; then _gate_error gate_run "cannot read $1"; fi
    local line
    while IFS= read -r line || [ -n "$line" ]; do
        case "$line" in
            *', skipped')
                printf '    %s\n' "$line"
                _GATE_SKIPPED=$((_GATE_SKIPPED + 1))
                ;;
        esac
    done < "$1"
}

_gate_slowest() {
    local count=$1 rank index best position
    local selected=()
    printf '%s: slowest steps\n' "$GATE_NAME"
    for ((rank = 0; rank < count && rank < 5; rank++)); do
        best=-1
        for ((position = 0; position < count; position++)); do
            index=${_GATE_RAN_STEPS[position]}
            if [ "${selected[index]:-0}" -eq 0 ]; then
                if [ "$best" -lt 0 ] || [ "${_GATE_TIMES[index]}" -gt "${_GATE_TIMES[best]}" ]; then
                    best=$index
                fi
            fi
        done
        selected[best]=1
        printf '  %ss %s\n' "${_GATE_TIMES[best]}" "${_GATE_LABELS[best]}"
    done
    return 0
}

gate_run() {
    _gate_workflow_environment
    local total=${#_GATE_LABELS[@]} index position os logdir='' log started status outcome
    local ran=0 red=0 stop='' _GATE_RUN_JOB _GATE_RUN_PREVIOUS _GATE_RUN_SKIP
    local _GATE_FAILED_JOBS=()
    _GATE_TIMES=()
    _GATE_RAN_STEPS=()
    if [ "$total" -eq 0 ]; then
        printf '%s: no steps registered\n' "$GATE_NAME" >&2
        exit 2
    fi
    if [ "$GATE_CHECK_MAP" -eq 1 ]; then
        if [ "$_GATE_SKIPPED" -gt 0 ]; then
            printf '%s: cannot check the map on this host: a tool is missing\n' "$GATE_NAME" >&2
            exit 75
        fi
        _gate_check_map
    fi
    _gate_selection
    total=${#_GATE_SELECTED[@]}
    if [ "$GATE_LIST" -eq 1 ]; then
        for index in "${_GATE_SELECTED[@]+"${_GATE_SELECTED[@]}"}"; do
            printf '%s\n' "${_GATE_LABELS[index]}"
        done
        exit 0
    fi
    for index in "${_GATE_SELECTED[@]+"${_GATE_SELECTED[@]}"}"; do
        if [ "${_GATE_MAC[index]}" -eq 1 ]; then
            if ! os=$(uname -s); then _gate_error gate_run 'cannot identify this host'; fi
            if [ "$os" != Darwin ]; then
                printf '%s: %s runs only on macOS and this host is %s; exit 75 so another host can run the gate\n' \
                    "$GATE_NAME" "${_GATE_LABELS[index]}" "$os" >&2
                exit 75
            fi
        fi
    done
    if [ "$total" -gt 0 ] && ! logdir=$(mktemp -d "${TMPDIR:-/tmp}/$GATE_NAME.XXXXXX"); then
        _gate_error gate_run 'cannot create a log directory'
    fi
    for ((position = 0; position < total; position++)); do
        index=${_GATE_SELECTED[position]}
        ran=$((position + 1))
        _GATE_RUN_JOB=${_GATE_STEP_JOBS[index]}
        _GATE_RUN_SKIP=0
        if [ -n "$_GATE_RUN_JOB" ]; then
            for _GATE_RUN_PREVIOUS in "${_GATE_FAILED_JOBS[@]+"${_GATE_FAILED_JOBS[@]}"}"; do
                if [ "$_GATE_RUN_PREVIOUS" = "$_GATE_RUN_JOB" ]; then _GATE_RUN_SKIP=1; break; fi
            done
        fi
        if [ "$_GATE_RUN_SKIP" -eq 1 ]; then
            printf 'step %s skipped %s (an earlier step of job %s failed)\n' "$ran" "${_GATE_LABELS[index]}" "${_GATE_RUN_JOB#*/}"
            continue
        fi
        log="$logdir/step-$ran.log"
        started=$SECONDS
        status=0
        if (cd "$_GATE_ROOT" && _gate_command "$index") < /dev/null > "$log" 2>&1; then
            status=0
        else
            status=$?
        fi
        _GATE_TIMES[index]=$((SECONDS - started))
        _GATE_RAN_STEPS[${#_GATE_RAN_STEPS[@]}]=$index
        printf 'step %s %ss %s\n' "$ran" "${_GATE_TIMES[index]}" "${_GATE_LABELS[index]}"
        outcome=red
        if [ "$status" -eq 0 ]; then
            outcome=pass
        elif [ "${_GATE_KINDS[index]}" != plain ]; then
            if [ "$status" -eq 75 ]; then
                outcome=unavailable
            elif [ "${_GATE_KINDS[index]}" = contract ] && [ "$status" -ne 1 ]; then
                outcome=internal
            fi
        fi
        case "$outcome" in
            pass)
                if [ "${_GATE_KINDS[index]}" != plain ]; then _gate_skips "$log"; fi
                ;;
            red)
                red=$((red + 1))
                if [ -n "$_GATE_RUN_JOB" ]; then _GATE_FAILED_JOBS[${#_GATE_FAILED_JOBS[@]}]=$_GATE_RUN_JOB; fi
                printf 'FAIL: %s (whole log: %s)\n' "${_GATE_LABELS[index]}" "$log" >&2
                _gate_tail "$log"
                ;;
            unavailable|internal)
                stop=$outcome
                if [ "$outcome" = unavailable ]; then
                    printf 'UNAVAILABLE: %s (exit 75: this host cannot run it; whole log: %s)\n' "${_GATE_LABELS[index]}" "$log" >&2
                else
                    printf 'ERROR: %s (exit %s; whole log: %s)\n' "${_GATE_LABELS[index]}" "$status" "$log" >&2
                fi
                _gate_tail "$log"
                printf '%s: stopped after step %s; %s steps not run\n' "$GATE_NAME" "$ran" "$((total - ran))" >&2
                break
                ;;
        esac
    done
    if [ "${#_GATE_RAN_STEPS[@]}" -gt 0 ]; then _gate_slowest "${#_GATE_RAN_STEPS[@]}"; fi
    if [ "$stop" = internal ]; then
        printf '%s: internal error (logs: %s)\n' "$GATE_NAME" "$logdir" >&2
        exit 2
    elif [ "$red" -gt 0 ]; then
        printf '%s: %s failed (logs: %s)\n' "$GATE_NAME" "$red" "$logdir" >&2
        exit 1
    elif [ "$stop" = unavailable ]; then
        printf '%s: this host cannot run the gate (logs: %s)\n' "$GATE_NAME" "$logdir" >&2
        exit 75
    fi
    if [ -n "$logdir" ] && ! rm -rf "$logdir"; then _gate_error gate_run 'cannot remove the log directory'; fi
    if [ "$_GATE_SKIPPED" -gt 0 ]; then
        printf '%s: all passed, %s skipped\n' "$GATE_NAME" "$_GATE_SKIPPED"
    else
        printf '%s: all passed\n' "$GATE_NAME"
    fi
    exit 0
}
