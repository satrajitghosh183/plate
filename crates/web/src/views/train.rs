//! Today and the session: the 3D hero behind a glass pane.
//! The pane settles at a different height for each step; the hero swaps from the
//! body model to the loaded bar while you log.

use chrono::Utc;
use gym_core::api::{Lift, Session};
use gym_core::plates::{Loading, load};
use gym_core::progress::{best_e1rm, e1rm};
use gym_core::split::Bucket;
use gym_core::units::{format_number, format_thousands};
use gym_core::{Equipment, Unit, Weight};
use leptos::prelude::*;
use leptos::task::spawn_local;
use uuid::Uuid;

use crate::icons::{self, icon};
use crate::net;
use crate::state::{Mode, Sheet, Tab, ctx};
use crate::ui::{self, Roll};

#[component]
pub fn Train() -> impl IntoView {
    let c = ctx();
    let mode_key = move || {
        if c.plan().is_none() && c.session.get().is_none() {
            return "empty";
        }
        c.mode.get().key()
    };
    let current_lift = Memo::new(move |_| c.session.get().and_then(|s| s.current_lift()).and_then(|id| c.lift(id)));
    let nobar = move || current_lift.get().and_then(|l| l.tags.map(|t| !t.equipment.uses_plates())).unwrap_or(false);
    let drawn = move || current_lift.get().and_then(|l| l.tags.map(|t| matches!(t.equipment, Equipment::Dumbbell | Equipment::Cable | Equipment::Machine | Equipment::Kettlebell))).unwrap_or(false);

    // When rest runs out: chime and go back to the bar.
    Effect::new(move |_| {
        let now = c.now.get();
        if c.mode.get_untracked() != Mode::Rest {
            return;
        }
        if let Some(until) = c.session.with_untracked(|s| s.as_ref().and_then(|s| s.rest_until))
            && now >= until
        {
            crate::bridge::chime();
            c.session.update(|s| {
                if let Some(s) = s {
                    s.rest_until = None;
                }
            });
            c.persist_session();
            c.mode.set(Mode::Logger);
        }
    });

    view! {
        <div class="train" data-mode=mode_key class:nobar=nobar class:drawn=drawn>
            <Hero current_lift=current_lift/>
            <div class="overbar" class:on=move || matches!(c.mode.get(), Mode::Today | Mode::Summary(_)) && c.plan().is_some()>
                <span class="meta" style="padding-left: 8px; color: var(--fg2)">{move || { c.now.get(); ui::day_label(ui::today()) }}</span>
                {move || {
                    let rest = today_index().is_none();
                    let has_session = c.session.with(|s| s.as_ref().is_some_and(|s| !s.closed && s.ended_at.is_none()));
                    let label = if has_session { "In progress" } else if rest { "Rest day" } else { "On plan" };
                    view! { <span class="pill glass"><span class="dot ok" class:off={rest && !has_session}></span>{label}</span> }
                }}
            </div>
            <LiftBar current_lift=current_lift/>
            <div class="pane">
                <TodayPane/>
                <EmptyPane/>
                <SessionPane/>
                <LoggerPane current_lift=current_lift/>
                <RestPane current_lift=current_lift/>
                <SummaryPane/>
            </div>
        </div>
    }
}

/// The plan day for today, or the day you picked.
pub fn today_index() -> Option<usize> {
    let c = ctx();
    c.chosen_day.get().or_else(|| c.data.with(|d| d.as_ref().and_then(|d| d.today.day_index)))
}

fn day_lifts(day: Option<usize>) -> Vec<Lift> {
    let c = ctx();
    let Some(plan) = c.plan() else { return vec![] };
    day.and_then(|i| plan.days.get(i)).map(|d| d.lifts.iter().filter_map(|id| c.lift(*id)).collect()).unwrap_or_default()
}

// ---------- hero ----------

struct PlateBox {
    key: String,
    style: [String; 5],
}

fn plate_boxes(l: &Loading) -> (Vec<PlateBox>, i32) {
    let cy = 210i32;
    let mut x = 152i32;
    let mut out = Vec::new();
    for (i, p) in l.per_side.iter().enumerate() {
        let r = p.radius as i32;
        let t = p.thickness as i32;
        let rx = (r as f64 * 0.28).round() as i32;
        let fx = x + t;
        let rw = (rx as f64 * 2.0 * 0.64).round() as i32;
        let rh = (r as f64 * 2.0 * 0.64).round() as i32;
        let hw = (rx as f64 * 2.0 * 0.3).round() as i32;
        let hh = (r as f64 * 2.0 * 0.3).round() as i32;
        let [c0, c1, c2] = p.colors;
        let top = cy - r;
        let h = r * 2;
        out.push(PlateBox {
            key: format!("{i}:{}", p.weight),
            style: [
                format!("left:{}px;top:{top}px;width:{}px;height:{h}px;border-radius:50%;background:{c2}", x - rx, rx * 2),
                format!("left:{x}px;top:{top}px;width:{t}px;height:{h}px;background:linear-gradient(180deg,{c2} 0%,{c0} 14%,{c1} 26%,{c0} 42%,{c2} 100%)"),
                format!("left:{}px;top:{top}px;width:{}px;height:{h}px;border-radius:50%;background:radial-gradient(ellipse at 35% 26%,{c1} 0%,{c0} 46%,{c2} 100%);box-shadow:inset 0 0 0 2px rgba(0,0,0,.2)", fx - rx, rx * 2),
                format!("left:{}px;top:{}px;width:{rw}px;height:{rh}px;border-radius:50%;box-shadow:inset 0 0 0 1px rgba(255,255,255,.3),0 0 0 1px rgba(0,0,0,.22)", fx - rw / 2, cy - rh / 2),
                format!("left:{}px;top:{}px;width:{hw}px;height:{hh}px;border-radius:50%;background:radial-gradient(ellipse at 35% 28%,#f2f2f5,#9a9aa0 55%,#4a4a4f)", fx - hw / 2, cy - hh / 2),
            ],
        });
        x = fx + 3;
    }
    let sleeve = if out.is_empty() { 152 } else { x - 3 };
    (out, sleeve)
}

