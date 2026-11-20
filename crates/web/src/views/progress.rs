//! Progress: estimated one-rep max for a lift, and past sessions.

use gym_core::api::LiftProgress;
use gym_core::units::{format_number, format_thousands};
use gym_core::Weight;
use leptos::prelude::*;
use leptos::task::spawn_local;
use uuid::Uuid;

use crate::net;
use crate::state::{Sheet, ctx};
use crate::ui;

#[component]
pub fn ProgressPage() -> impl IntoView {
    let c = ctx();
    // Lifts that have been logged, most-trained first.
    let trained = Memo::new(move |_| {
        c.data.with(|d| {
            let Some(d) = d else { return vec![] };
            let mut counts: Vec<(Uuid, usize)> = Vec::new();
            for s in &d.recent_sessions {
                for x in &s.sets {
                    match counts.iter_mut().find(|(id, _)| *id == x.lift_id) {
                        Some(e) => e.1 += 1,
                        None => counts.push((x.lift_id, 1)),
                    }
                }
            }
            counts.sort_by_key(|c| std::cmp::Reverse(c.1));
            counts.into_iter().map(|(id, _)| id).filter(|id| d.lifts.iter().any(|l| l.id == *id)).collect::<Vec<_>>()
        })
    });
    Effect::new(move |_| {
        if c.progress_lift.get().is_none()
            && let Some(first) = trained.get().first()
        {
            c.progress_lift.set(Some(*first));
        }
    });
    let progress: RwSignal<Option<LiftProgress>> = RwSignal::new(None);
    Effect::new(move |_| {
        let Some(id) = c.progress_lift.get() else { return };
        spawn_local(async move {
            match net::get::<LiftProgress>(&format!("/api/lifts/{id}/progress")).await {
                Ok(p) => progress.set(Some(p)),
                Err(e) => {
                    progress.set(None);
                    if !matches!(e, net::NetError::Unreachable) {
                        c.fail(&e);
                    }
                }
            }
        });
    });
    let sessions = move || c.data.with(|d| d.as_ref().map(|d| d.recent_sessions.clone()).unwrap_or_default());

    view! {
        <div class="scroll">
            {move || if trained.get().is_empty() {
                view! {
                    <div class="stack g8">
                        <span class="meta">"Estimated one-rep max"</span>
                        <h1 class="title">"Progress"</h1>
                        <p class="sub">"Log a session and your lifts show up here, with the best set of each day"</p>
                    </div>
                }.into_any()
            } else {
                view! {
                    <div class="stack g8">
                        <span class="meta">"Estimated one-rep max"</span>
                        <h1 class="title">{move || c.progress_lift.get().map(|id| c.lift_name(id)).unwrap_or_default()}</h1>
                    </div>
                    <div class="chips" style="flex-wrap: nowrap; overflow-x: auto; margin: 0 -20px; padding: 0 20px">
                        <For each=move || trained.get() key=|id| *id children=move |id| view! {
                            <button type="button" class="chip pick press" class:on=move || c.progress_lift.get() == Some(id) on:click=move |_| c.progress_lift.set(Some(id))>{c.lift_name(id)}</button>
                        }/>
                    </div>
                    {move || progress.get().map(|p| view! { <Chart p=p/> })}
                }.into_any()
            }}
            <div class="stack g8">
                <span class="meta">"Sessions"</span>
                <div class="list">
                    {move || sessions().into_iter().take(20).map(|s| {
                        let id = s.id;
                        let vol = Weight(s.volume());
                        view! {
                            <button type="button" class="row press" on:click=move |_| c.sheet.set(Some(Sheet::SessionDetail(id)))>
                                <span class="stack grow"><span>{s.day_name.clone()}</span><span class="meta">{format!("{} · {} {}", ui::day_label(ui::local(s.started_at).date_naive()), s.sets.len(), if s.sets.len() == 1 { "set" } else { "sets" })}</span></span>
                                <span class="r num">{format!("{} {}", format_thousands(vol.display_value(c.unit()).round() as i64), c.unit().label())}</span>
                            </button>
                        }
                    }).collect_view()}
                </div>
            </div>
        </div>
    }
}

