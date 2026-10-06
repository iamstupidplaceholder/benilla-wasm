use crate::messages::MovementInfo;
use crate::wire::Vector3d;

/// The complete live mover snapshot, shared by TCP writer and WebSocket adapter.
pub fn movement_snapshot(
    pos: [f32; 3],
    orientation: f32,
    flags: u32,
    pitch: f32,
    fall_time: u32,
    jump: Option<crate::JumpInfo>,
    transport: Option<crate::TransportPose>,
) -> MovementInfo {
    let mut info = movement_info(pos, orientation, flags);
    info.pitch = pitch;
    info.fall_time = fall_time;
    info.jump = jump;
    info.transport = transport;
    info
}

/// `MovementFlags::FORWARD` — the only movement flag benilla sets.
pub(super) const MOVEMENT_FLAG_FORWARD: u32 = 0x1;

/// A monotonic **client-uptime** millisecond stamp for the `MovementInfo` time field — the kind the real
/// 1.12 client sends (`GetTickCount()`, ms since the client started), **not** wall-clock. vmangos reads it
/// into `MovementInfo::ctime` and uses it only two ways (VERIFIED, decision 0057): a non-zero guard
/// (`ctime == 0` reads as "not a client packet" and *pauses* its movement extrapolation) and *deltas*
/// (`getMSTimeDiff`, so the epoch cancels). It never compares the value to its own clock, and it
/// overwrites the field with its server clock before relaying to nearby players — so this value never
/// reaches observers. Hence: match the real client's clock kind, wrap cleanly as a full `u32` (~49.7 days,
/// which `getMSTimeDiff` handles), and keep it non-zero. (The old value was wall-clock masked to 31 bits,
/// which folded backwards every ~24.85 days — a delta glitch the clean `u32` wrap avoids.)
pub fn client_uptime_ms() -> u32 {
    use benilla_time::Instant;
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    (START.get_or_init(Instant::now).elapsed().as_millis() as u32).max(1)
}

/// Build a `MovementInfo` stamped with the client-uptime time ([`client_uptime_ms`]).
pub fn movement_info(pos: [f32; 3], orientation: f32, flags: u32) -> MovementInfo {
    MovementInfo {
        flags,
        timestamp: client_uptime_ms(),
        position: Vector3d {
            x: pos[0],
            y: pos[1],
            z: pos[2],
        },
        orientation,
        // benilla never sets ON_TRANSPORT outbound yet (boarding/riding is decision 0438 phase 2) — the
        // writer only emits the transport tail while the flag is set, so this is always dropped.
        transport: None,
        // Level swim (0) by default; the writer only emits the pitch tail while SWIMMING is set, and the
        // controller supplies the live swim pitch through `send_movement` when it does.
        pitch: 0.0,
        fall_time: 0,
        jump: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_uptime_ms_is_nonzero_and_monotonic() {
        // The load-bearing property: vmangos treats `ctime == 0` as "not a client packet" and pauses its
        // extrapolation, so our stamp must never be zero (the bug the old wall-clock value worked around).
        // It must also be non-decreasing so `getMSTimeDiff` deltas stay sane.
        let a = client_uptime_ms();
        let b = client_uptime_ms();
        assert!(a >= 1, "stamp is non-zero: {a}");
        assert!(b >= a, "stamp is monotonic non-decreasing: {a} -> {b}");
    }

    #[test]
    fn movement_info_is_stamped_nonzero() {
        let info = movement_info([1.0, 2.0, 3.0], 0.5, MOVEMENT_FLAG_FORWARD);
        assert_ne!(
            info.timestamp, 0,
            "a movement packet must carry a non-zero time"
        );
    }

    #[test]
    fn full_snapshot_preserves_live_tails() {
        let jump = crate::JumpInfo {
            zspeed: 4.0,
            cos_angle: 0.5,
            sin_angle: 0.25,
            xy_speed: 7.0,
        };
        let info = movement_snapshot(
            [1.0, 2.0, 3.0],
            0.5,
            0x2000 | 0x200000,
            0.75,
            123,
            Some(jump),
            None,
        );
        assert_ne!(info.timestamp, 0);
        assert_eq!(info.position.x, 1.0);
        assert_eq!(info.orientation, 0.5);
        assert_eq!(info.pitch, 0.75);
        assert_eq!(info.fall_time, 123);
        assert_eq!(info.jump.unwrap().xy_speed, 7.0);
        // Existing serializers must retain the exact echoed speed and counter.
        let body = crate::messages::force_speed_ack(7, 42, &info, 9.0);
        assert_eq!(&body[..8], &7u64.to_le_bytes());
        assert_eq!(&body[8..12], &42u32.to_le_bytes());
        assert_eq!(&body[body.len() - 4..], &9.0f32.to_le_bytes());
    }
}
