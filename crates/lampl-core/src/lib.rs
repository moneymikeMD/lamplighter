// SPDX-License-Identifier: Apache-2.0

//! Memory model, reciprocal-rank fusion and relevance floor for lamplighter. No I/O.

mod model;
mod ranking;

pub use model::{
    EMBEDDING_DIM, Embedding, Link, LinkType, Memory, MemoryType, NewMemory, ParseError,
};
pub use ranking::{
    CoverageNorm, Floor, PARITY_QUERIES, RRF_K, floor_coverage, fts_coverage, idf, passes_floor,
    query_embed_text, query_terms, relevance, rrf_fuse, term_stem,
};
