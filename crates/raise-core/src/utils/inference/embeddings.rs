// FICHIER : src-tauri/src/utils/inference/embeddings.rs

use crate::utils::prelude::*;

/// Modèle de vectorisation par défaut (Optimisé pour tourner sur une VRAM modeste)
pub const DEFAULT_EMBED_MODEL: &str = "BAAI/bge-small-en-v1.5";

/// 🧠 MOTEUR D'EMBEDDING (Text-to-Vector)
///
/// Cette forteresse isole la librairie tierce `fastembed`. Elle permet de
/// transformer du texte en vecteurs pour notre graphe de connaissances sémantique.
/// Si demain on décide d'utiliser une API distante ou un autre modèle ONNX,
/// seule l'implémentation de cette structure changera.
pub struct TextEmbedder {
    // L'implémentation sous-jacente est totalement masquée au reste du code.
    inner: fastembed::TextEmbedding,
}

impl TextEmbedder {
    /// Initialise le moteur de vectorisation de manière sécurisée.
    pub fn new() -> RaiseResult<Self> {
        let config = AppConfig::get();
        let base_embeddings_dir = config.resolve_asset_path(
            config
                .system_assets
                .ai_assets_paths
                .as_ref()
                .and_then(|p| p.embeddings.as_ref()),
            "ai-assets/embeddings",
        )?;
        let cache_dir = base_embeddings_dir.join("bge-small");

        // Petite fonction utilitaire pour charger un fichier en RAM avec gestion d'erreur stricte
        let load_file = |name: &str| -> RaiseResult<Vec<u8>> {
            match std::fs::read(cache_dir.join(name)) {
                Ok(data) => Ok(data),
                Err(e) => raise_error!(
                    "ERR_AI_FASTEMBED_MISSING_FILE",
                    error = e.to_string(),
                    context = json_value!({ "file": name, "path": cache_dir.to_string_lossy() })
                ),
            }
        };

        // 🎯 2. Injection manuelle des fichiers
        let custom_model = fastembed::UserDefinedEmbeddingModel {
            onnx_file: load_file("model.onnx")?,
            tokenizer_files: fastembed::TokenizerFiles {
                tokenizer_file: load_file("tokenizer.json")?,
                config_file: load_file("config.json")?,
                special_tokens_map_file: load_file("special_tokens_map.json")?,
                tokenizer_config_file: load_file("tokenizer_config.json")?,
            },
            // 🎯 MÉTADONNÉES D'INFÉRENCE STRICTES (Spécifiques à BGE-Small)
            pooling: Some(fastembed::Pooling::Cls), // BGE-Small extrait le contexte via le token [CLS]
            external_initializers: vec![],          // Pas de poids séparés pour ce modèle
            output_key: None,                       // Utilisation du nœud de sortie ONNX par défaut
            quantization: Default::default(),       // Pas de quantification forcée
        };

        // 🎯 3. Initialisation "Zéro Réseau"
        match fastembed::TextEmbedding::try_new_from_user_defined(custom_model, Default::default())
        {
            Ok(inner) => Ok(Self { inner }),
            Err(e) => {
                raise_error!(
                    "ERR_INFERENCE_EMBEDDER_INIT",
                    error = e.to_string(),
                    context = json_value!({
                        "action": "init_user_defined_fastembed",
                        "mode": "absolute_air_gap"
                    })
                );
            }
        }
    }

    /// Transforme un lot (batch) de textes en vecteurs denses.
    /// Cette opération est encapsulée pour capturer toute erreur OOM (Out Of Memory).
    pub fn embed_batch(&mut self, texts: Vec<&str>) -> RaiseResult<Vec<Vec<f32>>> {
        let batch_size = texts.len();

        match self.inner.embed(texts, None) {
            Ok(embeddings) => Ok(embeddings),
            Err(e) => {
                raise_error!(
                    "ERR_INFERENCE_EMBEDDING_FAIL",
                    error = e,
                    context = json_value!({
                        "batch_size": batch_size,
                        "action": "generate_embeddings"
                    })
                );
            }
        }
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_embedder_initialization() {
        let _ = crate::utils::data::config::AppConfig::init();
        // Vérifie que le constructeur ne panique pas et charge bien le modèle
        let result = TextEmbedder::new();
        assert!(
            result.is_ok(),
            "L'initialisation de FastEmbed a échoué. Cause: {:?}",
            result.err()
        );
    }

    #[test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_single_embedding_dimension() {
        let _ = crate::utils::data::config::AppConfig::init();
        let mut embedder = TextEmbedder::new().expect("Le modèle devrait s'initialiser");
        let texts = vec!["L'architecture RAISE garantit le Zéro Dette."];

        let result = embedder
            .embed_batch(texts)
            .expect("La vectorisation a échoué");

        // 1. On vérifie qu'un seul vecteur est retourné
        assert_eq!(result.len(), 1);

        // 2. On vérifie la taille stricte du vecteur (384 pour BGE-Small)
        assert_eq!(
            result[0].len(),
            384,
            "La dimension du vecteur ne correspond pas au modèle BGE-Small"
        );
    }

    #[test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    fn test_batch_embeddings_processing() {
        let _ = crate::utils::data::config::AppConfig::init();
        let mut embedder = TextEmbedder::new().expect("Le modèle devrait s'initialiser");

        let texts = vec![
            "Court.",
            "Une phrase de taille moyenne pour tester l'analyse sémantique.",
            "Voici un texte beaucoup plus long qui représente le contenu d'un document SysML complet, avec des spécifications techniques, des contraintes de performance et des exigences de traçabilité strictes."
        ];

        let result = embedder
            .embed_batch(texts.clone())
            .expect("La vectorisation par lot a échoué");

        assert_eq!(
            result.len(),
            texts.len(),
            "Le nombre de vecteurs retournés ne correspond pas au batch initial"
        );

        for (i, embedding) in result.iter().enumerate() {
            assert_eq!(
                embedding.len(),
                384,
                "Le vecteur à l'index {} a une dimension invalide",
                i
            );

            let sum: f32 = embedding.iter().map(|v| v.abs()).sum();
            assert!(
                sum > 0.0,
                "Le vecteur à l'index {} est vide (somme nulle)",
                i
            );
        }
    }
}
