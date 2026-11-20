//! Connect: where data comes from, which agents can reach the plan, and what they changed.

use gym_core::api::{Activity, ActivityStatus, Agent, ProviderStatus};
use leptos::prelude::*;
use leptos::task::spawn_local;
use uuid::Uuid;

use crate::icons::{self, icon};
use crate::net;
use crate::state::{Sheet, ctx};
use crate::ui;

#[derive(Clone, Copy, PartialEq)]
enum Part {
    Sources,
    Agents,
}

#[component]
pub fn ConnectPage() -> impl IntoView {
    let c = ctx();
    let pending = move || c.data.with(|d| d.as_ref().map(|d| d.pending_activity).unwrap_or(0));
    let part = RwSignal::new(if pending() > 0 { Part::Agents } else { Part::Sources });
    view! {
        <div class="scroll">
            <div class="rowx between">
                <h1 class="title">"Connect"</h1>
                <button type="button" class="iconbtn small press" aria-label="Settings" on:click=move |_| c.sheet.set(Some(Sheet::Settings))>{icon(icons::SETTINGS)}</button>
            </div>
            <div class="seg" role="group" aria-label="Show">
                <i class="segthumb" style=move || format!("width: calc((100% - 6px) / 2); transform: translateX({}%)", if part.get() == Part::Agents { 100 } else { 0 })></i>
                <button type="button" class:on=move || part.get() == Part::Sources on:click=move |_| part.set(Part::Sources)>"Sources"</button>
                <button type="button" class:on=move || part.get() == Part::Agents on:click=move |_| part.set(Part::Agents)>
                    {move || if pending() > 0 { format!("Agents · {}", pending()) } else { "Agents".into() }}
                </button>
            </div>
            {move || match part.get() {
                Part::Sources => view! { <Sources/> }.into_any(),
                Part::Agents => view! { <Agents/> }.into_any(),
            }}
        </div>
    }
}

#[component]
fn Sources() -> impl IntoView {
    let c = ctx();
    let conn = move || c.data.with(|d| d.as_ref().map(|d| d.connections.clone()));
    view! {
        <div class="list fade-in">
            {move || conn().map(|k| {
                let delivered = k.health_last_delivery;
                view! {
                    <div class="row tall">
                        <span class="dot" class:ok=delivered.is_some() class:off=delivered.is_none() style="margin-top: 7px"></span>
                        <span class="stack g4 grow">
                            <span class="h2">"Apple Health"</span>
                            <span class="sub">"Workouts, energy burned, heart rate. Closes your session when the watch workout ends"</span>
                            <span class="meta">{match delivered { Some(t) => format!("Through Shortcuts on your iPhone · last delivery {}", ui::ago(t)), None if k.health_key_set => "Phone key made · nothing delivered yet".into(), None => "Not set up yet: run plate phone-key on the server".into() }}</span>
                        </span>
                    </div>
                    <div class="row tall">
                        <span class="dot" class:ok=delivered.is_some() class:off=delivered.is_none() style="margin-top: 7px"></span>
                        <span class="stack g4 grow">
                            <span class="h2">"Lifesum"</span>
                            <span class="sub">"Food and macros, by way of Apple Health"</span>
                            <span class="meta">"Day totals only. Turn on Apple Health in Lifesum's settings"</span>
                        </span>
                    </div>
                    <Provider name="Strava" key="strava" status=k.strava.clone() blurb="Runs and rides from the last 7 days. Never shown to agents"/>
                    <Provider name="Spotify" key="spotify" status=k.spotify.clone() blurb="Play, pause, skip and pick a playlist without leaving the app"/>
                }
            })}
        </div>
        <p class="sub">"Everything here is connected on the server, with the plate command. This app never holds a key, a password or a token"</p>
    }
}

#[component]
fn Provider(name: &'static str, key: &'static str, status: ProviderStatus, blurb: &'static str) -> impl IntoView {
    let c = ctx();
    let connected = status.connected;
    let meta = match (&status.account, &status.detail) {
        _ if !connected => format!("Not connected. Connect it on the server: plate authorize {key}"),
        (Some(a), Some(d)) => format!("{a} · {d}"),
        (None, Some(d)) => d.clone(),
        (Some(a), None) => a.clone(),
        (None, None) => "Connected".to_string(),
    };
    view! {
        <div class="row tall">
            <span class="dot" class:ok=connected class:off={!connected} style="margin-top: 7px"></span>
            <span class="stack g4 grow">
                <span class="h2">{name}</span>
                <span class="sub">{blurb}</span>
                <span class="meta">{meta}</span>
            </span>
            {(connected && key == "spotify").then(|| view! {
                <button type="button" class="btn btn-quiet btn-small press" on:click=move |_| c.sheet.set(Some(Sheet::Music))>"Player"</button>
            })}
        </div>
    }
}

