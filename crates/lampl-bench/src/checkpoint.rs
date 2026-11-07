// SPDX-License-Identifier: Apache-2.0

//! Append-only JSONL checkpoint: a header line fingerprinting the run, then one scored row per finished question.

use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::Path;

use serde_json::Value;

use crate::report::Row;

pub(crate) struct Checkpoint {
    file: File,
    pub(crate) done: Vec<Row>,
}

impl Checkpoint {
    /// Opens `path`, creating it with `fingerprint` as the header, or loads its finished rows after checking the header matches. A torn final line from a kill is dropped.
    pub(crate) fn open(path: &Path, fingerprint: &Value) -> Result<Self, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("reading {}: {e}", path.display())),
        };
        let mut done = Vec::new();
        let mut seen = HashSet::new();
        let mut lines = text.lines().filter(|l| !l.trim().is_empty());
        let fresh = match lines.next() {
            None => true,
            Some(head) => {
                let head: Value = serde_json::from_str(head)
                    .map_err(|e| format!("{}: unreadable header: {e}", path.display()))?;
                if &head != fingerprint {
                    return Err(format!(
                        "{} was written by a different run (header {head}, this run {fingerprint}); use another file or delete it",
                        path.display()
                    ));
                }
                for l in lines {
                    if let Ok(row) = serde_json::from_str::<Row>(l)
                        && seen.insert(row.id.clone())
                    {
                        done.push(row);
                    }
                }
                false
            }
        };
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| format!("opening {}: {e}", path.display()))?;
        if fresh {
            writeln!(file, "{fingerprint}").map_err(|e| e.to_string())?;
        } else if !text.ends_with('\n') {
            writeln!(file).map_err(|e| e.to_string())?;
        }
        Ok(Self { file, done })
    }

    pub(crate) fn append(&mut self, row: Row) -> Result<(), String> {
        let line = serde_json::to_string(&row).map_err(|e| e.to_string())?;
        writeln!(self.file, "{line}").map_err(|e| e.to_string())?;
        self.file.sync_data().map_err(|e| e.to_string())?;
        self.done.push(row);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::score;
    use serde_json::json;

    fn row(id: &str) -> Row {
        Row {
            id: id.to_owned(),
            kind: "k".to_owned(),
            score: score(&["a".to_owned()], &["a".to_owned()]),
            memories: 3,
            mode: "hybrid".to_owned(),
        }
    }

    #[test]
    fn resumes_finished_rows_and_drops_a_torn_line() {
        let dir = std::env::temp_dir().join(format!("lampl-ckpt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("c.jsonl");
        let _ = std::fs::remove_file(&path);
        let fp = json!({"seed": 1});
        let mut c = Checkpoint::open(&path, &fp).unwrap();
        c.append(row("q1")).unwrap();
        c.append(row("q2")).unwrap();
        drop(c);
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        write!(f, "{{\"id\":\"q3\",\"ki").unwrap();
        drop(f);

        let mut c = Checkpoint::open(&path, &fp).unwrap();
        assert_eq!(c.done.len(), 2);
        assert!(c.done.iter().all(|r| r.id != "q3"));
        c.append(row("q3")).unwrap();
        drop(c);
        assert_eq!(Checkpoint::open(&path, &fp).unwrap().done.len(), 3);
        assert!(Checkpoint::open(&path, &json!({"seed": 2})).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
