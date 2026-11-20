//! Sign in with a passkey, your email and password, or Google. Which of these show depends on
//! what is switched on at the server. The very first passkey needs the setup code too.

use gym_core::api::AuthStatus;
use leptos::prelude::*;
use leptos::task::spawn_local;
use serde_json::{Value, json};

use crate::bridge;
use crate::net;
use crate::state::{Auth, ctx};

#[component]
pub fn SignIn() -> impl IntoView {
    let c = ctx();
    let setup = move || c.auth.get() == Auth::NeedsSetup;
    let code = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let supported = bridge::passkeys_supported();
    // Which ways in are switched on; until the server answers, offer the passkey.
    let methods = RwSignal::new(AuthStatus { signed_in: false, needs_setup: false, passkey: true, password: false, google: false });
    spawn_local(async move {
        if let Ok(s) = net::get::<AuthStatus>("/api/auth/status").await {
            methods.set(s);
        }
    });
    // Coming back from a refused Google sign-in.
    if let Some(reason) = signin_reason() {
        error.set(Some(match reason.as_str() {
            "google-not-you" => "That Google account isn't the one this app belongs to".to_string(),
            "google-cancelled" => "Google sign-in was cancelled".to_string(),
            "google-expired" => "That Google sign-in took too long. Try again".to_string(),
            "google-off" => "Google sign-in is switched off on the server".to_string(),
            _ => "Google sign-in didn't work. Try again".to_string(),
        }));
    }
    let email = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let finish = move |result: Result<(), String>| {
        busy.set(false);
        match result {
            Ok(()) => {
                c.auth.set(Auth::SignedIn);
                spawn_local(async move { c.refresh().await });
            }
            Err(e) => error.set(Some(e)),
        }
    };
    let password_in = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        busy.set(true);
        let (e, p) = (email.get_untracked(), password.get_untracked());
        spawn_local(async move {
            let r: Result<AuthStatus, _> = net::post("/api/auth/password", &json!({ "email": e.trim(), "password": p })).await;
            password.set(String::new());
            finish(r.map(|_| ()).map_err(|e| e.message()));
        });
    };

    let go = move || {
        error.set(None);
        busy.set(true);
        spawn_local(async move {
            let result = if setup() { register(code.get_untracked()).await } else { login().await };
            busy.set(false);
            match result {
                Ok(()) => {
                    c.auth.set(Auth::SignedIn);
                    c.refresh().await;
                }
                Err(e) => error.set(Some(e)),
            }
        });
    };

    view! {
        <div class="signin">
            <img class="body" src="/body.jpg" alt=""/>
            <div class="stack g8">
                <h1 class="display">"Plate"</h1>
                <p class="sub">{move || if setup() {
                    "First time here. Enter the setup code from the server, then save a passkey on this phone. After that, only your passkey opens the app"
                } else {
                    "Sign in with the passkey on this phone"
                }}</p>
            </div>
            {move || setup().then(|| view! {
                <div class="field">
                    <label for="setup-code">"Setup code"</label>
                    <input id="setup-code" class="input num" inputmode="numeric" autocomplete="one-time-code" maxlength="64"
                        prop:value=move || code.get() on:input=move |e| code.set(event_target_value(&e))/>
                </div>
            })}
            {move || error.get().map(|e| view! { <p class="sub bad" role="alert">{e}</p> })}
            {move || (setup() || methods.get().passkey).then(|| view! {
                {(!supported).then(|| view! { <p class="sub bad">"This browser can't use passkeys. Open the app in Safari on your iPhone"</p> })}
                <button type="button" class="btn btn-accent press" disabled=move || busy.get() || !supported || (setup() && code.get().trim().is_empty())
                    on:click=move |_| go()>
                    {move || if busy.get() { "Waiting for your passkey" } else if setup() { "Create a passkey" } else { "Sign in with passkey" }}
                </button>
            })}
            {move || (!setup() && methods.get().google).then(|| view! {
                <a class="btn btn-line press" href="/api/auth/google/start" rel="external">"Continue with Google"</a>
            })}
            {move || (!setup() && methods.get().password).then(|| view! {
                <form class="stack g8" on:submit=password_in>
                    <div class="field">
                        <label for="email">"Email"</label>
                        <input id="email" class="input" type="email" autocomplete="username" inputmode="email"
                            prop:value=move || email.get() on:input=move |e| email.set(event_target_value(&e))/>
                    </div>
                    <div class="field">
                        <label for="password">"Password"</label>
                        <input id="password" class="input" type="password" autocomplete="current-password"
                            prop:value=move || password.get() on:input=move |e| password.set(event_target_value(&e))/>
                    </div>
                    <button type="submit" class="btn btn-line press" disabled=move || busy.get() || email.get().trim().is_empty() || password.get().is_empty()>
                        "Sign in with password"
                    </button>
                </form>
            })}
        </div>
    }
}

async fn register(code: String) -> Result<(), String> {
    let start: Value = net::post("/api/auth/register/start", &json!({ "setup_code": code.trim() })).await.map_err(|e| e.message())?;
    let options = start["options"].to_string();
    let cred = bridge::passkey_create(&options).await.map_err(|e| bridge::passkey_error(&e))?;
    let cred: Value = serde_json::from_str(&cred.as_string().unwrap_or_default()).map_err(|_| "The passkey reply was unreadable".to_string())?;
    let _: AuthStatus = net::post("/api/auth/register/finish", &json!({ "ceremony": start["ceremony"], "credential": cred, "name": "iPhone" })).await.map_err(|e| e.message())?;
    Ok(())
}

async fn login() -> Result<(), String> {
    let start: Value = net::post("/api/auth/login/start", &json!({})).await.map_err(|e| e.message())?;
    let options = start["options"].to_string();
    let cred = bridge::passkey_get(&options).await.map_err(|e| bridge::passkey_error(&e))?;
    let cred: Value = serde_json::from_str(&cred.as_string().unwrap_or_default()).map_err(|_| "The passkey reply was unreadable".to_string())?;
    let _: AuthStatus = net::post("/api/auth/login/finish", &json!({ "ceremony": start["ceremony"], "credential": cred })).await.map_err(|e| e.message())?;
    Ok(())
}

/// Add another passkey (a second phone, or a new one) while signed in.
pub async fn add_passkey() -> Result<(), String> {
    register(String::new()).await
}

/// The `?signin=` reason the server adds after a refused Google sign-in, removed from the address.
fn signin_reason() -> Option<String> {
    let w = web_sys::window()?;
    let search = w.location().search().ok()?;
    let reason = search.trim_start_matches('?').split('&').find_map(|kv| kv.strip_prefix("signin=")).map(str::to_string)?;
    if let Ok(h) = w.history() {
        let _ = h.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some("/"));
    }
    Some(reason)
}
