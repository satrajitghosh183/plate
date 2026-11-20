#!/usr/bin/env bash
# Run a plate command against the real database, with your .env loaded.
#   scripts/plate.sh status | phone-key | agent add Muse | agent list | agent revoke Muse
#   scripts/plate.sh authorize strava | authorize spotify
# Your .env needs DATABASE_URL and TOKEN_KEY (the same values Render has), PUBLIC_URL for
# printed links, and the STRAVA_ or SPOTIFY_ CLIENT_ID and CLIENT_SECRET before authorize.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:$PATH"
[ -f .env ] || { echo "No .env here. Copy .env.example to .env and fill it in"; exit 1; }
set -a; . ./.env; set +a
export PUBLIC_URL="${PUBLIC_URL:-https://plate-api.onrender.com}" APP_URL="${APP_URL:-https://plate.vercel.app}"
cargo build -q --release -p gym-server
exec ./target/release/gym-server "$@"
