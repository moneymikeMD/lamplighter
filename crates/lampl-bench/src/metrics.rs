// SPDX-License-Identifier: Apache-2.0

//! Retrieval metrics over a ranked list of session ids.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

pub(crate) const KS: [usize; 4] = [1, 3, 5, 10];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct Score {
    pub(crate) recall_any: BTreeMap<usize, f64>,
    pub(crate) recall_all: BTreeMap<usize, f64>,
    pub(crate) mrr: f64,
}

/// Scores one question: `any` is 1 when at least one evidence session is in the top k, `all` when every one is.
pub(crate) fn score(ranked: &[String], evidence: &[String]) -> Score {
    let mut recall_any = BTreeMap::new();
    let mut recall_all = BTreeMap::new();
    for k in KS {
        let top = &ranked[..ranked.len().min(k)];
        let found = evidence.iter().filter(|e| top.contains(e)).count();
        recall_any.insert(k, f64::from(u8::from(found > 0)));
        recall_all.insert(
            k,
            f64::from(u8::from(!evidence.is_empty() && found == evidence.len())),
        );
    }
    let mrr = ranked
        .iter()
        .position(|s| evidence.contains(s))
        .map_or(0.0, |i| 1.0 / (i as f64 + 1.0));
    Score {
        recall_any,
        recall_all,
        mrr,
    }
}

/// Mean of per-question scores; an empty slice gives zeros.
pub(crate) fn mean(scores: &[&Score]) -> Score {
    let n = scores.len().max(1) as f64;
    let avg = |pick: &dyn Fn(&Score) -> BTreeMap<usize, f64>| {
        KS.iter()
            .map(|k| {
                let sum: f64 = scores.iter().map(|s| pick(s)[k]).sum();
                (*k, sum / n)
            })
            .collect()
    };
    Score {
        recall_any: avg(&|s| s.recall_any.clone()),
        recall_all: avg(&|s| s.recall_all.clone()),
        mrr: scores.iter().map(|s| s.mrr).sum::<f64>() / n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn single_evidence_session() {
        let s = score(&ids(&["a", "b", "c", "d"]), &ids(&["c"]));
        assert_eq!(s.recall_any[&1], 0.0);
        assert_eq!(s.recall_any[&3], 1.0);
        assert!((s.mrr - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn all_needs_every_evidence_session() {
        let s = score(&ids(&["a", "b", "c"]), &ids(&["a", "c"]));
        assert_eq!(s.recall_any[&1], 1.0);
        assert_eq!(s.recall_all[&1], 0.0);
        assert_eq!(s.recall_all[&3], 1.0);
    }

    #[test]
    fn miss_scores_zero_and_mean_averages() {
        let miss = score(&ids(&["a"]), &ids(&["z"]));
        assert_eq!(miss.mrr, 0.0);
        let hit = score(&ids(&["z"]), &ids(&["z"]));
        let m = mean(&[&miss, &hit]);
        assert_eq!(m.recall_any[&5], 0.5);
        assert_eq!(m.mrr, 0.5);
        assert_eq!(mean(&[]).mrr, 0.0);
    }
}
