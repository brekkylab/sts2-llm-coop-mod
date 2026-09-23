use std::path::Path;

use anyhow::{Context as _, Result};
use cortex::console::Console;
use cortex::fs::{FuseTMount, PassthroughFs, WorkFs};

/// A cortex console whose working directory is a workfs with only the agent's own
/// notes mounted (as `notes`). What it reads is already in the prompt; the human's
/// files are kept out of the way, so the model can't edit them by misreading a rule.
///
/// This is NOT isolation. The local console resolves paths with `root.join(path)`
/// ("A join, not containment", cortex-console-servers/local/src/server/mod.rs), so an
/// absolute path or enough `..` escapes, and the `shell` tool from `system_tools()`
/// runs `sh -c` on the host. Don't build on this as a security boundary; that needs
/// a confining backend. More mounts can be added with another `try_with_mount`.
pub async fn open(session_root: &Path, mount_point: &Path) -> Result<Console> {
    // Must exist and be empty.
    std::fs::create_dir_all(mount_point)
        .with_context(|| format!("creating mount point {}", mount_point.display()))?;

    let workfs = WorkFs::new()
        .try_with_mount("notes", PassthroughFs::new(session_root))
        .context("mounting the notes directory as `notes`")?;
    let mount = FuseTMount::try_new(workfs, mount_point)
        .with_context(|| format!("mounting at {}", mount_point.display()))?;

    // PATH goes to the server process directly: `set_var` is unsound once tokio's
    // worker threads run, and the local console has no other way to take an env.
    let cli_dir = std::env::current_exe()?
        .parent()
        .context("current_exe has no parent")?
        .to_path_buf();

    // The console server lives in a sibling cortex checkout, not on PATH. Four levels
    // up from target/release is the workspace root; override with STS2_CORTEX_BIN.
    let console_dir = match std::env::var("STS2_CORTEX_BIN") {
        Ok(dir) => std::path::PathBuf::from(dir),
        Err(_) => {
            let guess = cli_dir.join("../../../../cortex/target/release");
            std::fs::canonicalize(&guess).unwrap_or(guess)
        }
    };

    if !console_dir.join("cortex-local-console").exists() {
        anyhow::bail!(
            "cortex-local-console 을 {} 에서 찾지 못했다. \
             cortex 를 `cargo build --release` 로 빌드했는지 보고, \
             다른 곳에 있으면 STS2_CORTEX_BIN 으로 가리켜라",
            console_dir.display()
        );
    }

    let path = std::env::var("PATH").unwrap_or_default();
    let path_arg = format!("PATH={}:{}:{path}", cli_dir.display(), console_dir.display());

    Console::builder()
        .stdio_client(&["env", &path_arg, "cortex-local-console"])
        .mount(mount)
        .build()
        .await
        .context("starting the console server — is cortex-local-console on PATH?")
}
