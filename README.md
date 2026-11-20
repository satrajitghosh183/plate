# Plate

A gym app for one person: an iPhone web app (PWA) written in Rust, with a plan agents can
write, sets that save the moment you log them, and sessions that close when your Apple Watch
workout ends. It runs on free plans only.

- `SPEC.md`: the plan and every decision. Start here.
- `DESIGN.md`: the look, and the rules for changing it.
- `RESEARCH.md`: what was checked, with sources.
- `shortcuts/README.md`: the two iPhone automations that bring Apple Health in.

## Layout

| Path | What |
|---|---|
| `crates/core` | Rules shared by both sides: lift sorting, split dealing, plates, sessions, the API types |
| `crates/server` | Axum backend: passkeys, the app API, health intake, the agent link (MCP), Strava, Spotify |
| `crates/web` | Leptos phone app, compiled to WebAssembly |
| `web/static` | Page, styles, service worker, manifest, icons, the Phase 0 test page |
| `data/catalogue` | Public-domain exercise catalogue (free-exercise-db) |
| `deploy`, `render.yaml` | Vercel and Render settings |
| `scripts` | `plate.sh` (connect things: phone key, agent links, Strava, Spotify), `check.sh` (all tests and lints), `build-web.sh`, `deploy-web.sh` (publish to Vercel) |

## Run it locally

Needs Rust with the `wasm32-unknown-unknown` target, `wasm-bindgen-cli` 0.2.129 and Postgres 17.

```sh
# A throwaway database
initdb -D .pgdata -U gym --auth=trust && pg_ctl -D .pgdata -o "-p 55432" start
createdb -h 127.0.0.1 -p 55432 -U gym gym

# Build the phone app into dist/, then run the backend serving it
./scripts/build-web.sh
DATABASE_URL=postgres://gym@127.0.0.1:55432/gym APP_URL=http://localhost:8080 \
  SETUP_CODE=$(openssl rand -hex 8) TOKEN_KEY=$(openssl rand -hex 32) STATIC_DIR=dist \
  cargo run -p gym-server
```

Open http://localhost:8080, enter the setup code and save a passkey (passkeys work on
`localhost`).

## Connecting things

Nothing is connected from the app. The app only shows status. On your computer, with `.env`
holding the same `DATABASE_URL` and `TOKEN_KEY` as Render:

```sh
scripts/plate.sh status               # what is connected
scripts/plate.sh phone-key            # key for the iPhone's Apple Health shortcuts (Lifesum rides along)
scripts/plate.sh agent add Muse       # Muse's link, shown once; also: agent list, agent revoke Muse
scripts/plate.sh authorize strava     # browser sign-in; tokens stored encrypted in the database
scripts/plate.sh authorize spotify
scripts/plate.sh login email you@example.com   # the only email password and Google accept
scripts/plate.sh login password                # typed, never shown or stored in plain text
scripts/plate.sh login google                  # needs GOOGLE_CLIENT_ID and GOOGLE_CLIENT_SECRET in .env
scripts/plate.sh logout-everywhere
```

For `authorize`, put the developer app's id and secret in `.env` first. Strava: any app at
strava.com/settings/api (its callback domain may stay `localhost`). Spotify: an app at
developer.spotify.com with the redirect URI `http://127.0.0.1:8765/callback`.

## Tests

```sh
createdb -h 127.0.0.1 -p 55432 -U gym gym_test
TEST_DATABASE_URL=postgres://gym@127.0.0.1:55432/gym_test cargo test --workspace --exclude gym-web
```

The end-to-end tests create and drop their own database for each test. They cover the rules
that must never break: agents can't reach logged sets; no agent tool returns Strava or Spotify
data; the phone key can't read; a retried set is not duplicated; switching splits leaves sets
alone; registration closes after the first passkey; a second close signal changes nothing.

## Deploy

See `SPEC.md` section 13. In short: Neon for the database, the Render blueprint for the
backend, `scripts/deploy-web.sh` for Vercel. All free. Run `./scripts/check.sh` before every push: there are no GitHub workflows.
