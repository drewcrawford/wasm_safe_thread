// SPDX-License-Identifier: MIT OR Apache-2.0
use super::{Mutex, NotAvailable};
use crate::guard::Guard;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

pub(crate) async fn lock_async<T>(mutex: &Mutex<T>) -> Guard<'_, T> {
    loop {
        let a = mutex.waiting_async_threads.with_mut(|senders| {
            match mutex.try_lock() {
                Ok(guard) => Ok(guard),
                Err(NotAvailable) => {
                    // Create a new channel to signal when the lock is available
                    let (sender, receiver) = r#continue::continuation();
                    senders.push(sender);
                    Err(receiver)
                }
            }
        });
        match a {
            Ok(guard) => return guard,
            Err(receiver) => {
                // Wait for the signal that the lock is available
                receiver.await;
            }
        }
    }
}

pub(crate) async fn lock_async_timeout<T>(
    mutex: &Mutex<T>,
    deadline: Instant,
) -> Option<Guard<'_, T>> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            // Try one last time
            if let Ok(guard) = mutex.try_lock() {
                return Some(guard);
            }
            return None;
        }

        let a = mutex.waiting_async_threads.with_mut(|senders| {
            match mutex.try_lock() {
                Ok(guard) => Ok(guard),
                Err(NotAvailable) => {
                    // Create a new channel to signal when the lock is available
                    let (sender, receiver) = r#continue::continuation();
                    senders.push(sender);
                    Err(receiver)
                }
            }
        });

        match a {
            Ok(guard) => return Some(guard),
            Err(receiver) => {
                // Create a channel for timeout
                let (timeout_sender, timeout_receiver) = r#continue::continuation();

                // Spawn a thread to handle the timeout
                thread::Builder::new()
                    .name("lock_async_timeout".to_string())
                    .spawn(move || {
                        let now = Instant::now();
                        if deadline > now {
                            let duration = deadline - now;
                            thread::sleep(duration);
                        }
                        // Send timeout signal
                        timeout_sender.send(());
                    })
                    .expect("Failed to spawn timeout thread");

                // Race between notification and timeout
                struct Race<F1, F2> {
                    notify: Option<F1>,
                    timeout: Option<F2>,
                }

                impl<F1: Future + Unpin, F2: Future + Unpin> Future for Race<F1, F2> {
                    type Output = bool; // true if timed out

                    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
                        // Poll notification future
                        if let Some(ref mut notify) = self.notify {
                            if Pin::new(notify).poll(cx).is_ready() {
                                self.notify = None;
                                return Poll::Ready(false); // Got notification
                            }
                        }

                        // Poll timeout future
                        if let Some(ref mut timeout) = self.timeout {
                            if Pin::new(timeout).poll(cx).is_ready() {
                                self.timeout = None;
                                return Poll::Ready(true); // Timed out
                            }
                        }

                        Poll::Pending
                    }
                }

                let timed_out = Race {
                    notify: Some(receiver),
                    timeout: Some(timeout_receiver),
                }
                .await;

                if timed_out {
                    return None;
                }
                // If not timed out, we loop and try to lock again
            }
        }
    }
}
