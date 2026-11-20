# Plate: plan and spec

Final plan as of 2026-11-20. This file is the source of truth for the build. Research and sources are in `RESEARCH.md`. Design rules are in `DESIGN.md`.

- Screens: https://claude.ai/artifact/9SoHgneHpAxfc8zdJE7rqd
- Motion prototype: https://claude.ai/artifact/GxZe17aduFPbYnod47vTpF
- Plan page with the hosting diagram: https://claude.ai/artifact/SZP4LsxzzVc7uEifTHhifg

Anything marked **untested** has not been proven on a real iPhone and is a Phase 0 item.

**Build status, 2026-11-20:** Phases 1 to 5 are built and run end to end locally (backend, phone app, agent link, Shortcuts intake, offline outbox). Not yet deployed: that needs your Neon, Render and Vercel accounts. Phase 0 needs your phone. See section 13.

## 1. What it is

A gym app for one person, used on an iPhone as an installed web app (PWA). The workout routine comes first: today's lifts, logging sets, rest, clips. Food and activity data are secondary and read-only. AI agents (Muse and Claude) can read results and write plans through a link.

Written in Rust end to end. The phone app is hosted on Vercel, the backend on Render.

**It costs nothing to run.** Every service is on a free plan, and the limits that come with that are accepted (section 3).

## 2. Decisions

| Area | Decision |
|---|---|
| Platform | PWA on iPhone. No native app. Live Activities dropped. |
| Language | Rust: Leptos (client-rendered WebAssembly) for the app, Axum for the backend. |
| Money | Zero. Free plans only, with their limits accepted on 2026-11-20. |
| Data store | Postgres on Neon's free plan. Render's free plan has no disk that survives a restart. |
| Sign-in | Passkey only. Registration locks after the first passkey. |
| Secrets | Every provider sign-in and token lives on the backend. The app never holds one. |
| Agents | MCP over HTTP on the backend. One revocable link per agent. |
| Apple Health | Free iPhone Shortcuts automations post to the backend. No Apple web API exists, and the paid bridge app is not used. |
| Lifesum | Arrives through Apple Health as day totals. No direct connection. |
| Strava | Display only, kept 7 days, never returned to agents. Only if you already subscribe, because Strava requires a subscription to create an API app. Otherwise off. |
| Music | Spotify remote control, only if you already have Premium, which Spotify's API requires. Otherwise off. YouTube Music dropped. |
| Clips | YouTube and Instagram reels play in a sheet inside the app. Muse supplies the links. The Instagram saved-folder connection is dropped. |
| Splits | One pool of lifts, sorted automatically, dealt into any split. Switching never touches logged sets. |
| Saving | Every set saves when logged. The session closes when the Apple Watch workout ends. |
| Units | Switchable kg and lb. |
| Look | True black, white as the only accent, Liquid Glass, 3D hero behind a glass pane. See `DESIGN.md`. |

## 3. Hosting

```
iPhone PWA ──HTTPS──> Vercel (static app + /api rewrite) ──> Render (Axum) ──> Neon (Postgres)
Agents (Muse, Claude) ───────────── MCP, per-agent key ───> Render
Phone Shortcuts ─────────────────── write-only key ───────> Render
Render ──> Strava API, Spotify API (tokens stay on Render)
```

**The app talks to one address.** Vercel serves the static files and rewrites `/api/*` to Render (`vercel.json` external rewrite, documented by Vercel). The browser only ever sees the Vercel host, so the passkey and the session cookie belong to one origin and no cross-site rules apply. Turn rewrite caching off for `/api/*`.

**Everything that is not the browser goes straight to Render.** That means agents, the phone's Shortcuts automations, the Strava webhook and the OAuth callbacks. They use keys, not cookies, and it keeps long-lived MCP responses off the Vercel rewrite, whose streaming limits are unverified.

**Addresses.** No domain is bought. The app lives at its free `<name>.vercel.app` address and the backend at its free `<name>.onrender.com` address. A passkey is tied to the host it was made on, so pick the Vercel project name once and keep it.

