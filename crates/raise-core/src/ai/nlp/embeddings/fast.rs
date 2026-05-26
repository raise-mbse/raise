// FICHIER : src-tauri/src/ai/nlp/embeddings/fast.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::inference::embeddings::TextEmbedder;
use crate::utils::prelude::*; // 🎯 Façade Unique // 🎯 Injection de NOTRE façade noyau

pub struct FastEmbedEngine {
    // 🎯 ZÉRO DETTE : On ne manipule plus la librairie tierce ici.
    // On s'appuie exclusivement sur la forteresse que vous avez bâtie.
    embedder: TextEmbedder,
}

impl FastEmbedEngine {
    /// Initialise le moteur FastEmbed en respectant l'isolation stricte du domaine RAISE.
    pub async fn new(manager: &CollectionsManager<'_>) -> RaiseResult<Self> {
        // 1. Appel du Gatekeeper (Tolérance aux pannes pour le moteur par défaut)
        let settings =
            match AppConfig::get_runtime_settings(manager, "ref:components:handle:ai_nlp").await {
                Ok(s) => s,
                Err(_) => {
                    // FastEmbed est le moteur léger de secours (Fallback absolu).
                    json_value!({})
                }
            };

        // 2. Extraction de la valeur (avec fallback par défaut)
        let model_name_str = settings
            .get("fastembed_model")
            .and_then(|v| v.as_str())
            .unwrap_or("BGESmallENV15");

        // 3. 🎯 DÉLÉGATION TOTALE : Initialisation "Air-Gap" via le Noyau
        // Plus de gestion de chemins ou de variables d'environnement ici !
        let embedder = TextEmbedder::new()?;

        user_info!(
            "MSG_NLP_FASTEMBED_READY",
            json_value!({ "model": model_name_str, "status": "initialized_via_core_facade" })
        );

        Ok(Self { embedder })
    }

    /// Vectorise un lot de textes (Batch Inference) pour optimiser le débit.
    pub fn embed_batch(&mut self, texts: Vec<String>) -> RaiseResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        // Adaptation du type (Vec<String> -> Vec<&str>) attendu par la façade noyau
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        // 🎯 Délégation directe
        self.embedder.embed_batch(text_refs)
    }

    /// Vectorise une requête unique.
    pub fn embed_query(&mut self, text: &str) -> RaiseResult<Vec<f32>> {
        // 🎯 On réutilise la puissance du batch de la façade noyau pour une seule phrase
        let mut embeddings = self.embedder.embed_batch(vec![text])?;

        // Extraction sécurisée du vecteur
        match embeddings.pop() {
            Some(vector) => Ok(vector),
            None => raise_error!(
                "ERR_AI_EMBEDDING_EMPTY_RESULT",
                error = "Le moteur n'a retourné aucun vecteur.",
                context = json_value!({ "text_len": text.len(), "provider": "FastEmbed_Core" })
            ),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Façade & Résilience)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::io::os::execute_native_inference;
    use crate::utils::testing::AgentDbSandbox;

    /// Test existant : Inférence simple
    #[async_test]
    #[serial_test::serial]
    async fn test_fast_embed_single() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = FastEmbedEngine::new(&manager).await?;
        let vec = engine.embed_query("Ceci est un test de la façade RAISE")?;

        assert_eq!(
            vec.len(),
            384,
            "BGE-Small-EN-V1.5 doit retourner 384 dimensions"
        );
        assert!(vec.iter().any(|&x| x != 0.0));
        Ok(())
    }

    /// Test existant : Inférence par lot
    #[async_test]
    #[serial_test::serial]
    async fn test_fast_embed_batch() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = FastEmbedEngine::new(&manager).await?;
        let inputs = vec![
            "Phrase 1".to_string(),
            "Phrase 2".to_string(),
            "Phrase 3".to_string(),
        ];

        let batch_res = execute_native_inference(move || engine.embed_batch(inputs)).await?;

        assert_eq!(batch_res.len(), 3);
        assert_eq!(batch_res[0].len(), 384);
        Ok(())
    }

    /// Résilience face à un domaine Système vide (Default Fallback)
    #[async_test]
    #[serial_test::serial]
    async fn test_fast_embed_resilience_empty_config() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "void", "void");

        let engine_res = FastEmbedEngine::new(&manager).await;
        assert!(
            engine_res.is_ok(),
            "Le moteur doit fallback sur les paramètres par défaut"
        );
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Inférence sur chaîne vide
    #[async_test]
    #[serial_test::serial]
    async fn test_fast_embed_empty_string() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = FastEmbedEngine::new(&manager).await?;
        let vec = engine.embed_query("");

        assert!(
            vec.is_ok(),
            "Le moteur ONNX doit gérer les chaînes vides sans paniquer"
        );
        Ok(())
    }
}
