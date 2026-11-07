// SPDX-License-Identifier: Apache-2.0

//! The adapter interface a system under test implements; lamplighter is the only arm shipped.

use crate::dataset::Question;

#[derive(Debug, Clone)]
pub(crate) struct Ranked {
    pub(crate) sessions: Vec<String>,
    pub(crate) mode: String,
}

pub(crate) trait Retriever {
    fn name(&self) -> &str;
    /// Loads one question's haystack so it can be recalled in isolation from the others.
    fn ingest(&mut self, q: &Question) -> Result<usize, String>;
    /// Blocks until everything ingested is searchable by the system's best arm.
    fn settle(&mut self) -> Result<(), String>;
    /// Evidence candidates for the question, best first, one entry per session.
    fn rank(&mut self, q: &Question) -> Result<Ranked, String>;
}
