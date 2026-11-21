//! The plan: one pool of lifts, dealt into whichever split you pick.

use gym_core::api::Plan;
use gym_core::split::{PoolLift, Split, builtin, deal, style_of, with_days};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::bridge;
use crate::net;
use crate::state::{Sheet, ToastAction, ctx};
use crate::ui;

/// Run a change to the plan with every lift chip gliding from its old day to its new one.
pub fn animate_plan(change: impl FnOnce() + 'static) {
    bridge::flip_first("[data-flip]");
    change();
    request_animation_frame(move || bridge::flip_play("[data-flip]", 560.0, 16.0));
}

/// Deal the current lifts into `split` on the phone, so the switch shows at once.
fn local_plan(plan: &Plan, split: &Split) -> Plan {
    let c = ctx();
    let lifts = c.data.with_untracked(|d| d.as_ref().map(|d| d.lifts.clone()).unwrap_or_default());
    let pool: Vec<PoolLift> = lifts.iter().filter(|l| !l.archived).map(|l| PoolLift { id: l.id, tags: l.tags.as_ref() }).collect();
    let dealt = deal(split, &pool);
    Plan {
        split: split.clone(),
        days: split.days.iter().zip(dealt.days).map(|(d, lifts)| gym_core::api::PlanDay { name: d.name.clone(), weekday: d.weekday, lifts }).collect(),
        unplaced: dealt.unplaced,
        can_undo: true,
        ..plan.clone()
    }
}

/// When the plan begins. Before that day, Today says when it starts and what comes first.
#[component]
fn StartDate() -> impl IntoView {
    let c = ctx();
    let starts = move || c.plan().and_then(|p| p.starts_on);
    let today = ui::today();
    let set = move |value: Option<chrono::NaiveDate>| {
        spawn_local(async move {
            match net::post::<_, Option<Plan>>("/api/plan/start", &serde_json::json!({ "starts_on": value })).await {
                Ok(p) => {
                    c.data.update(|d| if let Some(d) = d { d.plan = p; });
                    c.refresh().await;
                    c.toast_with(
                        match value { Some(d) if d > ui::today() => format!("The plan starts {}", ui::day_label(d)), _ => "The plan starts today".into() },
                        Some(("Undo".into(), ToastAction::UndoPlan)),
                    );
                }
                Err(e) => c.fail(&e),
            }
        });
    };
    view! {
        <div class="list">
            <label class="row" for="plan-start">
                <span class="stack grow">
                    <span>"Starts"</span>
                    <span class="sub">{move || match starts() {
                        Some(d) if d > today => format!("Nothing to train until {}", ui::day_label(d)),
                        Some(d) => format!("Started {}", ui::day_label(d)),
                        None => "Started. Pick a date to begin later, for example next Monday".into(),
                    }}</span>
                </span>
                <input id="plan-start" type="date" class="dateinput num"
                    prop:value=move || starts().map(|d| d.format("%Y-%m-%d").to_string()).unwrap_or_default()
                    on:change=move |e| {
                        let v = event_target_value(&e);
                        set(chrono::NaiveDate::parse_from_str(&v, "%Y-%m-%d").ok());
                    }/>
            </label>
            {move || starts().filter(|d| *d > today).map(|_| view! {
                <button type="button" class="row press" on:click=move |_| set(None)>
                    <span class="grow">"Start today instead"</span>
                </button>
            })}
        </div>
    }
}

