//! `plate`: the server, and the commands that connect things to it.
//!
//! With no arguments it serves. Everything that connects a data source or an agent is a
//! command here instead of a screen in the app. Run them on your computer with
//! `scripts/plate.sh <command>`, which loads your `.env`.

use gym_server::config::Config;
use gym_server::providers::Provider;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::EnvFilter;

const USAGE: &str = "plate: the Plate server and its setup commands

  plate                      run the server
  plate status               what is connected
  plate phone-key            new key for the iPhone's Apple Health shortcuts (old key stops working)
  plate agent add <name>     new link for an agent such as Muse (shown once)
  plate agent list           agents and when they were last used
  plate agent revoke <name>  stop an agent's link working
  plate authorize strava     sign in to Strava in your browser; tokens are stored encrypted
  plate authorize spotify    the same for Spotify
  plate login email <addr>   your email, the only one password and Google sign-in accept
  plate login password       set or change the password (typed, not shown)
  plate login google         switch on Google sign-in (GOOGLE_CLIENT_ID/SECRET from .env)
  plate login off password|google
  plate logout-everywhere    sign out every device";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["serve"] => serve().await,
        ["help"] | ["--help"] | ["-h"] => {
            println!("{USAGE}");
            Ok(())
        }
        _ => command(&words).await,
    }
}

async fn serve() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info,tower_http=info,sqlx=warn"))).init();
    let cfg = Config::from_env()?;
    let port = cfg.port;
    if cfg.setup_code.is_none() {
        tracing::warn!("SETUP_CODE is not set: nobody can register the first passkey");
    }
    if cfg.token_key.is_none() {
        tracing::warn!("TOKEN_KEY is not set: Strava and Spotify stay switched off");
    }
    let st = gym_server::build_state(cfg).await?;
    gym_server::spawn_housekeeping(st.clone());
    let app = gym_server::router(st);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", port)).await?;
    tracing::info!("listening on {}", listener.local_addr()?);
    axum::serve(listener, app.into_make_service_with_connect_info::<std::net::SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn command(words: &[&str]) -> anyhow::Result<()> {
    let cfg = Config::from_env()?;
    let db = PgPoolOptions::new().max_connections(2).acquire_timeout(std::time::Duration::from_secs(30)).connect(&cfg.database_url).await?;
    gym_server::MIGRATOR.run(&db).await?;
    match words {
        ["status"] => {
            for line in gym_server::admin::status(&db).await? {
                println!("{line}");
            }
        }
        ["phone-key"] => {
            let k = gym_server::admin::new_phone_key(&db, &cfg).await?;
            println!("New phone key (shown once; the old one no longer works):\n\n  {}\n", k.key);
            println!("Shortcuts send it in a header named x-ingest-key to:");
            println!("  when a workout ends:  POST {}", k.workout_ended_url);
            println!("  every evening:        POST {}", k.health_url);
            println!("\nStep by step: shortcuts/README.md");
        }
        ["agent", "add", name @ ..] if !name.is_empty() => {
            let a = gym_server::admin::add_agent(&db, &cfg, &name.join(" ")).await?;
            println!("Link for {} (shown once):\n\n  {}\n", a.name, a.link);
            println!("Or, for clients that send a header:\n  {}\n  Authorization: Bearer {}", a.endpoint, a.key);
        }
        ["agent", "list"] => {
            let list = gym_server::admin::list_agents(&db).await?;
            if list.is_empty() {
                println!("No agents yet. Add one: plate agent add Muse");
            }
            for a in list {
                let state = if a.revoked { "revoked" } else if a.ask_first { "active, asks you first" } else { "active" };
                let used = a.last_used_at.map(|t| t.format("%Y-%m-%d %H:%M UTC").to_string()).unwrap_or_else(|| "never".into());
                println!("{:<16} {:<24} made {}  last used {}", a.name, state, a.created_at.format("%Y-%m-%d"), used);
            }
        }
        ["agent", "revoke", name @ ..] if !name.is_empty() => {
            let n = gym_server::admin::revoke_agent(&db, &name.join(" ")).await?;
            println!("{}", if n == 0 { "No active agent by that name".to_string() } else { "Revoked. Its link stopped working".to_string() });
        }
        ["authorize", which] => {
            let p = Provider::parse(which).ok_or_else(|| anyhow::anyhow!("Use strava or spotify"))?;
            let who = gym_server::admin::authorize(&db, &cfg, p).await?;
            println!("{} is connected as {who}. The tokens are stored encrypted on the server.", p.label());
        }
        ["login", "email", addr] => {
            let e = gym_server::admin::set_owner_email(&db, addr).await?;
            println!("Sign-in email set to {e}.");
        }
        ["login", "password"] => {
            let p1 = rpassword::prompt_password("New password (12+ characters): ")?;
            let p2 = rpassword::prompt_password("Again: ")?;
            if p1 != p2 {
                anyhow::bail!("The two didn't match. Nothing changed");
            }
            gym_server::admin::set_password(&db, &p1).await?;
            println!("Password saved (hashed). Sign in with your email and this password.");
        }
        ["login", "google"] => {
            let redirect = gym_server::admin::set_google(&db, &cfg).await?;
            println!("Google sign-in is on. The Google app's authorized redirect URI must be:\n  {redirect}");
        }
        ["login", "off", which] => {
            gym_server::admin::sign_in_off(&db, which).await?;
            println!("{which} sign-in is off.");
        }
        ["logout-everywhere"] => {
            let n = gym_server::admin::sign_out_everywhere(&db).await?;
            println!("Signed out {n} device(s).");
        }
        _ => {
            println!("{USAGE}");
            std::process::exit(2);
        }
    }
    Ok(())
}
