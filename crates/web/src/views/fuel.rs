//! Fuel: food from Lifesum and energy from Apple Health, as day totals.
//!
//! One day in full (calories against the goal, protein, carbs and fat in grams and as a
//! share of calories, burned and the balance), then the chosen range: averages, how often
//! the goals were met, a bar per day, and every logged day as a row. Tap a bar or a row to
//! see that day.

use chrono::{Datelike, Duration, NaiveDate};
use gym_core::api::HealthDay;
use gym_core::units::{format_number, format_thousands};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::net;
use crate::state::{Tab, ctx};
use crate::ui;

const RANGES: [(u32, &str); 4] = [(7, "7 days"), (30, "30 days"), (90, "90 days"), (365, "Year")];

fn kcal(v: Option<f64>) -> String {
    v.filter(|v| *v > 0.0).map(|v| format_thousands(v.round() as i64)).unwrap_or_else(|| "–".into())
}

fn grams(v: Option<f64>) -> String {
    v.filter(|v| *v > 0.0).map(|v| format!("{} g", format_number(v.round()))).unwrap_or_else(|| "–".into())
}

fn logged(h: &HealthDay) -> bool {
    h.energy_eaten_kcal.is_some_and(|v| v > 0.0) || h.protein_g.is_some_and(|v| v > 0.0)
}

/// Calories from each macro: protein and carbs 4 kcal a gram, fat 9.
fn macro_kcal(h: &HealthDay) -> (f64, f64, f64) {
    (h.protein_g.unwrap_or(0.0) * 4.0, h.carbs_g.unwrap_or(0.0) * 4.0, h.fat_g.unwrap_or(0.0) * 9.0)
}

fn avg(xs: impl Iterator<Item = f64>) -> Option<f64> {
    let v: Vec<f64> = xs.filter(|v| *v > 0.0).collect();
    (!v.is_empty()).then(|| v.iter().sum::<f64>() / v.len() as f64)
}

fn date_label(d: NaiveDate, today: NaiveDate) -> String {
    if d == today {
        "Today".into()
    } else if d == today - Duration::days(1) {
        "Yesterday".into()
    } else {
        format!("{} {} {}", ui::weekday_short(d.weekday().num_days_from_monday() as u8), d.day(), month_short(d.month()))
    }
}

fn month_short(m: u32) -> &'static str {
    ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][(m.clamp(1, 12) - 1) as usize]
}

#[component]
pub fn FuelPage() -> impl IntoView {
    let c = ctx();
    let today = ui::today();
    let range = RwSignal::new(7u32);
    let selected = RwSignal::new(today);
    let loading = RwSignal::new(false);
    // Start from the week the app already has, then load the chosen range.
    let rows: RwSignal<Vec<HealthDay>> = RwSignal::new(c.data.with_untracked(|d| d.as_ref().map(|d| d.health_week.clone()).unwrap_or_default()));
    Effect::new(move |_| {
        let n = range.get();
        loading.set(true);
        spawn_local(async move {
            if let Ok(r) = net::get::<Vec<HealthDay>>(&format!("/api/health?days={n}")).await {
                rows.set(r);
            }
            loading.set(false);
        });
    });
    // Today's numbers change during the day: follow the app's own refreshes too.
    Effect::new(move |_| {
        let week = c.data.with(|d| d.as_ref().map(|d| d.health_week.clone()).unwrap_or_default());
        if let Some(t) = week.into_iter().find(|h| h.date == today) {
            rows.update(|r| match r.iter_mut().find(|h| h.date == today) {
                Some(h) => *h = t,
                None => r.push(t),
            });
        }
    });

    let goals = move || c.data.with(|d| d.as_ref().map(|d| (d.settings.kcal_goal as f64, d.settings.protein_goal_g as f64)).unwrap_or((2600.0, 160.0)));
    let day_of = move |d: NaiveDate| rows.with(|r| r.iter().find(|h| h.date == d).cloned()).unwrap_or(HealthDay { date: d, ..Default::default() });
    let has_any = move || rows.with(|r| r.iter().any(|h| logged(h) || h.energy_burned_kcal.is_some_and(|v| v > 0.0)));

    view! {
        <div class="scroll">
            <div class="stack g8">
                <span class="meta">"From Lifesum, through Apple Health"</span>
                <h1 class="title">"Fuel"</h1>
            </div>
            <div class="seg" role="group" aria-label="Range">
                <i class="segthumb" style=move || {
                    let i = RANGES.iter().position(|(n, _)| *n == range.get()).unwrap_or(0);
                    format!("width: calc((100% - 6px) / {}); transform: translateX({}%)", RANGES.len(), i * 100)
                }></i>
                {RANGES.iter().map(|(n, label)| {
                    let n = *n;
                    view! { <button type="button" class:on=move || range.get() == n on:click=move |_| range.set(n)>{*label}</button> }
                }).collect_view()}
            </div>
            {move || if !has_any() && !loading.get() {
                view! {
                    <p class="sub">"Nothing from Apple Health in this range. The phone sends each day's food and energy; older days come in with an Apple Health import"</p>
                    <button type="button" class="btn btn-quiet press" on:click=move |_| c.tab.set(Tab::Connect)>"Set up Apple Health"</button>
                }.into_any()
            } else {
                view! { <DayCard day=Signal::derive(move || day_of(selected.get())) today=today goals=Signal::derive(goals)/> }.into_any()
            }}
            {move || has_any().then(|| view! {
                <Summary rows=rows.into() goals=Signal::derive(goals)/>
                <Chart rows=rows.into() range=range.into() selected=selected today=today goals=Signal::derive(goals)/>
                <DayTable rows=rows.into() selected=selected today=today goals=Signal::derive(goals)/>
            })}
        </div>
    }
}