#[component]
fn Hero(current_lift: Memo<Option<Lift>>) -> impl IntoView {
    let c = ctx();
    let weight = Memo::new(move |_| {
        let Some(l) = current_lift.get() else { return c.unit().bar() };
        draft(l.id).0
    });
    let loading = Memo::new(move |_| load(weight.get(), c.unit()));
    // The body render that lights the day's main muscle group.
    let render = Memo::new(move |_| {
        let lifts: Vec<Lift> = match c.session.get() {
            Some(s) if !s.closed => s.lifts.iter().filter_map(|id| c.lift(*id)).collect(),
            _ => day_lifts(today_index().or_else(|| c.data.with(|d| d.as_ref().and_then(|d| d.today.next_day_index)))),
        };
        let mut counts = [0usize; 4];
        for l in &lifts {
            if let Some(t) = &l.tags {
                counts[match gym_core::split::bucket(t) { Bucket::Push => 0, Bucket::Pull => 1, Bucket::Legs => 2, Bucket::Core => 3 }] += 1;
            }
        }
        let total: usize = counts.iter().sum();
        let top = (0..3).max_by_key(|&i| counts[i]).unwrap_or(0);
        // Only a clear majority picks a group; a mixed day keeps the push render.
        match (total > 0 && counts[top] * 2 > total, top) {
            (true, 1) => ("/body-pull.jpg", ""),
            (true, 2) => ("/body-legs.jpg", "legs"),
            _ => ("/body.jpg", ""),
        }
    });
    view! {
        <div class="hero" role="img" aria-label=move || loading.get().describe(c.unit())>
            <div class="rig">
                <img class=move || format!("body {}", render.get().1) src=move || render.get().0 alt=""/>
                <div class="bell" aria-hidden="true">
                    <i style="left:70px;top:300px;width:280px;height:36px;background:radial-gradient(closest-side,rgba(255,255,255,.13),rgba(255,255,255,0));border-radius:50%"></i>
                    <i style="left:-20px;top:203px;width:172px;height:14px;background:linear-gradient(180deg,#4a4a4f 0%,#f2f2f5 28%,#9a9aa0 52%,#3a3a3e 100%)"></i>
                    <i style="left:132px;top:186px;width:12px;height:48px;border-radius:50%;background:#55555a"></i>
                    <i style="left:138px;top:186px;width:8px;height:48px;background:linear-gradient(180deg,#4a4a4f 0%,#f2f2f5 26%,#9a9aa0 50%,#3a3a3e 100%)"></i>
                    <i style="left:140px;top:186px;width:12px;height:48px;border-radius:50%;background:radial-gradient(ellipse at 35% 28%,#f2f2f5,#9a9aa0 55%,#4a4a4f)"></i>
                    <For
                        each=move || plate_boxes(&loading.get()).0
                        key=|p| p.key.clone()
                        children=move |p| {
                            let [a, b, cc, d, e] = p.style;
                            view! { <div class="plate"><i style=a></i><i style=b></i><i style=cc></i><i style=d></i><i style=e></i></div> }
                        }
                    />
                    <i style=move || { let s = plate_boxes(&loading.get()).1; format!("left:{s}px;top:197px;width:{}px;height:26px;background:linear-gradient(180deg,#4a4a4f 0%,#f7f7fa 24%,#a6a6ac 48%,#3a3a3e 100%)", (326 - s).max(0)) }></i>
                    <i style="left:322px;top:197px;width:8px;height:26px;border-radius:50%;background:radial-gradient(ellipse at 40% 30%,#ffffff,#b4b4ba 60%,#6a6a70)"></i>
                </div>
                <span class="load meta num">{move || {
                    let unit = c.unit();
                    let eq = current_lift.get().and_then(|l| l.tags.map(|t| t.equipment)).unwrap_or(Equipment::Barbell);
                    let w = format_number(weight.get());
                    match eq {
                        Equipment::Barbell | Equipment::EzBar => {
                            let l = loading.get();
                            let mut s = l.describe(unit);
                            if l.leftover_per_side > 0.0 {
                                s.push_str(&format!(" · {} {} per side can't be made", format_number(l.leftover_per_side), unit.label()));
                            }
                            s
                        }
                        Equipment::Dumbbell => format!("A pair of {w} {} dumbbells", unit.label()),
                        Equipment::Kettlebell => format!("{w} {} kettlebell", unit.label()),
                        Equipment::Cable | Equipment::Machine => format!("Pin at {w} {}", unit.label()),
                        Equipment::Bodyweight if weight.get() > 0.0 => format!("Bodyweight plus {w} {}", unit.label()),
                        _ => String::new(),
                    }
                }}</span>
                {move || current_lift.get().and_then(|l| {
                    let eq = l.tags.as_ref().map(|t| t.equipment)?;
                    if eq.uses_plates() || !c.mode.get().in_session() { return None; }
                    let w = weight.get();
                    Some(match eq {
                        Equipment::Dumbbell => dumbbell().into_any(),
                        Equipment::Cable | Equipment::Machine => stack(w, c.unit()).into_any(),
                        Equipment::Kettlebell => kettlebell().into_any(),
                        _ => view! { <div class="equip" aria-hidden="true"><div class="bigname">{equip_word(eq)}</div></div> }.into_any(),
                    })
                })}
            </div>
        </div>
    }
}