**Fallback.** If the rewrite misbehaves in the installed app (cookies, latency), serve the static files from the Axum binary on Render and drop Vercel. Nothing else in this plan changes.

**Build and deploy.** No GitHub workflows (your call, 2026-11-20). `scripts/check.sh` runs every test and lint on this Mac before a push. `scripts/deploy-web.sh` builds the WebAssembly bundle and publishes `dist/` to Vercel with the Vercel CLI. The build writes `dist/vercel.json` with the `/api` rewrite to `BACKEND_URL`, and stamps a content security policy into the page. Render builds the Axum binary from the same repository (`render.yaml`). Vercel's Rust function runtime is in beta and is not used.

**Cost: zero.** Free plans, limits checked 2026-11-20. Third-party figures are marked; confirm them on each provider's own pages in Phase 1.

| Service | Free plan | Limit that matters here |
|---|---|---|
| Vercel Hobby | Personal, non-commercial use | 100 GB of traffic a month. Fine for one person. |
| Render free web service | One Rust service | Sleeps after 15 minutes without traffic and takes about a minute to wake. 750 hours a month for the whole workspace, so run only this one service. 512 MB of memory (third-party figure). No disk that survives a restart. Render says free services are not for production. |
| Neon free Postgres | One project | 1 GB of storage, 100 compute-hours a month, pauses when idle and resumes on the next query. |
| iPhone Shortcuts | Built in | Fragile to build, one automation per kind of data. |

**What the free limits mean in use**
- **Opening the app after a break.** The app pings the backend as it opens. Logging never waits: sets save on the phone first and sync once the backend is awake, up to about a minute later. That ping goes through the Vercel rewrite, so the app treats an error or a reply that is not JSON as "still waking" and tries again.
- **Agents.** The first call after a quiet spell can take about a minute. Later calls are normal.
- **Workout-ended ping.** It may hit a sleeping backend. Whether the Shortcut waits long enough is untested, so the tap and the 90-minute safety net stay.
- **Deploys.** Render's free plan has a monthly build allowance (500 minutes, third-party figure) and Rust builds are slow, so deploy in batches.
- **Not used:** Render's free Postgres, which is deleted after 30 days.

**Neon address.** Use Neon's direct connection string (host without `-pooler`) with `sslmode=require`. The backend uses prepared statements and runs migrations at start-up, which the pooled address can break. TLS to Neon was tested on 2026-11-20: the backend connected, migrated and answered.

**Agent links in logs.** The link carries the agent's key in its path, so it can appear in Render's request logs. Revoke and replace a link if those logs are ever shared. Clients that can send a header should use `PUBLIC_URL/mcp` with the bearer key instead.

**Backups.** Settings › Download everything saves all your data as one file; do it now and then. Neon also keeps a short restore window on its free plan (length not checked). For a full database copy: `pg_dump "$DATABASE_URL" > plate.dump`.

**Costs:** the body renders came from your Canva account's free image generations.

## 4. Sign-in and secrets

- **You:** passkey (`webauthn-rs`). The first registration needs a one-time setup code from a Render environment variable, then registration closes. Session cookie is HttpOnly, Secure, SameSite=Lax, and the backend checks the Origin header on writes.
- **Providers (Strava, Spotify):** connected on the server with `scripts/plate.sh authorize strava|spotify`, never from the app (your call, 2026-11-20). The command signs in through your browser on your own computer and stores the developer app id, secret and tokens in the database, sealed with TOKEN_KEY. Tokens are stored encrypted in the database with a key held only in Render's environment, and are never sent to the app or to agents. The app's Connect screen shows status only. Spotify's refresh token expires 6 months after sign-in, so the app shows "Reconnect Spotify" when that is near.
- **Phone Shortcuts:** one write-only key. It can post health data and the workout-ended ping. It cannot read anything.
- **Agents:** one key per agent, stored hashed, with scopes, a rate limit and a revoke switch. Every agent write is recorded with who, when and what changed.

