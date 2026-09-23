//! Adds an rpath for `libfuse-t.dylib`. cortex's build.rs does the same, but
//! `cargo::rustc-link-arg` doesn't propagate to dependents. An env var won't do:
//! macOS strips `DYLD_*` when exec'ing system binaries such as `/bin/sh`.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    if !cfg!(target_os = "macos") {
        return;
    }

    let Ok(fuse_t) = pkg_config::Config::new().probe("fuse-t") else {
        // Not fatal; only the console path is affected.
        println!("cargo::warning=FUSE-T not found; the console path will not link");
        return;
    };

    for path in &fuse_t.link_paths {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{}", path.display());
    }
}