/// One day: calories against the goal, the three macros, burned and the balance.
#[component]
fn DayCard(day: Signal<HealthDay>, today: NaiveDate, goals: Signal<(f64, f64)>) -> impl IntoView {
    view! {
        {move || {
            let d = day.get();
            let (kcal_goal, protein_goal) = goals.get();
            let eaten = d.energy_eaten_kcal.unwrap_or(0.0);
            let burned = d.energy_burned_kcal.unwrap_or(0.0);
            let (p, cb, f) = macro_kcal(&d);
            let total = (p + cb + f).max(1.0);
            let pct = |x: f64| (x / total * 100.0).round();
            let bar = |v: f64, goal: f64| format!("transform: scaleX({:.3})", (v / goal.max(1.0)).clamp(0.0, 1.0));
            let label = date_label(d.date, today);
            let has_macros = p + cb + f > 0.0;
            view! {
                <div class="stack g12 fuelday">
                    <span class="meta">{label}</span>
                    <div class="rowx g8 wrap" style="align-items: baseline">
                        <span class="fig num">{kcal(d.energy_eaten_kcal).replace('–', "0")}</span>
                        <span class="sub">{format!("of {} kcal eaten", format_thousands(kcal_goal as i64))}</span>
                    </div>
                    <div class="track"><i style=bar(eaten, kcal_goal)></i></div>
                    <div class="list">
                        <MacroRow name="Protein" grams=d.protein_g share=pct(p) goal=Some(protein_goal) color="var(--protein)"/>
                        <MacroRow name="Carbs" grams=d.carbs_g share=pct(cb) goal=None color="var(--carbs)"/>
                        <MacroRow name="Fat" grams=d.fat_g share=pct(f) goal=None color="var(--fat)"/>
                    </div>
                    {has_macros.then(|| view! {
                        <div class="stack g8">
                            <div class="split" role="img" aria-label=format!("Calories from protein {}%, carbs {}%, fat {}%", pct(p), pct(cb), pct(f))>
                                <i style=format!("flex: {p:.1}; background: var(--protein)")></i>
                                <i style=format!("flex: {cb:.1}; background: var(--carbs)")></i>
                                <i style=format!("flex: {f:.1}; background: var(--fat)")></i>
                            </div>
                            <div class="rowx g12 wrap meta">
                                <span><b class="dot" style="background: var(--protein)"></b>{format!("Protein {}%", pct(p))}</span>
                                <span><b class="dot" style="background: var(--carbs)"></b>{format!("Carbs {}%", pct(cb))}</span>
                                <span><b class="dot" style="background: var(--fat)"></b>{format!("Fat {}%", pct(f))}</span>
                            </div>
                        </div>
                    })}
                    <div class="list">
                        <div class="row"><span class="fg2">"Burned (active)"</span><span class="r num">{if burned > 0.0 { format!("{} kcal", kcal(Some(burned))) } else { "Not yet".into() }}</span></div>
                        <div class="row"><span class="fg2">"Left to goal"</span><span class="r num">{
                            let left = kcal_goal - eaten;
                            if left >= 0.0 { format!("{} kcal", format_thousands(left.round() as i64)) } else { format!("{} kcal over", format_thousands((-left).round() as i64)) }
                        }</span></div>
                        <div class="row"><span class="fg2">"Protein per 1,000 kcal"</span><span class="r num">{
                            if eaten > 0.0 { format!("{} g", format_number((d.protein_g.unwrap_or(0.0) / eaten * 1000.0).round())) } else { "–".into() }
                        }</span></div>
                    </div>
                </div>
            }
        }}
    }
}

