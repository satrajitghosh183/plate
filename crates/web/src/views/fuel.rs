//! Fuel: food from Lifesum and energy from Apple Health, as day totals. Secondary to training.
//! Strava runs show here too, for you only.

use chrono::{Datelike, Duration};
use gym_core::api::{HealthDay, StravaActivity};
use gym_core::units::{format_number, format_thousands};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::net;
use crate::state::{Tab, ctx};
use crate::ui;

#[component]
pub fn FuelPage() -> impl IntoView {
    let c = ctx();
    let week = move || c.data.with(|d| d.as_ref().map(|d| d.health_week.clone()).unwrap_or_default());
    let today = ui::today();
    let today_row = move || week().into_iter().find(|h| h.date == today).unwrap_or(HealthDay { date: today, ..Default::default() });
    let goals = move || c.data.with(|d| d.as_ref().map(|d| (d.settings.kcal_goal, d.settings.protein_goal_g)).unwrap_or((2600, 160)));
    let runs: RwSignal<Vec<StravaActivity>> = RwSignal::new(vec![]);
    let strava = move || c.data.with(|d| d.as_ref().is_some_and(|d| d.connections.strava.connected));
    Effect::new(move |_| {
        if strava() {
            spawn_local(async move {
                if let Ok(r) = net::get::<Vec<StravaActivity>>("/api/strava/activities").await {
                    runs.set(r);
                }
            });
        }
    });
    let days: Vec<chrono::NaiveDate> = (0..7).rev().map(|k| today - Duration::days(k)).collect();
    let has_any = move || week().iter().any(|h| h.energy_eaten_kcal.is_some() || h.energy_burned_kcal.is_some());

    view! {
        <div class="scroll">
            <div class="stack g8">
                <span class="meta">"From Lifesum, through Apple Health"</span>
                <h1 class="title">"Fuel"</h1>
            </div>
            {move || if !has_any() {
                view! {
                    <p class="sub">"Nothing from Apple Health yet. Set up the phone on the Connect tab and the day's food and energy arrive each evening"</p>
                    <button type="button" class="btn btn-quiet press" on:click=move |_| c.tab.set(Tab::Connect)>"Set up Apple Health"</button>
                }.into_any()
            } else {
                let t = today_row();
                let (kcal_goal, protein_goal) = goals();
                view! {
                    <div class="rowx g8 wrap" style="align-items: baseline">
                        <span class="fig num">{t.energy_eaten_kcal.map(|k| format_thousands(k.round() as i64)).unwrap_or_else(|| "0".into())}</span>
                        <span class="sub">{format!("of {} kcal eaten today", format_thousands(kcal_goal as i64))}</span>
                    </div>
                    <div class="list">
                        <div class="row"><span class="fg2">"Burned so far"</span><span class="r num">{t.energy_burned_kcal.map(|k| format!("{} kcal", format_thousands(k.round() as i64))).unwrap_or_else(|| "Not yet".into())}</span></div>
                        <div class="row"><span class="fg2">"Protein"</span><span class="r num">{format!("{} of {protein_goal} g", t.protein_g.map(|p| format_number(p.round())).unwrap_or_else(|| "0".into()))}</span></div>
                        <div class="row"><span class="fg2">"Carbs · fat"</span><span class="r num">{format!("{} g · {} g", t.carbs_g.map(|p| format_number(p.round())).unwrap_or_else(|| "0".into()), t.fat_g.map(|p| format_number(p.round())).unwrap_or_else(|| "0".into()))}</span></div>
                    </div>
                }.into_any()
            }}
            {move || has_any().then(|| {
                let w = week();
                let max = w.iter().filter_map(|h| h.energy_eaten_kcal).fold(goals().0 as f64, f64::max).max(1.0);
                let days = days.clone();
                view! {
                    <div class="stack g8">
                        <span class="meta">"Eaten, last 7 days"</span>
                        <div class="bars" role="img" aria-label="Calories eaten on each of the last seven days">
                            {days.iter().enumerate().map(|(i, d)| {
                                let v = w.iter().find(|h| h.date == *d).and_then(|h| h.energy_eaten_kcal).unwrap_or(0.0);
                                view! { <i class:now={*d == today} style=format!("height: {:.0}px; animation-delay: {}ms", (v / max * 120.0).max(2.0), 120 + i * 45)></i> }
                            }).collect_view()}
                        </div>
                        <div class="days7 meta">
                            {days.iter().map(|d| view! { <span style=if *d == today { "color: var(--fg)" } else { "" }>{ui::weekday_short(d.weekday().num_days_from_monday() as u8)}</span> }).collect_view()}
                        </div>
                    </div>
                }
            })}
            {move || (strava() && !runs.get().is_empty()).then(|| view! {
                <div class="stack g8">
                    <span class="meta">"Runs and rides this week · Strava, shown to you only"</span>
                    <div class="list">
                        {runs.get().into_iter().map(|r| view! {
                            <div class="row">
                                <span class="stack grow"><span class="ellipsis">{r.name.clone()}</span><span class="meta">{format!("{} · {}", r.sport, ui::day_label(ui::local(r.start).date_naive()))}</span></span>
                                <span class="r num">{format!("{} km · {} min", format_number((r.distance_km * 10.0).round() / 10.0), r.moving_min.round())}</span>
                            </div>
                        }).collect_view()}
                    </div>
                </div>
            })}
        </div>
    }
}
