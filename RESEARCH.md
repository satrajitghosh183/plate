# Research notes

Checked on 2026-11-19. "Unverified" means no primary source confirmed it; test before relying on it.

The budget became zero on 2026-11-20. Where the notes below recommend something paid (the Health Auto Export app, Render at $7 a month, a domain), the section "Running it for free" at the end replaces it.

## Apple Health

- There is no web or server API. Health data stays on the iPhone and is read by native apps with permission.
  Sources: https://developer.apple.com/documentation/healthkit, https://developer.apple.com/documentation/healthkit/protecting-user-privacy
- Nothing can read Health while the phone is locked. This limits every bridge.
- Lowest-effort bridge for one user: the Health Auto Export app (Premium, about $25 lifetime) posting JSON to our endpoint with a secret header. Delivery cadence is best-effort. Turn "Summarize" off so samples carry their source.
  Sources: https://help.healthyapps.dev/en/health-auto-export/automations/rest-api/, https://github.com/Lybron/health-auto-export/wiki/Supported-Data
- Alternatives: Shortcuts automations (fragile, one per data type), aggregators such as Terra (need their SDK in a native app, from $399/month), or our own native companion app ($99/year developer account; free provisioning expires every 7 days).

## Lifesum

- No public API found.
- Lifesum writes food energy, nutrients, weight and water to Apple Health, so it rides the same bridge. It stamps samples with sync time, not meal time, so show day totals only.
- Known bug: browsing old diary dates in Lifesum can delete synced calories from Health. Never let a re-export zero a past day.
- Backfill: account export (ZIP of CSV and JSON) at account.lifesum.com/your-data, one per 24 hours.
  Sources: https://help.lifesum.com/en/article/eagtau/, https://help.lifesum.com/en/article/1es3b9v/
- Unverified: whether samples are per food item or per day.

## Strava

- API Policy effective 2026-06-01 forbids using Strava data with any AI application, including putting it in a model's context, and forbids operating an MCP server that exposes it. It also caps caching at 7 days.
  Source: https://www.strava.com/legal/api_policy
- Consequence: Strava data is shown to the user only, kept 7 days, and never returned by our agent interface. Agents that need runs use Strava's own MCP server (subscribers only): https://support.strava.com/en-us/articles/15401531-how-do-i-connect-strava-to-an-ai-assistant
- Possible alternative for run history: workouts that reach Apple Health (recorded on Apple Watch or in the Strava app with "Send to Health" on) arrive through the Health bridge. Only third-party guides describe this; unverified against Strava's own help pages.
- Mechanics: OAuth with 6-hour access tokens and rotating refresh tokens; one webhook subscription per app; 100 read requests per 15 minutes and 1,000 per day; no review needed for a single athlete; a Strava subscription is required to create an API app.
  Sources: https://developers.strava.com/docs/authentication/, https://developers.strava.com/docs/webhooks/, https://developers.strava.com/docs/rate-limits/

## Exercise videos inside the app

- YouTube: the IFrame player with `playsinline=1` plays inline on iOS. Shorts embed at `/embed/ID`. Errors 101/150 mean the owner disallows embedding; 153 means a missing Referer. Tapping the YouTube title opens YouTube; blocking that with an iframe sandbox works technically but breaks YouTube's developer policies.
  Sources: https://developers.google.com/youtube/player_parameters, https://developers.google.com/youtube/iframe_api_reference, https://developers.google.com/youtube/terms/required-minimum-functionality
- Instagram: the official embed route (oEmbed) needs Meta app review and business verification. The plain `/reel/CODE/embed` iframe is undocumented; it played in a desktop test, and there are reports of it freezing on iOS web views. Private or embed-disabled accounts never play. No compliant way to get the video file.
  Source: https://developers.facebook.com/docs/instagram-platform/oembed
- Must be tested on a real iPhone in home-screen mode before promising "never leaves the app": title and logo taps, whether a reel plays past the first frame, whether sound needs a tap inside the player.

## iPhone home-screen web app (PWA)

- Screen Wake Lock works in home-screen apps from iOS 18.4. Web Push works for installed apps. No Background Sync and no vibration API, so queued sets are sent when the app is next open.
  Sources: https://firt.dev/notes/pwa-ios/, https://bugs.webkit.org/show_bug.cgi?id=254545
- Live Activities: every source found describes them as a native ActivityKit feature started by a native app or an APNs push to one. Nothing found lets a web app start one. A small native companion app could provide both Live Activities and the Health bridge later.
  Source: https://developer.apple.com/documentation/activitykit

## Music

- Spotify: control what the phone's Spotify app is playing through the Web API player endpoints (play, pause, skip, pick a playlist). Needs Premium. A single-user app in Development Mode is allowed. The phone often is not listed as a device until Spotify has been opened once. Refresh tokens expire 6 months after authorization.
  Sources: https://developer.spotify.com/documentation/web-api/reference/start-a-users-playback, https://developer.spotify.com/documentation/web-api/concepts/quota-modes, https://developer.spotify.com/blog/2026-06-18-refresh-token-expiration
- Spotify policy: show cover art and attribution, link back, and do not feed Spotify content into an AI model, so keep now-playing data away from agents.
  Sources: https://developer.spotify.com/policy, https://developer.spotify.com/documentation/design
