#!/usr/bin/env bash
#
# File-size gate.
#
# Soft thresholds today (warn; CI continues). The intent is to prevent
# AI-generated mega-files from landing — a 2 000-line `.tsx` is almost
# always a tier-violation hiding in plain sight. See:
#   docs/contributor/architecture-rules.md
#   docs/contributor/ai-assisted-contributions.md §5
#
# Existing files over threshold are tracked separately in the cleanup
# sweep; this script lists them but does not fail CI for them. New
# violations introduced in a PR will be highlighted in the CI summary.

set -uo pipefail

FRONTEND_MAX=500
RUST_MAX=1500

# Paths to scan: (dir, extension list, max lines)
declare -a TARGETS=(
    "src           tsx,ts          $FRONTEND_MAX"
    "src-tauri/src rs              $RUST_MAX"
    "crates        rs              $RUST_MAX"
)

# Files we explicitly exempt — generated code, large protocol envelopes
# that are intrinsically big, etc. Each line is a regex matched against
# the path relative to the repo root.
EXEMPT_REGEX='(^|/)(gen|generated|capnp_envelope)/|_capnp\.rs$|_pb\.rs$'

violations=0
warnings=0
total_files=0

print_warn() {
    # Use GitHub Actions ::warning::file=...,line=1::message format
    # if running in CI; plain warning otherwise.
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        printf '::warning file=%s,line=1::File exceeds %s lines (%s lines)\n' \
            "$1" "$3" "$2"
    else
        printf '  warn: %s — %s lines (threshold %s)\n' "$1" "$2" "$3"
    fi
}

print_err() {
    if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
        printf '::error file=%s,line=1::File exceeds hard cap of %s lines (%s lines)\n' \
            "$1" "$3" "$2"
    else
        printf '  ERROR: %s — %s lines (HARD CAP %s)\n' "$1" "$2" "$3"
    fi
}

for spec in "${TARGETS[@]}"; do
    # shellcheck disable=SC2086
    set -- $spec
    dir=$1
    exts=$2
    threshold=$3

    if [[ ! -d "$dir" ]]; then
        continue
    fi

    # Build a find expression for the extensions: -name '*.tsx' -o -name '*.ts'
    find_args=()
    IFS=',' read -ra ext_arr <<<"$exts"
    for i in "${!ext_arr[@]}"; do
        if [[ $i -gt 0 ]]; then
            find_args+=(-o)
        fi
        find_args+=(-name "*.${ext_arr[$i]}")
    done

    while IFS= read -r -d '' file; do
        rel=${file#./}
        # Skip exempt patterns
        if [[ "$rel" =~ $EXEMPT_REGEX ]]; then
            continue
        fi
        lines=$(wc -l <"$file" 2>/dev/null | awk '{print $1}')
        # shellcheck disable=SC2034
        total_files=$((total_files + 1))
        if [[ -z "$lines" ]]; then
            continue
        fi
        # Hard cap = 2 × threshold; we still warn rather than error today
        # but emit a louder signal at the hard cap.
        hard_cap=$((threshold * 2))
        if [[ "$lines" -gt "$hard_cap" ]]; then
            print_err "$rel" "$lines" "$hard_cap"
            violations=$((violations + 1))
        elif [[ "$lines" -gt "$threshold" ]]; then
            print_warn "$rel" "$lines" "$threshold"
            warnings=$((warnings + 1))
        fi
    done < <(find "$dir" -type f \( "${find_args[@]}" \) -print0 2>/dev/null)
done

printf '\n— file-size gate —\n'
printf '  warnings: %s\n' "$warnings"
printf '  errors:   %s\n' "$violations"
printf '\nThe gate is currently warn-only for the existing oversized files\n'
printf 'tracked in the cleanup sweep (docs/roadmap.md). New PRs that add\n'
printf 'oversized files will surface in CI; please split the file before\n'
printf 'merging.\n'

# Today: never fail. Promote to "exit 1 if violations > 0" once existing
# violations are cleaned up.
exit 0