const STEEL: &str = "linear-gradient(180deg,#4a4a4f 0%,#f2f2f5 28%,#9a9aa0 52%,#3a3a3e 100%)";
const RUBBER: [&str; 3] = ["#2c2c30", "#5a5a60", "#0e0e10"];

/// One dumbbell, drawn like the bar: rubber heads on a knurled steel handle.
fn dumbbell() -> impl IntoView {
    let [c0, c1, c2] = RUBBER;
    let head = move |x: i32| {
        let (r, t, cy) = (58, 34, 210);
        let rx = (r as f64 * 0.3).round() as i32;
        view! {
            <i style=format!("left:{}px;top:{}px;width:{}px;height:{}px;border-radius:50%;background:{c2}", x - rx, cy - r, rx * 2, r * 2)></i>
            <i style=format!("left:{x}px;top:{}px;width:{t}px;height:{}px;background:linear-gradient(180deg,{c2} 0%,{c0} 14%,{c1} 28%,{c0} 44%,{c2} 100%)", cy - r, r * 2)></i>
            <i style=format!("left:{}px;top:{}px;width:{}px;height:{}px;border-radius:50%;background:radial-gradient(ellipse at 35% 26%,{c1} 0%,{c0} 50%,{c2} 100%);box-shadow:inset 0 0 0 2px rgba(255,255,255,.08)", x + t - rx, cy - r, rx * 2, r * 2)></i>
            <i style=format!("left:{}px;top:{}px;width:{}px;height:{}px;border-radius:50%;background:radial-gradient(ellipse at 35% 28%,#f2f2f5,#9a9aa0 55%,#4a4a4f)", x + t - 6, cy - 9, 12, 18)></i>
        }
    };
    view! {
        <div class="kitrig" aria-hidden="true">
            <i style="left:70px;top:300px;width:250px;height:36px;background:radial-gradient(closest-side,rgba(255,255,255,.12),rgba(255,255,255,0));border-radius:50%"></i>
            {head(96)}
            <i style=format!("left:130px;top:202px;width:136px;height:16px;border-radius:3px;background:{STEEL}")></i>
            <i style="left:150px;top:202px;width:96px;height:16px;background:repeating-linear-gradient(90deg,rgba(0,0,0,.18) 0 2px,transparent 2px 5px)"></i>
            {head(262)}
        </div>
    }
}

/// A cable or machine stack, with the pin at the plate that matches the weight.
fn stack(weight: f64, unit: Unit) -> impl IntoView {
    let step = if unit == Unit::Kg { 5.0 } else { 10.0 };
    let plates = 14usize;
    let pinned = ((weight / step).round() as usize).clamp(1, plates);
    view! {
        <div class="kitrig" aria-hidden="true">
            <i style="left:70px;top:330px;width:250px;height:30px;background:radial-gradient(closest-side,rgba(255,255,255,.1),rgba(255,255,255,0));border-radius:50%"></i>
            <i style=format!("left:189px;top:40px;width:12px;height:280px;border-radius:6px;background:{STEEL};opacity:.7")></i>
            {(0..plates).map(|i| {
                let top = 96 + i as i32 * 16;
                let lifted = i < pinned;
                let face = if lifted { "linear-gradient(180deg,#d9d9de,#8a8a90 60%,#4a4a4f)" } else { "linear-gradient(180deg,#3a3a3e,#1c1c1e)" };
                view! { <i style=format!("left:115px;top:{top}px;width:160px;height:13px;border-radius:2px;background:{face};box-shadow:inset 0 1px 0 rgba(255,255,255,.18)")></i> }
            }).collect_view()}
            <i style=format!("left:270px;top:{}px;width:34px;height:8px;border-radius:4px;background:radial-gradient(ellipse at 30% 30%,#ffffff,#9a9aa0 60%,#4a4a4f)", 96 + (pinned as i32 - 1) * 16 + 2)></i>
        </div>
    }
}

fn kettlebell() -> impl IntoView {
    view! {
        <div class="kitrig" aria-hidden="true">
            <i style="left:90px;top:318px;width:210px;height:30px;background:radial-gradient(closest-side,rgba(255,255,255,.12),rgba(255,255,255,0));border-radius:50%"></i>
            <i style="left:135px;top:80px;width:120px;height:110px;border-radius:60px 60px 18px 18px;border:16px solid #3a3a3e;border-bottom:0;box-shadow:inset 0 2px 0 rgba(255,255,255,.12)"></i>
            <i style="left:105px;top:150px;width:180px;height:170px;border-radius:50%;background:radial-gradient(ellipse at 34% 28%,#6a6a70 0%,#2c2c30 45%,#0e0e10 100%);box-shadow:inset 0 2px 0 rgba(255,255,255,.1)"></i>
        </div>
    }
}

