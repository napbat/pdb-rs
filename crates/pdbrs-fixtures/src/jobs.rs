//! Run the work of a command on worker threads (#45).
//!
//! A command makes one [`Jobs`] value with the worker count of `--jobs`. All parallel work of
//! the command, also nested work, shares its helper tokens: the calling thread always works,
//! and each other busy thread holds one token. Thus at most the worker count of threads do
//! work at the same time. With one worker, all work runs on the calling thread in item order.

use std::num::NonZeroUsize;
use std::panic;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::thread::{self, Scope};

/// The worker budget of a command.
#[derive(Debug)]
pub struct Jobs {
    /// The number of helper threads that can start now.
    free: AtomicUsize,
}

impl Jobs {
    /// Make a budget for `workers` threads, the calling thread included.
    pub const fn new(workers: NonZeroUsize) -> Self {
        Self {
            free: AtomicUsize::new(workers.get() - 1),
        }
    }

    /// Return the default worker count: the available parallelism, or 1 if it is not known.
    pub fn available() -> NonZeroUsize {
        thread::available_parallelism().unwrap_or(NonZeroUsize::MIN)
    }

    /// Take a helper token. Return `false` if no token is free.
    fn take(&self) -> bool {
        self.free
            .try_update(Ordering::AcqRel, Ordering::Acquire, |free| {
                free.checked_sub(1)
            })
            .is_ok()
    }

    /// Give a helper token back.
    fn give(&self) {
        self.free.fetch_add(1, Ordering::AcqRel);
    }

    /// Apply `function` to each item and return the results in item order.
    ///
    /// The calling thread works on the items. Before it starts an item, a thread starts a
    /// helper if a token is free and more items are waiting.
    pub fn map<T, R, F>(&self, items: &[T], function: F) -> Vec<R>
    where
        T: Sync,
        R: Send,
        F: Fn(&T) -> R + Sync,
    {
        let work = Work {
            jobs: self,
            items,
            function,
            next: AtomicUsize::new(0),
            results: Mutex::new((0..items.len()).map(|_| None).collect()),
        };
        thread::scope(|scope| work.run(scope));
        work.results
            .into_inner()
            .unwrap_or_else(PoisonError::into_inner)
            .into_iter()
            .flatten()
            .collect()
    }

    /// Run `first` and `second` and return both results. Run `first` on a helper if a token
    /// is free; else run `first` and then `second` on the calling thread.
    pub fn join<A, B, RA, RB>(&self, first: A, second: B) -> (RA, RB)
    where
        A: FnOnce() -> RA + Send,
        B: FnOnce() -> RB,
        RA: Send,
    {
        if !self.take() {
            return (first(), second());
        }
        thread::scope(|scope| {
            let helper = scope.spawn(|| {
                let result = first();
                self.give();
                result
            });
            let second = second();
            let first = helper
                .join()
                .unwrap_or_else(|payload| panic::resume_unwind(payload));
            (first, second)
        })
    }
}

/// The shared state of one [`Jobs::map`] call.
struct Work<'a, T, R, F> {
    jobs: &'a Jobs,
    items: &'a [T],
    function: F,
    /// The index of the next item to start.
    next: AtomicUsize,
    /// The result of each finished item.
    results: Mutex<Vec<Option<R>>>,
}

impl<T, R, F> Work<'_, T, R, F>
where
    T: Sync,
    R: Send,
    F: Fn(&T) -> R + Sync,
{
    /// Start and finish items until no item is left.
    fn run<'scope>(&'scope self, scope: &'scope Scope<'scope, '_>) {
        loop {
            let index = self.next.fetch_add(1, Ordering::Relaxed);
            let Some(item) = self.items.get(index) else {
                break;
            };
            if index + 1 < self.items.len() && self.jobs.take() {
                scope.spawn(move || {
                    self.run(scope);
                    self.jobs.give();
                });
            }
            let result = (self.function)(item);
            self.results.lock().unwrap_or_else(PoisonError::into_inner)[index] = Some(result);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Jobs;
    use std::num::NonZeroUsize;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    fn jobs(workers: usize) -> Jobs {
        Jobs::new(NonZeroUsize::new(workers).unwrap())
    }

    #[test]
    fn map_keeps_the_item_order() {
        let items: Vec<u32> = (0..100).collect();
        let expected: Vec<u32> = items.iter().map(|item| item * 2).collect();
        for workers in [1, 2, 7, 64] {
            assert_eq!(jobs(workers).map(&items, |item| item * 2), expected);
            assert_eq!(jobs(workers).map(&[] as &[u32], |item| *item), Vec::new());
        }
    }

    #[test]
    fn one_worker_runs_everything_on_the_calling_thread_in_order() {
        let caller = thread::current().id();
        let jobs = jobs(1);
        let order = std::sync::Mutex::new(Vec::new());
        let threads = jobs.map(&[0, 1, 2, 3], |item| {
            order.lock().unwrap().push(*item);
            let (first, second) = jobs.join(|| thread::current().id(), || thread::current().id());
            [thread::current().id(), first, second]
        });
        assert_eq!(order.into_inner().unwrap(), [0, 1, 2, 3]);
        assert!(threads.iter().flatten().all(|id| *id == caller));
    }

    #[test]
    fn nested_work_never_exceeds_the_worker_count() {
        let items: Vec<usize> = (0..16).collect();
        let jobs = jobs(3);
        let busy = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        let leaf = |item: &usize| {
            let now = busy.fetch_add(1, Ordering::SeqCst) + 1;
            most.fetch_max(now, Ordering::SeqCst);
            thread::yield_now();
            busy.fetch_sub(1, Ordering::SeqCst);
            *item
        };
        let sums = jobs.map(&items, |outer| {
            let (left, right) = jobs.join(
                || jobs.map(&items, leaf).iter().sum::<usize>(),
                || leaf(outer),
            );
            left + right
        });
        let total: usize = items.iter().sum();
        assert_eq!(
            sums,
            items.iter().map(|item| total + item).collect::<Vec<_>>()
        );
        assert!(most.load(Ordering::SeqCst) <= 3);
    }

    #[test]
    fn join_returns_both_results_in_order() {
        for workers in [1, 2] {
            assert_eq!(jobs(workers).join(|| 1, || "two"), (1, "two"));
        }
    }
}
