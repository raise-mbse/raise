// FICHIER : src-tauri/src/ai/graph_store/features.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*; // 🎯 LA FAÇADE GLOBALE

pub struct GraphFeatures {
    /// Le tenseur des caractéristiques [N, D] (H-Matrix)
    pub matrix: NeuralTensor,
}

impl GraphFeatures {
    /// 🎯 ÉTAPE 1 : Extraction pure (E/S Asynchrone)
    /// Lit la base de données et prépare les textes sémantiques.
    pub async fn extract_texts(
        manager: &CollectionsManager<'_>,
        index_to_uri: &[String],
    ) -> RaiseResult<Vec<String>> {
        let n_nodes = index_to_uri.len();
        if n_nodes == 0 {
            raise_error!(
                "ERR_GNN_EMPTY_FEATURES",
                error =
                    "La liste des URIs est vide. Impossible de construire la matrice de features."
            );
        }

        user_info!(
            "MSG_GNN_FEATURES_EXTRACTION_START",
            json_value!({ "nodes_count": n_nodes })
        );

        let mut texts: Vec<String> = Vec::with_capacity(n_nodes);

        for uri in index_to_uri {
            let mut semantic_text = String::new();
            let parts: Vec<&str> = uri.split(':').collect();

            if parts.len() >= 2 {
                let col = parts[0];
                let id = parts[1];

                if let Ok(Some(doc)) = manager.get_document(col, id).await {
                    semantic_text =
                        crate::ai::graph_store::store::extract_rich_semantic_content(&doc);
                } else if let Ok(Some(doc)) = manager.get_document(col, uri).await {
                    semantic_text =
                        crate::ai::graph_store::store::extract_rich_semantic_content(&doc);
                }
            }

            if semantic_text.trim().is_empty() {
                semantic_text = uri.replace([':', '_'], " ");
            }

            texts.push(semantic_text);
        }

        Ok(texts)
    }

    /// 🎯 ÉTAPE 2 : Compilation du Tenseur (Calcul CPU/GPU isolé)
    /// Prend les vecteurs générés par l'orchestrateur et construit la matrice H.
    pub async fn build_from_vectors(
        vectors: Vec<Vec<f32>>,
        device: &ComputeHardware,
    ) -> RaiseResult<Self> {
        let n_nodes = vectors.len();
        if n_nodes == 0 {
            raise_error!("ERR_GNN_EMPTY_VECTORS", error = "Aucun vecteur fourni.");
        }

        let expected_dim = vectors[0].len();
        let flat_data: Vec<f32> = vectors.into_iter().flatten().collect();

        if flat_data.len() != n_nodes * expected_dim {
            raise_error!(
                "ERR_GNN_DIMENSION_MISMATCH",
                error = "Incohérence des dimensions d'embedding détectée lors de l'aplatissement.",
                context = json_value!({ "expected_total": n_nodes * expected_dim, "got": flat_data.len() })
            );
        }

        let device_clone = device.clone();

        // 🎯 BOUCLIER CPU (Façade Raise pure, Zéro map_err)
        let tensor_result = os::execute_native_inference(move || {
            let t = match NeuralTensor::from_vec(flat_data, (n_nodes, expected_dim), &device_clone)
            {
                Ok(t_val) => t_val,
                Err(e) => raise_error!(
                    "ERR_GNN_FEATURES_TENSOR_FAILED",
                    error = e.to_string(),
                    context = json_value!({ "nodes": n_nodes, "dim": expected_dim })
                ),
            };
            Ok(t)
        })
        .await;

        let matrix = match tensor_result {
            Ok(m) => m,
            Err(e) => return Err(e),
        };

        user_success!(
            "MSG_GNN_FEATURES_READY",
            json_value!({ "shape": format!("[{}, {}]", n_nodes, expected_dim) })
        );

        Ok(Self { matrix })
    }
}

// =========================================================================
// TESTS UNITAIRES (Le Test joue le rôle de l'Orchestrateur)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::nlp::embeddings::EmbeddingEngine;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_graph_features_generation_batch_mode() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        for col in &["la", "sa", "pa"] {
            manager.create_collection(col, &schema_uri).await?;
        }

        manager
            .insert_raw(
                "la",
                &json_value!({"_id": "F1", "name": "Radar", "description": "Detection"}),
            )
            .await?;
        manager
            .insert_raw("sa", &json_value!({"_id": "S1", "name": "Defense"}))
            .await?;
        manager
            .insert_raw("pa", &json_value!({"_id": "H1", "name": "Antenna"}))
            .await?;

        let mut engine = EmbeddingEngine::new(&manager).await?;
        let uris = vec![
            "la:F1".to_string(),
            "sa:S1".to_string(),
            "pa:H1".to_string(),
        ];

        // 🎯 L'ORCHESTRATION "ZÉRO DETTE" COMMENCE ICI

        // 1. Extraction des textes
        let texts = GraphFeatures::extract_texts(&manager, &uris).await?;

        // 2. Inférence isolée (Transfert de propriété temporaire du moteur)
        let inference_result = os::execute_native_inference(move || {
            let vectors = match engine.embed_batch(texts) {
                Ok(v) => v,
                Err(e) => raise_error!("ERR_NLP_BATCH", error = e.to_string()),
            };
            Ok((vectors, engine)) // On renvoie les vecteurs ET le moteur !
        })
        .await;

        let (vectors, _engine_returned) = match inference_result {
            Ok(res) => res,
            Err(e) => return Err(e),
        };

        // 3. Construction du Tenseur
        let feat = GraphFeatures::build_from_vectors(vectors, &ComputeHardware::Cpu).await?;

        assert_eq!(feat.matrix.dims(), &[3, 384]);
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_features_empty_list_fails() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let res = GraphFeatures::extract_texts(&manager, &[]).await;

        match res {
            Err(AppError::Structured(err)) => assert_eq!(err.code, "ERR_GNN_EMPTY_FEATURES"),
            _ => panic!("Le moteur aurait dû lever ERR_GNN_EMPTY_FEATURES"),
        }
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_features_fallback_on_missing_docs() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = EmbeddingEngine::new(&manager).await?;

        let uris = vec!["ghost:entity_01".to_string()];

        // 1. Extraction
        let texts = GraphFeatures::extract_texts(&manager, &uris).await?;

        // 2. Inférence
        let inference_result = os::execute_native_inference(move || {
            let vectors = match engine.embed_batch(texts) {
                Ok(v) => v,
                Err(e) => raise_error!("ERR_NLP_BATCH", error = e.to_string()),
            };
            Ok(vectors)
        })
        .await;

        let vectors = match inference_result {
            Ok(v) => v,
            Err(e) => return Err(e),
        };

        // 3. Tenseur
        let feat = GraphFeatures::build_from_vectors(vectors, &ComputeHardware::Cpu).await?;

        assert_eq!(feat.matrix.dims(), &[1, 384]);
        Ok(())
    }
}
