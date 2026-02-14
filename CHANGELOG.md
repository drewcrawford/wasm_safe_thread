# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- **Integrated synchronization primitives** - merged `wasm_safe_mutex` functionality directly into this crate, adding `Mutex`, `RwLock`, `Condvar`, `Spinlock`, and `mpsc` modules to `wasm_safe_thread`.

### Changed

- **WASM spawn routing** - browser worker spawns now relay to the root parent context when available, avoiding nested worker startup stalls under blocking schedules.
- **Crate root docs** - updated `src/lib.rs` documentation to describe both thread APIs and the new built-in synchronization/channel APIs.
- **Dependency graph** - removed direct dependency on `wasm_safe_mutex`; `wasm_safe_thread` now owns these implementations internally, eliminating the local mutual-dependency risk.

### Removed

- **Experimental APIs** - removed `spawn_strict` and public broker init/state APIs introduced during investigation.

### Tests

- Added a wasm regression test for the `Poll::Pending -> spawn() -> wake` pattern (`test_spawn_from_poll_pending_wakes_future`) to guard against browser hangs.

## [0.1.0] - 2026-02-03

Initial release of `wasm_safe_thread`, a `std::thread` replacement for wasm32 with proper async integration.

### Features

- **Thread spawning** - `spawn()`, `spawn_named()`, and `Builder` pattern for creating threads that work identically on native and wasm32 targets
- **Joining threads** - `JoinHandle::join()` for synchronous waiting, plus `join_async()` for async contexts (essential for wasm main thread)
- **Thread operations** - `current()`, `sleep()`, `yield_now()`, `park()`, `park_timeout()`, and `Thread::unpark()` primitives
- **Thread local storage** - `thread_local!` macro and `LocalKey` for per-thread data
- **Spawn hooks** - `register_spawn_hook()`, `remove_spawn_hook()`, and `clear_spawn_hooks()` for running callbacks when threads start
- **Parallelism detection** - `available_parallelism()` to query hardware concurrency
- **Event loop integration** - `yield_to_event_loop_async()` for cooperative scheduling with the browser event loop
- **Async task tracking** - `task_begin()` and `task_finished()` to ensure workers wait for async tasks to complete

### Platform Support

- Native platforms via `std::thread` backend
- Browser environments via Web Workers with shared memory
- Node.js via `worker_threads` module

### Requirements

- Rust 1.85.1+ (2024 edition)
- For wasm32: SharedArrayBuffer support (requires COOP/COEP headers)