fn equip_word(e: Equipment) -> &'static str {
    match e {
        Equipment::Dumbbell => "Dumbbells",
        Equipment::Cable => "Cable",
        Equipment::Machine => "Machine",
        Equipment::Bodyweight => "Bodyweight",
        Equipment::Kettlebell => "Kettlebell",
        Equipment::Band => "Band",
        _ => "",
    }
}

#[component]
fn LiftBar(current_lift: Memo<Option<Lift>>) -> impl IntoView {
    let c = ctx();
    let elapsed = move || {
        let s = c.session.get()?;
        let secs = (c.now.get() - s.started_at).num_seconds().max(0);
        Some(format!("{}:{:02}", secs / 60, secs % 60))
    };
    let position = move || {
        let s = c.session.get()?;
        Some(format!("Lift {} of {}", s.current + 1, s.lifts.len().max(1)))
    };
    let spotify = move || c.data.with(|d| d.as_ref().is_some_and(|d| d.connections.spotify.connected));
    view! {
        <div class="overbar" class:on=move || matches!(c.mode.get(), Mode::Logger | Mode::Rest | Mode::Session)>
            <button type="button" class="iconbtn glass press" aria-label="All lifts in this session"
                on:click=move |_| c.mode.set(if c.mode.get_untracked() == Mode::Session { Mode::Logger } else { Mode::Session })>
                {move || if c.mode.get() == Mode::Session { icon(icons::CLOSE).into_any() } else { icon(icons::LIST).into_any() }}
            </button>
            <span class="meta num" style="color: var(--fg2)">{move || match (position(), elapsed()) { (Some(p), Some(e)) => format!("{p} · {e}"), _ => String::new() }}</span>
            <span class="rowx g8">
                {move || spotify().then(|| view! {
                    <button type="button" class="iconbtn glass press" aria-label="Music" on:click=move |_| c.sheet.set(Some(Sheet::Music))>{icon(icons::MUSIC)}</button>
                })}
                <button type="button" class="iconbtn glass press" aria-label="Watch how to do this lift"
                    on:click=move |_| if let Some(l) = current_lift.get_untracked() { c.sheet.set(Some(Sheet::Clips(l.id))) }>
                    {icon(icons::PLAY)}
                </button>
            </span>
        </div>
    }
}

// ---------- Today ----------

#[component]
fn TodayPane() -> impl IntoView {
    let c = ctx();
    let on = move || c.mode.get() == Mode::Today && c.plan().is_some();
    let day = move || today_index();
    let rest_day = move || day().is_none();
    let next = move || c.data.with(|d| d.as_ref().and_then(|d| d.today.next_day_index));
    let shown = move || day().or_else(next);
    let lifts = move || day_lifts(shown());
    let plan = move || c.plan();
    let default_rest = move || c.data.with(|d| d.as_ref().map(|d| d.settings.rest_seconds).unwrap_or(120));
    let in_progress = move || c.session.with(|s| s.as_ref().is_some_and(|s| !s.closed && s.ended_at.is_none()));
    view! {
        <div class="pc tabbed" class:on=on inert=move || !on()>
            <div class="stack g8">
                <span class="meta">{move || plan().map(|p| match (p.block_week, p.block_weeks) {
                    (Some(w), Some(n)) => format!("{} · week {w} of {n}", p.name),
                    _ => p.name.clone(),
                })}</span>
                <h1 class="display">{move || {
                    let p = plan();
                    match (rest_day(), shown(), p) {
                        (true, _, _) => "Rest day".to_string(),
                        (false, Some(i), Some(p)) => day_title(&p.days[i].name),
                        _ => "Today".to_string(),
                    }
                }}</h1>
                <p class="sub">{move || {
                    let l = lifts();
                    if rest_day() {
                        match (next(), plan()) {
                            (Some(n), Some(p)) => format!("Next is {} on {}. Rest, or train it now", p.days[n].name.to_lowercase(), ui::weekday_name(p.days[n].weekday)),
                            _ => "Nothing planned today".into(),
                        }
                    } else if l.is_empty() {
                        "No lifts on this day yet. Ask Muse to fill it, or add one from the plan".into()
                    } else {
                        format!("{} · {} lifts, about {} minutes", ui::muscles_of(&l), ui::count_word(l.len()), ui::minutes_of(&l, default_rest()))
                    }
                }}</p>
            </div>
            <div>
                <For each=lifts key=move |l| (l.id, l.prescription.sets, l.prescription.reps, l.prescription.weight.0, c.unit()) children=move |l| {
                    let (sr, wt) = ui::rx(&l.prescription, c.unit());
                    let id = l.id;
                    view! {
                        <button type="button" class="lift press" on:click=move |_| c.sheet.set(Some(Sheet::Clips(id)))>
                            <span class="ellipsis">{l.name.clone()}</span><span class="m">{sr}</span><span class="m">{wt}</span>
                        </button>
                    }
                }/>
            </div>
            <div class="stack g8 push">
                {move || if in_progress() {
                    view! { <button type="button" class="btn btn-accent press" on:click=move |_| c.mode.set(Mode::Logger)>"Back to your session"</button> }.into_any()
                } else {
                    let label = if rest_day() { shown().and_then(|i| plan().map(|p| format!("Train {} now", p.days[i].name.to_lowercase()))).unwrap_or_else(|| "Start a session".into()) } else { "Start session".into() };
                    view! { <button type="button" class="btn btn-accent press" disabled=move || lifts().is_empty() on:click=move |_| c.start_session(shown())>{label}</button> }.into_any()
                }}
                {move || (plan().is_some_and(|p| p.days.len() > 1) && !in_progress()).then(|| view! {
                    <button type="button" class="textlink" on:click=move |_| c.sheet.set(Some(Sheet::ChooseDay))>"Train a different day"</button>
                })}
            </div>
        </div>
    }
}

