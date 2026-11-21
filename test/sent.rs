//! `src/sent.rs` against a simulated send ring (review finding 6aeccbfbe450,
//! #37): a caller that transmits far more often than it calls `GetStatus` gets
//! `NOT_READY` once the list is full, and every buffer it handed over comes
//! back, in order, none lost. Run by `scripts/test-host.sh`.
extern crate alloc;

#[path = "../src/sent.rs"]
mod sent;
use sent::Sent;

const CAP: usize = 1024;
/// Usable ring slots, as `eth.rs` (TX_TXBBS − HEADROOM − 1).
const RING: usize = 128 - 33 - 1;

/// The port's TX side: a ring of in-flight tokens the device completes in order.
struct Port {
    ring: Vec<usize>,
    sent: Sent,
}

impl Port {
    fn new() -> Port {
        Port { ring: Vec::new(), sent: Sent::new(CAP) }
    }
    /// The device completes every queued send (what `reap_tx` reaps).
    fn complete(&mut self) {
        for t in self.ring.drain(..) {
            self.sent.push(t);
        }
    }
    /// `Transmit`: `false` is `NOT_READY`.
    fn send(&mut self, token: usize) -> bool {
        self.complete();
        if self.ring.len() >= RING || !self.sent.admits(self.ring.len()) {
            return false;
        }
        self.ring.push(token);
        true
    }
    /// `GetStatus`.
    fn take(&mut self) -> Option<usize> {
        self.complete();
        self.sent.take()
    }
}

#[test]
fn never_collected_is_refused_not_dropped() {
    let mut p = Port::new();
    let mut n = 0;
    while p.send(0x1000 + n) {
        n += 1;
        assert!(n <= CAP, "accepted more sends than the list holds");
    }
    assert_eq!(n, CAP);
    // Still refused however often it is asked.
    for _ in 0..5000 {
        assert!(!p.send(0xdead));
    }
    for i in 0..CAP {
        assert_eq!(p.take(), Some(0x1000 + i));
    }
    assert_eq!(p.take(), None);
}

#[test]
fn collecting_one_admits_one() {
    let mut p = Port::new();
    let mut n = 0;
    while p.send(n) {
        n += 1;
    }
    assert_eq!(p.take(), Some(0));
    assert!(p.send(n));
    assert!(!p.send(n + 1));
}

#[test]
fn long_transfer_returns_every_token() {
    // Polls GetStatus once per 7 transmits: more sends than CAP overall.
    let mut p = Port::new();
    let mut next = 0usize;
    let mut back = Vec::new();
    for round in 0..20_000 {
        if p.send(next) {
            next += 1;
        } else {
            while let Some(t) = p.take() {
                back.push(t);
            }
        }
        if round % 7 == 0 {
            if let Some(t) = p.take() {
                back.push(t);
            }
        }
        // Tokens handed over and not yet back: on the ring or in the list.
        assert!(next - back.len() <= CAP);
    }
    while let Some(t) = p.take() {
        back.push(t);
    }
    assert!(next > CAP);
    assert_eq!(back, (0..next).collect::<Vec<_>>());
}

#[test]
fn empty_and_admits_at_bounds() {
    let s = Sent::new(2);
    assert!(s.is_empty());
    assert!(s.admits(1));
    assert!(!s.admits(2));
}
