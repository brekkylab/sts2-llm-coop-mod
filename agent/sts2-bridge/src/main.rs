mod bridge;
mod decision;
mod gui;

use anyhow::Result;
use sts2_bridge::event::Events;

fn main() -> Result<()> {
    // ailoy doesn't read .env at runtime; load API keys here (searching upward).
    dotenvy::dotenv().ok();

    let rt = tokio::runtime::Runtime::new()?;
    let events = Events::new();
    let handle = rt.spawn(bridge::run(events.clone()));

    if std::env::args().any(|a| a == "--no-gui") {
        return rt.block_on(handle)?;
    }

    // macOS needs the GUI on the main thread; the runtime runs beside it.
    let opts = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default().with_inner_size([1100.0, 700.0]),
        ..Default::default()
    };
    eframe::run_native(
        "cortex",
        opts,
        Box::new(move |cc| Ok(Box::new(gui::Ui::new(cc, events)))),
    )
    .map_err(|e| anyhow::anyhow!("창을 띄우지 못했다: {e}"))?;
    Ok(())
}