#[component]
fn EmptyPane() -> impl IntoView {
    let c = ctx();
    let busy = RwSignal::new(false);
    view! {
        <div class="pc tabbed" class:on=move || c.plan().is_none() && c.session.get().is_none() inert=move || !(c.plan().is_none() && c.session.get().is_none())>
            <div class="stack g8">
                <span class="meta">"Welcome"</span>
                <h1 class="display">"No plan yet"</h1>
                <p class="sub">"Give Muse the agent link and ask for a plan, or start from a push, pull, legs week of fourteen lifts and change it later"</p>
            </div>
            <div class="stack g8 push">
                <button type="button" class="btn btn-accent press" disabled=move || busy.get() on:click=move |_| {
                    busy.set(true);
                    spawn_local(async move {
                        match net::post::<_, Option<gym_core::api::Plan>>("/api/plan/starter", &serde_json::json!({})).await {
                            Ok(_) => { c.refresh().await; c.say("Strength base is ready. Push, pull, legs, three days a week"); }
                            Err(e) => c.fail(&e),
                        }
                        busy.set(false);
                    });
                }>"Start from the strength base"</button>
                <button type="button" class="btn btn-quiet press" on:click=move |_| c.tab.set(Tab::Connect)>"Connect an agent"</button>
            </div>
        </div>
    }
}

// ---------- session overview ----------

#[component]
fn SessionPane() -> impl IntoView {
    let c = ctx();
    let rows = move || {
        let s = c.session.get()?;
        Some(s.lifts.iter().enumerate().filter_map(|(i, id)| c.lift(*id).map(|l| (i, l, s.sets_for(*id).len()))).collect::<Vec<_>>())
    };
    let all_done = move || rows().is_some_and(|r| r.iter().all(|(_, l, n)| *n as u32 >= l.prescription.sets));
    view! {
        <div class="pc" class:on=move || c.mode.get() == Mode::Session inert=move || c.mode.get() != Mode::Session>
            <div class="stack g4">
                <h1 class="title">{move || c.session.get().map(|s| s.day_name).unwrap_or_default()}</h1>
                <span class="meta num">{move || c.session.get().map(|s| format!("Started {} · {} {} logged", ui::clock(s.started_at), s.sets.len(), if s.sets.len() == 1 { "set" } else { "sets" })).unwrap_or_default()}</span>
            </div>
            <div class="list">
                {move || rows().unwrap_or_default().into_iter().map(|(i, l, done)| {
                    let target = l.prescription.sets as usize;
                    let current = c.session.with(|s| s.as_ref().map(|s| s.current) == Some(i));
                    view! {
                        <button type="button" class="row press" on:click=move |_| {
                            c.session.update(|s| if let Some(s) = s { s.current = i; });
                            c.persist_session();
                            c.mode.set(Mode::Logger);
                        }>
                            <span class="stack grow g4">
                                <span class="ellipsis" class:fg2={done >= target}>{l.name.clone()}</span>
                                <span class="setdots" aria-label=format!("{done} of {target} sets")>
                                    {(0..target.max(done)).map(|k| view! { <i class:done={k < done} class:now={k == done && current}></i> }).collect_view()}
                                </span>
                            </span>
                            <span class="r meta num">{ui::rx(&l.prescription, c.unit()).1}</span>
                        </button>
                    }
                }).collect_view()}
            </div>
            <button type="button" class="textlink" on:click=move |_| c.sheet.set(Some(Sheet::AddToSession))>"Add a lift to this session"</button>
            <div class="stack g8 push">
                <button type="button" class=move || if all_done() { "btn btn-accent press" } else { "btn btn-quiet press" } on:click=move |_| c.end_session()>"End session"</button>
                <button type="button" class="textlink" on:click=move |_| c.mode.set(Mode::Today)>"Leave it running and look around"</button>
                <p class="meta center">"Your Apple Watch ends it for you when the workout ends"</p>
            </div>
        </div>
    }
}

// ---------- logging ----------

/// The stepper values for a lift: the draft, else the last set this session, else the plan.
pub fn draft(lift: Uuid) -> (f64, u32) {
    let c = ctx();
    if let Some(d) = c.drafts.with(|d| d.0.get(&lift).copied()) {
        return d;
    }
    let unit = c.unit();
    if let Some(last) = c.session.with(|s| s.as_ref().and_then(|s| s.sets.iter().rev().find(|x| x.lift_id == lift).cloned())) {
        return (last.weight.display_value(unit), last.reps);
    }
    match c.lift(lift) {
        // On a bar, start on a weight the plates can make, even if the plan was written in
        // the other unit. Dumbbells, stacks and machines keep the planned number.
        Some(l) if l.tags.as_ref().is_none_or(|t| t.equipment.uses_plates()) => (l.prescription.weight.loadable(unit, step_for(&l, unit)), l.prescription.reps),
        Some(l) => (l.prescription.weight.display_value(unit), l.prescription.reps),
        None => (unit.bar(), 5),
    }
}

fn step_for(l: &Lift, unit: Unit) -> f64 {
    let eq = l.tags.as_ref().map(|t| t.equipment).unwrap_or(Equipment::Barbell);
    match (eq, unit) {
        (Equipment::Dumbbell | Equipment::Kettlebell, Unit::Kg) => 2.0,
        (_, Unit::Kg) => 2.5,
        (_, Unit::Lb) => 5.0,
    }
}

