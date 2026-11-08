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
