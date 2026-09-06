//! Task D5, Step 1: the pure drain regression. `run_all` used to perform one
//! capped selection, so a backlog larger than the cap stayed pending (Astra
//! 17); `drain_batches` is the bounded batch loop the production drain is
//! built on.

use hieronymus::dreaming::drain_batches;
use std::sync::atomic::AtomicBool;
#[test]
fn dream_all_drains_more_than_one_batch() {
    let mut batches = [2_usize, 2, 1, 0].into_iter();
    let count = drain_batches(|| Ok(batches.next().unwrap_or(0)), &AtomicBool::new(false)).unwrap();
    assert_eq!(count, 5);
}