fn last_time(lift: Uuid, before: Uuid) -> Option<String> {
    let c = ctx();
    let unit = c.unit();
    c.data.with(|d| {
        let d = d.as_ref()?;
        let s = d.recent_sessions.iter().find(|s| s.id != before && s.sets.iter().any(|x| x.lift_id == lift))?;
        let sets: Vec<_> = s.sets.iter().filter(|x| x.lift_id == lift).collect();
        let same = sets.iter().all(|x| x.weight == sets[0].weight);
        Some(if same {
            format!("{} × {}", sets[0].weight.display_with_unit(unit), sets.iter().map(|x| x.reps.to_string()).collect::<Vec<_>>().join(", "))
        } else {
            sets.iter().map(|x| format!("{} × {}", x.weight.display(unit), x.reps)).collect::<Vec<_>>().join(", ")
        })
    })
}

#[component]
fn LoggerPane(current_lift: Memo<Option<Lift>>) -> impl IntoView {
    let c = ctx();
    let id = move || current_lift.get().map(|l| l.id);
    let values = move || id().map(draft).unwrap_or((0.0, 0));
    let went_down = RwSignal::new(false);
    let set_draft = move |w: f64, r: u32| {
        if let Some(id) = id() {
            c.drafts.update(|d| {
                d.0.insert(id, (w, r));
            });
        }
    };
    let bump_w = move |dir: f64| {
        let Some(l) = current_lift.get_untracked() else { return };
        let unit = c.unit();
        let (w, r) = values();
        let min = if l.tags.as_ref().is_some_and(|t| t.equipment.uses_plates()) { unit.bar() } else { 0.0 };
        let nw = ((w + dir * step_for(&l, unit)) * 100.0).round() / 100.0;
        if nw >= min && nw <= 1000.0 {
            went_down.set(dir < 0.0);
            set_draft(nw, r);
        }
    };
    let bump_r = move |dir: i32| {
        let (w, r) = values();
        let nr = (r as i32 + dir).clamp(0, 100) as u32;
        went_down.set(dir < 0);
        set_draft(w, nr);
    };
    let done_count = move || id().map(|id| c.session.with(|s| s.as_ref().map(|s| s.sets_for(id).len()).unwrap_or(0))).unwrap_or(0);
    let target = move || current_lift.get().map(|l| l.prescription.sets as usize).unwrap_or(3);
    let set_no = move || done_count() + 1;
    let logged = move || {
        let unit = c.unit();
        let id = id()?;
        let sets: Vec<_> = c.session.with(|s| s.as_ref().map(|s| s.sets_for(id).into_iter().cloned().collect()).unwrap_or_default());
        if sets.is_empty() {
            return Some("Nothing yet".to_string());
        }
        let same = sets.iter().all(|x: &gym_core::api::SetEntry| x.weight == sets[0].weight);
        Some(if same {
            format!("{} × {}", sets[0].weight.display_with_unit(unit), sets.iter().map(|x| x.reps.to_string()).collect::<Vec<_>>().join(", "))
        } else {
            sets.iter().map(|x| format!("{} × {}", x.weight.display(unit), x.reps)).collect::<Vec<_>>().join(", ")
        })
    };
    view! {
        <div class="pc" class:on=move || c.mode.get() == Mode::Logger inert=move || c.mode.get() != Mode::Logger>
            <div class="stack g4">
                <button type="button" class="title" style="text-align: left" on:click=move |_| if let Some(id) = id() { c.sheet.set(Some(Sheet::Lift(id))) }>
                    {move || current_lift.get().map(|l| l.name).unwrap_or_else(|| "Pick a lift".into())}
                </button>
                <span class="meta num">{move || current_lift.get().map(|l| {
                    let extra = if set_no() > target() { " · extra set" } else { "" };
                    format!("Set {} of {} · target {} × {}{extra}", set_no(), target(), if l.prescription.weight.0 == 0 { "body".into() } else { l.prescription.weight.display_with_unit(c.unit()) }, l.prescription.reps)
                })}</span>
            </div>
            <div>
                <div class="step">
                    <button type="button" class="iconbtn glass press" aria-label=move || format!("Take off {} {}", current_lift.get().map(|l| format_number(step_for(&l, c.unit()))).unwrap_or_default(), c.unit().label()) on:click=move |_| bump_w(-1.0)>{icon(icons::MINUS)}</button>
                    <span class="rowx g8" style="align-items: baseline">
                        <Roll class="fig" value=Signal::derive(move || format_number(values().0)) down=Signal::derive(move || went_down.get())/>
                        <span class="sub">{move || c.unit().label()}</span>
                    </span>
                    <button type="button" class="iconbtn glass press" aria-label=move || format!("Add {} {}", current_lift.get().map(|l| format_number(step_for(&l, c.unit()))).unwrap_or_default(), c.unit().label()) on:click=move |_| bump_w(1.0)>{icon(icons::PLUS)}</button>
                </div>
                <div class="step">
                    <button type="button" class="iconbtn glass press" aria-label="One rep fewer" on:click=move |_| bump_r(-1)>{icon(icons::MINUS)}</button>
                    <span class="rowx g8" style="align-items: baseline">
                        <Roll class="fig" value=Signal::derive(move || values().1.to_string()) down=Signal::derive(move || went_down.get())/>
                        <span class="sub">"reps"</span>
                    </span>
                    <button type="button" class="iconbtn glass press" aria-label="One rep more" on:click=move |_| bump_r(1)>{icon(icons::PLUS)}</button>
                </div>
            </div>
            <div class="stack g4">
                <div class="rowx between num" style="font-size: 14px; gap: 12px"><span class="fg2">"Logged so far"</span><span class="ellipsis">{logged}</span></div>
                {move || {
                    let s = c.session.get()?;
                    let lt = last_time(id()?, s.id)?;
                    Some(view! { <div class="rowx between" style="gap: 12px"><span class="meta">"Last time"</span><span class="meta num ellipsis">{lt}</span></div> })
                }}
            </div>
            <button type="button" class="btn btn-accent press push" style="height: 56px; border-radius: 28px" disabled=move || id().is_none() || values().1 == 0
                on:click=move |_| { if let Some(id) = id() { let (w, r) = values(); c.log_set(id, w, r); } }>
                {move || format!("Log set {}", set_no())}
            </button>
        </div>
    }
}