#[component]
fn Chart(p: LiftProgress) -> impl IntoView {
    let c = ctx();
    let unit = c.unit();
    let pts: Vec<f64> = p.points.iter().map(|x| x.e1rm.display_value(unit)).collect();
    let recent = p.recent.clone();
    if pts.is_empty() {
        return view! { <p class="sub">"No sets for this lift yet"</p> }.into_any();
    }
    let last = *pts.last().unwrap_or(&0.0);
    let first = pts[0];
    let lo = pts.iter().cloned().fold(f64::MAX, f64::min);
    let hi = pts.iter().cloned().fold(f64::MIN, f64::max);
    // Round the axis to whole steps around the data.
    let step = if hi - lo > 40.0 { 10.0 } else if hi - lo > 12.0 { 5.0 } else { 2.0 };
    let y0 = (lo / step).floor() * step - step;
    let y1 = (hi / step).ceil() * step + step;
    let (w, h, left, top, bottom) = (350.0, 176.0, 28.0, 20.0, 150.0);
    let n = pts.len();
    let x_at = |i: usize| if n == 1 { (left + w) / 2.0 } else { left + (w - 8.0 - left) * i as f64 / (n - 1) as f64 };
    let y_at = |v: f64| bottom - (v - y0) / (y1 - y0) * (bottom - top);
    let line: String = pts.iter().enumerate().map(|(i, v)| format!("{}{:.1} {:.1}", if i == 0 { "M" } else { " " }, x_at(i), y_at(*v))).collect();
    let area = format!("{line}V{bottom}H{:.1}Z", x_at(0));
    let ticks = [y0 + step, (y0 + y1) / 2.0, y1 - step];
    let change = last - first;
    let first_label = ui::day_label(p.points[0].date);
    view! {
        <div class="rowx g8 wrap" style="align-items: baseline">
            <span class="fig num">{format_number((last * 10.0).round() / 10.0)}</span>
            <span class="sub">{if n > 1 { format!("{} · {} {} since {first_label}", unit.label(), if change >= 0.0 { "up" } else { "down" }, format_number((change.abs() * 10.0).round() / 10.0)) } else { unit.label().to_string() }}</span>
        </div>
        <svg class="chart" viewBox=format!("0 0 {w} {h}") width="100%" role="img" aria-label=format!("Estimated one-rep max over {n} sessions, now {} {}", format_number(last), unit.label())>
            <g stroke="rgba(255,255,255,.1)" stroke-width="1">
                {ticks.iter().map(|t| view! { <path d=format!("M{left} {:.1}H{w}", y_at(*t))></path> }).collect_view()}
            </g>
            <g fill="#8a8a8f" font-family="Geist Mono, ui-monospace, monospace" font-size="11">
                {ticks.iter().map(|t| view! { <text x="0" y=format!("{:.1}", y_at(*t) - 6.0)>{format_number(*t)}</text> }).collect_view()}
            </g>
            <path d=area fill="rgba(255,255,255,.07)" stroke="none" class="fade-in"></path>
            <path class="line" pathLength="1" d=line fill="none" stroke="#f5f5f7" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"></path>
            <circle class="end" cx=format!("{:.1}", x_at(n - 1)) cy=format!("{:.1}", y_at(last)) r="5" fill="#f5f5f7"></circle>
            <g fill="#8a8a8f" font-family="Geist Mono, ui-monospace, monospace" font-size="11">
                <text x=left y="170">{first_label.clone()}</text>
                <text x=w y="170" text-anchor="end">{ui::day_label(p.points[n - 1].date)}</text>
            </g>
        </svg>
        <div class="list">
            {recent.into_iter().map(|(d, s)| view! {
                <div class="row"><span>{ui::day_label(d)}</span><span class="r num">{s}</span></div>
            }).collect_view()}
        </div>
    }.into_any()
}
