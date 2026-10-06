//! Connection-local keepalive statistics shared by TCP and WebSocket IO.
use benilla_time::{Duration, Instant};

/// vmangos rejects repeated pings faster than 27 seconds; the client uses 30.
pub(super) const PING_INTERVAL: Duration = Duration::from_secs(30);
// The reference's 16-slot ring keeps one empty slot: 15 usable samples.
pub(super) const RTT_RING: usize = 15;

#[derive(Default)]
pub(crate) struct PingClock {
    pub(crate) sequence: u32,
    pub(crate) sent_at: Option<Instant>,
    pub(crate) last_rtt_ms: Option<u32>,
    pub(super) rtt_ring: std::collections::VecDeque<u32>,
}

impl PingClock {
    /// Record at the IO boundary, not later in the gameplay event drain.
    pub(crate) fn record_pong(&mut self, sequence: u32) -> Option<u32> {
        let sent = self.sent_at.filter(|_| self.sequence == sequence)?;
        let rtt = sent.elapsed().as_millis().min(u128::from(u32::MAX)) as u32;
        self.file_rtt(rtt);
        Some(rtt)
    }

    pub(super) fn file_rtt(&mut self, rtt: u32) {
        self.last_rtt_ms = Some(rtt);
        if self.rtt_ring.len() == RTT_RING {
            self.rtt_ring.pop_front();
        }
        self.rtt_ring.push_back(rtt);
    }

    pub(crate) fn avg_latency_ms(&self) -> Option<u32> {
        if self.rtt_ring.is_empty() {
            return None;
        }
        let sum: u64 = self.rtt_ring.iter().map(|&ms| u64::from(ms)).sum();
        Some((sum / self.rtt_ring.len() as u64) as u32)
    }

    pub(crate) fn clear(&mut self) {
        *self = Self::default();
    }
}