#[component]
pub fn PlanPage() -> impl IntoView {
    let c = ctx();
    let plan = move || c.plan();
    let splits = builtin();
    let options: Vec<(String, &'static str)> = splits
        .iter()
        .map(|s| {
            let short = match s.key.as_str() {
                "ppl" => "PPL",
                "upper_lower" => "Upper lower",
                "bro" => "Bro split",
                _ => "Full body",
            };
            (s.key.clone(), short)
        })
        .collect();
    // The built-in style ("ppl" for "ppl@013457" too), so the picker shows it whatever days you train.
    let current_key = move || plan().map(|p| style_of(&p.split.key).to_string()).unwrap_or_default();
    let is_custom = move || builtin().iter().all(|s| s.key != current_key());
    let weekdays = move || plan().map(|p| p.days.iter().map(|d| d.weekday).collect::<Vec<u8>>()).unwrap_or_default();
    let n_options = options.len();
    let busy = RwSignal::new(false);

    // Apply a split: shown at once on the phone, then saved. `what` is the toast.
    let apply = move |split: Split, what: String| {
        let Some(p) = plan() else { return };
        if busy.get_untracked() || p.split.key == split.key {
            return;
        }
        let optimistic = local_plan(&p, &split);
        animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = Some(optimistic); }));
        busy.set(true);
        spawn_local(async move {
            match net::post::<_, Option<Plan>>("/api/plan/switch", &serde_json::json!({ "split": split })).await {
                Ok(server) => {
                    c.data.update(|d| if let Some(d) = d { d.plan = server; });
                    c.toast_with(what, Some(("Undo".into(), ToastAction::UndoPlan)));
                }
                Err(e) => {
                    animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = Some(p); }));
                    c.fail(&e);
                }
            }
            busy.set(false);
        });
    };
    // Another style on the days you already train.
    let switch = move |key: String| {
        if current_key() == key {
            return;
        }
        let days = weekdays();
        let Some(split) = with_days(&key, &days).or_else(|| builtin().into_iter().find(|s| s.key == key)) else { return };
        let name = split.name.to_lowercase();
        apply(split, format!("Now on {name}. Logged sets are untouched"));
    };
    // Train on one more or one fewer weekday, same style.
    let toggle_day = move |wd: u8| {
        let mut days = weekdays();
        if let Some(i) = days.iter().position(|d| *d == wd) {
            if days.len() == 1 {
                c.say("Keep at least one training day");
                return;
            }
            days.remove(i);
        } else {
            days.push(wd);
        }
        let style = if is_custom() { "full_body".to_string() } else { current_key() };
        let Some(split) = with_days(&style, &days) else { return };
        let n = split.days.len();
        apply(split, format!("{n} training day{} a week. Logged sets are untouched", if n == 1 { "" } else { "s" }));
    };

    view! {
        <div class="scroll">
            {move || match plan() {
                None => view! {
                    <div class="stack g8">
                        <h1 class="title">"Plan"</h1>
                        <p class="sub">"No plan yet. Start one from Today, or ask Muse for one"</p>
                    </div>
                }.into_any(),
                Some(p) => {
                    let total = c.data.with(|d| d.as_ref().map(|d| d.lifts.len()).unwrap_or(0));
                    view! {
                        <div class="stack g8">
                            <span class="meta">{match (p.block_week, p.block_weeks) { (Some(w), Some(n)) => format!("Week {w} of {n}"), _ => format!("Since {}", ui::day_label(ui::local(p.created_at).date_naive())) }}</span>
                            <h1 class="title">{p.name.clone()}</h1>
                            <p class="sub">{format!("{total} lifts, sorted by muscle and movement. Choose how the week is dealt")}</p>
                        </div>
                    }.into_any()
                }
            }}
            {move || plan().map(|_| {
                let opts = options.clone();
                let count = if is_custom() { n_options + 1 } else { n_options };
                view! {
                    <div class="stack g8">
                        <div class="seg" role="group" aria-label="Split">
                            <i class="segthumb" style=move || {
                                let idx = if is_custom() { n_options } else { options_index(&current_key()) };
                                format!("width: calc((100% - 6px) / {count}); transform: translateX({}%)", idx * 100)
                            }></i>
                            {opts.into_iter().map(|(key, label)| {
                                let k2 = key.clone();
                                let k3 = key.clone();
                                view! {
                                    <button type="button" class:on=move || current_key() == k2 aria-pressed=move || (current_key() == k3).to_string()
                                        on:click=move |_| switch(key.clone())>{label}</button>
                                }
                            }).collect_view()}
                            {move || is_custom().then(|| view! { <button type="button" class="on" aria-pressed="true">"Custom"</button> })}
                        </div>
                        <span class="meta">{move || plan().map(|p| p.split.summary)}</span>
                    </div>
                    <div class="stack g8">
                        <span class="meta">"Training days"</span>
                        <div class="weekdays" role="group" aria-label="Training days">
                            {(0u8..7).map(|wd| view! {
                                <button type="button" class="chip pick press" class:on=move || weekdays().contains(&wd)
                                    aria-pressed=move || weekdays().contains(&wd).to_string()
                                    disabled=move || busy.get()
                                    on:click=move |_| toggle_day(wd)>{ui::weekday_short(wd)}</button>
                            }).collect_view()}
                        </div>
                    </div>
                    <StartDate/>
                }
            })}
            {move || plan().map(|p| {
                let unplaced = p.unplaced.clone();
                view! {
                    <div>
                        {p.days.into_iter().map(|d| view! {
                            <div class="day">
                                <span class="stack"><span class="h2">{d.name.clone()}</span><span class="meta">{ui::weekday_short(d.weekday)}</span></span>
                                <span class="chips">
                                    {d.lifts.into_iter().map(|id| view! {
                                        <button type="button" class="chip press" data-flip=id.to_string() on:click=move |_| c.sheet.set(Some(Sheet::Lift(id)))>{c.lift_name(id)}</button>
                                    }).collect_view()}
                                </span>
                            </div>
                        }).collect_view()}
                        {(!unplaced.is_empty()).then(|| view! {
                            <div class="day">
                                <span class="stack"><span class="h2">"Needs a look"</span><span class="meta">"Not sorted"</span></span>
                                <span class="chips">
                                    {unplaced.into_iter().map(|id| view! {
                                        <button type="button" class="chip press" data-flip=id.to_string() on:click=move |_| c.sheet.set(Some(Sheet::Lift(id)))>{c.lift_name(id)}</button>
                                    }).collect_view()}
                                </span>
                            </div>
                        })}
                    </div>
                }
            })}
            <div class="list">
                <button type="button" class="row press" on:click=move |_| c.sheet.set(Some(Sheet::Library))>
                    <span class="stack grow"><span>"All lifts"</span><span class="sub">"Add a lift and it sorts itself. Fix anything that sorted wrong"</span></span>
                    {crate::icons::icon(crate::icons::CHEVRON)}
                </button>
                {move || plan().filter(|p| p.can_undo).map(|_| view! {
                    <button type="button" class="row press" on:click=move |_| spawn_local(async move {
                        match net::post::<_, Option<Plan>>("/api/plan/undo", &serde_json::json!({})).await {
                            Ok(p) => { animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = p; })); c.say("Back to the previous version"); }
                            Err(e) => c.fail(&e),
                        }
                    })>
                        <span class="stack grow"><span>"Previous version"</span><span class="sub">{move || plan().map(|p| format!("This one is by {}, {}", p.created_by, ui::ago(p.created_at)))}</span></span>
                        <span class="r">"Go back"</span>
                    </button>
                })}
            </div>
        </div>
    }
}

fn options_index(key: &str) -> usize {
    builtin().iter().position(|s| s.key == key).unwrap_or(0)
}
