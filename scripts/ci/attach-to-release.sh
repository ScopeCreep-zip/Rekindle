#!/usr/bin/env bash
# Attach files to the GitHub Release for a tag, the way
# softprops/action-gh-release did before the SBOM workflow moved to the
# runner's own `gh` (zizmor superfluous-actions): the release that
# release.yml's tauri-action creates (a draft prerelease named
# "Rekindle <tag>") is used when it exists, and created with those same
# settings when the SBOM job gets there first. No files is not an error
# (the old `fail_on_unmatched_files: false`).
#
# Usage: attach-to-release.sh <tag> [file...]   (GH_TOKEN in the env)
set -euo pipefail

tag="$1"
shift
if [ "$#" -eq 0 ]; then
    echo "no files to attach to ${tag}"
    exit 0
fi

if ! gh release view "${tag}" >/dev/null 2>&1; then
    gh release create "${tag}" --verify-tag --draft --prerelease \
        --title "Rekindle ${tag}" \
        --notes "See the assets below to download and install this version."
fi
gh release upload "${tag}" "$@" --clobber
