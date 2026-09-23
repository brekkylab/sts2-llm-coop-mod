//! Left pane: the session files that matter while playing, read and written
//! directly. The agent no longer reads files during combat (its input is the
//! prompt, dumped to `last_prompt.md`), so there's no point going through its
//! console, whose mount only holds `notes/` anyway.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use eframe::egui;

/// One fixed file in the pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Slot {
    /// Relative to the session root.
    pub path: &'static str,
    pub label: &'static str,
    pub editable: bool,
    pub hint: &'static str,
}

/// `memory.md` first: it's the only way the human gives standing instructions.
pub const SLOTS: [Slot; 4] = [
    Slot {
        path: "memory.md",
        label: "내 지시",
        editable: true,
        hint: "한 줄에 하나씩 `- ` 로 시작해 적는다. 그렇지 않은 줄은 실리지 않는다. \
               저장하면 다음 판단부터 실리고, 에이전트는 이 파일을 고치지 않는다.",
    },
    Slot {
        path: "notes/learned.md",
        label: "에이전트가 알게 된 것",
        editable: true,
        hint: "전투가 끝날 때마다 에이전트가 다시 쓴다. 틀린 줄을 지우고 저장하면 \
               다음 판단부터 빠진다. 내 지시와 부딪히면 내 지시가 이긴다.",
    },
    Slot {
        path: "last_prompt.md",
        label: "방금 보낸 프롬프트",
        editable: false,
        hint: "에이전트가 마지막 판단에서 받은 것 전부. 읽기 전용.",
    },
    Slot {
        path: "brief.md",
        label: "지금 판",
        editable: false,
        hint: "읽기 전용. 판이 바뀔 때마다 새로 쓰인다.",
    },
];

pub struct FilesView {
    root: PathBuf,
    selected: usize,
    /// Contents of the selected file, as being edited.
    body: String,
    dirty: bool,
    /// mtime of what `body` was loaded from, to notice outside changes.
    loaded_at: Option<SystemTime>,
    status: String,
}

impl FilesView {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let mut f = Self {
            root: root.into(),
            selected: 0,
            body: String::new(),
            dirty: false,
            loaded_at: None,
            status: String::new(),
        };
        f.load();
        f
    }

    fn path(&self) -> PathBuf {
        self.root.join(SLOTS[self.selected].path)
    }

    fn load(&mut self) {
        let p = self.path();
        // A missing file is an empty one; saving creates it.
        self.body = std::fs::read_to_string(&p).unwrap_or_default();
        self.loaded_at = mtime(&p);
        self.dirty = false;
    }

    fn save(&mut self) {
        let p = self.path();
        let r = p
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&p, &self.body));
        match r {
            Ok(()) => {
                self.loaded_at = mtime(&p);
                self.dirty = false;
                self.status = "저장했다".into();
            }
            Err(e) => self.status = format!("저장 실패: {e}"),
        }
    }

    /// Once per frame: pick up changes made by the bridge or the agent, unless
    /// there are unsaved edits.
    pub fn poll(&mut self) {
        if !self.dirty && mtime(&self.path()) != self.loaded_at {
            self.load();
        }
    }

    fn select(&mut self, i: usize) {
        if i != self.selected {
            self.selected = i;
            self.status.clear();
            self.load();
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        for (i, s) in SLOTS.iter().enumerate() {
            let label = format!("{}  ({})", s.label, s.path);
            if ui.selectable_label(i == self.selected, label).clicked() {
                // Switching away drops unsaved edits; don't let that happen silently.
                if self.dirty && i != self.selected {
                    self.status = "저장하지 않은 것이 있다".into();
                } else {
                    self.select(i);
                }
            }
        }

        ui.separator();
        let slot = SLOTS[self.selected];
        ui.weak(slot.hint);
        if slot.editable {
            ui.horizontal(|ui| {
                if ui.add_enabled(self.dirty, egui::Button::new("저장")).clicked() {
                    self.save();
                }
                if self.dirty && ui.button("되돌리기").clicked() {
                    self.load();
                }
                if !self.status.is_empty() {
                    ui.weak(&self.status);
                }
            });
        }

        egui::ScrollArea::vertical().id_salt("body").show(ui, |ui| {
            let before = self.body.clone();
            ui.add(
                egui::TextEdit::multiline(&mut self.body)
                    .code_editor()
                    .interactive(slot.editable)
                    .desired_width(f32::INFINITY),
            );
            if self.body != before {
                self.dirty = true;
            }
        });
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let tid = std::thread::current().id();
        let d = std::env::temp_dir().join(format!("sts2-files-{ns}-{tid:?}"));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The human's file must be reachable and editable; it's the only way to
    /// give the agent standing instructions.
    #[test]
    fn opens_on_the_humans_instructions() {
        let root = tmp();
        std::fs::write(root.join("memory.md"), "- 방어 먼저\n").unwrap();

        let f = FilesView::new(&root);
        assert_eq!(SLOTS[f.selected].path, "memory.md");
        assert!(SLOTS[f.selected].editable);
        assert_eq!(f.body, "- 방어 먼저\n");

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn saving_writes_the_file_even_when_missing() {
        let root = tmp();
        let mut f = FilesView::new(&root);
        f.select(1); // notes/learned.md, directory doesn't exist yet
        f.body = "- 동료는 방어를 아낀다\n".into();
        f.dirty = true;
        f.save();

        let on_disk = std::fs::read_to_string(root.join("notes/learned.md")).unwrap();
        assert_eq!(on_disk, "- 동료는 방어를 아낀다\n");
        assert!(!f.dirty);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn prompt_and_brief_are_read_only() {
        let read_only: Vec<&str> =
            SLOTS.iter().filter(|s| !s.editable).map(|s| s.path).collect();
        assert_eq!(read_only, ["last_prompt.md", "brief.md"]);
    }

    /// The summary rewrites learned.md; the pane shows it without a click, but
    /// never over unsaved edits.
    #[test]
    fn reloads_outside_changes_unless_dirty() {
        let root = tmp();
        let p = root.join("memory.md");
        std::fs::write(&p, "- a\n").unwrap();
        let mut f = FilesView::new(&root);

        // Make sure the mtime moves even on coarse filesystems.
        f.loaded_at = Some(SystemTime::UNIX_EPOCH);
        std::fs::write(&p, "- b\n").unwrap();
        f.poll();
        assert_eq!(f.body, "- b\n");

        f.body = "- mine\n".into();
        f.dirty = true;
        f.loaded_at = Some(SystemTime::UNIX_EPOCH);
        f.poll();
        assert_eq!(f.body, "- mine\n", "unsaved edits must survive");

        std::fs::remove_dir_all(&root).ok();
    }
}
