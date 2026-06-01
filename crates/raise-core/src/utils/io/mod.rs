// FICHIER : src-tauri/src/utils/io/mod.rs

pub mod audio;
pub mod compression;
pub mod fs;
pub mod io_traits;
pub mod os;
pub mod os_types;

// =========================================================================
// FAÇADE `io` : Interactions Système et Fichiers (AI-Ready Strict)
// =========================================================================
// 🤖 IA NOTE : La nomenclature de ce module est stricte :
// - Les fonctions `_async` nécessitent `.await` (idéal pour le réseau et les I/O non bloquantes).
// - Les fonctions `_sync` bloquent le thread courant (à utiliser dans les scripts ou les phases d'initialisation).

pub use compression::{compress, decompress};

// --- Compression & Décompression (Alias Sémantiques) ---
/// 🤖 IA NOTE : Utilisez `CompressionDecoder` pour lire des flux de données compressés.
pub use zstd::Decoder as CompressionDecoder;
/// 🤖 IA NOTE : Utilisez `CompressionEncoder` pour créer des flux d'écriture compressés.
pub use zstd::Encoder as CompressionEncoder;

pub use fs::{
    copy_async,
    copy_dir_recursive_async,
    copy_sync,
    create_dir_all_async,
    create_dir_all_sync,
    ensure_dir_async,
    ensure_dir_sync,
    exists_async,
    exists_sync,
    include_dir,
    open_async,
    open_sync,
    read_async,
    read_compressed_async,
    read_compressed_sync,
    read_dir_async,
    read_dir_sync,
    read_json_async,
    read_json_compressed_async,
    read_json_compressed_sync,
    read_json_sync,
    read_sync,
    read_to_string_async,
    read_to_string_sync,
    remove_dir_all_async,
    remove_dir_all_sync,
    remove_file_async,
    remove_file_sync,
    rename_async,
    rename_sync,
    tempdir,
    write_async,
    write_atomic_async,
    write_atomic_sync,
    write_compressed_atomic_async,
    write_compressed_atomic_sync,
    write_json_atomic_async,
    write_json_atomic_sync,
    write_json_compressed_atomic_async,
    write_json_compressed_atomic_sync,
    write_sync,
    // Types et Objets
    Component,
    Dir,
    Path,
    PathBuf,
    ProjectScope,
    TempDir,
};

pub use os::{
    exec_command_async, exec_command_sync, execute_native_inference, flush_stdout, pipe_through,
    prompt, read_stdin_line,
};
// --- Flux Standards (Alias de Fonctions) ---
/// 🤖 IA NOTE : Point d'entrée pour le flux d'erreur standard.
pub use std::io::stderr as stderr_raw;
/// 🤖 IA NOTE : Point d'entrée pour le flux d'entrée standard.
pub use std::io::stdin as stdin_raw;
/// 🤖 IA NOTE : Point d'entrée pour le flux de sortie standard.
pub use std::io::stdout as stdout_raw;
