// FICHIER : crates/raise-core/src/model_engine/capella/mod.rs

pub mod diagram_generator;
pub mod model_reader;
pub mod model_writer;
pub mod xmi_parser;

// Re-exports
pub use model_reader::CapellaReader;
pub use xmi_parser::CapellaXmiParser;
