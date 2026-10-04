//! Pure, bounded import parsers. No parser performs I/O or Store publication.

mod native;
mod yaml_guard;

pub use native::{
    MAX_NATIVE_DEPTH, MAX_NATIVE_ENTRIES, MAX_NATIVE_NODES, ParsedSource, ParserError, parse_native,
};
