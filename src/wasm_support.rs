// SPDX-License-Identifier: MIT OR Apache-2.0

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(inline_js = "
export function _wsm_supportsAtomicsWait() {
    if (typeof SharedArrayBuffer === 'undefined') return false;
    if (typeof Atomics === 'undefined' || typeof Atomics.wait !== 'function') return false;

    try {
        const sab = new SharedArrayBuffer(4);
        const ia = new Int32Array(sab);
        const result = Atomics.wait(ia, 0, 0, 0);
        return result === 'timed-out' || result === 'not-equal';
    } catch (_) {
        return false;
    }
}
")]
extern "C" {
    fn _wsm_supportsAtomicsWait() -> bool;
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn atomics_wait_supported() -> bool {
    _wsm_supportsAtomicsWait()
}
