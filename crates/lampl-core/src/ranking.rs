// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::hash::Hash;

use rust_stemmers::{Algorithm, Stemmer};

pub const RRF_K: f64 = 60.0;

/// Reciprocal-rank fusion over ranked id lists (best first); returns ids by descending fused score, ties broken by first appearance.
pub fn rrf_fuse<T: Clone + Eq + Hash>(rankings: &[Vec<T>], k: f64) -> Vec<(T, f64)> {
    let mut scores: HashMap<T, (f64, usize)> = HashMap::new();
    let mut seen = 0usize;
    for ranking in rankings {
        for (idx, id) in ranking.iter().enumerate() {
            let entry = scores.entry(id.clone()).or_insert_with(|| {
                seen += 1;
                (0.0, seen)
            });
            entry.0 += 1.0 / (k + (idx + 1) as f64);
        }
    }
    let mut fused: Vec<(T, (f64, usize))> = scores.into_iter().collect();
    fused.sort_by(|a, b| b.1.0.total_cmp(&a.1.0).then(a.1.1.cmp(&b.1.1)));
    fused.into_iter().map(|(id, (s, _))| (id, s)).collect()
}

/// Final ordering key for candidates that cleared the floor: cosine plus IDF coverage (reciprocal-rank fusion only picks the candidates).
pub fn relevance(cosine: Option<f64>, coverage: f64) -> f64 {
    cosine.unwrap_or(0.0) + coverage
}

/// Task instruction Qwen3-Embedding expects in front of a query; stored memories are embedded without one.
const QUERY_INSTRUCTION: &str = "Instruct: Given a description of a problem, retrieve stored memories about the same problem\nQuery:";

/// The text a recall query is embedded from: the task instruction plus the lower-cased query, so results do not depend on query case.
pub fn query_embed_text(query: &str) -> String {
    format!("{QUERY_INSTRUCTION}{}", query.to_lowercase())
}

/// Postgres's `english` text-search stop words, so a term that can never match is not counted.
const STOP_WORDS: &[&str] = &[
    "i",
    "me",
    "my",
    "myself",
    "we",
    "our",
    "ours",
    "ourselves",
    "you",
    "your",
    "yours",
    "yourself",
    "yourselves",
    "he",
    "him",
    "his",
    "himself",
    "she",
    "her",
    "hers",
    "herself",
    "it",
    "its",
    "itself",
    "they",
    "them",
    "their",
    "theirs",
    "themselves",
    "what",
    "which",
    "who",
    "whom",
    "this",
    "that",
    "these",
    "those",
    "am",
    "is",
    "are",
    "was",
    "were",
    "be",
    "been",
    "being",
    "have",
    "has",
    "had",
    "having",
    "do",
    "does",
    "did",
    "doing",
    "a",
    "an",
    "the",
    "and",
    "but",
    "if",
    "or",
    "because",
    "as",
    "until",
    "while",
    "of",
    "at",
    "by",
    "for",
    "with",
    "about",
    "against",
    "between",
    "into",
    "through",
    "during",
    "before",
    "after",
    "above",
    "below",
    "to",
    "from",
    "up",
    "down",
    "in",
    "out",
    "on",
    "off",
    "over",
    "under",
    "again",
    "further",
    "then",
    "once",
    "here",
    "there",
    "when",
    "where",
    "why",
    "how",
    "all",
    "any",
    "both",
    "each",
    "few",
    "more",
    "most",
    "other",
    "some",
    "such",
    "no",
    "nor",
    "not",
    "only",
    "own",
    "same",
    "so",
    "than",
    "too",
    "very",
    "s",
    "t",
    "can",
    "will",
    "just",
    "don",
    "should",
    "now",
];

/// The lexeme Postgres's `english` configuration gives a lower-cased token: dotted tokens such as hosts and numbers stay whole, other words are Snowball-stemmed.
pub fn term_stem(term: &str) -> String {
    if term.contains('.') || term.chars().all(|c| c.is_ascii_digit()) {
        return term.to_owned();
    }
    Stemmer::create(Algorithm::English).stem(term).into_owned()
}

/// Queries both stores' term logic is checked against: Postgres's own lexemes must equal `term_stem` of `query_terms` for each.
#[doc(hidden)]
pub const PARITY_QUERIES: &[&str] = &[
    "ssh: connect to host github.com port 22: Operation timed out",
    "Failed connections to the docker daemon were retried",
    "Running migrations on postgres 17.2, ports and sockets",
    "the and of to",
    "Errors: ERROR_CODE 404 not found, errors again",
    "Why does the Kubernetes scheduler keep restarting pods?",
];

