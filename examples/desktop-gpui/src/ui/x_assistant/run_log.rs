use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

/// Incremental, read-only tail for X Assistant log files.
/// The viewer owns no writer handles and never creates log files.
pub(super) struct LogTail {
    sources: Vec<Source>,
    lines: Vec<String>,
}

struct Source {
    path: PathBuf,
    offset: u64,
    pending: String,
}

impl LogTail {
    pub(super) fn new() -> Self {
        let directory = app_log::log_dir().unwrap_or_else(|_| PathBuf::from("logs"));
        let mut viewer = Self {
            sources: ["x-assistant.log", "auto-post.log"]
                .into_iter()
                .map(|name| Source {
                    path: directory.join(name),
                    offset: 0,
                    pending: String::new(),
                })
                .collect(),
            lines: Vec::new(),
        };
        viewer.load_initial_tail();
        viewer
    }

    pub(super) fn lines(&self) -> &[String] {
        &self.lines
    }

    pub(super) fn poll(&mut self) -> bool {
        let mut changed = false;
        for source in &mut self.sources {
            let Ok(mut file) = File::open(&source.path) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            if metadata.len() < source.offset {
                source.offset = 0;
                source.pending.clear();
            }
            if file.seek(SeekFrom::Start(source.offset)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_err() || bytes.is_empty() {
                continue;
            }
            source.offset = source.offset.saturating_add(bytes.len() as u64);
            let text = String::from_utf8_lossy(&bytes);
            source.pending.push_str(&text);
            while let Some(index) = source.pending.find('\n') {
                let mut line = source.pending.drain(..=index).collect::<String>();
                while line.ends_with('\r') || line.ends_with('\n') {
                    line.pop();
                }
                if !line.is_empty() {
                    self.lines.push(line);
                    changed = true;
                }
            }
        }
        if changed {\n            self.lines.sort();\n        }\n        if changed && self.lines.len() > 500 {
            let excess = self.lines.len() - 500;
            self.lines.drain(..excess);
        }
        changed
    }

    fn load_initial_tail(&mut self) {
        for source in &mut self.sources {
            let Ok(mut file) = File::open(&source.path) else {
                continue;
            };
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            let start = metadata.len().saturating_sub(256 * 1024);
            if file.seek(SeekFrom::Start(start)).is_err() {
                continue;
            }
            let mut bytes = Vec::new();
            if file.read_to_end(&mut bytes).is_err() {
                continue;
            }
            source.offset = metadata.len();
            let text = String::from_utf8_lossy(&bytes);
            let mut lines = text.lines().map(str::to_owned).collect::<Vec<_>>();
            if start > 0 && !lines.is_empty() {
                lines.remove(0);
            }
            self.lines.extend(lines);
        }
        self.lines.sort();\n        if self.lines.len() > 200 {
            let excess = self.lines.len() - 200;
            self.lines.drain(..excess);
        }
    }
}