#[component]
fn Agents() -> impl IntoView {
    let c = ctx();
    let agents: RwSignal<Vec<Agent>> = RwSignal::new(vec![]);
    let activity: RwSignal<Vec<Activity>> = RwSignal::new(vec![]);
    let loaded = RwSignal::new(false);
    let reload = move || {
        spawn_local(async move {
            let (a, b) = (net::get::<Vec<Agent>>("/api/agents").await, net::get::<Vec<Activity>>("/api/activity").await);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    agents.set(a);
                    activity.set(b);
                }
                (Err(e), _) | (_, Err(e)) => c.fail(&e),
            }
            loaded.set(true);
        });
    };
    reload();
    let patch = move |id: Uuid, body: serde_json::Value| {
        spawn_local(async move {
            match net::write::<_, Agent>("PATCH", &format!("/api/agents/{id}"), &body).await {
                Ok(_) => reload(),
                Err(e) => c.fail(&e),
            }
        });
    };
    let act = move |id: Uuid, what: &'static str| {
        spawn_local(async move {
            match net::call::<()>("POST", &format!("/api/activity/{id}/{what}"), None).await {
                Ok(()) => {
                    c.say(match what {
                        "undo" => "Undone",
                        "accept" => "Accepted",
                        _ => "Declined",
                    });
                    reload();
                    c.refresh().await;
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    view! {
        <div class="stack g24 fade-in">
            <div class="stack g8">
                <p class="sub">"Each agent has its own link, made on the server. It can read your plan, sessions and food totals, and write plans, lifts and clips. It can never change a logged set, or see Strava or Spotify"</p>
                <div class="list">
                    {move || agents.get().into_iter().map(|a| {
                        let id = a.id;
                        let ask = a.ask_first;
                        view! {
                            <div class="row tall">
                                <span class="dot" class:ok={!a.revoked} class:off=a.revoked style="margin-top: 7px"></span>
                                <span class="stack g4 grow">
                                    <span class="h2">{a.name.clone()}</span>
                                    <span class="meta">{if a.revoked { "Link revoked".to_string() } else { match a.last_used_at { Some(t) => format!("Last used {}", ui::ago(t)), None => "Not used yet".into() } }}</span>
                                    {(!a.revoked).then(|| view! {
                                        <span class="rowx g12" style="margin-top: 8px">
                                            <button type="button" class="toggle" role="switch" aria-checked=ask.to_string() aria-label="Ask me first" on:click=move |_| patch(id, serde_json::json!({ "ask_first": !ask }))></button>
                                            <span class="sub">"Ask me first"</span>
                                        </span>
                                    })}
                                </span>
                                {(!a.revoked).then(|| view! { <button type="button" class="btn btn-line btn-small btn-danger press" on:click=move |_| patch(id, serde_json::json!({ "revoke": true }))>"Revoke"</button> })}
                            </div>
                        }
                    }).collect_view()}
                </div>
                <p class="meta">"Add or revoke a link on the server: plate agent add Muse"</p>
            </div>
            <div class="stack g8">
                <span class="meta">"Changes by agents"</span>
                {move || (loaded.get() && activity.get().is_empty()).then(|| view! { <p class="sub">"Nothing yet. When an agent changes the plan, it shows here with an undo"</p> })}
                <div class="list">
                    {move || activity.get().into_iter().map(|x| {
                        let id = x.id;
                        let status = match x.status {
                            ActivityStatus::Applied => "Applied",
                            ActivityStatus::Proposed => "Waiting for you",
                            ActivityStatus::Queued => "Waits for your session to end",
                            ActivityStatus::Declined => "Declined",
                            ActivityStatus::Undone => "Undone",
                        };
                        view! {
                            <div class="row tall">
                                <span class="stack g4 grow">
                                    <span>{x.summary.clone()}</span>
                                    {x.detail.iter().take(4).map(|d| view! { <span class="sub">{d.clone()}</span> }).collect_view()}
                                    <span class="meta">{format!("{} · {} · {status}", x.agent_name, ui::ago(x.at))}</span>
                                </span>
                                <span class="stack g8">
                                    {(x.status == ActivityStatus::Proposed).then(|| view! {
                                        <button type="button" class="btn btn-accent btn-small press" on:click=move |_| act(id, "accept")>"Accept"</button>
                                        <button type="button" class="btn btn-line btn-small press" on:click=move |_| act(id, "decline")>"Decline"</button>
                                    })}
                                    {(x.status == ActivityStatus::Queued).then(|| view! { <button type="button" class="btn btn-line btn-small press" on:click=move |_| act(id, "decline")>"Cancel"</button> })}
                                    {x.can_undo.then(|| view! { <button type="button" class="btn btn-line btn-small press" on:click=move |_| act(id, "undo")>"Undo"</button> })}
                                </span>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}
