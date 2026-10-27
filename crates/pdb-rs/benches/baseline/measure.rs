//! The timed runs of one scenario.

use std::time::{Duration, Instant};

use crate::Outcome;

/// The timed runs of one scenario in a child process.
pub(crate) struct Bench {
    /// The number of timed runs after the warm-up run.
    runs: usize,
    /// The duration of each timed run.
    times: Vec<Duration>,
    /// The item count of the runs. All runs must give the same count.
    count: Option<u64>,
}

impl Bench {
    /// Make a bench with `runs` timed runs.
    pub(crate) const fn new(runs: usize) -> Self {
        Self {
            runs,
            times: Vec::new(),
            count: None,
        }
    }

    /// Call `run` once as a warm-up, then `runs` times with a timer.
    ///
    /// `run` returns the number of items that it processed. A scenario calls this function
    /// once, after it has loaded its input.
    pub(crate) fn measure(&mut self, mut run: impl FnMut() -> Outcome<u64>) -> Outcome {
        if self.count.is_some() {
            return Err("a scenario measures only once".into());
        }
        let expected = std::hint::black_box(run()?);
        for _ in 0..self.runs {
            let start = Instant::now();
            let count = std::hint::black_box(run()?);
            self.times.push(start.elapsed());
            if count != expected {
                return Err(format!("a run gave {count} items, the warm-up run {expected}").into());
            }
        }
        self.count = Some(expected);
        Ok(())
    }

    /// Return the item count and the durations of the timed runs.
    pub(crate) fn finish(self) -> Outcome<(u64, Vec<Duration>)> {
        let count = self.count.ok_or("the scenario measured nothing")?;
        Ok((count, self.times))
    }
}
