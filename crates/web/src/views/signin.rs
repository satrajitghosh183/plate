//! Sign in with a passkey. The very first time, the setup code from the server is needed too.

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
            {(!supported).then(|| view! { <p class="sub bad">"This browser can't use passkeys. Open the app in Safari on your iPhone"</p> })}
            <button type="button" class="btn btn-accent press" disabled=move || busy.get() || !supported || (setup() && code.get().trim().is_empty())
                on:click=move |_| go()>
                {move || if busy.get() { "Waiting for your passkey" } else if setup() { "Create a passkey" } else { "Sign in with passkey" }}
            </button>
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
