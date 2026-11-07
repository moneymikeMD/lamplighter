// SPDX-License-Identifier: Apache-2.0

//! Deterministic question selection: a seeded shuffle inside each question type, then round-robin across types.

use std::collections::BTreeMap;

use clap::ValueEnum;

use crate::dataset::{Head, Question};

pub(crate) const CI_QUESTIONS: usize = 6;
pub(crate) const CI_HAYSTACK: usize = 12;
pub(crate) const DEFAULT_SEED: u64 = 20_261_106;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub(crate) enum Subset {
    /// A small fixed-seed sample for pull requests
    Ci,
    /// Every scorable question
    Full,
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Abstention questions (`_abs`) have no evidence session to retrieve and are skipped.
pub(crate) fn select(
    heads: &[Head],
    subset: Subset,
    limit: Option<usize>,
    seed: u64,
) -> Vec<String> {
    let mut by_type: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for h in heads.iter().filter(|h| !h.id.ends_with("_abs")) {
        by_type.entry(&h.kind).or_default().push(&h.id);
    }
    let mut state = seed;
    for ids in by_type.values_mut() {
        ids.sort_unstable();
        for i in (1..ids.len()).rev() {
            let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
            ids.swap(i, j);
        }
    }
    let total: usize = by_type.values().map(Vec::len).sum();
    let want = limit.unwrap_or(match subset {
        Subset::Ci => CI_QUESTIONS,
        Subset::Full => total,
    });
    let mut out = Vec::new();
    let mut round = 0;
    while out.len() < want.min(total) {
        for ids in by_type.values() {
            if let Some(id) = ids.get(round)
                && out.len() < want
            {
                out.push((*id).to_owned());
            }
        }
        round += 1;
    }
    out
}

/// Keeps every evidence session plus a seeded sample of the others, up to `keep` sessions in all, in original order.
pub(crate) fn thin(q: &mut Question, keep: usize, seed: u64) {
    if q.sessions.len() <= keep {
        return;
    }
    let mut state = seed
        ^ q.id
            .bytes()
            .fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(u64::from(b)));
    let mut others: Vec<usize> = (0..q.session_ids.len())
        .filter(|i| !q.answer_session_ids.contains(&q.session_ids[*i]))
        .collect();
    for i in (1..others.len()).rev() {
        let j = (splitmix64(&mut state) % (i as u64 + 1)) as usize;
        others.swap(i, j);
    }
    let room = keep.saturating_sub(q.session_ids.len() - others.len());
    let mut kept: Vec<usize> = (0..q.session_ids.len())
        .filter(|i| q.answer_session_ids.contains(&q.session_ids[*i]))
        .chain(others.into_iter().take(room))
        .collect();
    kept.sort_unstable();
    q.session_ids = kept.iter().map(|i| q.session_ids[*i].clone()).collect();
    q.sessions = kept
        .iter()
        .map(|i| std::mem::take(&mut q.sessions[*i]))
        .collect();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn heads() -> Vec<Head> {
        let mut v = Vec::new();
        for kind in ["a", "b", "c"] {
            for i in 0..10 {
                v.push(Head {
                    id: format!("{kind}{i}"),
                    kind: kind.to_owned(),
                });
            }
        }
        v.push(Head {
            id: "x_abs".to_owned(),
            kind: "a".to_owned(),
        });
        v
    }

    #[test]
    fn same_seed_same_pick_and_types_are_balanced() {
        let one = select(&heads(), Subset::Ci, Some(6), 7);
        assert_eq!(one, select(&heads(), Subset::Ci, Some(6), 7));
        assert_ne!(one, select(&heads(), Subset::Ci, Some(6), 8));
        for kind in ["a", "b", "c"] {
            assert_eq!(one.iter().filter(|i| i.starts_with(kind)).count(), 2);
        }
    }

    #[test]
    fn thin_keeps_evidence_and_is_deterministic() {
        let mk = || Question {
            id: "q".to_owned(),
            kind: "a".to_owned(),
            question: String::new(),
            answer_session_ids: vec!["s7".to_owned()],
            session_ids: (0..20).map(|i| format!("s{i}")).collect(),
            sessions: (0..20).map(|_| Vec::new()).collect(),
        };
        let (mut a, mut b) = (mk(), mk());
        thin(&mut a, 5, 3);
        thin(&mut b, 5, 3);
        assert_eq!(a.session_ids, b.session_ids);
        assert_eq!(a.session_ids.len(), 5);
        assert_eq!(a.sessions.len(), 5);
        assert!(a.session_ids.contains(&"s7".to_owned()));
    }

    #[test]
    fn abstention_skipped_and_full_takes_all() {
        let all = select(&heads(), Subset::Full, None, 1);
        assert_eq!(all.len(), 30);
        assert!(!all.contains(&"x_abs".to_owned()));
    }
}
