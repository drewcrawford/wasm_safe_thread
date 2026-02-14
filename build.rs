// SPDX-License-Identifier: MIT OR Apache-2.0

use std::process::Command;

fn main() {
    // Declare our custom cfg name so rustc's cfg checking accepts #[cfg(nightly_rustc)].
    println!("cargo:rustc-check-cfg=cfg(nightly_rustc)");

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let is_nightly = Command::new(rustc)
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|v| v.contains("nightly"));

    if is_nightly {
        println!("cargo:rustc-cfg=nightly_rustc");
    }
}
