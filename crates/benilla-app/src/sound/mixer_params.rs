//! Common native/browser mixer parameter conversion, not a second DSP backend.
use kira::{Decibels, Tween};
pub(crate) const GLIDE_MS: u64 = 15;
pub(crate) fn freeverb_projection(
    decay: f32,
    hf: f32,
    room_hf: i32,
    room: i32,
    reverb: i32,
) -> (f64, f64, Decibels) {
    let feedback = if decay > 0.0 {
        10f64.powf(-0.108 / f64::from(decay)).min(0.98)
    } else {
        0.0
    };
    let damping = ((1.0 - f64::from(hf) / 2.0).max(0.0) * 0.7
        + f64::from(-room_hf) / 10000.0 * 0.3)
        .clamp(0.0, 1.0);
    let wet_db = ((room + reverb) as f32 / 100.0).min(6.0);
    (
        feedback,
        damping,
        if wet_db <= Decibels::SILENCE.0 {
            Decibels::SILENCE
        } else {
            Decibels(wet_db)
        },
    )
}
pub(crate) fn glide() -> Tween {
    Tween {
        duration: std::time::Duration::from_millis(GLIDE_MS),
        ..Default::default()
    }
}
pub(crate) fn amp_to_db(amp: f32) -> Decibels {
    if amp <= 1e-3 {
        Decibels::SILENCE
    } else {
        Decibels(20.0 * amp.log10())
    }
}
