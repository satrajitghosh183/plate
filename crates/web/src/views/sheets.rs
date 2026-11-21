//! Bottom sheets: they rise over the screen behind, and drag down to close.

use gym_core::api::{ClipProvider, Lift, LiftPatch, MusicCommand, NowPlaying, Playlist, Prescription, Settings};
use gym_core::split::bucket;
use gym_core::units::format_number;
use gym_core::{Equipment, LiftTags, Mechanic, Muscle, Pattern, TagSource, Unit, Weight};
use leptos::prelude::*;
use leptos::task::spawn_local;
use uuid::Uuid;

use crate::icons::{self, icon, icon_fill};
use crate::net;
use crate::state::{Auth, Mode, Sheet, Tab, ctx, forget};
use crate::ui;

#[component]
pub fn Sheets() -> impl IntoView {
    let c = ctx();
    let open = move || c.sheet.get().is_some();
    let drag = RwSignal::new(0.0f64);
    let start = RwSignal::new(None::<(f64, f64)>);
    let close = move || {
        drag.set(0.0);
        c.sheet.set(None);
    };
    // Keep the last content while the sheet slides away.
    let shown = RwSignal::new(None::<Sheet>);
    Effect::new(move |_| {
        if let Some(s) = c.sheet.get() {
            shown.set(Some(s));
        }
    });
    view! {
        <div class="scrim" class:on=open on:click=move |_| close() style=move || { if drag.get() > 0.0 { format!("opacity: {:.3}; transition: none", (1.0 - drag.get() / 520.0).max(0.0)) } else { String::new() } }></div>
        <div class="sheet" class:on=open role="dialog" aria-modal="true" inert=move || !open()
            style=move || { if drag.get() > 0.0 { format!("transform: translateY({}px); transition: none", drag.get()) } else { String::new() } }>
            <div class="grabzone"
                on:pointerdown=move |e| {
                    if e.target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()).is_some_and(|el| el.closest("button, a, input").ok().flatten().is_some()) { return; }
                    start.set(Some((e.client_y() as f64, js_sys::Date::now())));
                    if let Some(t) = e.current_target().and_then(|t| wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(t).ok()) { let _ = t.set_pointer_capture(e.pointer_id()); }
                }
                on:pointermove=move |e| if let Some((y0, _)) = start.get_untracked() { drag.set((e.client_y() as f64 - y0).max(0.0)); }
                on:pointerup=move |_| {
                    if let Some((_, t0)) = start.get_untracked() {
                        let dy = drag.get_untracked();
                        let v = dy / (js_sys::Date::now() - t0).max(1.0);
                        start.set(None);
                        if dy > 130.0 || (v > 0.6 && dy > 30.0) { close(); } else { drag.set(0.0); }
                    }
                }
                on:pointercancel=move |_| { start.set(None); drag.set(0.0); }>
                <span class="grab"></span>
                <div class="rowx between g12">
                    <h2 class="h2 grow ellipsis">{move || shown.get().map(|s| title(&s))}</h2>
                    <button type="button" class="iconbtn small press" aria-label="Close" on:click=move |_| close()>{icon(icons::CLOSE)}</button>
                </div>
            </div>
            {move || shown.get().map(|s| match s {
                Sheet::Clips(id) => view! { <Clips id=id/> }.into_any(),
                Sheet::Lift(id) => view! { <LiftEditor id=id/> }.into_any(),
                Sheet::Library => view! { <Library/> }.into_any(),
                Sheet::Music => view! { <Music/> }.into_any(),
                Sheet::Settings => view! { <SettingsSheet/> }.into_any(),
                Sheet::SessionDetail(id) => view! { <SessionDetail id=id/> }.into_any(),
                Sheet::AddToSession => view! { <AddToSession/> }.into_any(),
                Sheet::ChooseDay => view! { <ChooseDay/> }.into_any(),
            })}
        </div>
    }
}

