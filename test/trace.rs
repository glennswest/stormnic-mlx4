//! `src/trace.rs`: the ring of suppressed trace lines a quiet failure
//! replays (#16). Run by `scripts/test-host.sh`.
extern crate alloc;

#[path = "../src/trace.rs"]
mod trace;
use trace::{Ring, KEEP};

fn line(i: usize) -> String {
    format!("  step {i}")
}

#[test]
fn keeps_everything_below_the_limit() {
    let mut r = Ring::new();
    for i in 0..5 {
        r.push(line(i));
    }
    let (lines, dropped) = r.take();
    assert_eq!(dropped, 0);
    assert_eq!(lines.into_iter().collect::<Vec<_>>(), (0..5).map(line).collect::<Vec<_>>());
}

#[test]
fn keeps_the_last_keep_lines_and_counts_the_rest() {
    let mut r = Ring::new();
    for i in 0..KEEP + 7 {
        r.push(line(i));
    }
    let (lines, dropped) = r.take();
    assert_eq!(dropped, 7);
    assert_eq!(lines.len(), KEEP);
    assert_eq!(lines.front().unwrap(), &line(7));
    assert_eq!(lines.back().unwrap(), &line(KEEP + 6));
}

#[test]
fn take_and_clear_empty_it() {
    let mut r = Ring::new();
    for i in 0..KEEP + 1 {
        r.push(line(i));
    }
    let _ = r.take();
    let (lines, dropped) = r.take();
    assert!(lines.is_empty());
    assert_eq!(dropped, 0);

    for i in 0..KEEP + 3 {
        r.push(line(i));
    }
    r.clear();
    r.push(line(99));
    let (lines, dropped) = r.take();
    assert_eq!(dropped, 0);
    assert_eq!(lines.into_iter().collect::<Vec<_>>(), [line(99)]);
}
