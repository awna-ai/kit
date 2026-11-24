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
    printf 'usage: %s [--tier full|land] [--list]\n' "${0##*/}" >&2
    exit 2
}

gate_init() {
    if [ "$_GATE_INITIALIZED" -eq 1 ]; then _gate_error gate_init 'called twice'; fi
    _GATE_INITIALIZED=1
    local name='' strict_var='' tier_seen=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --name)
                if [ "$#" -lt 2 ]; then _gate_error gate_init 'missing --name value'; fi
                name=$2
                if [[ ! "$name" =~ ^[a-z0-9][a-z0-9._-]*$ ]]; then
                    _gate_error gate_init 'invalid --name'
                fi
                GATE_NAME=$name
                shift 2
                ;;
            --strict-var)
                if [ "$#" -lt 2 ]; then _gate_error gate_init 'missing --strict-var value'; fi
                strict_var=$2
                if [[ ! "$strict_var" =~ ^[A-Za-z_][A-Za-z0-9_]*$ ]]; then
                    _gate_error gate_init 'invalid --strict-var'
                fi
                shift 2
                ;;
            --) shift; break ;;
            *) _gate_error gate_init "unknown option: $1" ;;
        esac
    done
    if [ -z "$name" ]; then _gate_error gate_init 'missing --name'; fi
    GATE_TIER=full
    GATE_LIST=0
    while [ "$#" -gt 0 ]; do
        case "$1" in
            --tier)
                if [ "$tier_seen" -ne 0 ] || [ "$#" -lt 2 ]; then _gate_usage; fi
                GATE_TIER=$2
                case "$GATE_TIER" in full|land) ;; *) _gate_usage ;; esac
                tier_seen=1
                shift 2
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
    return 0
}

gate_missing() {
    if [ "$_GATE_STRICT" -eq 1 ] && [ "$GATE_LIST" -eq 0 ]; then
        printf '%s: %s is missing on this host (%s=1)\n' "$GATE_NAME" "$1" "$_GATE_STRICT_VAR" >&2
        exit 75
    fi
    if [ "$GATE_LIST" -eq 1 ]; then
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
    local kind=plain mac=0 label command
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
    local count=$1 rank index best
    local selected=()
    printf '%s: slowest steps\n' "$GATE_NAME"
    for ((rank = 0; rank < count && rank < 5; rank++)); do
        best=-1
        for ((index = 0; index < count; index++)); do
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
    local total=${#_GATE_LABELS[@]} index os logdir log started status outcome
    local ran=0 red=0 stop=''
    _GATE_TIMES=()
    if [ "$total" -eq 0 ]; then
        printf '%s: no steps registered\n' "$GATE_NAME" >&2
        exit 2
    fi
    if [ "$GATE_LIST" -eq 1 ]; then
        printf '%s\n' "${_GATE_LABELS[@]+"${_GATE_LABELS[@]}"}"
        exit 0
    fi
    for ((index = 0; index < total; index++)); do
        if [ "${_GATE_MAC[index]}" -eq 1 ]; then
            if ! os=$(uname -s); then _gate_error gate_run 'cannot identify this host'; fi
            if [ "$os" != Darwin ]; then
                printf '%s: %s runs only on macOS and this host is %s; exit 75 so another host can run the gate\n' \
                    "$GATE_NAME" "${_GATE_LABELS[index]}" "$os" >&2
                exit 75
            fi
        fi
    done
    if ! logdir=$(mktemp -d "${TMPDIR:-/tmp}/$GATE_NAME.XXXXXX"); then
        _gate_error gate_run 'cannot create a log directory'
    fi
    for ((index = 0; index < total; index++)); do
        ran=$((index + 1))
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
    _gate_slowest "$ran"
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
    if ! rm -rf "$logdir"; then _gate_error gate_run 'cannot remove the log directory'; fi
    if [ "$_GATE_SKIPPED" -gt 0 ]; then
        printf '%s: all passed, %s skipped\n' "$GATE_NAME" "$_GATE_SKIPPED"
    else
        printf '%s: all passed\n' "$GATE_NAME"
    fi
    exit 0
}