fn title(s: &Sheet) -> String {
    let c = ctx();
    match s {
        Sheet::Clips(id) | Sheet::Lift(id) => c.lift_name(*id),
        Sheet::Library => "All lifts".into(),
        Sheet::Music => "Music".into(),
        Sheet::Settings => "Settings".into(),
        Sheet::SessionDetail(_) => "Session".into(),
        Sheet::AddToSession => "Add a lift".into(),
        Sheet::ChooseDay => "Train a different day".into(),
    }
}

// ---------- clips ----------

#[component]
fn Clips(id: Uuid) -> impl IntoView {
    let c = ctx();
    let lift = move || c.lift(id);
    let index = RwSignal::new(0usize);
    let url = RwSignal::new(String::new());
    let add = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let u = url.get_untracked();
        if u.trim().is_empty() {
            return;
        }
        spawn_local(async move {
            match net::post::<_, Lift>(&format!("/api/lifts/{id}/clips"), &serde_json::json!({ "url": u.trim() })).await {
                Ok(l) => {
                    let n = l.clips.len();
                    c.data.update(|d| if let Some(d) = d && let Some(x) = d.lifts.iter_mut().find(|x| x.id == id) { *x = l; });
                    index.set(n.saturating_sub(1));
                    url.set(String::new());
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    view! {
        {move || {
            let l = lift()?;
            let clips = l.clips.clone();
            let i = index.get().min(clips.len().saturating_sub(1));
            let clip = clips.get(i).cloned();
            Some(view! {
                <span class="meta">{match &clip { Some(cl) => format!("Clip {} of {} · {}", i + 1, clips.len(), cl.provider.label()), None => "No clips yet".into() }}</span>
                <div class="vid" class:reel={clip.as_ref().is_some_and(|cl| cl.provider == ClipProvider::Instagram)}>
                    {match clip.clone() {
                        Some(cl) if cl.embed_url.is_some() => view! {
                            <iframe src=cl.embed_url.clone().unwrap_or_default() title=cl.title.clone() 
                                allow="autoplay; encrypted-media; picture-in-picture; fullscreen" referrerpolicy="strict-origin-when-cross-origin"></iframe>
                        }.into_any(),
                        Some(_) => view! { <div class="empty">{icon(icons::VIDEO_OFF)}<span class="h2">"This link can't play here"</span><span class="sub">"Open it below, or ask Muse for a clip that embeds"</span></div> }.into_any(),
                        None => view! { <div class="empty">{icon(icons::VIDEO_OFF)}<span class="h2">"No clip for this lift"</span><span class="sub">"Paste a YouTube or Instagram link below, or ask Muse to add one"</span></div> }.into_any(),
                    }}
                </div>
                {clip.clone().map(|cl| view! {
                    <a class="textlink" href=cl.url.clone() target="_blank" rel="noopener">
                        {format!("Won't play? Open in {}", if cl.provider == ClipProvider::Instagram { "Instagram" } else if cl.provider == ClipProvider::Youtube { "YouTube" } else { "the browser" })}
                    </a>
                })}
                {(!l.cues.is_empty()).then(|| view! {
                    <div class="stack g8">
                        <span class="h2">"Cues"</span>
                        <div class="stack g4">{l.cues.iter().map(|q| view! { <span class="sub">{q.clone()}</span> }).collect_view()}</div>
                    </div>
                })}
                {(clips.len() > 1).then(|| view! {
                    <div class="list">
                        {clips.iter().enumerate().map(|(k, cl)| view! {
                            <button type="button" class="row press" on:click=move |_| index.set(k)>
                                <span class="dot" class:ok={k == i} class:off={k != i}></span>
                                <span class="grow ellipsis" class:fg2={k != i}>{cl.title.clone()}</span>
                                <span class="meta">{cl.provider.label()}</span>
                            </button>
                        }).collect_view()}
                    </div>
                })}
            })
        }}
        <form class="rowx g8" on:submit=add>
            <input class="input grow" type="url" inputmode="url" placeholder="Paste a YouTube or Instagram link" aria-label="Clip link"
                prop:value=move || url.get() on:input=move |e| url.set(event_target_value(&e))/>
            <button type="submit" class="btn btn-quiet btn-small press" style="height: 48px; border-radius: 24px">"Add"</button>
        </form>
        {move || c.mode.get().in_session().then(|| view! {
            <button type="button" class="btn btn-quiet press" on:click=move |_| c.sheet.set(None)>"Back to the set"</button>
        })}
    }
}

// ---------- lift editor ----------

fn enum_label<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_value(v).ok().and_then(|x| x.as_str().map(str::to_string)).unwrap_or_default()
}

/// A select for one tag. Empty until something is picked, so an unsorted lift is never
/// saved with tags nobody chose.
#[component]
fn Pick<T: Copy + PartialEq + serde::Serialize + Send + Sync + 'static>(label: &'static str, options: Vec<(T, &'static str)>, value: RwSignal<Option<T>>) -> impl IntoView {
    view! {
        <div class="field">
            <label>{label}</label>
            <select class="input" on:change=move |e| {
                let v = event_target_value(&e);
                value.set(options.iter().find(|(o, _)| enum_label(o) == v).map(|(x, _)| *x));
            }>
                <option value="" disabled=true selected=move || value.get().is_none()>"Pick one"</option>
                {options.iter().map(|(o, l)| { let key = enum_label(o); let o = *o; view! { <option value=key selected=move || value.get() == Some(o)>{*l}</option> } }).collect_view()}
            </select>
        </div>
    }
}

#[component]
fn Stepper(label: &'static str, value: RwSignal<f64>, step: f64, min: f64, max: f64, #[prop(optional)] suffix: &'static str) -> impl IntoView {
    view! {
        <div class="row">
            <span class="fg2 grow">{label}</span>
            <button type="button" class="iconbtn small press" aria-label=format!("Less {label}") on:click=move |_| value.update(|v| *v = ((*v - step) * 100.0).round().max(min * 100.0) / 100.0)>{icon(icons::MINUS)}</button>
            <span class="num" style="min-width: 72px; text-align: center">{move || format!("{}{suffix}", format_number(value.get()))}</span>
            <button type="button" class="iconbtn small press" aria-label=format!("More {label}") on:click=move |_| value.update(|v| *v = ((*v + step) * 100.0).round().min(max * 100.0) / 100.0)>{icon(icons::PLUS)}</button>
        </div>
    }
}

#[component]
fn LiftEditor(id: Uuid) -> impl IntoView {
    let c = ctx();
    let Some(l) = c.lift(id) else { return view! { <p class="sub">"This lift was retired"</p> }.into_any() };
    let unit = c.unit();
    let name = RwSignal::new(l.name.clone());
    let sets = RwSignal::new(l.prescription.sets as f64);
    let reps = RwSignal::new(l.prescription.reps as f64);
    let weight = RwSignal::new(l.prescription.weight.display_value(unit));
    let rest = RwSignal::new(l.prescription.rest_seconds.map(|r| r as f64).unwrap_or(0.0));
    let cues = RwSignal::new(l.cues.join("\n"));
    let pattern = RwSignal::new(l.tags.as_ref().map(|t| t.pattern));
    let primary = RwSignal::new(l.tags.as_ref().map(|t| t.primary));
    let equipment = RwSignal::new(l.tags.as_ref().map(|t| t.equipment));
    let mechanic = RwSignal::new(l.tags.as_ref().map(|t| t.mechanic));
    let tags_touched = RwSignal::new(false);
    Effect::new(move |prev: Option<()>| {
        let _ = (pattern.get(), primary.get(), equipment.get(), mechanic.get());
        if prev.is_some() {
            tags_touched.set(true);
        }
    });
    let source = l.tag_source;
    let needs = l.tags.is_none();
    let original = l.clone();
    let saving = RwSignal::new(false);
    let save = move |_| {
        saving.set(true);
        let patch = LiftPatch {
            name: Some(name.get_untracked()).filter(|n| n.trim() != original.name),
            tags: match (tags_touched.get_untracked(), pattern.get_untracked(), primary.get_untracked(), equipment.get_untracked(), mechanic.get_untracked()) {
                (true, Some(pattern), Some(primary), Some(equipment), Some(mechanic)) => {
                    let secondary = original.tags.as_ref().map(|t| t.secondary.clone()).unwrap_or_default();
                    Some(LiftTags { pattern, primary, secondary, equipment, mechanic })
                }
                _ => None,
            },
            prescription: Some(Prescription {
                sets: sets.get_untracked() as u32,
                reps: reps.get_untracked() as u32,
                weight: Weight::from_unit(weight.get_untracked(), unit),
                rest_seconds: Some(rest.get_untracked() as u32).filter(|r| *r > 0),
            }),
            cues: Some(cues.get_untracked().lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()),
            archived: None,
        };
        spawn_local(async move {
            match net::write::<_, Lift>("PATCH", &format!("/api/lifts/{id}"), &patch).await {
                Ok(_) => {
                    c.refresh().await;
                    c.sheet.set(None);
                    c.say("Saved");
                }
                Err(e) => c.fail(&e),
            }
            saving.set(false);
        });
    };
    let resort = move |_| {
        spawn_local(async move {
            match net::post::<_, Lift>(&format!("/api/lifts/{id}/resort"), &serde_json::json!({})).await {
                Ok(l) => {
                    c.refresh().await;
                    c.say(if l.tags.is_some() { "Sorted" } else { "Still can't sort it from the name. Pick the tags yourself" });
                    c.sheet.set(None);
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    let retire = move |_| {
        spawn_local(async move {
            match net::write::<_, Lift>("PATCH", &format!("/api/lifts/{id}"), &serde_json::json!({ "archived": true })).await {
                Ok(_) => {
                    c.refresh().await;
                    c.sheet.set(None);
                    c.say("Retired. Its history stays");
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    let patterns: Vec<(Pattern, &'static str)> = Pattern::ALL.iter().map(|p| (*p, p.label())).collect();
    let muscles: Vec<(Muscle, &'static str)> = Muscle::ALL.iter().map(|m| (*m, m.label())).collect();
    let equip: Vec<(Equipment, &'static str)> = Equipment::ALL.iter().map(|e| (*e, e.label())).collect();
    view! {
        <div class="field">
            <label for="lift-name">"Name"</label>
            <input id="lift-name" class="input" maxlength="80" prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/>
        </div>
        <div class="list">
            <Stepper label="Sets" value=sets step=1.0 min=1.0 max=20.0/>
            <Stepper label="Reps" value=reps step=1.0 min=1.0 max=100.0/>
            <Stepper label="Weight" value=weight step=if unit == Unit::Kg { 2.5 } else { 5.0 } min=0.0 max=1000.0 suffix=if unit == Unit::Kg { " kg" } else { " lb" }/>
            <Stepper label="Rest" value=rest step=15.0 min=0.0 max=900.0 suffix=" s"/>
        </div>
        <span class="meta">{move || if rest.get() == 0.0 { "Rest 0 uses your default from Settings".to_string() } else { String::new() }}</span>
        <div class="stack g8">
            <span class="rowx between"><span class="h2">"Sorting"</span><span class="meta">{if needs { "Needs a look".to_string() } else { source.map(|s| s.label().to_string()).unwrap_or_default() }}</span></span>
            <div class="stack g12">
                <Pick label="Movement" options=patterns value=pattern/>
                <Pick label="Main muscle" options=muscles value=primary/>
                <Pick label="Equipment" options=equip value=equipment/>
                <Pick label="Kind" options=vec![(Mechanic::Compound, "Compound"), (Mechanic::Isolation, "Isolation")] value=mechanic/>
            </div>
            <span class="meta">{move || {
                let all = pattern.get().is_some() && primary.get().is_some() && equipment.get().is_some() && mechanic.get().is_some();
                if needs && !all { "Pick all four, then save. Until then it stays out of the plan".to_string() }
                else if tags_touched.get() { "Tags you set yourself are never changed by sorting or by agents".to_string() }
                else { String::new() }
            }}</span>
            {(source != Some(TagSource::User)).then(|| view! { <button type="button" class="textlink" on:click=resort>"Sort it again from the name"</button> })}
        </div>
        <div class="field">
            <label for="lift-cues">"Cues, one per line"</label>
            <textarea id="lift-cues" class="input" rows="3" prop:value=move || cues.get() on:input=move |e| cues.set(event_target_value(&e))></textarea>
        </div>
        <button type="button" class="row press" on:click=move |_| c.sheet.set(Some(Sheet::Clips(id)))>
            <span class="grow">"Clips"</span><span class="r">{format!("{}", l.clips.len())}</span>{icon(icons::CHEVRON)}
        </button>
        <button type="button" class="btn btn-accent press" disabled=move || saving.get() on:click=save>"Save"</button>
        <button type="button" class="textlink bad" on:click=retire>"Retire this lift"</button>
    }
    .into_any()
}

// ---------- library ----------

#[component]
fn Library() -> impl IntoView {
    let c = ctx();
    let name = RwSignal::new(String::new());
    let adding = RwSignal::new(false);
    let add = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        let n = name.get_untracked().trim().to_string();
        if n.is_empty() {
            return;
        }
        adding.set(true);
        spawn_local(async move {
            match net::post::<_, Lift>("/api/lifts", &serde_json::json!({ "name": n })).await {
                Ok(l) => {
                    name.set(String::new());
                    let msg = match &l.tags {
                        Some(t) => format!("Added {}. Sorted as {} · {}", l.name, t.pattern.label().to_lowercase(), t.primary.label().to_lowercase()),
                        None => format!("Added {}. It needs a look: pick its tags", l.name),
                    };
                    let id = l.id;
                    let needs = l.tags.is_none();
                    c.refresh().await;
                    c.say(msg);
                    if needs {
                        c.sheet.set(Some(Sheet::Lift(id)));
                    }
                }
                Err(e) => c.fail(&e),
            }
            adding.set(false);
        });
    };
    let groups = move || {
        let lifts = c.data.with(|d| d.as_ref().map(|d| d.lifts.clone()).unwrap_or_default());
        let mut out: Vec<(&'static str, Vec<Lift>)> = vec![("Needs a look", vec![]), ("Push", vec![]), ("Pull", vec![]), ("Legs", vec![]), ("Core", vec![])];
        for l in lifts {
            let key = l.tags.as_ref().map(|t| bucket(t).label()).unwrap_or("Needs a look");
            if let Some(g) = out.iter_mut().find(|g| g.0 == key) {
                g.1.push(l);
            }
        }
        out.into_iter().filter(|g| !g.1.is_empty()).collect::<Vec<_>>()
    };
    view! {
        <form class="rowx g8" on:submit=add>
            <input class="input grow" placeholder="Add a lift, for example Hip thrust" maxlength="80" aria-label="New lift"
                prop:value=move || name.get() on:input=move |e| name.set(event_target_value(&e))/>
            <button type="submit" class="btn btn-accent btn-small press" style="height: 48px; border-radius: 24px" disabled=move || adding.get()>"Add"</button>
        </form>
        <p class="meta">"New lifts sort themselves by name. Anything that can't be sorted waits under Needs a look"</p>
        {move || groups().into_iter().map(|(label, lifts)| view! {
            <div class="stack g8">
                <span class="meta">{label}</span>
                <div class="list">
                    {lifts.into_iter().map(|l| {
                        let id = l.id;
                        let line = l.tags.as_ref().map(|t| format!("{} · {}", t.primary.label(), t.equipment.label())).unwrap_or_else(|| "Pick its tags".into());
                        let (sr, wt) = ui::rx(&l.prescription, c.unit());
                        view! {
                            <button type="button" class="row press" on:click=move |_| c.sheet.set(Some(Sheet::Lift(id)))>
                                <span class="stack grow"><span class="ellipsis">{l.name.clone()}</span><span class="meta">{line}</span></span>
                                <span class="r meta num">{format!("{sr} · {wt}")}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
        }).collect_view()}
    }
}

// ---------- music ----------

#[component]
fn Music() -> impl IntoView {
    let c = ctx();
    let now: RwSignal<Option<NowPlaying>> = RwSignal::new(None);
    let lists: RwSignal<Vec<Playlist>> = RwSignal::new(vec![]);
    let load = move || {
        spawn_local(async move {
            match net::get::<NowPlaying>("/api/music/now").await {
                Ok(n) => now.set(Some(n)),
                Err(e) => c.fail(&e),
            }
        });
    };
    load();
    spawn_local(async move {
        if let Ok(p) = net::get::<Vec<Playlist>>("/api/music/playlists").await {
            lists.set(p);
        }
    });
    let poll = set_interval_with_handle(load, std::time::Duration::from_secs(10)).ok();
    on_cleanup(move || {
        if let Some(h) = poll {
            h.clear();
        }
    });
    let send = move |action: &'static str, playlist: Option<String>| {
        spawn_local(async move {
            match net::call("POST", "/api/music/command", Some(&MusicCommand { action: action.into(), playlist_id: playlist })).await {
                Ok(()) => {
                    gloo_timers::future::TimeoutFuture::new(450).await;
                    load();
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    view! {
        {move || now.get().map(|n| {
            let playing = n.playing;
            view! {
                <div class="rowx g12">
                    {match n.album_art.clone() { Some(a) => view! { <img class="cover" src=a alt="Album cover" style="width: 72px; height: 72px; border-radius: 12px"/> }.into_any(), None => view! { <span class="cover" style="width: 72px; height: 72px; border-radius: 12px"></span> }.into_any() }}
                    <span class="stack grow">
                        <span class="h2 ellipsis">{n.track.clone().unwrap_or_else(|| if n.no_device { "Nothing playing".into() } else { "Paused".into() })}</span>
                        <span class="sub ellipsis">{n.artist.clone().unwrap_or_default()}</span>
                        <span class="meta">{n.device.clone().map(|d| format!("On {d}")).unwrap_or_else(|| "Open Spotify once if your phone isn't listed".into())}</span>
                    </span>
                </div>
                <div class="rowx" style="justify-content: center; gap: 24px">
                    <button type="button" class="iconbtn glass press" aria-label="Previous" on:click=move |_| send("previous", None)>{icon_fill(icons::PREV)}</button>
                    <button type="button" class="iconbtn press" aria-label=if playing { "Pause" } else { "Play" } style="width: 64px; height: 64px; background: #f5f5f7; color: #000" on:click=move |_| send(if playing { "pause" } else { "play" }, None)>
                        {if playing { icon_fill(icons::PAUSE).into_any() } else { icon_fill(icons::PLAY).into_any() }}
                    </button>
                    <button type="button" class="iconbtn glass press" aria-label="Next" on:click=move |_| send("next", None)>{icon_fill(icons::NEXT)}</button>
                </div>
                {n.track_url.clone().map(|u| view! { <a class="textlink" href=u target="_blank" rel="noopener">"Listen on Spotify"</a> })}
            }
        })}
        {move || (!lists.get().is_empty()).then(|| view! {
            <div class="stack g8">
                <span class="meta">"Your playlists"</span>
                <div class="list">
                    {lists.get().into_iter().map(|p| {
                        let id = p.id.clone();
                        view! {
                            <button type="button" class="row press" on:click=move |_| send("playlist", Some(id.clone()))>
                                {match p.image.clone() { Some(i) => view! { <img class="cover" src=i alt=""/> }.into_any(), None => view! { <span class="cover"></span> }.into_any() }}
                                <span class="grow ellipsis">{p.name.clone()}</span>
                                {icon_fill(icons::PLAY)}
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
        })}
        <p class="meta">"Music from Spotify. Playback happens in the Spotify app"</p>
    }
}

// ---------- settings ----------

#[component]
fn SettingsSheet() -> impl IntoView {
    let c = ctx();
    let s = c.data.with_untracked(|d| d.as_ref().map(|d| d.settings.clone()).unwrap_or_default());
    let unit = RwSignal::new(s.unit);
    let rest = RwSignal::new(s.rest_seconds as f64);
    let kcal = RwSignal::new(s.kcal_goal as f64);
    let protein = RwSignal::new(s.protein_goal_g as f64);
    let save = move |_| {
        let body = Settings { unit: unit.get_untracked(), rest_seconds: rest.get_untracked() as u32, kcal_goal: kcal.get_untracked() as u32, protein_goal_g: protein.get_untracked() as u32 };
        spawn_local(async move {
            match net::write::<_, Settings>("PUT", "/api/settings", &body).await {
                Ok(_) => {
                    c.drafts.update(|d| d.0.clear());
                    c.refresh().await;
                    c.sheet.set(None);
                    c.say("Saved");
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    let add_key = move |_| {
        spawn_local(async move {
            match crate::views::signin::add_passkey().await {
                Ok(()) => c.say("Passkey added"),
                Err(e) => c.say(e),
            }
        });
    };
    let sign_out = move |_| {
        spawn_local(async move {
            let _ = net::call::<()>("POST", "/api/auth/logout", None).await;
            forget("gym.cache");
            forget("gym.session");
            forget("gym.outbox");
            c.data.set(None);
            c.sheet.set(None);
            c.tab.set(Tab::Today);
            c.auth.set(Auth::SignedOut);
        });
    };
    view! {
        <div class="stack g8">
            <span class="meta">"Units"</span>
            <div class="seg" role="group" aria-label="Units">
                <i class="segthumb" style=move || format!("width: calc((100% - 6px) / 2); transform: translateX({}%)", if unit.get() == Unit::Lb { 100 } else { 0 })></i>
                <button type="button" class:on=move || unit.get() == Unit::Kg on:click=move |_| unit.set(Unit::Kg)>"Kilograms"</button>
                <button type="button" class:on=move || unit.get() == Unit::Lb on:click=move |_| unit.set(Unit::Lb)>"Pounds"</button>
            </div>
            <span class="meta">"Logged sets keep the unit they were logged in; everything is shown in this one"</span>
        </div>
        <div class="list">
            <Stepper label="Default rest" value=rest step=15.0 min=15.0 max=900.0 suffix=" s"/>
            <Stepper label="Calories a day" value=kcal step=50.0 min=800.0 max=8000.0/>
            <Stepper label="Protein a day" value=protein step=5.0 min=0.0 max=600.0 suffix=" g"/>
        </div>
        <button type="button" class="btn btn-accent press" on:click=save>"Save"</button>
        <div class="list">
            <a class="row press" href="/api/export" download="">
                <span class="stack grow"><span>"Download everything"</span><span class="sub">"Lifts, plans, sessions and food totals as one file"</span></span>{icon(icons::CHEVRON)}
            </a>
            <button type="button" class="row press" on:click=add_key>
                <span class="stack grow"><span>"Add a passkey"</span><span class="sub">"For a new phone. You stay signed in here"</span></span>{icon(icons::KEY)}
            </button>
            <button type="button" class="row press" on:click=sign_out><span class="grow">"Sign out"</span></button>
        </div>
    }
}

// ---------- sessions ----------

#[component]
fn SessionDetail(id: Uuid) -> impl IntoView {
    let c = ctx();
    view! {
        {move || c.session_view(id).map(|s| {
            let unit = c.unit();
            let mut ids: Vec<Uuid> = Vec::new();
            for x in &s.sets { if !ids.contains(&x.lift_id) { ids.push(x.lift_id); } }
            view! {
                <div class="stack g4">
                    <span class="h3">{s.day_name.clone()}</span>
                    <span class="meta">{format!("{} · {} · {} {} moved", ui::day_label(ui::local(s.started_at).date_naive()), ui::clock(s.started_at), gym_core::units::format_thousands(Weight(s.volume()).display_value(unit).round() as i64), unit.label())}</span>
                    {s.close_reason.map(|r| view! { <span class="meta">{r.label()}</span> })}
                    {s.workout.clone().map(|w| view! { <span class="meta">{[w.duration_min.map(|m| format!("{} min", m.round())), w.energy_kcal.map(|k| format!("{} kcal", k.round())), w.avg_heart_rate.map(|h| format!("average {} bpm", h.round()))].into_iter().flatten().collect::<Vec<_>>().join(" · ")}</span> })}
                </div>
                <div class="list">
                    {ids.into_iter().map(|lid| {
                        let sets: Vec<_> = s.sets.iter().filter(|x| x.lift_id == lid).cloned().collect();
                        view! {
                            <div class="row tall">
                                <span class="stack grow g8">
                                    <span>{c.lift_name(lid)}</span>
                                    <span class="rowx wrap g8">{sets.into_iter().map(|x| {
                                        let label = format!("{} × {}", x.weight.display(unit), x.reps);
                                        view! { <button type="button" class="chip del press" aria-label=format!("Delete set {label}") on:click=move |_| c.delete_set(&x)>{label.clone()}{crate::icons::icon(crate::icons::CLOSE)}</button> }
                                    }).collect_view()}</span>
                                </span>
                            </div>
                        }
                    }).collect_view()}
                </div>
                <p class="meta">"Tap a set to delete it. Only you can change logged sets"</p>
            }
        })}
    }
}

#[component]
fn AddToSession() -> impl IntoView {
    let c = ctx();
    let lifts = move || {
        let inside = c.session.with(|s| s.as_ref().map(|s| s.lifts.clone()).unwrap_or_default());
        c.data.with(|d| d.as_ref().map(|d| d.lifts.iter().filter(|l| !inside.contains(&l.id)).cloned().collect::<Vec<_>>()).unwrap_or_default())
    };
    view! {
        <div class="list">
            {move || lifts().into_iter().map(|l| {
                let id = l.id;
                view! {
                    <button type="button" class="row press" on:click=move |_| {
                        c.session.update(|s| if let Some(s) = s { s.lifts.push(id); s.current = s.lifts.len() - 1; });
                        c.persist_session();
                        c.sheet.set(None);
                        c.mode.set(Mode::Logger);
                    }>
                        <span class="stack grow"><span>{l.name.clone()}</span><span class="meta">{ui::bucket_label(&l)}</span></span>
                        {icon(icons::PLUS)}
                    </button>
                }
            }).collect_view()}
        </div>
    }
}

#[component]
fn ChooseDay() -> impl IntoView {
    let c = ctx();
    view! {
        <div class="list">
            {move || c.plan().map(|p| p.days.into_iter().enumerate().map(|(i, d)| view! {
                <button type="button" class="row press" on:click=move |_| { c.chosen_day.set(Some(i)); c.sheet.set(None); }>
                    <span class="stack grow"><span>{d.name.clone()}</span><span class="meta">{format!("{} · {} lifts", ui::weekday_name(d.weekday), d.lifts.len())}</span></span>
                    {icon(icons::CHEVRON)}
                </button>
            }).collect_view())}
        </div>
        <button type="button" class="textlink" on:click=move |_| { c.chosen_day.set(None); c.sheet.set(None); }>"Back to today's plan"</button>
    }
}