#[component]
fn MacroRow(name: &'static str, grams: Option<f64>, share: f64, goal: Option<f64>, color: &'static str) -> impl IntoView {
    let g = grams.unwrap_or(0.0);
    let right = match goal {
        Some(goal) => format!("{} of {} g", format_number(g.round()), format_number(goal.round())),
        None => format!("{} g", format_number(g.round())),
    };
    let fill = match goal {
        Some(goal) => (g / goal.max(1.0)).clamp(0.0, 1.0),
        None => (share / 100.0).clamp(0.0, 1.0),
    };
    view! {
        <div class="row tall macro">
            <span class="stack g8 grow">
                <span class="rowx between"><span>{name}</span><span class="num fg2">{right}</span></span>
                <span class="track"><i style=format!("transform: scaleX({fill:.3}); background: {color}")></i></span>
                <span class="meta">{if g > 0.0 { format!("{share}% of calories · {} kcal", format_thousands((g * if name == "Fat" { 9.0 } else { 4.0 }).round() as i64)) } else { "Nothing logged".into() }}</span>
            </span>
        </div>
    }
}

/// Averages over the logged days in the range, and how often the goals were met.
#[component]
fn Summary(rows: Signal<Vec<HealthDay>>, goals: Signal<(f64, f64)>) -> impl IntoView {
    view! {
        {move || {
            let r = rows.get();
            let days: Vec<&HealthDay> = r.iter().filter(|h| logged(h)).collect();
            let n = days.len();
            let (kcal_goal, protein_goal) = goals.get();
            let a_eat = avg(days.iter().filter_map(|h| h.energy_eaten_kcal));
            let a_p = avg(days.iter().filter_map(|h| h.protein_g));
            let a_c = avg(days.iter().filter_map(|h| h.carbs_g));
            let a_f = avg(days.iter().filter_map(|h| h.fat_g));
            let a_burn = avg(r.iter().filter_map(|h| h.energy_burned_kcal));
            let protein_hit = days.iter().filter(|h| h.protein_g.unwrap_or(0.0) >= protein_goal).count();
            let on_kcal = days.iter().filter(|h| (h.energy_eaten_kcal.unwrap_or(0.0) - kcal_goal).abs() <= kcal_goal * 0.1).count();
            let total_eaten: f64 = days.iter().filter_map(|h| h.energy_eaten_kcal).sum();
            view! {
                <div class="stack g8">
                    <span class="meta">{if n == 0 { "No food logged in this range".to_string() } else { format!("Daily average over {n} logged day{}", if n == 1 { "" } else { "s" }) }}</span>
                    <div class="tiles">
                        <div class="tile"><span class="meta">"Eaten"</span><span class="tv num">{kcal(a_eat)}</span><span class="meta">"kcal"</span></div>
                        <div class="tile"><span class="meta">"Protein"</span><span class="tv num">{a_p.map(|v| format_number(v.round())).unwrap_or_else(|| "–".into())}</span><span class="meta">"g"</span></div>
                        <div class="tile"><span class="meta">"Carbs"</span><span class="tv num">{a_c.map(|v| format_number(v.round())).unwrap_or_else(|| "–".into())}</span><span class="meta">"g"</span></div>
                        <div class="tile"><span class="meta">"Fat"</span><span class="tv num">{a_f.map(|v| format_number(v.round())).unwrap_or_else(|| "–".into())}</span><span class="meta">"g"</span></div>
                        <div class="tile"><span class="meta">"Burned"</span><span class="tv num">{kcal(a_burn)}</span><span class="meta">"kcal active"</span></div>
                    </div>
                    <div class="list">
                        <div class="row"><span class="fg2">"Protein goal met"</span><span class="r num">{format!("{protein_hit} of {n} days")}</span></div>
                        <div class="row"><span class="fg2">"Calories within 10% of goal"</span><span class="r num">{format!("{on_kcal} of {n} days")}</span></div>
                        <div class="row"><span class="fg2">"Total eaten"</span><span class="r num">{format!("{} kcal", format_thousands(total_eaten.round() as i64))}</span></div>
                    </div>
                </div>
            }
        }}
    }
}

