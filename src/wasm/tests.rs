//! Tests for WASM backend

use super::wasm_utils::is_node;
use super::*;
use std::time::Duration;
use wasm_bindgen::prelude::*;

#[wasm_bindgen_test::wasm_bindgen_test]
fn test_is_main_thread() {
    assert!(is_main_thread());
}

#[wasm_bindgen_test::wasm_bindgen_test]
fn test_default_shim_url_is_wasm_bindgen_test() {
    // Verify that the default shim URL ends with "wasm-bindgen-test.js"
    // This is the expected behavior when running wasm-bindgen-test tests
    let url = get_shim_url_for_testing("wasm-bindgen-test");
    assert!(
        url.ends_with("/wasm-bindgen-test.js"),
        "Expected shim URL to end with /wasm-bindgen-test.js, got: {}",
        url
    );
}

#[wasm_bindgen_test::wasm_bindgen_test]
fn test_detect_shim_url_finds_wasm_bindgen_test() {
    // Performance API resource tracking only works in browsers, not Node.js
    if is_node() {
        // In Node.js, detection returns None - this is expected
        assert!(
            get_detected_shim_url().is_none(),
            "In Node.js, shim URL detection should return None"
        );
        return;
    }

    // When running in a browser, we should detect wasm-bindgen-test.js
    let debug_info = get_performance_resources_debug();
    let detected = get_detected_shim_url();
    assert!(
        detected.is_some(),
        "Should detect shim URL from loaded resources. Debug info:\n{}",
        debug_info
    );
    let url = detected.unwrap();
    assert!(
        url.ends_with("/wasm-bindgen-test.js"),
        "Expected detected shim URL to end with /wasm-bindgen-test.js, got: {}",
        url
    );
}

// Test that park_timeout works on Node.js main thread (Atomics.wait is available there)
#[wasm_bindgen_test::wasm_bindgen_test]
fn test_park_timeout_on_node_main_thread() {
    if !is_node() {
        // Skip this test in browser - Atomics.wait will panic on main thread
        // which is the expected behavior, but we can't test it here
        return;
    }
    // In Node.js, Atomics.wait works on main thread, so park should succeed
    park_timeout(Duration::from_millis(1));
}

// Test that park panics on browser main thread (Atomics.wait unavailable there).
// This produces "RuntimeError: unreachable" in console - expected for wasm panics.
#[wasm_bindgen_test::wasm_bindgen_test]
#[should_panic(expected = "Atomics.wait is not available")]
fn test_park_panics_on_browser_main_thread() {
    if is_node() {
        // In Node.js, we need to manually panic to satisfy should_panic
        panic!("Atomics.wait is not available in this context (likely main thread)");
    }
    // In browser, this will actually panic
    park_timeout(Duration::from_millis(1));
}

#[wasm_bindgen(inline_js = r#"
export function schedule_delayed_throw() {
    setTimeout(() => {
        throw new Error('delayed error after closure returned');
    }, 0);
}
"#)]
extern "C" {
    fn schedule_delayed_throw();
}

// Test that join_async waits for worker exit and returns error if worker errors after sending value.
// This only works on Node.js - browser's close() cancels pending timers.
#[wasm_bindgen_test::wasm_bindgen_test]
async fn test_join_async_error_after_value_sent() {
    if !is_node() {
        // Browser cancels setTimeout when close() is called, so this test only works on Node
        return;
    }

    // Spawn a worker that:
    // 1. Schedules a setTimeout that throws an error
    // 2. Returns a value successfully
    // The setTimeout fires AFTER the closure returns (because Node keeps the event loop alive)
    let handle = spawn(|| {
        schedule_delayed_throw();
        42
    });

    // Yield to event loop to let the worker start
    yield_to_event_loop_async().await;

    // join_async should:
    // 1. Receive the value 42 from the channel
    // 2. Wait for the exit signal
    // 3. See exit_state = 2 (error) because the setTimeout threw
    // 4. Return an error
    let result = handle.join_async().await;
    assert!(
        result.is_err(),
        "Expected error from delayed throw after value sent, got {:?}",
        result
    );
}