/// Lower-cased query terms in order, mirroring Postgres `to_tsvector('english', ..)`: stop words dropped and terms with the same stem collapsed. A `.` between alphanumerics stays inside the term, as in a host name.
pub fn query_terms(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.to_lowercase().chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    for (i, &c) in chars.iter().enumerate() {
        let inner_dot =
            c == '.' && !cur.is_empty() && chars.get(i + 1).is_some_and(|n| n.is_alphanumeric());
        if c.is_alphanumeric() || inner_dot {
            cur.push(c);
        } else if !cur.is_empty() {
            tokens.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        tokens.push(cur);
    }
    let mut seen = HashSet::new();
    tokens
        .into_iter()
        .filter(|t| !STOP_WORDS.contains(&t.as_str()) && seen.insert(term_stem(t)))
        .collect()
}

/// BM25-style inverse document frequency; never negative.
pub fn idf(total_docs: u64, doc_freq: u64) -> f64 {
    let n = total_docs as f64;
    let df = doc_freq as f64;
    (((n - df + 0.5) / (df + 0.5)) + 1.0).ln()
}

/// Fraction of the query's total IDF weight carried by the terms the document matched.
pub fn fts_coverage(term_idf: &[(String, f64)], matched: &HashSet<String>) -> f64 {
    let total: f64 = term_idf.iter().map(|(_, w)| w).sum();
    if total <= 0.0 {
        return 0.0;
    }
    let hit: f64 = term_idf
        .iter()
        .filter(|(t, _)| matched.contains(t))
        .map(|(_, w)| w)
        .sum();
    hit / total
}

/// How the floor's coverage is computed. `Plain` is the shipped default; the others exist for benchmarking only (LAMP-22, docs/bench/coverage-normalisation.md).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CoverageNorm {
    #[default]
    Plain,
    /// Coverage over the query's `k` highest-IDF terms only.
    Salient(usize),
    /// All matched IDF weight over the weight of the query's `k` highest-IDF terms, capped at 1.
    Capped(usize),
}

impl std::str::FromStr for CoverageNorm {
    type Err = String;

    /// Parses `plain`, `salient:<k>` or `capped:<k>` with `k >= 1`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        if s.eq_ignore_ascii_case("plain") {
            return Ok(CoverageNorm::Plain);
        }
        let (kind, k) = s
            .split_once(':')
            .ok_or_else(|| format!("expected plain, salient:<k> or capped:<k>, got `{s}`"))?;
        let k: usize = k
            .trim()
            .parse()
            .ok()
            .filter(|k| *k >= 1)
            .ok_or_else(|| format!("`{s}`: k must be a positive integer"))?;
        match kind.trim().to_ascii_lowercase().as_str() {
            "salient" => Ok(CoverageNorm::Salient(k)),
            "capped" => Ok(CoverageNorm::Capped(k)),
            _ => Err(format!("unknown coverage variant `{s}`")),
        }
    }
}

/// The coverage the relevance floor tests under `norm`; `Plain` equals `fts_coverage`. Term weight ties are broken by term so the top `k` is deterministic.
pub fn floor_coverage(
    term_idf: &[(String, f64)],
    matched: &HashSet<String>,
    norm: CoverageNorm,
) -> f64 {
    let k = match norm {
        CoverageNorm::Plain => return fts_coverage(term_idf, matched),
        CoverageNorm::Salient(k) | CoverageNorm::Capped(k) => k,
    };
    let mut ranked: Vec<&(String, f64)> = term_idf.iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let top: Vec<(String, f64)> = ranked.into_iter().take(k).cloned().collect();
    match norm {
        CoverageNorm::Salient(_) => fts_coverage(&top, matched),
        _ => {
            let denom: f64 = top.iter().map(|(_, w)| w).sum();
            if denom <= 0.0 {
                return 0.0;
            }
            let hit: f64 = term_idf
                .iter()
                .filter(|(t, _)| matched.contains(t))
                .map(|(_, w)| w)
                .sum();
            (hit / denom).min(1.0)
        }
    }
}

/// A candidate survives when its cosine similarity or its IDF full-text coverage clears the threshold.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Floor {
    pub cosine: f64,
    pub coverage: f64,
}

impl Default for Floor {
    /// Calibrated on Qwen3-Embedding-0.6B vectors; method and numbers in docs/ranking.md.
    fn default() -> Self {
        Floor {
            cosine: 0.52,
            coverage: 0.35,
        }
    }
}

