// SPDX-License-Identifier: MIT OR Apache-2.0
use super::{Mutex, NotAvailable};
use crate::guard::Guard;

#[cfg(target_arch = "wasm32")]
use crate as thread;
#[cfg(not(target_arch = "wasm32"))]
use std::thread;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

pub(crate) fn lock_block<T>(mutex: &Mutex<T>) -> Guard<'_, T> {
    //insert our thread into the waiting list
    loop {
        let r = mutex.waiting_sync_threads.with_mut(|threads| {
            match mutex.try_lock() {
                Ok(guard) => {
                    // Return the guard
                    Ok(guard)
                }
                Err(_) => {
                    let handle = thread::current();
                    threads.push(handle);
                    Err(NotAvailable)
                }
            }
        });
        match r {
            Ok(guard) => return guard,
            Err(NotAvailable) => thread::park(),
        }
    }
}

pub(crate) fn lock_block_timeout<T>(mutex: &Mutex<T>, deadline: Instant) -> Option<Guard<'_, T>> {
    loop {
        let now = Instant::now();
        if now >= deadline {
            // Try one last time
            if let Ok(guard) = mutex.try_lock() {
                return Some(guard);
            }
            return None;
        }

        let r = mutex
            .waiting_sync_threads
            .with_mut(|threads| match mutex.try_lock() {
                Ok(guard) => Ok(guard),
                Err(_) => {
                    let handle = thread::current();
                    threads.push(handle);
                    Err(NotAvailable)
                }
            });

        match r {
            Ok(guard) => return Some(guard),
            Err(NotAvailable) => {
                let remaining = deadline - Instant::now();
                thread::park_timeout(remaining);
            }
        }
    }
}
