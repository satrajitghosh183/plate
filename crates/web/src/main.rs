//! The phone app. Leptos compiled to WebAssembly, installed from the web.

mod bridge;
mod icons;
mod net;
mod state;
mod ui;
mod views;

use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    bridge::register_worker();
    mount_to_body(views::app::App);
    bridge::hide_boot();
}
