//! Small shared pieces: number formatting, dates, the rolling figure.

use chrono::{DateTime, Local, NaiveDate, Utc};
use gym_core::api::{Lift, Prescription};
use gym_core::split::bucket;
use gym_core::{Unit, Weight};
use leptos::prelude::*;

pub fn w(weight: Weight, unit: Unit) -> String {
    weight.display_with_unit(unit)
}

pub fn rx(p: &Prescription, unit: Unit) -> (String, String) {
    let weight = if p.weight.0 == 0 { "Body".to_string() } else { w(p.weight, unit) };
    (format!("{} × {}", p.sets, p.reps), weight)
}

pub fn local(t: DateTime<Utc>) -> DateTime<Local> {
    t.with_timezone(&Local)
}

pub fn today() -> NaiveDate {
    Local::now().date_naive()
}

/// "Thu 19 Nov"
pub fn day_label(d: NaiveDate) -> String {
    d.format("%a %-d %b").to_string()
}

pub fn weekday_name(wd: u8) -> &'static str {
    ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"][wd as usize % 7]
}

pub fn weekday_short(wd: u8) -> &'static str {
    ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"][wd as usize % 7]
}

/// "8 min ago", "Wed 18:05"
pub fn ago(t: DateTime<Utc>) -> String {
    let mins = (Utc::now() - t).num_minutes();
    match mins {
        i64::MIN..=0 => "just now".into(),
        1..=59 => format!("{mins} min ago"),
        60..=1439 => format!("{} h ago", mins / 60),
        _ => local(t).format("%a %-d %b").to_string(),
    }
}

pub fn clock(t: DateTime<Utc>) -> String {
    local(t).format("%H:%M").to_string()
}

/// "Chest, shoulders, triceps"
pub fn muscles_of(lifts: &[Lift]) -> String {
    let mut seen: Vec<&'static str> = Vec::new();
    for l in lifts {
        if let Some(t) = &l.tags {
            let label = t.primary.label();
            if !seen.contains(&label) {
                seen.push(label);
            }
        }
    }
    let mut s = seen.iter().take(3).map(|x| x.to_lowercase()).collect::<Vec<_>>().join(", ");
    if let Some(first) = s.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    s
}

/// Rough minutes: about 45 seconds a set plus the rest after it.
pub fn minutes_of(lifts: &[Lift], default_rest: u32) -> u32 {
    let secs: u32 = lifts.iter().map(|l| l.prescription.sets * (45 + l.prescription.rest_seconds.unwrap_or(default_rest))).sum();
    ((secs as f64 / 60.0 / 5.0).round() * 5.0) as u32
}

pub fn count_word(n: usize) -> String {
    ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "eleven", "twelve"].get(n).map(|s| s.to_string()).unwrap_or_else(|| n.to_string())
}

pub fn bucket_label(l: &Lift) -> &'static str {
    l.tags.as_ref().map(|t| bucket(t).label()).unwrap_or("Needs a look")
}

/// A number that rolls when it changes. Keyed by value so each change mounts a fresh span.
#[component]
pub fn Roll(#[prop(into)] value: Signal<String>, #[prop(optional, into)] class: String, #[prop(optional, into)] down: Signal<bool>) -> impl IntoView {
    view! {
        <span class=move || format!("roll {class} {}", if down.get() { "down" } else { "" })>
            {move || {
                let v = value.get();
                view! { <span>{v}</span> }
            }}
        </span>
    }
}