#[component]
fn RestPane(current_lift: Memo<Option<Lift>>) -> impl IntoView {
    let c = ctx();
    let left = move || {
        let until = c.session.get().and_then(|s| s.rest_until)?;
        Some((until - c.now.get()).num_seconds().max(0))
    };
    let total = move || c.session.get().map(|s| s.rest_total.max(1)).unwrap_or(1);
    let shift = move |secs: i64| {
        c.session.update(|s| {
            if let Some(s) = s
                && let Some(u) = s.rest_until
            {
                let nu = (u + chrono::Duration::seconds(secs)).max(Utc::now() + chrono::Duration::seconds(5));
                s.rest_until = Some(nu);
                s.rest_total = s.rest_total.max((nu - Utc::now()).num_seconds().max(0) as u32);
            }
        });
        c.persist_session();
    };
    let next_line = move || {
        let l = current_lift.get()?;
        let done = c.session.with(|s| s.as_ref().map(|s| s.sets_for(l.id).len()).unwrap_or(0));
        let (w, r) = draft(l.id);
        Some(format!("{} · set {} of {} · {} {} × {r}", l.name, done + 1, l.prescription.sets, format_number(w), c.unit().label()))
    };
    let last_logged = move || c.session.get().map(|s| s.sets.len()).unwrap_or(0);
    view! {
        <div class="pc" class:on=move || c.mode.get() == Mode::Rest inert=move || c.mode.get() != Mode::Rest>
            <span class="meta">{move || format!("Rest · set {} logged", last_logged())}</span>
            <div class="step" style="height: 96px">
                <button type="button" class="iconbtn glass press num" aria-label="15 seconds less" style="font: 400 13px/1 var(--mono)" on:click=move |_| shift(-15)>"−15"</button>
                <span class="fig num timer" aria-live="off">
                    <Roll value=Signal::derive(move || (left().unwrap_or(0) / 60).to_string())/>
                    <span>":"</span>
                    <Roll value=Signal::derive(move || ((left().unwrap_or(0) % 60) / 10).to_string())/>
                    <Roll value=Signal::derive(move || (left().unwrap_or(0) % 10).to_string())/>
                </span>
                <button type="button" class="iconbtn glass press num" aria-label="15 seconds more" style="font: 400 13px/1 var(--mono)" on:click=move |_| shift(15)>"+15"</button>
            </div>
            <div class="track"><i style=move || format!("transform: scaleX({:.4}); transition: transform 1s linear", left().unwrap_or(0) as f64 / total() as f64)></i></div>
            <div class="rowx between" style="font-size: 14px; gap: 12px"><span class="fg2">"Next"</span><span class="num ellipsis">{next_line}</span></div>
            <div class="stack push">
                <button type="button" class="btn btn-accent press" on:click=move |_| {
                    c.session.update(|s| if let Some(s) = s { s.rest_until = None; });
                    c.persist_session();
                    c.mode.set(Mode::Logger);
                }>{move || {
                    let l = current_lift.get();
                    let done = l.as_ref().map(|l| c.session.with(|s| s.as_ref().map(|s| s.sets_for(l.id).len()).unwrap_or(0))).unwrap_or(0);
                    if done == 0 { format!("Start {}", l.map(|l| l.name.to_lowercase()).unwrap_or_default()) } else { format!("Start set {}", done + 1) }
                }}</button>
                <button type="button" class="textlink" on:click=move |_| c.end_session()>"End session"</button>
            </div>
        </div>
    }
}

// ---------- summary ----------

fn best_yet(s: &Session) -> Option<(String, String)> {
    let c = ctx();
    let unit = c.unit();
    let history: Vec<Session> = c.data.with(|d| d.as_ref().map(|d| d.recent_sessions.clone()).unwrap_or_default());
    let mut best: Option<(Weight, String)> = None;
    let mut lift_ids: Vec<Uuid> = s.sets.iter().map(|x| x.lift_id).collect();
    lift_ids.dedup();
    for id in lift_ids {
        let mine = best_e1rm(s.sets.iter().filter(|x| x.lift_id == id).map(|x| (x.weight, x.reps)));
        let before = best_e1rm(history.iter().filter(|h| h.id != s.id && h.started_at < s.started_at).flat_map(|h| h.sets.iter()).filter(|x| x.lift_id == id).map(|x| (x.weight, x.reps)));
        if let Some(m) = mine
            && before.is_none_or(|b| m > b)
            && best.as_ref().is_none_or(|(bw, _)| m > *bw)
        {
            let top = s.sets.iter().filter(|x| x.lift_id == id).max_by_key(|x| e1rm(x.weight, x.reps))?;
            best = Some((m, format!("{} · {} × {}", c.lift_name(id), top.weight.display_with_unit(unit), top.reps)));
        }
    }
    best.map(|(w, line)| (line, format!("Estimated one-rep max {}", w.display_with_unit(unit))))
}