pub fn passes_floor(floor: Floor, cosine: Option<f64>, coverage: f64) -> bool {
    cosine.is_some_and(|c| c >= floor.cosine) || coverage >= floor.coverage
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosts_stay_whole_and_plurals_collapse() {
        assert_eq!(
            query_terms("connect to github.com on port 22: ports connected."),
            ["connect", "github.com", "port", "22"]
        );
    }

    #[test]
    fn query_embed_text_ignores_case() {
        assert_eq!(
            super::query_embed_text("Fix THE Build"),
            super::query_embed_text("fix the build")
        );
        assert!(super::query_embed_text("Fix").ends_with("Query:fix"));
    }

    use super::*;

    #[test]
    fn rrf_orders_by_combined_rank() {
        let fts = vec!["a", "b", "c"];
        let vec = vec!["c", "b", "d"];
        let fused = rrf_fuse(&[fts, vec], RRF_K);
        let ids: Vec<_> = fused.iter().map(|(i, _)| *i).collect();
        assert_eq!(ids, ["c", "b", "a", "d"]);
        let expected_c = 1.0 / 63.0 + 1.0 / 61.0;
        assert!((fused[0].1 - expected_c).abs() < 1e-12);
    }

    #[test]
    fn rrf_of_nothing_is_empty() {
        assert!(rrf_fuse::<u8>(&[], RRF_K).is_empty());
    }

    #[test]
    fn terms_are_lowercased_deduplicated_and_stop_words_dropped() {
        assert_eq!(
            query_terms("The Postgres, postgres a DB!"),
            ["postgres", "db"]
        );
    }

    #[test]
    fn rare_terms_weigh_more_and_coverage_is_weighted() {
        assert!(idf(100, 1) > idf(100, 90));
        let w = vec![("rare".to_owned(), 4.0), ("common".to_owned(), 1.0)];
        let only_rare: HashSet<String> = ["rare".to_owned()].into();
        assert!((fts_coverage(&w, &only_rare) - 0.8).abs() < 1e-12);
        assert_eq!(fts_coverage(&[], &only_rare), 0.0);
    }

    #[test]
    fn coverage_variants_use_the_top_k_terms() {
        let w = vec![
            ("rare".to_owned(), 4.0),
            ("mid".to_owned(), 2.0),
            ("common".to_owned(), 1.0),
            ("tied".to_owned(), 1.0),
        ];
        let rare_common: HashSet<String> = ["rare", "common"].map(String::from).into();
        let plain = floor_coverage(&w, &rare_common, CoverageNorm::Plain);
        assert!((plain - fts_coverage(&w, &rare_common)).abs() < 1e-12);
        assert!(
            (floor_coverage(&w, &rare_common, CoverageNorm::Salient(2)) - 4.0 / 6.0).abs() < 1e-12
        );
        assert!(
            (floor_coverage(&w, &rare_common, CoverageNorm::Capped(2)) - 5.0 / 6.0).abs() < 1e-12
        );
        assert!((floor_coverage(&w, &rare_common, CoverageNorm::Capped(1)) - 1.0).abs() < 1e-12);
        let common: HashSet<String> = ["common"].map(String::from).into();
        assert!((floor_coverage(&w, &common, CoverageNorm::Salient(3)) - 1.0 / 7.0).abs() < 1e-12);
        assert!((floor_coverage(&w, &rare_common, CoverageNorm::Salient(9)) - plain).abs() < 1e-12);
        assert_eq!(floor_coverage(&[], &common, CoverageNorm::Capped(2)), 0.0);
    }

    #[test]
    fn coverage_variants_parse() {
        assert_eq!("plain".parse(), Ok(CoverageNorm::Plain));
        assert_eq!(" Salient:4 ".parse(), Ok(CoverageNorm::Salient(4)));
        assert_eq!("capped:6".parse(), Ok(CoverageNorm::Capped(6)));
        for bad in ["", "salient", "salient:0", "capped:x", "len:3"] {
            assert!(bad.parse::<CoverageNorm>().is_err(), "{bad}");
        }
    }

    #[test]
    fn floor_is_cosine_or_coverage() {
        let f = Floor {
            cosine: 0.6,
            coverage: 0.5,
        };
        assert!(passes_floor(f, Some(0.7), 0.0));
        assert!(passes_floor(f, None, 0.5));
        assert!(!passes_floor(f, Some(0.59), 0.49));
        assert!(!passes_floor(f, None, 0.0));
    }
}
