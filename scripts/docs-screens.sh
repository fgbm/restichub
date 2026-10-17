#!/usr/bin/env bash
# Writes the screenshots of the documentation (docs/src/assets/screens) in the pinned Playwright
# image, so the fonts are the same as in CI. With --check, the script only compares.
set -euo pipefail

root=$(cd "$(dirname "$0")/.." && pwd)
version=$(node -p "require('$root/ui/package.json').devDependencies['@playwright/test']")
mode=--update-snapshots=all
if [ "${1:-}" = --check ]; then mode=--update-snapshots=none; fi

docker run --rm --init --ipc=host --network=host --user "$(id -u):$(id -g)" -e HOME=/tmp \
	-v "$root:/work" -w /work/ui "mcr.microsoft.com/playwright:v$version-noble" \
	bash -c "npm ci --no-audit --no-fund && VITE_USE_MOCK=true npm run build \
		&& npx playwright test -c playwright.docs.config.ts $mode"
