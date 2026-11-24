#!/usr/bin/env bash
# A sourced library: leave the caller's options and traps alone.
if [ "${_GATE_LOADED:-0}" = 1 ] && declare -F gate_run > /dev/null; then
    return 0
fi
_GATE_LOADED=1
_GATE_INITIALIZED=0

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
    _GATE_SKIPPED=0
    _GATE_LABELS=()
    _GATE_COMMANDS=()
    _GATE_KINDS=()
    _GATE_MAC=()
    _GATE_ALWAYS=()
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
    local kind=plain mac=0 always=0 label command
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
    label=$1
    command=${2-$label}
    if [ -z "$label" ] || [ -z "$command" ] || [[ "$label$command" == *$'\n'* ]]; then
        _gate_error gate_step 'label and command must be non-empty single lines'
    fi
    _GATE_LABELS[${#_GATE_LABELS[@]}]=$label
    _GATE_COMMANDS[${#_GATE_COMMANDS[@]}]=$command
    _GATE_KINDS[${#_GATE_KINDS[@]}]=$kind
    _GATE_MAC[${#_GATE_MAC[@]}]=$mac
    _GATE_ALWAYS[${#_GATE_ALWAYS[@]}]=$always
    return 0
}


# Both map users share one parser; patterns remain data throughout.
_gate_map_problem() {
    if [ "$GATE_CHECK_MAP" -eq 1 ]; then
        printf 'FAIL: %s\n' "$1" >&2
        _GATE_PROBLEMS=$((_GATE_PROBLEMS + 1))
    elif [ -z "$_GATE_REASON" ]; then
        _GATE_REASON=$1
    fi
    return 0
}

_gate_read_map() {
    local file line path_pattern step_pattern label path matched number=0
    _GATE_PATH_PATTERNS=()
    _GATE_STEP_PATTERNS=()
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
    local base file path previous duplicate index row mapped word text
    local LC_ALL=C
    local paths=() chosen=() words=()
    _GATE_REASON=''
    if ! command -v git > /dev/null 2>&1; then
        _GATE_REASON='git is unavailable'
        return 0
    fi
    _gate_read_map
    if [ -n "$_GATE_REASON" ]; then return 0; fi
    if ! base=$(git -C "$_GATE_ROOT" merge-base HEAD "$_GATE_BASE" 2> /dev/null); then
        _GATE_REASON="no merge base with $_GATE_BASE"
        return 0
    fi
    if ! file=$(mktemp "${TMPDIR:-/tmp}/$GATE_NAME.XXXXXX"); then
        _GATE_REASON='could not list the change'
        return 0
    fi
    if ! git -C "$_GATE_ROOT" diff -z --name-only --no-renames --no-ext-diff --no-textconv "$base" -- > "$file" 2> /dev/null ||
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
            for text in "${_GATE_LABELS[index]}" "${_GATE_COMMANDS[index]}"; do
                IFS=$' \t\n' read -r -a words <<< "$text"
                for word in "${words[@]+"${words[@]}"}"; do
                    if [ "$word" = "$path" ]; then chosen[index]=1; mapped=1; fi
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
            index=${_GATE_SELECTED[position]}
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
    local total=${#_GATE_LABELS[@]} index position os logdir='' log started status outcome
    local ran=0 red=0 stop=''
    _GATE_TIMES=()
    if [ "$total" -eq 0 ]; then
        printf '%s: no steps registered\n' "$GATE_NAME" >&2
        exit 2
    fi
    if [ "$GATE_CHECK_MAP" -eq 1 ]; then _gate_check_map; fi
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
        log="$logdir/step-$ran.log"
        started=$SECONDS
        status=0
        if (cd "$_GATE_ROOT" && bash -c "${_GATE_COMMANDS[index]}") < /dev/null > "$log" 2>&1; then
            status=0
        else
            status=$?
        fi
        _GATE_TIMES[index]=$((SECONDS - started))
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
    if [ "$ran" -gt 0 ]; then _gate_slowest "$ran"; fi
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