/// A bar per day, oldest on the left, with the calorie goal as a line. Tap to pick a day.
#[component]
fn Chart(rows: Signal<Vec<HealthDay>>, range: Signal<u32>, selected: RwSignal<NaiveDate>, today: NaiveDate, goals: Signal<(f64, f64)>) -> impl IntoView {
    view! {
        {move || {
            let n = range.get() as i64;
            let r = rows.get();
            let days: Vec<NaiveDate> = (0..n).rev().map(|k| today - Duration::days(k)).collect();
            let (goal, _) = goals.get();
            let max = r.iter().filter_map(|h| h.energy_eaten_kcal).fold(goal, f64::max).max(1.0) * 1.08;
            let gap = if n <= 7 { 10 } else if n <= 30 { 3 } else { 1 };
            let first = days.first().copied().unwrap_or(today);
            view! {
                <div class="stack g8">
                    <span class="meta">"Eaten each day · line is your goal"</span>
                    <div class="chart">
                        <div class="goal" style=format!("bottom: {:.1}%", goal / max * 100.0)></div>
                        <div class="bars dyn" style=format!("grid-template-columns: repeat({n}, minmax(0, 1fr)); gap: {gap}px") role="img" aria-label="Calories eaten each day">
                            {days.iter().map(|d| {
                                let d = *d;
                                let v = r.iter().find(|h| h.date == d).and_then(|h| h.energy_eaten_kcal).unwrap_or(0.0);
                                let over = v > goal * 1.1;
                                view! {
                                    <i class:now=move || selected.get() == d class:over=over title=format!("{}: {} kcal", d, format_thousands(v.round() as i64))
                                        style=format!("height: {:.1}%", (v / max * 100.0).max(1.5))
                                        on:click=move |_| selected.set(d)></i>
                                }
                            }).collect_view()}
                        </div>
                    </div>
                    <div class="rowx between meta">
                        <span>{date_label(first, today)}</span>
                        <span>"Today"</span>
                    </div>
                </div>
            }
        }}
    }
}

/// Every logged day, newest first.
#[component]
fn DayTable(rows: Signal<Vec<HealthDay>>, selected: RwSignal<NaiveDate>, today: NaiveDate, goals: Signal<(f64, f64)>) -> impl IntoView {
    view! {
        {move || {
            let mut r: Vec<HealthDay> = rows.get().into_iter().filter(|h| logged(h) || h.energy_burned_kcal.is_some_and(|v| v > 0.0)).collect();
            r.sort_by_key(|h| std::cmp::Reverse(h.date));
            let (_, protein_goal) = goals.get();
            view! {
                <div class="stack g8">
                    <span class="meta">"Day by day"</span>
                    <div class="daytable">
                        <div class="dt head meta"><span>"Day"</span><span>"kcal"</span><span>"Protein"</span><span>"Carbs"</span><span>"Fat"</span><span>"Burned"</span></div>
                        {r.into_iter().map(|h| {
                            let d = h.date;
                            let hit = h.protein_g.unwrap_or(0.0) >= protein_goal;
                            view! {
                                <button type="button" class="dt press num" class:on=move || selected.get() == d on:click=move |_| {
                                    selected.set(d);
                                    // Bring that day's breakdown into view.
                                    if let Some(el) = web_sys::window().and_then(|w| w.document()).and_then(|d| d.query_selector(".fuelday").ok().flatten()) {
                                        el.scroll_into_view();
                                    }
                                }>
                                    <span class="ellipsis">{date_label(d, today)}</span>
                                    <span>{kcal(h.energy_eaten_kcal)}</span>
                                    <span class:hit=hit>{grams(h.protein_g)}</span>
                                    <span>{grams(h.carbs_g)}</span>
                                    <span>{grams(h.fat_g)}</span>
                                    <span class="fg2">{kcal(h.energy_burned_kcal)}</span>
                                </button>
                            }
                        }).collect_view()}
                    </div>
                </div>
            }
        }}
    }
}