- In-browser playback (Web Playback SDK) is supported in iOS Safari, but behaviour under screen lock in a home-screen app is unverified, and forum reports say it stops.
- YouTube Music: no official API or remote control. Only option inside the app is a visible YouTube playlist player (at least 200 by 200, no background or audio-only playback under YouTube's policies).
  Sources: https://developers.google.com/youtube/terms/developer-policies, https://developers.google.com/youtube/terms/required-minimum-functionality

## Rust stack and hosting

- Vercel: official Rust runtime for Functions is in public beta; static hosting of a WASM bundle works (build in CI, deploy prebuilt).
  Source: https://vercel.com/docs/functions/runtimes/rust
- Render: native Rust runtime. Free services sleep after 15 minutes and take about a minute to wake. Cheapest always-on is $7/month. Disks cost $0.25/GB/month, paid services only. Free Postgres expires after 30 days.
  Sources: https://render.com/pricing, https://render.com/docs/free, https://render.com/docs/disks
- Suggested shape: Leptos (client-rendered WASM) with a service worker and an IndexedDB outbox for offline logging, served statically; one Axum binary for the API, sign-in, ingest and the agent interface; SQLite on a Render disk. About $7.25/month plus a domain. A custom domain is needed so one passkey covers both hosts; serving everything from Render is the simpler alternative.
- Versions seen: Leptos 0.8.23, Axum 0.8.9, sqlx 0.9.1, rmcp 3.11.0, webauthn-rs 0.6.2.
- Agent interface: MCP over Streamable HTTP using `rmcp`. Claude connectors need OAuth with PKCE; Claude Code also accepts a static bearer header. Current spec revision 2026-07-28.
  Sources: https://github.com/modelcontextprotocol/rust-sdk, https://claude.com/docs/connectors/building/authentication.md

## Exercise catalogue for automatic sorting

- free-exercise-db is a public-domain (Unlicense) JSON dataset; each entry has primary and secondary muscles, force (push, pull, static), mechanic (compound, isolation) and equipment. Some fields are null in places.
  Source: https://github.com/yuhonas/free-exercise-db

## Added 2026-11-20

### Instagram saved posts

- No Instagram API exposes saved posts or collections, and personal accounts have no API at all.
  Source: https://developers.facebook.com/docs/instagram-platform/instagram-graph-api/reference/ig-user
- Routes that stay inside Instagram's terms: an iOS share-sheet Shortcut that posts a reel link to the backend; Instagram's scheduled data export to Google Drive or Dropbox, read by the backend (days of delay, file format unverified); a manual export upload. Scraping breaks the Terms of Use.
  Source: https://developers.facebook.com/documentation/data-portability/deep-link-guide
- Decision: dropped. Muse supplies clip links.

### Ending a session from the watch

- iPhone Shortcuts has a personal automation trigger "Apple Watch Workout" with Starts, Ends, or both, and it can be limited to a workout type. Apple lists it among triggers that can run automatically.
  Sources: https://support.apple.com/guide/shortcuts/apd932ff833f/ios, https://support.apple.com/guide/shortcuts/apd602971e63/ios
- The automation runs on the iPhone, not the watch (forum report): https://forums.macrumors.com/threads/shortcut-automation.2447564
- Unverified: whether it fires with the phone locked or out of range, and whether a web request action runs without a prompt.
- Health Auto Export cannot read new health data while the phone is locked, and its schedule is best-effort.
  Source: https://plums-coyote-p76r.squarespace.com/how-to-configure-automatic-apple-health-exports

### Vercel in front of Render

- `vercel.json` rewrites can forward `/api/:path*` to an external origin while the browser stays on the Vercel host. Caching of external rewrites can be turned off with the `x-vercel-enable-rewrite-caching: 0` header.
  Source: https://vercel.com/docs/routing/rewrites
- Unverified: streaming and timeout limits through an external rewrite, so agents connect to Render directly.

### Running it for free (checked 2026-11-20)

- Render free web service: spins down after 15 minutes without inbound traffic and takes about a minute to wake; 750 instance hours a month per workspace; free Postgres is deleted after 30 days. Third-party figures: 512 MB memory, 0.1 CPU, 500 build minutes a month. Persistent disks are a paid feature. Render says free instances are not for production.
  Sources: https://render.com/docs/free, https://render.com/articles/platforms-with-a-real-free-tier-for-developers-in-2026, https://costbench.com/software/developer-tools/render/free-plan
- Neon free plan (official FAQ, updated 2026-10-01): 1 GB storage per project, 100 compute-hours per project a month, no overage billing; writes fail past the storage cap. Third-party: computes pause after 5 idle minutes.
  Source: https://neon.com/faqs/free-plan-limits-and-quotas
- Turso free plan: sources disagree on the numbers, so it was not chosen.
  Source: https://turso.tech/pricing.md
- Vercel Hobby: free, personal non-commercial use only, 100 GB bandwidth, no way to buy more.
  Sources: https://vercel.com/docs/plans/hobby, https://vercel.com/docs/limits/fair-use-guidelines
- Free Apple Health route: a Shortcuts flow of "Find Health Samples", then "Get Contents of URL" as a POST. Active energy and heart rate are supported types. Reports disagree on whether workouts can be selected, and JSON built in Shortcuts breaks on newlines, so accept plain text too.
  Sources: https://www.positioniseverything.net/apple-health-data-api-build-a-free-shortcut-export-pipeline/, https://talk.macpowerusers.com/t/struggling-to-extract-workout-data-from-watch-via-shortcuts/39152, https://jngiam.bearblog.dev/syncing-apple-health-data-to-claude-code/
- Decision: Vercel Hobby + Render free + Neon free + Shortcuts. No domain. Spotify and Strava only if already paid for.
