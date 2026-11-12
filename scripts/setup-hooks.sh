#!/bin/sh
# Turn on the repo's git hooks: every commit and push runs scripts/check.sh first.
set -eu
cd "$(dirname "$0")/.."
git config core.hooksPath .githooks
echo "Git hooks enabled (.githooks)."
