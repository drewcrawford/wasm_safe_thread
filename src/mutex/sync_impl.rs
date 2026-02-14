// SPDX-License-Identifier: MIT OR Apache-2.0
use super::Mutex;
use crate::guard::Guard;

#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;
#[cfg(target_arch = "wasm32")]
use web_time::Instant;

pub(crate) fn lock_sync<T>(mutex: &Mutex<T>) -> Guard<'_, T> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        mutex.lock_block()
    }
    #[cfg(target_arch = "wasm32")]
    {
        if crate::wasm_support::atomics_wait_supported() {
            mutex.lock_block()
        } else {
            // Fallback to spin lock if Atomics.wait is not supported
            mutex.lock_spin()
        }
    }
}

pub(crate) fn lock_sync_timeout<T>(mutex: &Mutex<T>, deadline: Instant) -> Option<Guard<'_, T>> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        mutex.lock_block_timeout(deadline)
    }
    #[cfg(target_arch = "wasm32")]
    {
        if crate::wasm_support::atomics_wait_supported() {
            mutex.lock_block_timeout(deadline)
        } else {
            mutex.lock_spin_timeout(deadline)
        }
    }
}
