//! The journal of two-way sync: one line per write it sent (or tried to), so you can see
//! afterwards what went to ILIAS. Kept next to the session as `<ilias host>.journal.jsonl`,
//! one JSON object per line: `{"run", "time", "action", "file", "ilias", "result"}`; `run`
//! is when that sync's writes started, the same for all its lines.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use color_eyre::Result;
use serde::Serialize;
use url::Url;

pub struct Journal {
    file: Option<PathBuf>,
    run: String,
}

#[derive(Serialize)]
struct Line<'a> {
    run: &'a str,
    time: String,
    action: &'a str,
    file: &'a Path,
    ilias: &'a str,
    result: &'a str,
}

impl Journal {
    /// A new run, written to this ILIAS' journal.
    pub fn open(base: &Url) -> Self {
        let file = crate::session::path(base)
            .ok()
            .map(|p| p.with_extension("journal.jsonl"));
        Self::new(file)
    }

    fn new(file: Option<PathBuf>) -> Self {
        Self { file, run: now() }
    }

    #[cfg(test)]
    pub fn in_file(file: PathBuf) -> Self {
        Self::new(Some(file))
    }

    /// Appends one write: `action` (e.g. `upload`) of `file` to the ILIAS object `ilias`,
    /// and how it went.
    pub fn write(&mut self, action: &str, file: &Path, ilias: &str, result: &str) -> Result<()> {
        let Some(path) = &self.file else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            crate::session::private_dir(dir)?;
        }
        let line = Line {
            run: &self.run,
            time: now(),
            action,
            file,
            ilias,
            result,
        };
        let mut out = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(out, "{}", serde_json::to_string(&line)?)?;
        Ok(())
    }
}

fn now() -> String {
    jiff::Zoned::now()
        .strftime("%Y-%m-%dT%H:%M:%S%:z")
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_one_line_per_write_with_its_run() {
        let dir = std::env::temp_dir().join(format!("ilias-tui-journal-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("ilias.example.journal.jsonl");
        let mut journal = Journal::in_file(file.clone());
        journal
            .write("upload", Path::new("/dl/a.pdf"), "crs 1 DEMO", "ok: file 9")
            .unwrap();
        journal
            .write("upload", Path::new("/dl/b.pdf"), "crs 1 DEMO", "failed: no")
            .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<serde_json::Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0]["run"], lines[1]["run"]);
        assert_eq!(lines[0]["action"], "upload");
        assert_eq!(lines[0]["file"], "/dl/a.pdf");
        assert_eq!(lines[1]["result"], "failed: no");
        let _ = std::fs::remove_dir_all(dir);
    }
}
