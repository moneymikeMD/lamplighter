// SPDX-License-Identifier: Apache-2.0

//! Result table and JSON; neither carries a host name or any server address.

use std::collections::BTreeMap;
use std::fmt::Write;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::metrics::{KS, Score, mean};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) kind: String,
    pub(crate) score: Score,
    #[serde(default)]
    pub(crate) memories: usize,
    #[serde(default)]
    pub(crate) mode: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) arm: String,
    pub(crate) dataset: String,
    pub(crate) subset: String,
    pub(crate) seed: u64,
    pub(crate) haystack_sessions: Option<usize>,
    pub(crate) questions: usize,
    pub(crate) memories: usize,
    pub(crate) recall_modes: Vec<String>,
    pub(crate) embedder: Value,
    pub(crate) rows: Vec<Row>,
}

fn line(label: &str, s: &Score, n: usize) -> String {
    let mut out = format!("{label:<26}{n:>4}");
    for k in KS {
        let _ = write!(out, "  {:>6.3}", s.recall_any[&k]);
    }
    for k in KS {
        let _ = write!(out, "  {:>6.3}", s.recall_all[&k]);
    }
    let _ = writeln!(out, "  {:>6.3}", s.mrr);
    out
}

impl Report {
    pub(crate) fn table(&self) -> String {
        let mut out = format!(
            "{} on {} (subset {}, seed {}, haystack {}): {} questions, {} memories, recall mode {}\n\n",
            self.arm,
            self.dataset,
            self.subset,
            self.seed,
            self.haystack_sessions
                .map_or_else(|| "all sessions".to_owned(), |n| format!("{n} sessions")),
            self.questions,
            self.memories,
            self.recall_modes.join("+"),
        );
        let _ = write!(out, "{:<26}{:>4}", "", "n");
        for k in KS {
            let _ = write!(out, "  any@{k:<3}");
        }
        for k in KS {
            let _ = write!(out, "  all@{k:<3}");
        }
        out.push_str("     MRR\n");
        let all: Vec<&Score> = self.rows.iter().map(|r| &r.score).collect();
        out.push_str(&line("overall", &mean(&all), all.len()));
        let mut by_kind: BTreeMap<&str, Vec<&Score>> = BTreeMap::new();
        for r in &self.rows {
            by_kind.entry(&r.kind).or_default().push(&r.score);
        }
        for (kind, scores) in by_kind {
            out.push_str(&line(kind, &mean(&scores), scores.len()));
        }
        let _ = writeln!(
            out,
            "\nrecall@5 {:.3}  MRR {:.3}",
            mean(&all).recall_any[&5],
            mean(&all).mrr
        );
        out
    }

    pub(crate) fn json(&self) -> String {
        let mut v = serde_json::to_string_pretty(self).unwrap_or_default();
        v.push('\n');
        v
    }
}
