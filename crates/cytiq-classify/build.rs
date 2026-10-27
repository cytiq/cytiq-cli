//! On macOS, ONNX Runtime's prebuilt `CoreML` objects (`model.mm`,
//! `host_utils.mm`) call `__isPlatformVersionAtLeast`, the runtime half of
//! Objective-C's `@available`. Rust's std defines it, but LTO internalizes std's
//! copy before the native objects are linked, so a release build fails with an
//! undefined symbol. Apple's compiler-rt defines it too: link it (unbundled, so
//! the linker loads only that member) for the native objects to resolve against.

use std::env;
use std::path::PathBuf;
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    match compiler_rt_dir() {
        Ok(directory) => {
            println!("cargo:rustc-link-search=native={}", directory.display());
            println!("cargo:rustc-link-lib=static:-bundle=clang_rt.osx");
        }
        // Builds without LTO still link through std's definition.
        Err(reason) => println!(
            "cargo:warning=compiler-rt not linked ({reason}); release builds with LTO will miss `__isPlatformVersionAtLeast`"
        ),
    }
}

/// `xcrun clang --print-runtime-dir`, when it holds `libclang_rt.osx.a`.
fn compiler_rt_dir() -> Result<PathBuf, String> {
    let output = Command::new("xcrun")
        .args(["clang", "--print-runtime-dir"])
        .output()
        .map_err(|error| format!("`xcrun clang --print-runtime-dir` did not run: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "`xcrun clang --print-runtime-dir` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let directory = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
    if directory.join("libclang_rt.osx.a").is_file() {
        Ok(directory)
    } else {
        Err(format!("no libclang_rt.osx.a in {}", directory.display()))
    }
}
