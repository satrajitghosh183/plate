//! The plan: one pool of lifts, dealt into whichever split you pick.

use gym_core::api::Plan;
use gym_core::split::{PoolLift, Split, builtin, deal};
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
    let current_key = move || plan().map(|p| p.split.key).unwrap_or_default();
    let is_custom = move || current_key().starts_with("custom");
    let n_options = options.len();
    let busy = RwSignal::new(false);

    let switch = move |key: String| {
        if busy.get_untracked() || current_key() == key {
            return;
        }
        let Some(p) = plan() else { return };
        let Some(split) = builtin().into_iter().find(|s| s.key == key) else { return };
        let name = split.name.to_lowercase();
        let optimistic = local_plan(&p, &split);
        animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = Some(optimistic); }));
        busy.set(true);
        spawn_local(async move {
            match net::post::<_, Option<Plan>>("/api/plan/switch", &serde_json::json!({ "key": key })).await {
                Ok(server) => {
                    c.data.update(|d| if let Some(d) = d { d.plan = server; });
                    c.toast_with(format!("Now on {name}. Logged sets are untouched"), Some(("Undo".into(), ToastAction::UndoPlan)));
                }
                Err(e) => {
                    animate_plan(move || c.data.update(|d| if let Some(d) = d { d.plan = Some(p); }));
                    c.fail(&e);
                }
            }
            busy.set(false);
        });
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
