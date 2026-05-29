// FICHIER : src-tauri/src/utils/io/fs.rs

use crate::raise_error;
use crate::utils::core::error::RaiseResult;
use crate::utils::core::spawn_cpu_task;
use crate::utils::data::json::{self, json_value};
use crate::utils::data::{DeserializableOwned, Serializable};

use tracing::instrument;

// --- RE-EXPORTS (Isolation de la couche OS) ---
pub use include_dir::{include_dir, Dir};
pub use std::fs::{Metadata, Permissions};
pub use std::path::{Component, Path, PathBuf};
pub use tempfile::{tempdir, TempDir};
pub use walkdir::WalkDir;

// =========================================================================
// 7. OUVERTURE DE FICHIERS (Handles)
// =========================================================================

/// 🤖 IA NOTE : Ouvre un descripteur de fichier en lecture seule de manière asynchrone.
/// Idéal pour streamer de gros fichiers (modèles IA, datasets) sans charger toute la RAM.
pub async fn open_async(path: impl AsRef<Path>) -> RaiseResult<tokio::fs::File> {
    let p = path.as_ref();
    match tokio::fs::File::open(p).await {
        Ok(file) => Ok(file),
        Err(e) => raise_error!(
            "ERR_FS_OPEN_ASYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

/// 🤖 IA NOTE : Ouvre un descripteur de fichier en lecture seule de manière synchrone.
pub fn open_sync(path: impl AsRef<Path>) -> RaiseResult<std::fs::File> {
    let p = path.as_ref();
    match std::fs::File::open(p) {
        Ok(file) => Ok(file),
        Err(e) => raise_error!(
            "ERR_FS_OPEN_SYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

// =========================================================================
// 1. LECTURE & ÉCRITURE BASIQUES
// =========================================================================

#[instrument(skip(path), fields(path = ?path.as_ref()))]
pub async fn read_async(path: impl AsRef<Path>) -> RaiseResult<Vec<u8>> {
    let p = path.as_ref();
    match tokio::fs::read(p).await {
        Ok(data) => Ok(data),
        Err(e) => raise_error!(
            "ERR_FS_READ_FILE",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

pub fn read_sync(path: impl AsRef<Path>) -> RaiseResult<Vec<u8>> {
    let p = path.as_ref();
    match std::fs::read(p) {
        Ok(data) => Ok(data),
        Err(e) => raise_error!(
            "ERR_FS_READ_FILE",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

pub async fn read_to_string_async(path: &Path) -> RaiseResult<String> {
    match tokio::fs::read_to_string(path).await {
        Ok(s) => Ok(s),
        Err(e) => raise_error!(
            "ERR_FS_READ_STR",
            error = e,
            context = json_value!({ "path": path.to_string_lossy() })
        ),
    }
}

pub fn read_to_string_sync(path: &Path) -> RaiseResult<String> {
    match std::fs::read_to_string(path) {
        Ok(s) => Ok(s),
        Err(e) => raise_error!(
            "ERR_FS_READ_STR",
            error = e,
            context = json_value!({ "path": path.to_string_lossy() })
        ),
    }
}

pub async fn write_async(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> RaiseResult<()> {
    let p = path.as_ref();
    if let Err(e) = tokio::fs::write(p, contents).await {
        raise_error!(
            "ERR_FS_WRITE_FILE",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        );
    }
    Ok(())
}

pub fn write_sync(path: impl AsRef<Path>, contents: impl AsRef<[u8]>) -> RaiseResult<()> {
    let p = path.as_ref();
    if let Err(e) = std::fs::write(p, contents) {
        raise_error!(
            "ERR_FS_WRITE_FILE",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        );
    }
    Ok(())
}

// =========================================================================
// 2. GESTION DES DOSSIERS ET FICHIERS
// =========================================================================

pub async fn exists_async(path: &Path) -> bool {
    tokio::fs::metadata(path).await.is_ok()
}

pub fn exists_sync(path: &Path) -> bool {
    std::fs::metadata(path).is_ok()
}

pub async fn ensure_dir_async(path: &Path) -> RaiseResult<()> {
    if !exists_async(path).await {
        if let Err(e) = tokio::fs::create_dir_all(path).await {
            raise_error!(
                "ERR_FS_ENSURE_DIR",
                error = e,
                context = json_value!({ "path": path.to_string_lossy() })
            );
        }
    }
    Ok(())
}

pub fn ensure_dir_sync(path: &Path) -> RaiseResult<()> {
    if !exists_sync(path) {
        if let Err(e) = std::fs::create_dir_all(path) {
            raise_error!(
                "ERR_FS_ENSURE_DIR",
                error = e,
                context = json_value!({ "path": path.to_string_lossy() })
            );
        }
    }
    Ok(())
}

/// 🤖 IA NOTE : Lit le contenu d'un répertoire de manière asynchrone.
/// Retourne un `tokio::fs::ReadDir` enveloppé dans un `RaiseResult`.
pub async fn read_dir_async<P: AsRef<Path>>(path: P) -> RaiseResult<tokio::fs::ReadDir> {
    match tokio::fs::read_dir(path.as_ref()).await {
        Ok(iter) => Ok(iter),
        Err(e) => crate::raise_error!(
            "ERR_FS_READ_DIR_ASYNC",
            error = e,
            context = json_value!({ "path": path.as_ref().display().to_string() })
        ),
    }
}

/// 🤖 IA NOTE : Lit le contenu d'un répertoire de manière synchrone (bloquant).
/// Retourne un `std::fs::ReadDir` enveloppé dans un `RaiseResult`.
pub fn read_dir_sync<P: AsRef<Path>>(path: P) -> RaiseResult<std::fs::ReadDir> {
    match std::fs::read_dir(path.as_ref()) {
        Ok(iter) => Ok(iter),
        Err(e) => crate::raise_error!(
            "ERR_FS_READ_DIR_SYNC",
            error = e,
            context = json_value!({ "path": path.as_ref().display().to_string() })
        ),
    }
}

pub async fn remove_dir_all_async(path: impl AsRef<Path>) -> RaiseResult<()> {
    let p = path.as_ref();
    // On vérifie d'abord l'existence pour éviter une erreur inutile si le dossier est déjà supprimé
    if exists_async(p).await {
        if let Err(e) = tokio::fs::remove_dir_all(p).await {
            crate::raise_error!(
                "ERR_FS_REMOVE_DIR",
                error = e,
                context = json_value!({ "path": p.to_string_lossy() })
            );
        }
    }
    Ok(())
}

pub fn remove_dir_all_sync(path: impl AsRef<Path>) -> RaiseResult<()> {
    let p = path.as_ref();
    if exists_sync(p) {
        if let Err(e) = std::fs::remove_dir_all(p) {
            crate::raise_error!(
                "ERR_FS_REMOVE_DIR",
                error = e,
                context = json_value!({ "path": p.to_string_lossy() })
            );
        }
    }
    Ok(())
}

pub async fn remove_file_async(path: &Path) -> RaiseResult<()> {
    if exists_async(path).await {
        if let Err(e) = tokio::fs::remove_file(path).await {
            raise_error!(
                "ERR_FS_REMOVE_FILE",
                error = e,
                context = json_value!({ "path": path.to_string_lossy() })
            );
        }
    }
    Ok(())
}

pub fn remove_file_sync(path: &Path) -> RaiseResult<()> {
    if exists_sync(path) {
        if let Err(e) = std::fs::remove_file(path) {
            raise_error!(
                "ERR_FS_REMOVE_FILE",
                error = e,
                context = json_value!({ "path": path.to_string_lossy() })
            );
        }
    }
    Ok(())
}

pub async fn copy_async(from: impl AsRef<Path>, to: impl AsRef<Path>) -> RaiseResult<u64> {
    let from_path = from.as_ref();
    let to_path = to.as_ref();
    match tokio::fs::copy(from_path, to_path).await {
        Ok(size) => Ok(size),
        Err(e) => raise_error!(
            "ERR_FS_COPY_FILE",
            error = e,
            context = json_value!({ "from": from_path.to_string_lossy(), "to": to_path.to_string_lossy() })
        ),
    }
}

pub fn copy_sync(from: impl AsRef<Path>, to: impl AsRef<Path>) -> RaiseResult<u64> {
    let from_path = from.as_ref();
    let to_path = to.as_ref();
    match std::fs::copy(from_path, to_path) {
        Ok(size) => Ok(size),
        Err(e) => raise_error!(
            "ERR_FS_COPY_FILE",
            error = e,
            context = json_value!({ "from": from_path.to_string_lossy(), "to": to_path.to_string_lossy() })
        ),
    }
}

pub async fn create_dir_all_async(path: impl AsRef<Path>) -> RaiseResult<()> {
    let p = path.as_ref();
    if let Err(e) = tokio::fs::create_dir_all(p).await {
        raise_error!(
            "ERR_FS_CREATE_DIR",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        );
    }
    Ok(())
}

pub fn create_dir_all_sync(path: impl AsRef<Path>) -> RaiseResult<()> {
    let p = path.as_ref();
    if let Err(e) = std::fs::create_dir_all(p) {
        raise_error!(
            "ERR_FS_CREATE_DIR",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        );
    }
    Ok(())
}

/// 🤖 IA NOTE : Renomme ou déplace un fichier/répertoire de manière asynchrone.
pub async fn rename_async<P: AsRef<Path>>(from: P, to: P) -> RaiseResult<()> {
    match tokio::fs::rename(from.as_ref(), to.as_ref()).await {
        Ok(_) => Ok(()),
        Err(e) => crate::raise_error!(
            "ERR_FS_RENAME_ASYNC",
            error = e,
            context = json_value!({
                "from": from.as_ref().display().to_string(),
                "to": to.as_ref().display().to_string()
            })
        ),
    }
}

/// 🤖 IA NOTE : Renomme ou déplace un fichier/répertoire de manière synchrone.
pub fn rename_sync<P: AsRef<Path>>(from: P, to: P) -> RaiseResult<()> {
    match std::fs::rename(from.as_ref(), to.as_ref()) {
        Ok(_) => Ok(()),
        Err(e) => crate::raise_error!(
            "ERR_FS_RENAME_SYNC",
            error = e,
            context = json_value!({
                "from": from.as_ref().display().to_string(),
                "to": to.as_ref().display().to_string()
            })
        ),
    }
}

// =========================================================================
// 3. ÉCRITURE ATOMIQUE (Anti-Corruption)
// =========================================================================

#[instrument(skip(content, path), fields(path = ?path))]
pub async fn write_atomic_async(path: &Path, content: &[u8]) -> RaiseResult<()> {
    use tokio::io::AsyncWriteExt;
    if let Some(parent) = path.parent() {
        ensure_dir_async(parent).await?;
    }
    let unique_id = crate::utils::prelude::UniqueId::new_v4().to_string();
    let tmp_path = path.with_extension(format!("tmp.{}", unique_id));
    let mut file = match tokio::fs::File::create(&tmp_path).await {
        Ok(f) => f,
        Err(e) => raise_error!(
            "ERR_FS_CREATE_TMP",
            error = e,
            context = json_value!({ "tmp_path": tmp_path.to_string_lossy() })
        ),
    };
    if let Err(e) = file.write_all(content).await {
        raise_error!(
            "ERR_FS_WRITE_TMP",
            error = e,
            context = json_value!({ "path": tmp_path.to_string_lossy() })
        );
    }
    file.flush().await.ok();
    file.sync_all().await.ok();
    if let Err(e) = tokio::fs::rename(&tmp_path, path).await {
        let _ = remove_file_async(&tmp_path).await;
        raise_error!(
            "ERR_FS_RENAME_ATOMIC",
            error = e,
            context = json_value!({ "final": path.to_string_lossy() })
        );
    }
    Ok(())
}

pub fn write_atomic_sync(path: &Path, content: &[u8]) -> RaiseResult<()> {
    use std::io::Write;
    if let Some(parent) = path.parent() {
        ensure_dir_sync(parent)?;
    }
    let unique_id = crate::utils::prelude::UniqueId::new_v4().to_string();
    let tmp_path = path.with_extension(format!("tmp.{}", unique_id));
    let mut file = match std::fs::File::create(&tmp_path) {
        Ok(f) => f,
        Err(e) => raise_error!(
            "ERR_FS_CREATE_TMP",
            error = e,
            context = json_value!({ "tmp_path": tmp_path.to_string_lossy() })
        ),
    };
    if let Err(e) = file.write_all(content) {
        raise_error!(
            "ERR_FS_WRITE_TMP",
            error = e,
            context = json_value!({ "path": tmp_path.to_string_lossy() })
        );
    }
    file.flush().ok();
    file.sync_all().ok();
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = remove_file_sync(&tmp_path);
        raise_error!(
            "ERR_FS_RENAME_ATOMIC",
            error = e,
            context = json_value!({ "final": path.to_string_lossy() })
        );
    }
    Ok(())
}

// =========================================================================
// 4. OPÉRATIONS SÉMANTIQUES (JSON & BINCODE)
// =========================================================================

pub async fn read_json_async<T: DeserializableOwned>(path: &Path) -> RaiseResult<T> {
    let content = read_to_string_async(path).await?;
    json::deserialize_from_str(&content)
}

pub fn read_json_sync<T: DeserializableOwned>(path: &Path) -> RaiseResult<T> {
    let content = read_to_string_sync(path)?;
    json::deserialize_from_str(&content)
}

pub async fn write_json_atomic_async<T: Serializable>(path: &Path, data: &T) -> RaiseResult<()> {
    let content = json::serialize_to_string_pretty(data)?;
    write_atomic_async(path, content.as_bytes()).await
}

pub fn write_json_atomic_sync<T: Serializable>(path: &Path, data: &T) -> RaiseResult<()> {
    let content = json::serialize_to_string_pretty(data)?;
    write_atomic_sync(path, content.as_bytes())
}

// =========================================================================
// 5. OPÉRATIONS COMPRESSÉES (Zstd)
// =========================================================================

pub async fn write_compressed_atomic_async(path: &Path, content: &[u8]) -> RaiseResult<()> {
    let data = content.to_vec();
    // 🎯 En asynchrone, on délègue la compression CPU-intensive à un pool bloquant !
    let join_res = spawn_cpu_task(move || super::compression::compress(&data)).await;
    let compressed = match join_res {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return Err(e),
        Err(e) => raise_error!("ERR_FS_COMPRESS_PANIC", error = e),
    };
    write_atomic_async(path, &compressed).await
}

pub fn write_compressed_atomic_sync(path: &Path, content: &[u8]) -> RaiseResult<()> {
    // 🎯 En synchrone, on compresse directement sur le thread courant
    let compressed = super::compression::compress(content)?;
    write_atomic_sync(path, &compressed)
}

pub async fn read_compressed_async(path: &Path) -> RaiseResult<Vec<u8>> {
    let compressed_data = read_async(path).await?;
    let join_res = spawn_cpu_task(move || super::compression::decompress(&compressed_data)).await;
    match join_res {
        Ok(Ok(d)) => Ok(d),
        Ok(Err(e)) => Err(e),
        Err(e) => raise_error!("ERR_FS_DECOMPRESS_PANIC", error = e),
    }
}

pub fn read_compressed_sync(path: &Path) -> RaiseResult<Vec<u8>> {
    let compressed_data = read_sync(path)?;
    super::compression::decompress(&compressed_data)
}

// --- JSON COMPRESSÉ ---
pub async fn write_json_compressed_atomic_async<T: Serializable>(
    path: &Path,
    data: &T,
) -> RaiseResult<()> {
    let content = json::serialize_to_string(data)?;
    write_compressed_atomic_async(path, content.as_bytes()).await
}

pub fn write_json_compressed_atomic_sync<T: Serializable>(
    path: &Path,
    data: &T,
) -> RaiseResult<()> {
    let content = json::serialize_to_string(data)?;
    write_compressed_atomic_sync(path, content.as_bytes())
}

pub async fn read_json_compressed_async<T: DeserializableOwned>(path: &Path) -> RaiseResult<T> {
    let decompressed = read_compressed_async(path).await?;

    let content = match String::from_utf8(decompressed) {
        Ok(c) => c,
        Err(e) => crate::raise_error!("ERR_DATA_CORRUPTION_UTF8", error = e),
    };

    json::deserialize_from_str(&content)
}

pub fn read_json_compressed_sync<T: DeserializableOwned>(path: &Path) -> RaiseResult<T> {
    let decompressed = read_compressed_sync(path)?;

    let content = match String::from_utf8(decompressed) {
        Ok(c) => c,
        Err(e) => crate::raise_error!("ERR_DATA_CORRUPTION_UTF8", error = e),
    };

    json::deserialize_from_str(&content)
}

// =========================================================================
// 6. SANDBOXING : PROJECT SCOPE
// =========================================================================

#[derive(Clone, Debug)]
pub struct ProjectScope {
    root: PathBuf,
}

impl ProjectScope {
    pub fn new_sync(root: impl Into<PathBuf>) -> RaiseResult<Self> {
        let root = root.into();
        ensure_dir_sync(&root)?;
        let canonical = match root.canonicalize() {
            Ok(path) => path,
            Err(e) => raise_error!(
                "ERR_FS_SCOPE_INIT",
                error = e,
                context = json_value!({ "root": root.to_string_lossy() })
            ),
        };
        Ok(Self { root: canonical })
    }

    fn validate_path(&self, relative_path: &Path) -> RaiseResult<PathBuf> {
        if relative_path
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            raise_error!("ERR_FS_SECURITY_VIOLATION", error = "Évasion détectée (..)");
        }
        let target = self.root.join(relative_path);
        if !target.starts_with(&self.root) {
            raise_error!("ERR_FS_SECURITY_VIOLATION", error = "Chemin hors limite");
        }
        Ok(target)
    }

    pub async fn write_async(
        &self,
        relative_path: impl AsRef<Path>,
        content: &[u8],
    ) -> RaiseResult<()> {
        let target = self.validate_path(relative_path.as_ref())?;
        write_atomic_async(&target, content).await
    }

    pub fn write_sync(&self, relative_path: impl AsRef<Path>, content: &[u8]) -> RaiseResult<()> {
        let target = self.validate_path(relative_path.as_ref())?;
        write_atomic_sync(&target, content)
    }
}

// =========================================================================
// 7. MÉTADONNÉES & PERMISSIONS
// =========================================================================

#[instrument(skip(path), fields(path = ?path.as_ref()))]
pub async fn metadata_async(path: impl AsRef<Path>) -> RaiseResult<Metadata> {
    let p = path.as_ref();
    match tokio::fs::metadata(p).await {
        Ok(meta) => Ok(meta),
        Err(e) => raise_error!(
            "ERR_FS_METADATA_ASYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

pub fn metadata_sync(path: impl AsRef<Path>) -> RaiseResult<Metadata> {
    let p = path.as_ref();
    match std::fs::metadata(p) {
        Ok(meta) => Ok(meta),
        Err(e) => raise_error!(
            "ERR_FS_METADATA_SYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

pub async fn get_permissions_async(path: impl AsRef<Path>) -> RaiseResult<Permissions> {
    let meta = metadata_async(path).await?;
    Ok(meta.permissions())
}

pub fn get_permissions_sync(path: impl AsRef<Path>) -> RaiseResult<Permissions> {
    let meta = metadata_sync(path)?;
    Ok(meta.permissions())
}

#[instrument(skip(path, perms), fields(path = ?path.as_ref()))]
pub async fn set_permissions_async(path: impl AsRef<Path>, perms: Permissions) -> RaiseResult<()> {
    let p = path.as_ref();
    match tokio::fs::set_permissions(p, perms).await {
        Ok(_) => Ok(()),
        Err(e) => raise_error!(
            "ERR_FS_SET_PERMISSIONS_ASYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}

pub fn set_permissions_sync(path: impl AsRef<Path>, perms: Permissions) -> RaiseResult<()> {
    let p = path.as_ref();
    match std::fs::set_permissions(p, perms) {
        Ok(_) => Ok(()),
        Err(e) => raise_error!(
            "ERR_FS_SET_PERMISSIONS_SYNC",
            error = e,
            context = json_value!({ "path": p.to_string_lossy() })
        ),
    }
}
