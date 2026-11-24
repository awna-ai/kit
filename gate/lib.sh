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
    if ! directory=$(CDPATH='' cd -- "$directory" 2> /dev/null && pwd -P); then return 1; fi
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
    _GATE_WORKFLOW_CALL=0
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
    local _GATE_WF_HOOK=$1
    shift
    if declare -F "$_GATE_WF_HOOK" > /dev/null; then
        if ( "$_GATE_WF_HOOK" "$@" ) < /dev/null; then return 0; fi
    fi
    return 1
}

# The parser helpers use the file reader's dynamically scoped job and step state.
_gate_workflow_finish_step() {
    if [ "$_GATE_WF_STEP_LINE" -eq 0 ]; then return 0; fi
    if [ "$((_GATE_WF_STEP_RUN + _GATE_WF_STEP_USES))" -ne 1 ]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_STEP_LINE: a step needs exactly one of run: and uses:"
    fi
    if [ "$_GATE_WF_STEP_RUN" -eq 1 ]; then _GATE_WF_COMMANDS[${#_GATE_WF_COMMANDS[@]}]=$_GATE_WF_COMMAND; fi
    _GATE_WF_STEP_KEYS=''
    _GATE_WF_STEP_LINE=0 _GATE_WF_STEP_RUN=0 _GATE_WF_STEP_USES=0 _GATE_WF_STEP_KEY='' _GATE_WF_COMMAND=''
    return 0
}

_gate_workflow_finish_job() {
    if [ -z "$_GATE_WF_JOB" ]; then return 0; fi
    _gate_workflow_finish_step
    if [ "$_GATE_WF_HAS_STEPS" -eq 1 ] && [ "$_GATE_WF_HAS_CALL" -eq 1 ]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" 'both steps and uses'
    fi
    if [ "$_GATE_WF_HAS_STEPS" -eq 1 ] && [ -z "$_GATE_WF_RUNNER" ]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" 'steps without runs-on'
    fi
    if [ "${#_GATE_WF_COMMANDS[@]}" -eq 0 ] && [ "$_GATE_WF_HAS_CALL" -eq 0 ]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" 'no run: steps to mirror'
    fi
    local _GATE_WF_ITEM _GATE_WF_INDEX _GATE_WF_MAC=0
    case "$_GATE_WF_RUNNER" in [Mm][Aa][Cc][Oo][Ss]-*) _GATE_WF_MAC=1 ;; esac
    for _GATE_WF_ITEM in "${_GATE_WF_COMMANDS[@]+"${_GATE_WF_COMMANDS[@]}"}"; do
        _GATE_WF_INDEX=${#_GATE_LABELS[@]}
        _gate_register_step plain "$_GATE_WF_MAC" 0 "$_GATE_WF_JOB: $_GATE_WF_ITEM" "$_GATE_WF_ITEM"
        _GATE_STEP_JOBS[_GATE_WF_INDEX]="$_GATE_WORKFLOW_CALL/$_GATE_WF_FILE/$_GATE_WF_JOB"
        _GATE_STEP_ENV_START[_GATE_WF_INDEX]=${#_GATE_ENV[@]}
        _GATE_STEP_ENV_COUNT[_GATE_WF_INDEX]=${#_GATE_WF_JOB_ENV[@]}
        _GATE_STEP_PASS_START[_GATE_WF_INDEX]=${#_GATE_PASSES[@]}
        _GATE_STEP_PASS_COUNT[_GATE_WF_INDEX]=${#_GATE_WF_PASSES[@]}
        _GATE_ENV=("${_GATE_ENV[@]+"${_GATE_ENV[@]}"}" "${_GATE_WF_JOB_ENV[@]+"${_GATE_WF_JOB_ENV[@]}"}")
        _GATE_PASSES=("${_GATE_PASSES[@]+"${_GATE_PASSES[@]}"}" "${_GATE_WF_PASSES[@]+"${_GATE_WF_PASSES[@]}"}")
    done
    _GATE_WF_JOB='' _GATE_WF_JOB_KEY='' _GATE_WF_RUNNER='' _GATE_WF_HAS_STEPS=0 _GATE_WF_HAS_CALL=0
    _GATE_WF_COMMANDS=() _GATE_WF_JOB_ENV=()
    _GATE_WF_JOB_KEYS='' _GATE_WF_ENV_NAMES='' _GATE_WF_SERVICE_NAMES=''
    return 0
}

_gate_workflow_value() {
    local _GATE_WF_VALUE=$1
    _GATE_WF_VALUE=${_GATE_WF_VALUE%%[$' \t']#*}
    _GATE_WF_VALUE=${_GATE_WF_VALUE#"${_GATE_WF_VALUE%%[! ]*}"}
    _GATE_WF_VALUE=${_GATE_WF_VALUE%"${_GATE_WF_VALUE##*[! ]}"}
    printf '%s' "$_GATE_WF_VALUE"
}

# Inline YAML scalars may not hide structure on later physical lines.
_gate_workflow_inline() {
    local LC_ALL=C
    local _GATE_WF_PATTERN=''
    case "$1" in
        \&*|\**|\!*)
            _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: anchors, aliases and tags are not mirrored"
            ;;
        \"*) _GATE_WF_PATTERN='^"[^"\\]*"$' ;;
        \'*) _GATE_WF_PATTERN="^'[^']*'$" ;;
        \[*) _GATE_WF_PATTERN="^\\[[^][{}\"']*\\]$" ;;
        \{*) _GATE_WF_PATTERN="^\\{[^][{}\"']*\\}$" ;;
    esac
    if [ -n "$_GATE_WF_PATTERN" ] && [[ ! "$1" =~ $_GATE_WF_PATTERN ]]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: a quoted or flow value must close on its own line"
    fi
    return 0
}

# Check keys and standalone values even in blocks whose contents are ignored.
# Structural callers keep their existing, context-specific inline-value rules.
_gate_workflow_line() {
    local LC_ALL=C
    local _GATE_WF_TEXT=$1 _GATE_WF_KEY _GATE_WF_VALUE
    local _GATE_WF_COLUMN=$_GATE_WF_INDENT
    while [[ "$_GATE_WF_TEXT" == '- '* ]]; do
        _GATE_WF_TEXT=${_GATE_WF_TEXT#-}
        _GATE_WF_TEXT=${_GATE_WF_TEXT#"${_GATE_WF_TEXT%%[! ]*}"}
    done
    _GATE_WF_COLUMN=$((_GATE_WF_COLUMN + ${#1} - ${#_GATE_WF_TEXT}))
    if [ "$_GATE_WF_TEXT" = - ]; then return 0; fi
    case "$_GATE_WF_TEXT" in
        '?'|'? '*|':'|': '*)
            _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: explicit keys are not mirrored"
            ;;
        [\"\'\[\{\&\*\!\|\>%\@\`]*)
            _GATE_WF_VALUE=$(_gate_workflow_value "$_GATE_WF_TEXT")
            case "$_GATE_WF_VALUE" in
                \|*|\>*) _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored" ;;
            esac
            _gate_workflow_inline "$_GATE_WF_VALUE"
            return 0
            ;;
    esac
    case "$_GATE_WF_TEXT" in
        *': '*) _GATE_WF_KEY=${_GATE_WF_TEXT%%: *}; _GATE_WF_VALUE=${_GATE_WF_TEXT#*: } ;;
        *:) _GATE_WF_KEY=${_GATE_WF_TEXT%:}; _GATE_WF_VALUE='' ;;
        *)
            _GATE_WF_VALUE=$(_gate_workflow_value "$_GATE_WF_TEXT")
            if [[ "$_GATE_WF_VALUE" == *[\"\'\[\]\{\}]* ]]; then
                _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: a quoted or flow value must close on its own line"
            fi
            return 0
            ;;
    esac
    if [[ "$_GATE_WF_KEY" == *[\"\'\[\]\{\}\#\&\*\!\|\>\,\?\`]* ]]; then
        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: a quoted or flow value must close on its own line"
    fi
    if [ "${2:-}" != keys ]; then
        _GATE_WF_VALUE=$(_gate_workflow_value "$_GATE_WF_VALUE")
        _gate_workflow_inline "$_GATE_WF_VALUE"
        if [[ "$_GATE_WF_VALUE" =~ ^[\|\>][1-9]?[+-]?$ || "$_GATE_WF_VALUE" =~ ^[\|\>][+-]?[1-9]?$ ]]; then
            _GATE_WF_BLOCK=$_GATE_WF_COLUMN
        fi
    fi
    return 0
}

# Key characters cannot contain spaces, so space-delimited scope sets are exact.
_gate_workflow_unique() {
    case " $1 " in
        *" $2 "*) _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: duplicate key '$2'" ;;
    esac
    return 0
}

_gate_workflow_file() {
    local LC_ALL=C
    local _GATE_WF_FILE=$1 _GATE_WF_DIRECTORY=$2 _GATE_WF_GIVEN_DIR=$3 _GATE_WF_PATH _GATE_WF_RESOLVED
    local _GATE_WF_LINE _GATE_WF_TEXT _GATE_WF_LEADING _GATE_WF_INDENT _GATE_WF_NUMBER=0
    local _GATE_WF_IN_JOBS=0 _GATE_WF_SAW_JOBS=0 _GATE_WF_TOP_KEY='' _GATE_WF_JOB='' _GATE_WF_JOB_KEY='' _GATE_WF_RUNNER=''
    local _GATE_WF_HAS_STEPS=0 _GATE_WF_HAS_CALL=0 _GATE_WF_STEP_LINE=0 _GATE_WF_STEP_RUN=0 _GATE_WF_STEP_USES=0
    local _GATE_WF_STEP_KEY='' _GATE_WF_COMMAND=''
    local _GATE_WF_KEY _GATE_WF_VALUE _GATE_WF_NAME _GATE_WF_ITEM _GATE_WF_DUPLICATE
    local _GATE_WF_COMMANDS=() _GATE_WF_JOB_ENV=()
    local _GATE_WF_TOP_KEYS='' _GATE_WF_JOB_IDS='' _GATE_WF_JOB_KEYS='' _GATE_WF_STEP_KEYS=''
    local _GATE_WF_ENV_NAMES='' _GATE_WF_SERVICE_NAMES='' _GATE_WF_CONTENT=0 _GATE_WF_BLOCK=-1
    _GATE_WF_PATH="$_GATE_WF_DIRECTORY/$_GATE_WF_FILE"
    if [ -L "$_GATE_WF_PATH" ]; then
        _gate_workflow_error "$_GATE_WF_FILE" '' 'workflow is a symbolic link'
    fi
    if [ ! -f "$_GATE_WF_PATH" ] || [ ! -r "$_GATE_WF_PATH" ]; then
        _gate_workflow_error "$_GATE_WF_FILE" '' "workflow not found in $_GATE_WF_GIVEN_DIR"
    fi
    # shellcheck disable=SC2094 # Both processes only read the workflow.
    if ! tr -d '\000' < "$_GATE_WF_PATH" | cmp -s - "$_GATE_WF_PATH"; then
        _gate_workflow_error "$_GATE_WF_FILE" '' 'a NUL byte'
    fi
    if _GATE_WF_RESOLVED=$(_gate_physical_file "$_GATE_WF_PATH"); then
        case "$_GATE_WF_RESOLVED" in
            "${_GATE_ROOT%/}"/*)
                _GATE_OWN_PATHS[${#_GATE_OWN_PATHS[@]}]=${_GATE_WF_RESOLVED#"${_GATE_ROOT%/}/"}
                ;;
        esac
    fi
    while IFS= read -r _GATE_WF_LINE || [ -n "$_GATE_WF_LINE" ]; do
        _GATE_WF_NUMBER=$((_GATE_WF_NUMBER + 1))
        _GATE_WF_LEADING=${_GATE_WF_LINE%%[!$' \t']*}
        if [[ "$_GATE_WF_LINE" == *$'\r'* || "$_GATE_WF_LINE" == *$'\t'* ]]; then
            _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: a carriage return or a tab"
        fi
        if [[ "$_GATE_WF_LINE" == *[!\ -\~]* ]]; then
            _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: a byte outside printable ASCII"
        fi
        _GATE_WF_TEXT=${_GATE_WF_LINE#"$_GATE_WF_LEADING"}
        _GATE_WF_INDENT=${#_GATE_WF_LEADING}
        if [ "$_GATE_WF_BLOCK" -ne -1 ]; then
            if [ -z "$_GATE_WF_TEXT" ] || [ "$_GATE_WF_INDENT" -gt "$_GATE_WF_BLOCK" ]; then
                continue
            fi
            _GATE_WF_BLOCK=-1
        fi
        case "$_GATE_WF_TEXT" in ''|\#*) continue ;; esac
        if [ "$_GATE_WF_TEXT" = --- ] && [ "$_GATE_WF_CONTENT" -eq 1 ]; then
            _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: a second YAML document"
        fi
        _GATE_WF_CONTENT=1
        if [ "$_GATE_WF_INDENT" -eq 0 ]; then
            _gate_workflow_finish_job
            _gate_workflow_line "$_GATE_WF_TEXT" keys
            _GATE_WF_IN_JOBS=0 _GATE_WF_TOP_KEY=''
            if [ "$_GATE_WF_TEXT" = --- ]; then continue; fi
            if [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: cannot be mirrored"
            fi
            _GATE_WF_KEY=${BASH_REMATCH[1]}
            _gate_workflow_unique "$_GATE_WF_TOP_KEYS" "$_GATE_WF_KEY"
            _GATE_WF_TOP_KEYS="$_GATE_WF_TOP_KEYS $_GATE_WF_KEY"
            _GATE_WF_TOP_KEY=$_GATE_WF_KEY
            _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
            _gate_workflow_inline "$_GATE_WF_VALUE"
            case "$_GATE_WF_KEY" in
                jobs)
                    if [ -n "$_GATE_WF_VALUE" ]; then _gate_workflow_error "$_GATE_WF_FILE" '' 'no jobs: block'; fi
                    _GATE_WF_IN_JOBS=1
                    ;;
                name|run-name|on|permissions|concurrency) ;;
                *) _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: top-level key '$_GATE_WF_KEY' is not mirrored" ;;
            esac
            continue
        fi
        if [ "$_GATE_WF_IN_JOBS" -eq 0 ]; then
            if [ -z "$_GATE_WF_TOP_KEY" ]; then _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
            _gate_workflow_line "$_GATE_WF_TEXT"
            continue
        fi
        if [ "$_GATE_WF_INDENT" -eq 2 ]; then
            _gate_workflow_finish_job
            _gate_workflow_line "$_GATE_WF_TEXT" keys
            if [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: cannot be mirrored"
            fi
            _GATE_WF_NAME=${BASH_REMATCH[1]}
            _gate_workflow_unique "$_GATE_WF_JOB_IDS" "$_GATE_WF_NAME"
            _GATE_WF_JOB_IDS="$_GATE_WF_JOB_IDS $_GATE_WF_NAME"
            _GATE_WF_SAW_JOBS=1
            _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
            _gate_workflow_inline "$_GATE_WF_VALUE"
            if [ -n "$_GATE_WF_VALUE" ]; then _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
            if ! _gate_workflow_hook gate_workflow_job "$_GATE_WF_FILE" "$_GATE_WF_NAME" > /dev/null; then
                _gate_workflow_error "$_GATE_WF_FILE" '' "job '$_GATE_WF_NAME' is not mirrored; define gate_workflow_job to allow it"
            fi
            _GATE_WF_JOB=$_GATE_WF_NAME
            continue
        fi
        if [ -z "$_GATE_WF_JOB" ]; then _gate_workflow_error "$_GATE_WF_FILE" '' "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
        _gate_workflow_line "$_GATE_WF_TEXT" keys
        if [ "$_GATE_WF_INDENT" -eq 4 ]; then
            _gate_workflow_finish_step
            if [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"
            fi
            _GATE_WF_KEY=${BASH_REMATCH[1]}
            _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
            _gate_workflow_inline "$_GATE_WF_VALUE"
            _gate_workflow_unique "$_GATE_WF_JOB_KEYS" "$_GATE_WF_KEY"
            _GATE_WF_JOB_KEYS="$_GATE_WF_JOB_KEYS $_GATE_WF_KEY"
            _GATE_WF_JOB_KEY=$_GATE_WF_KEY
            case "$_GATE_WF_KEY" in
                name|needs|permissions|timeout-minutes) ;;
                runs-on)
                    if [[ ! "$_GATE_WF_VALUE" =~ ^[A-Za-z0-9._-]+$ ]]; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "runs-on '$_GATE_WF_VALUE' is not a plain runner label"
                    fi
                    _GATE_WF_RUNNER=$_GATE_WF_VALUE
                    ;;
                env|services|steps)
                    if [ -n "$_GATE_WF_VALUE" ]; then _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "$_GATE_WF_KEY must be a block"; fi
                    if [ "$_GATE_WF_KEY" = steps ]; then _GATE_WF_HAS_STEPS=1; fi
                    ;;
                uses)
                    _GATE_WF_NAME=${_GATE_WF_VALUE#./.github/workflows/}
                    if [ "$_GATE_WF_NAME" = "$_GATE_WF_VALUE" ] || [[ ! "$_GATE_WF_NAME" =~ ^[A-Za-z0-9._-]+\.ya?ml$ ]]; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "uses '$_GATE_WF_VALUE' is not a local reusable workflow"
                    fi
                    _GATE_WF_HAS_CALL=1 _GATE_WF_DUPLICATE=0
                    for _GATE_WF_ITEM in "${_GATE_WORKFLOW_QUEUE[@]+"${_GATE_WORKFLOW_QUEUE[@]}"}"; do
                        if [ "$_GATE_WF_NAME" = "$_GATE_WF_ITEM" ]; then _GATE_WF_DUPLICATE=1; break; fi
                    done
                    if [ "$_GATE_WF_DUPLICATE" -eq 0 ]; then _GATE_WORKFLOW_QUEUE[${#_GATE_WORKFLOW_QUEUE[@]}]=$_GATE_WF_NAME; fi
                    ;;
                *)
                    if ! _gate_workflow_hook gate_workflow_job_key "$_GATE_WF_FILE" "$_GATE_WF_JOB" "$_GATE_WF_KEY" > /dev/null; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "job key '$_GATE_WF_KEY' is not mirrored"
                    fi
                    ;;
            esac
            continue
        fi
        # Only the nested blocks below can ignore otherwise invalid indentation.
        if [ "$_GATE_WF_INDENT" -gt 4 ]; then
            case "$_GATE_WF_JOB_KEY" in
                env)
                    if [ "$_GATE_WF_INDENT" -ne 6 ] || [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z_][A-Za-z0-9_]*):( .*)?$ ]]; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: env must be single-line NAME: value pairs"
                    fi
                    _GATE_WF_NAME=${BASH_REMATCH[1]}
                    _gate_workflow_unique "$_GATE_WF_ENV_NAMES" "$_GATE_WF_NAME"
                    _GATE_WF_ENV_NAMES="$_GATE_WF_ENV_NAMES $_GATE_WF_NAME"
                    _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
                    _gate_workflow_inline "$_GATE_WF_VALUE"
                    if ! _GATE_WF_VALUE=$(_gate_workflow_hook gate_workflow_env "$_GATE_WF_FILE" "$_GATE_WF_JOB" "$_GATE_WF_NAME") || [[ "$_GATE_WF_VALUE" == *$'\n'* ]]; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "env variable $_GATE_WF_NAME is not mirrored"
                    fi
                    _GATE_WF_JOB_ENV[${#_GATE_WF_JOB_ENV[@]}]="$_GATE_WF_NAME=$_GATE_WF_VALUE"
                    continue
                    ;;
                services)
                    if [ "$_GATE_WF_INDENT" -gt 6 ]; then _gate_workflow_line "$_GATE_WF_TEXT"; continue; fi
                    if [ "$_GATE_WF_INDENT" -ne 6 ] || [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"
                    fi
                    _GATE_WF_NAME=${BASH_REMATCH[1]}
                    _gate_workflow_unique "$_GATE_WF_SERVICE_NAMES" "$_GATE_WF_NAME"
                    _GATE_WF_SERVICE_NAMES="$_GATE_WF_SERVICE_NAMES $_GATE_WF_NAME"
                    _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
                    _gate_workflow_inline "$_GATE_WF_VALUE"
                    if [ -n "$_GATE_WF_VALUE" ]; then _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
                    if ! _gate_workflow_hook gate_workflow_service "$_GATE_WF_FILE" "$_GATE_WF_JOB" "$_GATE_WF_NAME" > /dev/null; then
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "service '$_GATE_WF_NAME' is not mirrored"
                    fi
                    continue
                    ;;
                runs-on|uses) _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored" ;;
                steps) ;;
                '') ;;
                *) _gate_workflow_line "$_GATE_WF_TEXT"; continue ;;
            esac
        fi
        if [ "$_GATE_WF_JOB_KEY" != steps ]; then _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
        if [ "$_GATE_WF_INDENT" -ge 10 ] && [ "$_GATE_WF_STEP_KEY" = with ]; then
            _gate_workflow_line "$_GATE_WF_TEXT"
            continue
        fi
        if [ "$_GATE_WF_INDENT" -eq 6 ]; then
            _gate_workflow_finish_step
            if [[ "$_GATE_WF_TEXT" == '- {'* ]] || [ "$_GATE_WF_TEXT" = - ]; then
                _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: write the step in block style"
            fi
            if [[ "$_GATE_WF_TEXT" != '- '* ]]; then _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"; fi
            _GATE_WF_TEXT=${_GATE_WF_TEXT#- }
            _GATE_WF_STEP_LINE=$_GATE_WF_NUMBER
        elif [ "$_GATE_WF_INDENT" -ne 8 ] || [ "$_GATE_WF_STEP_LINE" -eq 0 ]; then
            _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"
        fi
        if [[ ! "$_GATE_WF_TEXT" =~ ^([A-Za-z0-9_-]+):( .*)?$ ]]; then
            _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: cannot be mirrored"
        fi
        _GATE_WF_KEY=${BASH_REMATCH[1]}
        _GATE_WF_VALUE=$(_gate_workflow_value "${_GATE_WF_TEXT#*:}")
        if [ "$_GATE_WF_KEY" != run ]; then _gate_workflow_inline "$_GATE_WF_VALUE"; fi
        _gate_workflow_unique "$_GATE_WF_STEP_KEYS" "$_GATE_WF_KEY"
        _GATE_WF_STEP_KEYS="$_GATE_WF_STEP_KEYS $_GATE_WF_KEY"
        _GATE_WF_STEP_KEY=$_GATE_WF_KEY
        case "$_GATE_WF_KEY" in
            name|id|with|timeout-minutes) ;;
            run)
                case "$_GATE_WF_VALUE" in
                    ''|\|*|\>*) _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: a multi-line run: step" ;;
                    [\'\"\[\]\{\}\&\*\!%\@\`\#\,\?:]*|'- '*|*': '*|*:)
                        _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: run: is not a plain one-line value"
                        ;;
                esac
                # shellcheck disable=SC2016 # The expression marker is data.
                if [[ "$_GATE_WF_VALUE" == *'${{'* ]]; then
                    _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "line $_GATE_WF_NUMBER: run: uses an expression"
                fi
                _GATE_WF_STEP_RUN=1 _GATE_WF_COMMAND=$_GATE_WF_VALUE
                ;;
            uses)
                if [[ ! "$_GATE_WF_VALUE" =~ ^[A-Za-z0-9._/@:-]+$ ]] ||
                    ! _gate_workflow_hook gate_workflow_uses "$_GATE_WF_FILE" "$_GATE_WF_JOB" "$_GATE_WF_VALUE" > /dev/null; then
                    _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "step uses '$_GATE_WF_VALUE' is not mirrored"
                fi
                _GATE_WF_STEP_USES=1
                ;;
            *) _gate_workflow_error "$_GATE_WF_FILE" "$_GATE_WF_JOB" "step key '$_GATE_WF_KEY' is not mirrored" ;;
        esac
    done < "$_GATE_WF_PATH"
    _gate_workflow_finish_job
    if [ "$_GATE_WF_SAW_JOBS" -eq 0 ]; then _gate_workflow_error "$_GATE_WF_FILE" '' 'no jobs: block'; fi
    return 0
}

gate_workflow() {
    local LC_ALL=C
    if [ "$_GATE_INITIALIZED" -eq 0 ]; then _gate_error gate_workflow 'gate_init has not run'; fi
    if shopt -q nocasematch; then _gate_error gate_workflow 'nocasematch is set'; fi
    _GATE_WORKFLOW_CALL=$((_GATE_WORKFLOW_CALL + 1))
    local _GATE_WF_DIRECTORY=.github/workflows _GATE_WF_GIVEN_DIR _GATE_WF_DIR_SEEN=0 _GATE_WF_FILE _GATE_WF_INDEX _GATE_WF_BEFORE=${#_GATE_LABELS[@]}
    local _GATE_WF_PASSES=()
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --dir)
                if [ "$_GATE_WF_DIR_SEEN" -eq 1 ] || [ "$#" -lt 2 ]; then _gate_error gate_workflow 'missing or repeated --dir'; fi
                _GATE_WF_DIRECTORY=$2 _GATE_WF_DIR_SEEN=1
                if [ -z "$_GATE_WF_DIRECTORY" ] || [[ "$_GATE_WF_DIRECTORY" == *$'\n'* ]]; then _gate_error gate_workflow 'invalid --dir'; fi
                shift 2
                ;;
            --pass)
                if [ "$#" -lt 2 ] || [[ ! "$2" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then _gate_error gate_workflow 'invalid --pass'; fi
                _GATE_WF_PASSES[${#_GATE_WF_PASSES[@]}]=$2
                shift 2
                ;;
            --) shift; break ;;
            -*) _gate_error gate_workflow "unknown option: $1" ;;
            *) break ;;
        esac
    done
    if [ "$#" -ne 1 ]; then _gate_error gate_workflow 'expected one workflow file'; fi
    _GATE_WF_FILE=$1
    if [[ ! "$_GATE_WF_FILE" =~ ^[A-Za-z0-9._-]+\.ya?ml$ ]]; then _gate_error gate_workflow 'invalid workflow file name'; fi
    _GATE_WF_GIVEN_DIR=$_GATE_WF_DIRECTORY
    case "$_GATE_WF_DIRECTORY" in /*) ;; *) _GATE_WF_DIRECTORY="$_GATE_ROOT/$_GATE_WF_DIRECTORY" ;; esac
    _GATE_WORKFLOW_QUEUE=("$_GATE_WF_FILE")
    for ((_GATE_WF_INDEX = 0; _GATE_WF_INDEX < ${#_GATE_WORKFLOW_QUEUE[@]}; _GATE_WF_INDEX++)); do
        _gate_workflow_file "${_GATE_WORKFLOW_QUEUE[_GATE_WF_INDEX]}" "$_GATE_WF_DIRECTORY" "$_GATE_WF_GIVEN_DIR"
    done
    if [ "${#_GATE_LABELS[@]}" -eq "$_GATE_WF_BEFORE" ]; then _gate_workflow_error "$_GATE_WF_FILE" '' 'no run: steps found'; fi
    return 0
}

# Snapshot the caller's environment before gate_run introduces local variables.
_gate_workflow_environment() {
    local _GATE_EXEC_NAME _GATE_EXEC_DECLARATION
    _GATE_BASE_ENV=()
    _GATE_PASS_VALUES=()
    _GATE_PASS_SET=()
    for _GATE_EXEC_NAME in HOME USER LOGNAME PATH TMPDIR LANG TERM; do
        if _GATE_EXEC_DECLARATION=$(declare -p "$_GATE_EXEC_NAME" 2> /dev/null) &&
            [[ "$_GATE_EXEC_DECLARATION" =~ ^declare\ -[^\ ]*x[^\ ]*\  ]] &&
            [ "${!_GATE_EXEC_NAME+set}" = set ]; then
            _GATE_BASE_ENV[${#_GATE_BASE_ENV[@]}]="$_GATE_EXEC_NAME=${!_GATE_EXEC_NAME}"
        fi
    done
    _GATE_BASE_ENV[${#_GATE_BASE_ENV[@]}]=CI=true
    for _GATE_EXEC_NAME in "${_GATE_PASSES[@]+"${_GATE_PASSES[@]}"}"; do
        if _GATE_EXEC_DECLARATION=$(declare -p "$_GATE_EXEC_NAME" 2> /dev/null) &&
            [[ "$_GATE_EXEC_DECLARATION" =~ ^declare\ -[^\ ]*x[^\ ]*\  ]] &&
            [ "${!_GATE_EXEC_NAME+set}" = set ]; then
            _GATE_PASS_SET[${#_GATE_PASS_SET[@]}]='set'
        else
            _GATE_PASS_SET[${#_GATE_PASS_SET[@]}]=''
        fi
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
        ! top=$(CDPATH='' cd -- "$top" && pwd -P); then return 1; fi
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
    local base file path previous duplicate index peer row mapped word text candidate commit count=0 own_file
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
    # Selecting a workflow command also selects its job's setup and later checks.
    for ((index = 0; index < ${#_GATE_LABELS[@]}; index++)); do
        if [ "${chosen[index]:-0}" -eq 1 ] && [ -n "${_GATE_STEP_JOBS[index]}" ]; then
            for ((peer = 0; peer < ${#_GATE_LABELS[@]}; peer++)); do
                if [ "${_GATE_STEP_JOBS[peer]}" = "${_GATE_STEP_JOBS[index]}" ]; then chosen[peer]=1; fi
            done
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

_gate_literal_tree() {
    local directory=$1 repository=$2 top=$3 child name mode kind sha target entries result
    entries=$(mktemp "$repository/entries.XXXXXX") || return 1
    for child in "$directory"/* "$directory"/.[!.]* "$directory"/..?*; do
        if [ ! -e "$child" ] && [ ! -L "$child" ]; then continue; fi
        name=${child##*/}
        if [ "$top" -eq 1 ] && [ "$name" = PIN ]; then continue; fi
        kind=blob
        if [ -L "$child" ]; then
            mode=120000
            target=$(readlink "$child" && printf '.') || return 1
            target=${target%$'\n.'}
            sha=$(printf '%s' "$target" | git --git-dir="$repository/.git" hash-object -w --stdin) || return 1
        elif [ -d "$child" ]; then
            mode=040000 kind=tree
            sha=$(_gate_literal_tree "$child" "$repository" 0) || return 1
        elif [ -f "$child" ]; then
            mode=100644
            if find "$child" -prune -perm -u+x | grep -q .; then mode=100755; fi
            sha=$(git --git-dir="$repository/.git" hash-object -w --no-filters -- "$child") || return 1
        else
            return 1
        fi
        printf '%s %s %s\t%s\0' "$mode" "$kind" "$sha" "$name" >> "$entries" || return 1
    done
    result=$(git --git-dir="$repository/.git" mktree -z < "$entries") || return 1
    rm -f "$entries" || return 1
    printf '%s\n' "$result"
}

# Build the tree id from bytes, modes and link targets with hash-object --no-filters and mktree
# in a throwaway repository: no index, attributes or configuration can change it; find reads the owner executable bit.
_gate_tree_hash() (
    # Bash 3.2 drops function locals before EXIT traps run; temporary stays in this subshell.
    local directory=$1 variable tree
    set +f
    unset GLOBIGNORE
    shopt -u dotglob failglob nocaseglob
    if ! directory=$(CDPATH='' cd -- "$directory" 2> /dev/null && pwd -P); then return 1; fi
    while IFS= read -r variable; do
        case "$variable" in GIT_*) unset "$variable" ;; esac
    done < <(compgen -e)
    if ! temporary=$(mktemp -d "${TMPDIR:-/tmp}/gate-tree.XXXXXX"); then return 1; fi
    trap 'rm -rf "$temporary"' EXIT
    export GIT_CONFIG_NOSYSTEM=1 GIT_ATTR_NOSYSTEM=1 GIT_NO_REPLACE_OBJECTS=1
    export HOME="$temporary" XDG_CONFIG_HOME="$temporary"
    if ! git init -q --object-format=sha1 "$temporary/repo" > /dev/null 2>&1; then
        rm -rf "$temporary" > /dev/null 2>&1 || return 1
        return 1
    fi
    if tree=$(_gate_literal_tree "$directory" "$temporary/repo" 1 2> /dev/null); then
        if ! rm -rf "$temporary"; then return 1; fi
        printf '%s\n' "$tree"
        return 0
    fi
    rm -rf "$temporary" > /dev/null 2>&1 || return 1
    return 1
)

_gate_read_pin() {
    local pin=$1 line tag='' tree='' invalid=0
    _GATE_PIN_TAG='' _GATE_PIN_TREE=''
    if [ -L "$pin" ] || [ ! -f "$pin" ] || [ ! -r "$pin" ]; then
        invalid=1
    else
        if IFS= read -r -d '' line < "$pin"; then invalid=1; fi
        while IFS= read -r line || [ -n "$line" ]; do
            case "$line" in
                ''|'#'*) ;;
                tag\ *)
                    if [ -n "$tag" ] || [[ ! "$line" =~ ^tag\ awna-gate-v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$ ]]; then
                        invalid=1
                    fi
                    tag=${line#tag }
                    ;;
                tree\ *)
                    if [ -n "$tree" ] || [[ ! "$line" =~ ^tree\ [0-9a-f]{40}$ ]]; then invalid=1; fi
                    tree=${line#tree }
                    ;;
                *) invalid=1 ;;
            esac
        done < "$pin"
    fi
    if [ "$invalid" -ne 0 ] || [ -z "$tag" ] || [ -z "$tree" ]; then return 1; fi
    _GATE_PIN_TAG=$tag _GATE_PIN_TREE=$tree
    return 0
}

_gate_check_pin() {
    local directory pin tag tree found
    if [ -z "$_GATE_LIBRARY_SOURCE" ] || [[ "$_GATE_LIBRARY_SOURCE" == *$'\n'* ]]; then _gate_error gate_run "cannot find the library's directory"; fi
    directory=${_GATE_LIBRARY_SOURCE%/*}
    pin="$directory/PIN"
    if [ ! -e "$pin" ] && [ ! -L "$pin" ]; then return 0; fi
    if ! command -v git > /dev/null 2>&1; then
        gate_missing git 'the library pin check'
        return 0
    fi
    if ! _gate_read_pin "$pin"; then
        printf "%s: the library's pin cannot be read: %s\n" "$GATE_NAME" "$pin" >&2
        exit 1
    fi
    tag=$_GATE_PIN_TAG tree=$_GATE_PIN_TREE
    if ! found=$(_gate_tree_hash "$directory"); then _gate_error gate_run 'cannot hash the library'; fi
    if [ "$found" != "$tree" ]; then
        printf '%s: the library in %s differs from its pin %s (tree %s, found %s); vendor it again\n' \
            "$GATE_NAME" "$directory" "$tag" "$tree" "$found" >&2
        exit 1
    fi
    printf '%s: library pinned at %s\n' "$GATE_NAME" "$tag"
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
    _gate_check_pin
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
            printf 'step %s skipped %s (an earlier step of job %s failed)\n' "$ran" "${_GATE_LABELS[index]}" "${_GATE_RUN_JOB##*/}"
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
