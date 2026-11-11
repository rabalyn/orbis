//! Work that waits for the network, done away from the frame: on a thread of its own, or in a
//! browser, where a page has one thread, by running the job again on each `poll` until every
//! download it needs has arrived. A job must therefore come to the same answer when run again,
//! which holds where it keeps what it has fetched, as the service modules all do.

use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Mutex;

#[cfg(target_arch = "wasm32")]
use crate::wtime::{Duration, Instant};

pub struct Worker<J, R> {
    #[cfg(not(target_arch = "wasm32"))]
    jobs: Sender<J>,
    #[cfg(target_arch = "wasm32")]
    state: Mutex<Inline<J, R>>,
    replies: Mutex<Receiver<R>>,
    #[cfg(target_arch = "wasm32")]
    reply_tx: Sender<R>,
}

#[cfg(target_arch = "wasm32")]
struct Inline<J, R> {
    queue: std::collections::VecDeque<J>,
    run: Box<dyn FnMut(J) -> R>,
    latest_only: bool,
    gap: Duration,
    last_done: Option<Instant>,
}

impl<J: Clone + Send + 'static, R: Send + 'static> Worker<J, R> {
    /// `latest_only` drops jobs that a newer one has replaced before they started. `gap` keeps
    /// jobs that far apart in a browser, where the job's own pauses cannot sleep.
    pub fn new(
        latest_only: bool,
        #[allow(unused)] gap: crate::wtime::Duration,
        #[allow(unused_mut)] mut run: impl FnMut(J) -> R + Send + 'static,
    ) -> Self {
        let (reply_tx, replies) = channel();
        #[cfg(not(target_arch = "wasm32"))]
        {
            let (jobs, job_rx) = channel::<J>();
            std::thread::spawn(move || {
                // The thread ends when the worker is dropped and the channel closes.
                while let Ok(mut job) = job_rx.recv() {
                    if latest_only {
                        while let Ok(newer) = job_rx.try_recv() {
                            job = newer;
                        }
                    }
                    if reply_tx.send(run(job)).is_err() {
                        break;
                    }
                }
            });
            Self {
                jobs,
                replies: Mutex::new(replies),
            }
        }
        #[cfg(target_arch = "wasm32")]
        Self {
            state: Mutex::new(Inline {
                queue: Default::default(),
                run: Box::new(run),
                latest_only,
                gap,
                last_done: None,
            }),
            replies: Mutex::new(replies),
            reply_tx,
        }
    }

    pub fn submit(&self, job: J) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = self.jobs.send(job);
        #[cfg(target_arch = "wasm32")]
        self.state.lock().unwrap().queue.push_back(job);
    }

    pub fn poll(&self) -> Option<R> {
        #[cfg(target_arch = "wasm32")]
        self.step();
        self.replies.lock().ok()?.try_recv().ok()
    }

    /// Runs the next job once. If it had to wait for a download, its answer is not the real one,
    /// so it stays first in line and runs again on a later poll.
    #[cfg(target_arch = "wasm32")]
    fn step(&self) {
        let mut s = self.state.lock().unwrap();
        if s.latest_only {
            while s.queue.len() > 1 {
                s.queue.pop_front();
            }
        }
        let Some(job) = s.queue.front().cloned() else {
            return;
        };
        if s.last_done.is_some_and(|t| t.elapsed() < s.gap) {
            return;
        }
        crate::web::clear_waiting();
        let reply = (s.run)(job);
        if crate::web::take_waiting() {
            return;
        }
        s.queue.pop_front();
        s.last_done = Some(Instant::now());
        let _ = self.reply_tx.send(reply);
    }
}
