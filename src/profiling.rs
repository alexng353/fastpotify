//! Completion timings for asynchronous data loads, including release builds.

use std::fmt::Display;
use std::time::Instant;

pub(crate) struct LoadTimer {
    started: Instant,
    operation: &'static str,
    context: String,
    finished: bool,
}

impl LoadTimer {
    pub(crate) fn new(operation: &'static str, context: impl Into<String>) -> Self {
        Self {
            started: Instant::now(),
            operation,
            context: context.into(),
            finished: false,
        }
    }

    pub(crate) fn begin_work(&mut self) {
        use std::fmt::Write;
        let _ = write!(
            self.context,
            " queue_ms={:.3}",
            self.started.elapsed().as_secs_f64() * 1000.0
        );
    }

    pub(crate) fn finish(&mut self, outcome: &str, details: impl Display) {
        log::info!(target: "fastpotify::loads",
            "{} {} elapsed_ms={:.3} {} {}",
            outcome, self.operation, self.started.elapsed().as_secs_f64() * 1000.0,
            self.context, details);
        self.finished = true;
    }

    pub(crate) fn result<T, E>(
        &mut self,
        result: &Result<T, E>,
        details: impl FnOnce(&T) -> String,
    ) {
        match result {
            Ok(value) => self.finish("Loaded", details(value)),
            Err(_) => self.finish("Load failed", "outcome=error"),
        }
    }
}

impl Drop for LoadTimer {
    fn drop(&mut self) {
        if !self.finished {
            self.finish("Load cancelled", "outcome=cancelled");
        }
    }
}
