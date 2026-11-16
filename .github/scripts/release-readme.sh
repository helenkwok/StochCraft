#!/usr/bin/env bash
# Writes the README.md that goes into a release archive: an archive carries no docs/ folder, so
# every link relative to the repository becomes an absolute link at the release's tag (pictures
# through /raw/, the rest through /blob/). Usage: release-readme.sh VERSION OUT
set -euo pipefail
version="$1"
out="$2"
repo="${GITHUB_SERVER_URL:-https://github.com}/${GITHUB_REPOSITORY:-helenkwok/StochCraft}"
tag="v$version"
sed -E \
  -e "s#\]\((docs/[^)]+\.(png|jpg|svg|gif))\)#](${repo}/raw/${tag}/\1)#g" \
  -e "s#\]\(((docs|bench)/[^)]+|[A-Z][A-Za-z_-]*(\.md)?)\)#](${repo}/blob/${tag}/\1)#g" \
  README.md >"$out"
