#!/usr/bin/env bash
set -euo pipefail

case "$EVENT" in
  pull_request)
    case "$PR_DRAFT" in
      true) printf 'plan=draft\n' >> "$GITHUB_OUTPUT"; exit 0 ;;
      false) ;;
      *) echo '::error::Missing pull request draft state'; exit 1 ;;
    esac
    base=$PR_BASE
    head=$PR_HEAD
    full_plan=full-pr
    ;;
  merge_group)
    base=$MQ_BASE
    head=$MQ_HEAD
    full_plan=full-queue
    ;;
  *) echo '::error::Unsupported CI event'; exit 1 ;;
esac

changes=$(mktemp)
trap 'rm -f "$changes"' EXIT

# Compare the event's exact commits. Disable rename detection so a rename out
# of a build path still includes that path. Retain modes to check text edits.
plan=$full_plan
if [[ $base =~ ^[0-9a-f]{40}$ && $head =~ ^[0-9a-f]{40}$ ]] &&
  git diff --raw --no-renames --no-ext-diff -z "$base" "$head" -- > "$changes"; then
  if [[ -s $changes ]]; then
    plan=license
    while IFS= read -r -d '' metadata; do
      if ! IFS= read -r -d '' path ||
        [[ $metadata != ':100644 100644 '*$' M' ]]; then
        plan=$full_plan
        break
      fi
      case "$path" in
        LICENSE-APACHE|LICENSE-MIT) ;;
        README.md|ARCHITECTURE.md|CODE_OF_CONDUCT.md|CONTRIBUTING.md|SECURITY.md|docs/README.md)
          plan=docs
          ;;
        *) plan=$full_plan; break ;;
      esac
    done < "$changes"
  fi
else
  echo '::warning::Could not read the event diff. Running all applicable checks.'
fi

printf 'CI plan: %s\n' "$plan"
printf 'plan=%s\n' "$plan" >> "$GITHUB_OUTPUT"
