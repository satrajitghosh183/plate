//! The root: sign-in gate, the five tabs, the floating tab bar, sheets and toasts.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::icons::{self, icon};
use crate::net;
use crate::state::{Auth, Ctx, Mode, Net, Tab, ToastAction, ctx};
use crate::views::{connect, fuel, plan, progress, sheets, signin, train};

#[component]
pub fn App() -> impl IntoView {
    let c = Ctx::new();
    provide_context(c);
    c.start();

    // A clock for timers and "ago" labels.
    let now = c.now;
    let handle = set_interval_with_handle(move || now.set(chrono::Utc::now()), std::time::Duration::from_millis(500)).ok();
    on_cleanup(move || {
        if let Some(h) = handle {
            h.clear();
        }
    });

    // Refresh while something is expected from the server: a session in progress
    // (the watch may close it) or a summary waiting for the watch's numbers.
    let poll = set_interval_with_handle(
        move || {
            if c.auth.get_untracked() != Auth::SignedIn {
                return;
            }
            let visible = web_sys::window().and_then(|w| w.document()).map(|d| d.visibility_state() == web_sys::VisibilityState::Visible).unwrap_or(true);
            let waiting = c.mode.get_untracked().in_session() || matches!(c.mode.get_untracked(), Mode::Summary(_));
            if visible && (waiting || !c.outbox.with_untracked(|o| o.is_empty())) {
                c.refresh_soon();
            }
        },
        std::time::Duration::from_secs(25),
    )
    .ok();
    on_cleanup(move || {
        if let Some(h) = poll {
            h.clear();
        }
    });

    let _vis = window_event_listener_untyped("visibilitychange", move |_| {
        let visible = web_sys::window().and_then(|w| w.document()).map(|d| d.visibility_state() == web_sys::VisibilityState::Visible).unwrap_or(false);
        if visible && c.auth.get_untracked() == Auth::SignedIn {
            c.refresh_soon();
        }
    });
    let _online = window_event_listener_untyped("online", move |_| {
        c.net.set(Net::Waking);
        c.refresh_soon();
    });
    let _offline = window_event_listener_untyped("offline", move |_| c.net.set(Net::Offline));
    // The first tap unlocks sound for the rest timer on iOS.
    let _tap = window_event_listener_untyped("pointerdown", move |_| crate::bridge::unlock_audio());

    view! {
        <main class="app">
            {move || match c.auth.get() {
                Auth::Checking => view! { <Waking/> }.into_any(),
                Auth::NeedsSetup | Auth::SignedOut => view! { <signin::SignIn/> }.into_any(),
                Auth::SignedIn => view! { <Signed/> }.into_any(),
            }}
            <Toasts/>
        </main>
    }
}

#[component]
fn Waking() -> impl IntoView {
    let c = ctx();
    view! {
        <div class="signin">
            <img class="body" src="/body.jpg" alt=""/>
            <div class="stack g8">
                <h1 class="display">"Plate"</h1>
                <p class="sub">{move || match c.net.get() {
                    Net::Offline => "You're offline. Open the app once with signal to sign in",
                    _ => "Waking the server. On the free plan this can take about a minute",
                }}</p>
            </div>
            <div class="rowx g8"><span class="spinner"></span><span class="meta">"Connecting"</span></div>
        </div>
    }
}

