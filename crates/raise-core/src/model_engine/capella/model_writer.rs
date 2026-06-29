// FICHIER : crates/raise-core/src/model_engine/capella/model_writer.rs

use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

pub struct CapellaWriter;

impl CapellaWriter {
    /// Sauvegarde le modèle au format JSON (RAISE native format) de manière asynchrone et atomique.
    pub async fn save_as_json(model: &ProjectModel, path: &Path) -> RaiseResult<()> {
        // 1. Sérialisation JSON (déjà asynchrone/RaiseResult via nos utils)
        let json_data = json::serialize_to_string_pretty(model)?;

        // 2. Écriture atomique
        fs::write_atomic_async(path, json_data.as_bytes()).await?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    async fn test_save_json() {
        let dir = tempdir().unwrap(); // tempdir() est aussi async
        let file_path = dir.path().join("model.json");

        let model = ProjectModel::default();

        // Appel async avec .await
        CapellaWriter::save_as_json(&model, &file_path)
            .await
            .expect("Save failed");

        assert!(file_path.exists());
    }
}
