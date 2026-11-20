#!/usr/bin/env bash
# Everything that must pass before a push: tests (with a throwaway Postgres), lints, the web build.
# Needs Postgres 17 binaries on PATH or at /opt/homebrew/opt/postgresql@17/bin.
set -euo pipefail
cd "$(dirname "$0")/.."
export PATH="$HOME/.cargo/bin:/opt/homebrew/opt/postgresql@17/bin:$PATH"
DATA=$(mktemp -d)
trap 'pg_ctl -D "$DATA" stop -m fast >/dev/null 2>&1 || true; rm -rf "$DATA"' EXIT
initdb -D "$DATA" -U gym --auth=trust -E UTF8 >/dev/null
pg_ctl -D "$DATA" -o "-p 55433 -k $DATA -c listen_addresses=127.0.0.1" -l "$DATA/log" start >/dev/null
createdb -h 127.0.0.1 -p 55433 -U gym gym_test
TEST_DATABASE_URL=postgres://gym@127.0.0.1:55433/gym_test cargo test --workspace --exclude gym-web
cargo clippy --workspace --exclude gym-web --all-targets -- -D warnings
cargo clippy -p gym-web --target wasm32-unknown-unknown -- -D warnings
./scripts/build-web.sh
echo "All checks passed"
