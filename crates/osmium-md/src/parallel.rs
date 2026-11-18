//! Running independent pieces of a read on several threads.
//!
//! What a read returns never depends on the number of threads or on how the pieces are
//! scheduled: pieces are numbered, and their results are put together in that order.

use std::{
    collections::BTreeMap,
    num::NonZeroUsize,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
};

/// The number of threads a read uses: one per available core.
pub(crate) fn threads() -> usize {
    thread::available_parallelism().map_or(1, NonZeroUsize::get)
}

/// Calls `step(state, piece)` once for every piece below `pieces`, on up to `threads`
/// threads that each own one state made by `init`, and returns the states.
///
/// Which state sees which pieces is not determined; callers combine the states with an
/// operation that does not depend on it.
pub(crate) fn fold<S: Send>(
    threads: usize,
    pieces: usize,
    init: impl Fn() -> S + Sync,
    step: impl Fn(&mut S, usize) + Sync,
) -> Vec<S> {
    let workers = threads.min(pieces);
    if workers <= 1 {
        let mut state = init();
        for piece in 0..pieces {
            step(&mut state, piece);
        }
        return vec![state];
    }
    let next = AtomicUsize::new(0);
    let work = || {
        let mut state = init();
        loop {
            let piece = next.fetch_add(1, Ordering::Relaxed);
            if piece >= pieces {
                return state;
            }
            step(&mut state, piece);
        }
    };
    thread::scope(|scope| {
        let handles: Vec<_> = (0..workers).map(|_| scope.spawn(work)).collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    })
}

/// Runs `task(piece)` for every piece below `pieces` on up to `threads` threads and hands
/// the results to `sink` on the calling thread, in piece order, each as soon as the
/// ones before it are done.
///
/// `sink` returns whether to go on. Once it returns `false` it is not called again, and
/// no further piece is started.
pub(crate) fn ordered<T: Send>(
    threads: usize,
    pieces: usize,
    task: impl Fn(usize) -> T + Sync,
    mut sink: impl FnMut(T) -> bool,
) {
    let workers = threads.min(pieces);
    if workers <= 1 {
        for piece in 0..pieces {
            if !sink(task(piece)) {
                return;
            }
        }
        return;
    }
    let next = AtomicUsize::new(0);
    // A worker waits when the calling thread falls this far behind, so the results in
    // flight stay a small multiple of the number of threads.
    let (sender, receiver) = mpsc::sync_channel(workers);
    thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                let sender = sender.clone();
                let (next, task) = (&next, &task);
                scope.spawn(move || {
                    loop {
                        let piece = next.fetch_add(1, Ordering::Relaxed);
                        if piece >= pieces || sender.send((piece, task(piece))).is_err() {
                            return;
                        }
                    }
                })
            })
            .collect();
        drop(sender);

        let mut early = BTreeMap::new();
        let mut wanted = 0;
        let mut going = true;
        for (piece, result) in receiver {
            if !going {
                continue;
            }
            early.insert(piece, result);
            while let Some(result) = early.remove(&wanted) {
                wanted += 1;
                if !sink(result) {
                    going = false;
                    early.clear();
                    next.store(pieces, Ordering::Relaxed);
                    break;
                }
            }
        }
        for handle in handles {
            handle
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic));
        }
    });
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::{fold, ordered};

    #[test]
    fn results_arrive_in_piece_order_whatever_the_number_of_threads() {
        for threads in [0, 1, 2, 3, 8, 64] {
            for pieces in [0, 1, 2, 7, 100] {
                let mut squares = Vec::new();
                ordered(
                    threads,
                    pieces,
                    |piece| piece * piece,
                    |square| {
                        squares.push(square);
                        true
                    },
                );
                let expected: Vec<usize> = (0..pieces).map(|piece| piece * piece).collect();
                assert_eq!(squares, expected, "{threads} threads, {pieces} pieces");
            }
        }
    }

    #[test]
    fn nothing_arrives_after_the_sink_says_stop() {
        for threads in [1, 2, 8] {
            let started = AtomicUsize::new(0);
            let mut seen = Vec::new();
            ordered(
                threads,
                10_000,
                |piece| {
                    started.fetch_add(1, Ordering::Relaxed);
                    piece
                },
                |piece| {
                    seen.push(piece);
                    piece < 5
                },
            );
            assert_eq!(seen, [0, 1, 2, 3, 4, 5], "{threads} threads");
            if threads == 1 {
                assert_eq!(started.load(Ordering::Relaxed), 6);
            }
        }
    }

    #[test]
    fn every_piece_is_folded_into_exactly_one_state() {
        for threads in [1, 2, 5, 16] {
            let made = AtomicUsize::new(0);
            let states = fold(
                threads,
                1_000,
                || {
                    made.fetch_add(1, Ordering::Relaxed);
                    Vec::new()
                },
                |seen: &mut Vec<usize>, piece| seen.push(piece),
            );
            assert_eq!(states.len(), made.load(Ordering::Relaxed));
            assert!(states.len() <= threads);
            let mut seen: Vec<usize> = states.into_iter().flatten().collect();
            seen.sort_unstable();
            assert_eq!(seen, (0..1_000).collect::<Vec<_>>());
        }
    }

    #[test]
    #[should_panic(expected = "piece 3 failed")]
    fn a_panic_in_a_piece_reaches_the_caller() {
        ordered(
            4,
            8,
            |piece| assert!(piece != 3, "piece 3 failed"),
            |()| true,
        );
    }
}
