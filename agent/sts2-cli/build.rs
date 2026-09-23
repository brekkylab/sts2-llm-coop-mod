//! Same as sts2-bridge/build.rs, and must stay so (link args don't propagate).
//! Without it the build passes but running fails with
//! `dyld: Library not loaded: @rpath/libfuse-t.dylib ... no LC_RPATH's found`.

fn main() {
    println!("cargo::rerun-if-changed=build.rs");

    if !cfg!(target_os = "macos") {
        return;
    }

    let Ok(fuse_t) = pkg_config::Config::new().probe("fuse-t") else {
        // Not fatal: sts2-play and sts2-say don't use the console.
        println!("cargo::warning=FUSE-T not found; sts2-replay will not link");
        return;
    };

    for path in &fuse_t.link_paths {
        println!("cargo::rustc-link-arg=-Wl,-rpath,{}", path.display());
    }
}