#[component]
fn SummaryPane() -> impl IntoView {
    let c = ctx();
    let sid = move || match c.mode.get() {
        Mode::Summary(id) => Some(id),
        _ => None,
    };
    let session = move || sid().and_then(|id| c.session_view(id));
    let shown_total = RwSignal::new(0i64);
    // Count the total up when the summary opens.
    Effect::new(move |_| {
        let Some(s) = session() else { return };
        let target = Weight(s.volume()).display_value(c.unit()).round() as i64;
        if crate::bridge::reduced_motion() {
            shown_total.set(target);
            return;
        }
        let start = js_sys::Date::now();
        shown_total.set(0);
        let tick = std::rc::Rc::new(std::cell::RefCell::new(None::<IntervalHandle>));
        let t2 = tick.clone();
        let h = set_interval_with_handle(
            move || {
                let p = ((js_sys::Date::now() - start) / 900.0).min(1.0);
                let e = 1.0 - (1.0 - p).powi(3);
                shown_total.set((target as f64 * e).round() as i64);
                if p >= 1.0
                    && let Some(h) = t2.borrow_mut().take()
                {
                    h.clear();
                }
            },
            std::time::Duration::from_millis(16),
        )
        .ok();
        *tick.borrow_mut() = h;
    });
    view! {
        <div class="pc" class:on=move || sid().is_some() inert=move || sid().is_none()>
            {move || session().map(|s| {
                let unit = c.unit();
                let minutes = (s.closed_at.unwrap_or_else(Utc::now) - s.started_at).num_minutes().max(1);
                let best = best_yet(&s);
                let workout = s.workout.clone();
                let mut lift_ids: Vec<Uuid> = Vec::new();
                for x in &s.sets { if !lift_ids.contains(&x.lift_id) { lift_ids.push(x.lift_id); } }
                let reason = s.close_reason;
                view! {
                    <div class="stack g4">
                        <span class="meta">{format!("{} · {minutes} min", ui::day_label(ui::local(s.started_at).date_naive()))}</span>
                        <h1 class="title">{format!("{}, done", s.day_name)}</h1>
                    </div>
                    <div class="rowx g8" style="align-items: baseline"><span class="fig num">{move || format_thousands(shown_total.get())}</span><span class="sub">{format!("{} moved", unit.label())}</span></div>
                    <div>
                        <div class="row"><span class="fg2">"Sets"</span><span class="r num">{s.sets.len()}</span></div>
                        {best.map(|(line, sub)| view! {
                            <div class="row tall"><span class="fg2">"Best yet"</span><span class="r stack" style="text-align: right"><span class="num">{line}</span><span class="meta">{sub}</span></span></div>
                        })}
                        <div class="watch">
                            {match workout {
                                Some(w) => view! {
                                    <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path class="tick" pathLength="1" d=icons::CHECK></path></svg>
                                    <span class="stack"><span>"Apple Watch workout attached"</span><span class="sub num">{
                                        [w.duration_min.map(|m| format!("{} min", m.round())), w.energy_kcal.map(|k| format!("{} kcal", k.round())), w.avg_heart_rate.map(|h| format!("average {} bpm", h.round()))].into_iter().flatten().collect::<Vec<_>>().join(" · ")
                                    }</span></span>
                                }.into_any(),
                                None => view! {
                                    <span class="dot wait"></span>
                                    <span class="stack"><span>"Waiting for your Apple Watch workout"</span><span class="sub">"It attaches by itself when your phone sends it, usually by the evening"</span></span>
                                }.into_any(),
                            }}
                        </div>
                    </div>
                    <p class="sub">{match reason {
                        Some(r) => format!("{}. Nothing to save: every set was stored the moment you logged it", r.label()),
                        None => "Nothing to save: every set was stored the moment you logged it".to_string(),
                    }}</p>
                    <div class="list">
                        {lift_ids.into_iter().map(|id| {
                            let sets: Vec<_> = s.sets.iter().filter(|x| x.lift_id == id).cloned().collect();
                            view! {
                                <div class="row tall">
                                    <span class="stack grow g4">
                                        <span>{c.lift_name(id)}</span>
                                        <span class="rowx wrap g8">{sets.into_iter().map(|x| {
                                            let label = format!("{} × {}", x.weight.display(unit), x.reps);
                                            view! { <button type="button" class="chip del press" aria-label=format!("Delete set {label}") on:click=move |_| c.delete_set(&x)>{label.clone()}{crate::icons::icon(crate::icons::CLOSE)}</button> }
                                        }).collect_view()}</span>
                                    </span>
                                </div>
                            }
                        }).collect_view()}
                    </div>
                }
            })}
            <button type="button" class="btn btn-accent press push" on:click=move |_| c.finish_summary()>"Done"</button>
        </div>
    }
}

/// "Push" -> "Push day"; "Upper A" and "Day B" stay as they are.
pub fn day_title(name: &str) -> String {
    if name.split_whitespace().count() == 1 && !name.to_lowercase().ends_with("day") { format!("{name} day") } else { name.to_string() }
}