## 5. Training

### Lifts and automatic sorting

Each lift carries: movement pattern (horizontal push, vertical push, horizontal pull, vertical pull, squat, hinge, lunge, carry, core, isolation), primary muscle, secondary muscles, equipment, mechanic (compound or isolation).

Sorting runs in this order and stops at the first confident answer:

1. Match against the free-exercise-db catalogue by normalised name and aliases.
2. Keyword rules on the name ("row", "curl", "press" plus qualifiers).
3. Tags supplied by an agent, accepted only if every value is a known one.
4. Your own correction, which always wins and is never overwritten.

A lift that none of these can place is flagged "needs a look" and stays out of the plan until you tag it.

### Splits

A split is data, not code: an ordered list of days, each with a rule for which lifts belong (by muscle group or movement pattern) and which weekdays it uses. Built in: push pull legs, upper lower, bro split, full body. Custom splits use the same shape, written by you or an agent.

Dealing the pool into a split: each lift goes to the days whose rule it matches, compounds first, volume balanced across days. Full body rotates lifts across its days.

Switching creates a new plan version. Logged sets point at lifts, not at days, so history is untouched. Undo restores the previous version.

### Sessions and sets

- A session opens on "Start session".
- A set is saved the moment it is logged, first on the phone, then on the backend. Each set has an id made on the phone so a retry never duplicates it.
- Agents cannot change or delete a logged set. You can.
- Weights are stored in grams with the unit they were entered in. Bar is 20 kg or 45 lb, steps are 2.5 kg or 5 lb.
- Progress uses estimated one-rep max: weight × (1 + reps / 30), best set per session.

### What closes a session

The first of these to arrive closes it. Any later one attaches its data and changes nothing else.

1. **Workout-ended ping.** An iPhone Shortcuts automation on "Apple Watch Workout ends" posts to the backend. Limit it to strength workout types so a run closes nothing.
2. **Workout numbers.** The same automation tries to read the finished workout from Apple Health and post duration, calories and heart rate. Apple Health cannot be read while the phone is locked, which is the usual state in the gym, so the evening automation sends the day's workout numbers again. They attach to the session they overlap in time, and a repeat changes nothing.
3. **Your tap** on "End session". This is the fallback for days without the watch.
4. **Safety net.** A session with no new set for 90 minutes closes at its last set.

Sets logged offline with a time before the close still attach to that session when they sync.