#[component]
fn Signed() -> impl IntoView {
    let c = ctx();
    let hide_bar = move || c.tab.get() == Tab::Today && (c.mode.get().in_session() || matches!(c.mode.get(), Mode::Summary(_)));
    view! {
        <section class="page" class:on=move || c.tab.get() == Tab::Today inert=move || c.tab.get() != Tab::Today aria-label="Today"><train::Train/></section>
        <section class="page" class:on=move || c.tab.get() == Tab::Plan inert=move || c.tab.get() != Tab::Plan aria-label="Plan">
            {move || (c.tab.get() == Tab::Plan).then(|| view! { <plan::PlanPage/> })}
        </section>
        <section class="page" class:on=move || c.tab.get() == Tab::Progress inert=move || c.tab.get() != Tab::Progress aria-label="Progress">
            {move || (c.tab.get() == Tab::Progress).then(|| view! { <progress::ProgressPage/> })}
        </section>
        <section class="page" class:on=move || c.tab.get() == Tab::Fuel inert=move || c.tab.get() != Tab::Fuel aria-label="Fuel">
            {move || (c.tab.get() == Tab::Fuel).then(|| view! { <fuel::FuelPage/> })}
        </section>
        <section class="page" class:on=move || c.tab.get() == Tab::Connect inert=move || c.tab.get() != Tab::Connect aria-label="Connect">
            {move || (c.tab.get() == Tab::Connect).then(|| view! { <connect::ConnectPage/> })}
        </section>
        <NetStatus/>
        <nav class="tabbar" class:away=hide_bar aria-label="Sections">
            <i class="tabpill" style=move || format!("transform: translateX(calc({} * (100% + 2px)))", c.tab.get().index())></i>
            {Tab::ALL.into_iter().map(|t| {
                let d = match t {
                    Tab::Today => icons::CALENDAR,
                    Tab::Plan => icons::LIST,
                    Tab::Progress => icons::TREND,
                    Tab::Fuel => icons::FLAME,
                    Tab::Connect => icons::LINK,
                };
                view! {
                    <button type="button" class="tab" class:on=move || c.tab.get() == t
                        aria-current=move || (c.tab.get() == t).then_some("page")
                        on:click=move |_| { c.tab.set(t); c.sheet.set(None); }>
                        {icon(d)}
                        {t.label()}
                        {move || (t == Tab::Connect).then(|| {
                            let n = c.data.with(|d| d.as_ref().map(|d| d.pending_activity).unwrap_or(0));
                            (n > 0).then(|| view! { <span class="badge">{n}</span> })
                        })}
                    </button>
                }
            }).collect_view()}
        </nav>
        <sheets::Sheets/>
    }
}

/// A quiet line when the server is asleep or the phone is offline. Logging never waits for it.
#[component]
fn NetStatus() -> impl IntoView {
    let c = ctx();
    let text = move || {
        let pending = c.outbox.with(|o| o.len());
        match c.net.get() {
            Net::Offline if pending > 0 => Some(format!("Offline · {pending} saved on this phone")),
            Net::Offline => Some("Offline".into()),
            Net::Waking if pending > 0 => Some(format!("Waking the server · {pending} to send")),
            Net::Waking => Some("Waking the server".into()),
            Net::Ok => None,
        }
    };
    view! {
        {move || text().map(|t| view! {
            <div class="status pill glass fade-in" role="status">
                <span class="dot wait" class:err=move || c.net.get() == Net::Offline></span>{t}
            </div>
        })}
    }
}

#[component]
fn Toasts() -> impl IntoView {
    let c = ctx();
    view! {
        {move || c.toast.get().map(|t| {
            let action = t.action.clone();
            let solo = action.is_none();
            view! {
                <div class="toast" class:solo=solo role="status">
                    <span class="grow">{t.text.clone()}</span>
                    {action.map(|(label, a)| view! {
                        <button type="button" class="press" on:click=move |_| {
                            c.toast.set(None);
                            match a.clone() {
                                ToastAction::UndoPlan => spawn_local(async move {
                                    match net::post::<_, Option<gym_core::api::Plan>>("/api/plan/undo", &serde_json::json!({})).await {
                                        Ok(p) => {
                                            crate::views::plan::animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = p; }));
                                            c.refresh_soon();
                                        }
                                        Err(e) => c.fail(&e),
                                    }
                                }),
                                ToastAction::RestoreSet(s) => c.restore_set(s),
                            }
                        }>{label}</button>
                    })}
                </div>
            }
        })}
    }
}
