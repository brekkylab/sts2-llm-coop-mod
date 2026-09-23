//! The window: files on the left, the agent's activity on the right.

mod files;
mod stream;

use eframe::egui;
use sts2_bridge::event::{AgentEvent, Events};
use tokio::sync::broadcast;

const KOREAN_FONT: &str = "/System/Library/Fonts/AppleSDGothicNeo.ttc";

/// egui's default fonts have no Hangul, which would render as boxes. Adds a macOS
/// system font; if it can't be read, warn and continue.
fn install_korean_font(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(KOREAN_FONT) else {
        eprintln!("한글 폰트를 못 읽었다({KOREAN_FONT}). 글자가 □ 로 보인다");
        return;
    };

    let mut fonts = egui::FontDefinitions::default();
    // egui 0.29 takes FontData directly (Arc from 0.30).
    fonts.font_data.insert(
        "korean".to_owned(),
        egui::FontData::from_owned(bytes).tweak(egui::FontTweak {
            scale: 1.0,
            ..Default::default()
        }),
    );

    // As a fallback, so Latin text keeps the default font.
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push("korean".to_owned());
    }
    ctx.set_fonts(fonts);
}

pub struct Ui {
    events: broadcast::Receiver<AgentEvent>,
    stream: stream::StreamView,
    files: files::FilesView,
    session_root: std::path::PathBuf,
}

impl Ui {
    pub fn new(cc: &eframe::CreationContext<'_>, events: Events) -> Self {
        install_korean_font(&cc.egui_ctx);
        // Must match the bridge's default.
        let session_root: std::path::PathBuf = std::env::var("STS2_SESSION")
            .unwrap_or_else(|_| "/tmp/sts2-session".into())
            .into();
        Self {
            events: events.subscribe(),
            stream: stream::StreamView::default(),
            files: files::FilesView::new(&session_root),
            session_root,
        }
    }
}

impl eframe::App for Ui {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(e) = self.events.try_recv() {
            // The window owns the main thread; the process ends only when it closes.
            if matches!(e, AgentEvent::Shutdown) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }

            self.stream.observe(e);
        }

        self.files.poll();
        egui::SidePanel::left("files")
            .default_width(340.0)
            .show(ctx, |ui| self.files.show(ui));

        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(text) = self.stream.show(ui) {
                if let Err(e) = sts2_core::said::say(&self.session_root, "human", &text) {
                    eprintln!("say: {e:#}");
                }
            }
        });

        // Repaint often while thinking so the timer runs.
        ctx.request_repaint_after(std::time::Duration::from_millis(
            if self.stream.thinking { 50 } else { 200 },
        ));
    }
}
