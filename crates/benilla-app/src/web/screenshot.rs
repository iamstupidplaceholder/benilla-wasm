//! Explicitly disabled browser screenshots, by product choice. No readback/export.
use bevy::prelude::*;

pub(crate) struct ScreenshotPlugin;
impl Plugin for ScreenshotPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, discard_requests.after(crate::bindings::BindingSet));
    }
}

fn discard_requests(
    script: Option<NonSendMut<benilla_ui::script::UiScript>>,
    mut reported: Local<bool>,
) {
    let Some(mut script) = script else {
        return;
    };
    let asks = script.take_screenshot_asks();
    if asks == 0 {
        return;
    }
    if !*reported {
        warn!("screenshots are disabled in the browser build");
        *reported = true;
    }
    for _ in 0..asks {
        script.fire_event("SCREENSHOT_FAILED", Vec::new());
    }
}
