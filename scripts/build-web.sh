#!/usr/bin/env bash
# Build the phone app into dist/: WebAssembly bundle, static files, versioned cache names.
set -euo pipefail
cd "$(dirname "$0")/.."
PROFILE="${PROFILE:-wasm-release}"
export PATH="$HOME/.cargo/bin:$PATH"

cargo build -p gym-web --target wasm32-unknown-unknown --profile "$PROFILE"
dir="$PROFILE"; [ "$PROFILE" = "dev" ] && dir="debug"
rm -rf dist && mkdir -p dist/pkg
wasm-bindgen --target web --no-typescript --out-dir dist/pkg --out-name gym_web "target/wasm32-unknown-unknown/$dir/gym-web.wasm"
if [ "${WASM_OPT:-0}" = "1" ] && command -v wasm-opt >/dev/null 2>&1 && [ "$PROFILE" != "dev" ]; then
  wasm-opt -Oz --enable-bulk-memory --enable-nontrapping-float-to-int -o dist/pkg/gym_web_bg.wasm dist/pkg/gym_web_bg.wasm
fi
cp -R web/static/. dist/

# One version for the whole build, from the content of what ships.
SNIPPETS=$(cd dist && find pkg/snippets -type f -name '*.js' | sort)
VERSION=$(cat dist/pkg/gym_web_bg.wasm dist/pkg/gym_web.js dist/app.css dist/sw.js $(for s in $SNIPPETS; do echo "dist/$s"; done) | shasum -a 256 | cut -c1-12)
# The worker must cache the JS snippets the app imports, or it can't start offline.
LIST=$(for s in $SNIPPETS; do printf '"/%s",' "$s"; done)
sed -i.bak "s#__SNIPPETS__#[${LIST%,}]#" dist/sw.js && rm dist/sw.js.bak
for f in dist/index.html dist/sw.js; do
  sed -i.bak "s/__VERSION__/$VERSION/g" "$f" && rm "$f.bak"
done
# Content security policy: only this app's own scripts (plus the one inline loader,
# by hash), Google Fonts, the two video players, and Spotify cover art.
INLINE=$(python3 -c 'import re,sys,hashlib,base64;s=open("dist/index.html").read();m=re.search(r"<script type=\"module\">(.*?)</script>",s,re.S);print(base64.b64encode(hashlib.sha256(m.group(1).encode()).digest()).decode())')
CSP="default-src 'self'; script-src 'self' 'wasm-unsafe-eval' 'sha256-$INLINE'; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src https://fonts.gstatic.com; img-src 'self' data: https://i.scdn.co https://mosaic.scdn.co https://image-cdn-ak.spotifycdn.com https://image-cdn-fa.spotifycdn.com; frame-src https://www.youtube-nocookie.com https://www.instagram.com; connect-src 'self' https://fonts.googleapis.com https://fonts.gstatic.com; worker-src 'self'; manifest-src 'self'; base-uri 'none'; form-action 'self'; object-src 'none'"
sed -i.bak "s|__CSP__|$CSP|" dist/index.html && rm dist/index.html.bak

# Vercel: forward /api to the backend on Render.
if [ -n "${BACKEND_URL:-}" ]; then
  sed "s|__BACKEND_URL__|${BACKEND_URL%/}|g" deploy/vercel.json > dist/vercel.json
fi
echo "built dist/ version $VERSION ($(du -h dist/pkg/gym_web_bg.wasm | cut -f1) wasm)"
