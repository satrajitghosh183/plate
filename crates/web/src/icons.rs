//! Line icons, one set, 1.5 stroke, coloured by their text.

use leptos::prelude::*;

pub fn icon(d: &'static str) -> impl IntoView {
    view! { <svg class="ic" viewBox="0 0 24 24" aria-hidden="true"><path d=d></path></svg> }
}

pub fn icon_fill(d: &'static str) -> impl IntoView {
    view! { <svg class="ic fill" viewBox="0 0 24 24" aria-hidden="true"><path d=d></path></svg> }
}

pub const CHEVRON: &str = "m9 18 6-6-6-6";
pub const PLAY: &str = "M7 4 20 12 7 20Z";
pub const PAUSE: &str = "M7 4h3v16H7zM14 4h3v16h-3z";
pub const NEXT: &str = "M5 4l10 8-10 8zM17 4h2v16h-2z";
pub const PREV: &str = "M19 4 9 12l10 8zM5 4h2v16H5z";
pub const MINUS: &str = "M5 12h14";
pub const PLUS: &str = "M5 12h14M12 5v14";
pub const CLOSE: &str = "M18 6 6 18M6 6l12 12";
pub const CHECK: &str = "M4 12.5 9.5 18 20 6.5";
pub const LIST: &str = "M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01";
pub const CALENDAR: &str = "M3 6a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2zM16 2v4M8 2v4M3 10h18";
pub const TREND: &str = "m22 7-8.5 8.5-5-5L2 17M16 7h6v6";
pub const FLAME: &str = "M8.5 14.5A2.5 2.5 0 0 0 11 12c0-1.38-.5-2-1-3-1.072-2.143-.224-4.054 2-6 .5 2.5 2 4.9 4 6.5 2 1.6 3 3.5 3 5.5a7 7 0 1 1-14 0c0-1.153.433-2.294 1-3a2.5 2.5 0 0 0 2.5 2.5z";
pub const LINK: &str = "M9 17H7A5 5 0 0 1 7 7h2M15 7h2a5 5 0 1 1 0 10h-2M8 12h8";
pub const MUSIC: &str = "M9 18V5l12-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z";
pub const SETTINGS: &str = "M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6zM19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z";
pub const VIDEO_OFF: &str = "M10.66 6H14a2 2 0 0 1 2 2v2.5l5.248-3.062A.5.5 0 0 1 22 7.87v8.196M16 16a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h2M2 2l20 20";
pub const KEY: &str = "M21 2l-2 2m-7.61 7.61a5.5 5.5 0 1 1-7.778 7.778 5.5 5.5 0 0 1 7.777-7.777zm0 0L15.5 7.5m0 0 3 3L22 7l-3-3m-3.5 3.5L19 4";
