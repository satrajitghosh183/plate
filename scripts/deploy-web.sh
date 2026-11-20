#!/usr/bin/env bash
# Build the phone app and publish it to Vercel from this Mac.
# Usage: BACKEND_URL=https://plate-api.onrender.com ./scripts/deploy-web.sh
# The first time, run `npx vercel login` and `npx vercel link` in dist/ (or let the prompt guide you).
set -euo pipefail
cd "$(dirname "$0")/.."
: "${BACKEND_URL:?Set BACKEND_URL to the Render address, e.g. https://plate-api.onrender.com}"
BACKEND_URL="$BACKEND_URL" ./scripts/build-web.sh
test -f dist/vercel.json || { echo "dist/vercel.json is missing"; exit 1; }
mkdir -p dist/.vercel && cp -f .vercel/project.json dist/.vercel/ 2>/dev/null || true
cd dist && npx --yes vercel deploy --prod --yes
# Keep the project link for next time.
mkdir -p ../.vercel && cp -f .vercel/project.json ../.vercel/ 2>/dev/null || true