**Untested:** whether the Shortcut fires with the phone locked, and with the phone out of range of the watch; whether Shortcuts can read a workout record at all (reports disagree; active energy and heart rate are readable, so the fallback is to total those over the session's time); and whether the ping survives a sleeping backend. Nothing reads Apple Health while the phone is locked, so the numbers can land after the session has closed.

### Offline in the gym

The service worker caches the app, including the JS bridge it imports. The phone keeps three things in localStorage: the last start-up bundle (`gym.cache`), the session in progress (`gym.session`) and an outbox of unsent sessions, sets, deletions and End taps (`gym.outbox`). iOS has no background sync, so the outbox flushes when the app is open and online, and again whenever the network or the app comes back. The screen stays awake during a session (Wake Lock, iOS 18.4 or later).

*Changed from the first plan:* localStorage instead of IndexedDB. The data is a few kilobytes, localStorage writes are synchronous (nothing is lost if the app is closed mid-write), and installed home-screen apps are exempt from Safari's seven-day storage clearing.

## 6. Clips

- A clip is a link attached to a lift, with its provider and who added it. Muse adds and replaces clips through the agent link.
- YouTube plays through the IFrame player with inline playback. Instagram reels use the reel embed address, which Meta does not document.
- If a clip refuses to embed, the sheet says so and offers the next clip or "Open in Instagram".
- Video files are never downloaded or stored.
- Tapping the YouTube logo inside the player opens YouTube. Blocking that breaks YouTube's terms, so it stays.

**Untested:** whether reels play past the first frame in the installed app, and whether sound needs a tap inside the player.

## 7. Data from other apps

| Source | Route | What the app shows | Agents see it |
|---|---|---|---|
| Apple Health | Shortcuts automations post to Render: one when a workout ends, one each evening that also re-sends the day's workouts | Workouts, energy burned, heart rate | Yes |
| Lifesum | Through Apple Health, in the evening automation | Calories eaten and macros, as day totals | Yes |
| Strava | OAuth on the backend, webhook. Only with an existing subscription | Recent activities, kept 7 days | Never |
| Spotify | OAuth on the backend, remote control. Only with existing Premium | What is playing, play, pause, skip, pick a playlist | Never |

Notes:
- Lifesum stamps entries with sync time, so only day totals are reliable. Never let a re-export zero a past day.
- Strava's API terms forbid giving its data to AI tools and cap storage at 7 days.
- Spotify needs Premium and the Spotify app opened once on the phone. Show cover art and a link back, as its terms require.
- Without a Strava subscription, runs recorded on the watch still reach Apple Health and can be shown from there. That route is unverified.
- Shortcuts builds JSON poorly, so the backend also accepts plain `key=value` lines.

## 8. Agents

The backend exposes MCP over Streamable HTTP: JSON-RPC over POST, tools only, JSON replies, stateless. Each agent gets its own address and key: `PUBLIC_URL/mcp/<key>` (the link you give Muse) or `PUBLIC_URL/mcp` with `Authorization: Bearer <key>`.

*Changed from the first plan:* the protocol is written by hand instead of with `rmcp`. With tools only and no server-to-client stream, it is about 150 lines, has no extra dependency, and is covered by the end-to-end tests.

On adding a lift, an agent's tags are used only if the catalogue and the name rules both fail. An agent can retag a lift explicitly, unless you tagged it yourself.

| Action | Muse | Claude | Notes |
|---|---|---|---|
| Read plan, lifts, sessions, progress | Yes | Yes | |
| Read food and health day totals | Yes | Yes | |
| Add or tag lifts | Yes | Yes | Tags validated against known values |
| Write plans and splits | Yes | Yes | New plan version, shown in the activity list with Undo |
| Add or replace clips | Yes | Yes | |
| Change or delete logged sets | No | No | Nobody but you |
| Read Strava or Spotify data | No | No | Not exposed by any tool |
| Read keys or tokens | No | No | |

- **Default:** agent changes apply at once and appear in the activity list with Undo. A per-agent "ask me first" switch turns them into proposals you accept or decline.
- A change to a session in progress waits until that session closes.
- **Connecting:** Claude connectors need OAuth with PKCE. Claude Code accepts a key in a header. For Muse the default is a link that carries its key.

## 9. Screens

Built in the app: sign-in and first-time setup, no plan yet, Today (or rest day), choose a day, session list, log a set, rest, summary, Plan with split switcher, all lifts with "needs a look", lift editor, clips, Progress, Fuel with Strava runs, Connect (sources, agents, changes with undo, accept and decline), music player, settings, the offline and waking-server line.

Equipment is drawn in code: the loaded bar with the exact plates, a dumbbell pair, a cable or machine stack with the pin at the right plate, and a kettlebell. Bodyweight lifts keep the body model.

Motion follows the prototype: only movement and fades, never animated blur. iPhone web apps cannot vibrate, so every tap answers with motion.

**3D.** The barbell is drawn in code and shows the exact plates. The body starts as still renders, one per muscle group, cross-faded. A real 3D model that turns is a later upgrade.

## 10. Build order

**Phase 0: phone tests.** A throwaway page installed on your iPhone. Each item is pass or fail.
- A YouTube clip and an Instagram reel play inline with sound in the installed app.
- The workout-ended Shortcut reaches the backend with the phone locked, and with the phone left in the locker.
- The Shortcut can read the finished workout, or failing that its active energy and heart rate, and post them.
- The workout-ended ping gets through when the backend is asleep.
- What the Vercel rewrite returns while the backend is waking.
- The evening automation can read Apple Health if it fires while the phone is locked.
- How long the app takes to become usable after the backend has slept.
- Passkey and session cookie survive through the Vercel rewrite in the installed app.
- Spotify lists the phone as a device, and play and skip work.
- Signing in to Spotify and Strava from the installed app comes back to the app, still signed in.
- Glass panes stay smooth while scrolling and during the pane transition.
- The outbox survives closing the app and a week without opening it.

**Phase 1: foundation.** Repository, Axum with Postgres on Neon, passkey sign-in, app shell with service worker, deploys to Vercel and Render, backups.

**Phase 2: training.** Lifts and sorting, splits and plan versions, Today, session, logging, rest, summary, progress, offline outbox, kg and lb.

**Phase 3: agents.** MCP tools, per-agent keys and scopes, activity list with Undo, proposals.

**Phase 4: other apps.** The Shortcuts automations and health intake with Lifesum totals, automatic session close, Fuel, Strava display, Spotify control, clip sheet.

**Phase 5: finish.** Motion, body renders per muscle group, remaining screens, accessibility pass.

## 11. Open questions

1. **How Muse connects.** Can Muse use an MCP address with a key in a header, does it need OAuth, or can it only take a plain link?
2. **Do you already pay for Spotify Premium or Strava?** Each one is switched on only if you do.
3. **Results of Phase 0.** Any failure there changes the matching section above.

## 12. Request check

| You asked for | Where |
|---|---|
| Runs on the phone in the gym | Sections 1, 5 (offline) |
| Agents can read and update it | Section 8 |
| Apple Health, Lifesum, Strava | Section 7 |
| Muse writes plans, you log results | Sections 5, 8 |
| Clips without leaving the app | Section 6 |
| Calories visible but secondary | Section 7, Fuel screen |
| Rust, fast, secure | Sections 2, 3, 4 |
| Spend no money | Sections 2, 3 |
| Vercel and Render | Section 3 |
| Any split from one pool, automatic sorting | Section 5 |
| Spotify inside the app | Section 7 |
| All API work on the backend | Section 4 |
| Strava display only | Sections 7, 8 |
| Give Muse a link | Section 8 |
| kg and lb | Section 5 |
| Session saves when the watch workout ends | Section 5 |
| YouTube Music | Dropped: no API exists |
| Live Activities | Dropped: native only |
| Instagram saved folder | Dropped: Muse supplies clips |

## 13. What only you can do

1. **Neon.** Create a free project and copy its connection string (with `sslmode=require`).
2. **Render.** New › Blueprint › this repository. Fill in `DATABASE_URL`, `APP_URL` (your Vercel address), `PUBLIC_URL` (the Render address) and a long `SETUP_CODE`. `TOKEN_KEY` is generated for you.
3. **Vercel.** Run `npx vercel login` once on this Mac, then `BACKEND_URL=https://plate-api.onrender.com ./scripts/deploy-web.sh`. Name the project `plate` and keep it: your passkey is tied to its address.
4. **GitHub.** The repository is `satrajitghosh183/plate` (private). Render deploys the backend from it on every push to `main`.
5. **Phone.** Open the app, enter the setup code, save the passkey. Then run `scripts/plate.sh phone-key` and build the two automations in `shortcuts/README.md`.
6. **Muse.** Run `scripts/plate.sh agent add Muse` and give Muse the link.
7. **Phase 0.** Open `/phase0.html` from the Home Screen app and record the results here.
8. **Spotify and Strava** (only if you already pay for them): create the developer apps, put their ids and secrets in `.env`, and run `scripts/plate.sh authorize spotify` and `scripts/plate.sh authorize strava`.

**No connect screens in the app.** Keys, agent links and provider sign-ins are made only with the `plate` command; the app shows their status. Sign-in and the phone's intake are rate limited to 30 requests a minute per address.
