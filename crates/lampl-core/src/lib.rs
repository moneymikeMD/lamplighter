// SPDX-License-Identifier: Apache-2.0

//! Memory model, reciprocal-rank fusion and relevance floor for lamplighter. No I/O.

mod model;
mod ranking;
mod scope;

pub use model::{
    EMBEDDING_DIM, Embedding, Link, LinkType, Memory, MemoryType, NewMemory, ParseError,
};
pub use ranking::{
    CoverageNorm, Floor, PARITY_QUERIES, RRF_K, floor_coverage, fts_coverage, idf,
    memory_embed_text, passes_floor, query_embed_text, query_terms, relevance, rrf_fuse, term_stem,
};
pub use scope::{
    DEFAULT_GLOBAL_PROJECTS, global_projects_from_env, parse_global_projects, recall_projects,
};
