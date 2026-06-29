// FICHIER : src-tauri/src/ai/llm/client.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::utils::data::json::Clearance;
use crate::utils::prelude::*;
use async_trait::async_trait;

// 🎯 Import des fournisseurs Cloud
use crate::ai::llm::providers::{claude, gemini, mistral};

#[derive(Clone, Debug, PartialEq)]
pub enum LlmBackend {
    Mistral,
    Claude,
    Gemini,
    Mock, // Utilisé pour intercepter les appels dans les tests
    LocalLlama,
    GoogleGemini,
    LlamaCpp,
    RustNative,
}

#[async_trait]
pub trait LlmEngine: Send + Sync {
    async fn generate(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> RaiseResult<String>;

    async fn generate_with_grammar(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
        grammar_str: &str,
    ) -> RaiseResult<String>;
}

#[derive(Clone)]
pub struct LlmClient {
    storage: SharedRef<StorageEngine>,
    pub space: String,
    pub db_name: String,
    native_engine: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
}

impl LlmClient {
    pub async fn new(
        manager: &CollectionsManager<'_>,
        storage: SharedRef<StorageEngine>,
        native_engine: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    ) -> RaiseResult<Self> {
        Ok(Self {
            storage,
            space: manager.space.to_string(),
            db_name: manager.db.to_string(),
            native_engine,
        })
    }

    /// Le "Gatekeeper" hybride : Route la requête en fonction de l'habilitation (Clearance).
    pub async fn ask(
        &self,
        backend: LlmBackend,
        system_prompt: &str,
        user_prompt: &str,
        clearance: Clearance,
    ) -> RaiseResult<String> {
        // 1. DÉLÉGATION DIRECTE CLOUD (Données Publiques)
        if clearance == Clearance::Public {
            return self.call_cloud(backend, system_prompt, user_prompt).await;
        }

        // 2. EXÉCUTION LOCALE SOUVERAINE (Priorité pour tous les autres niveaux)
        if let Some(engine_ref) = &self.native_engine {
            let mut engine = engine_ref.lock().await;

            match engine.generate(system_prompt, user_prompt, 1024).await {
                Ok(response) => return Ok(response),
                Err(e) => {
                    // Si l'exécution locale échoue, on vérifie si la loi/stratégie autorise la fuite Cloud
                    if !clearance.is_cloud_authorized() {
                        return Err(build_error!(
                    "ERR_SECURITY_AIR_GAP",
                    error = format!("Échec du moteur local ({}). Fallback Cloud interdit pour le niveau {:?}.", e, clearance)
                ));
                    }
                }
            }
        } else if !clearance.is_cloud_authorized() {
            // Aucun GPU/Moteur local disponible et interdiction stricte de sortir
            return Err(build_error!(
                "ERR_SECURITY_AIR_GAP",
                error = format!(
                    "Moteur local indisponible. Déploiement bloqué pour protéger la donnée ({:?}).",
                    clearance
                )
            ));
        }

        // 3. FALLBACK CLOUD (Uniquement autorisé si is_cloud_authorized() est vrai)
        if clearance.is_cloud_authorized() {
            user_warn!(
                "AI_LOCAL_UNAVAILABLE",
                json_value!({"hint": format!("Bascule sur le réseau distant autorisée (Niveau: {:?}).", clearance)})
            );
            return self.call_cloud(backend, system_prompt, user_prompt).await;
        }

        unreachable!()
    }

    /// Wrapper interne pour l'appel aux API distantes
    async fn call_cloud(
        &self,
        backend: LlmBackend,
        system_prompt: &str,
        user_prompt: &str,
    ) -> RaiseResult<String> {
        // Interception pour les tests unitaires afin d'éviter les vrais appels réseau
        if backend == LlmBackend::Mock {
            return Ok("[CLOUD_MOCK_RESPONSE] Réponse générée par le réseau distant.".to_string());
        }

        let manager = CollectionsManager::new(self.storage.as_ref(), &self.space, &self.db_name);
        match backend {
            LlmBackend::Claude => claude::ask(&manager, system_prompt, user_prompt).await,
            LlmBackend::Gemini => gemini::ask(&manager, system_prompt, user_prompt).await,
            _ => mistral::ask(&manager, system_prompt, user_prompt).await,
        }
    }

    /// Méthode de commodité par défaut (Internal)
    pub async fn generate(&self, user_prompt: &str) -> RaiseResult<String> {
        self.ask(
            LlmBackend::Mistral,
            "Tu es un assistant IA expert et concis.",
            user_prompt,
            Clearance::Internal,
        )
        .await
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation du Routage et du Gatekeeper de Sécurité)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::mock::MockLlmEngine;
    use crate::utils::testing::AgentDbSandbox;

    #[async_test]
    #[serial_test::serial]
    async fn test_llm_client_lightweight_init() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let client = LlmClient::new(
            &manager,
            sandbox.db.clone(),
            Some(sandbox.shared_engine.clone()),
        )
        .await?;

        assert_eq!(client.space, config.mount_points.system.domain);
        assert_eq!(client.db_name, config.mount_points.system.db);

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_llm_client_default_generation_routing() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let response_mock = r#"{"message": "Test unitaire validé avec succès", "artifacts": []}"#;
        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: response_mock.to_string(),
        }));

        let client = LlmClient::new(&manager, sandbox.db.clone(), Some(mock_engine)).await?;

        let result = client.generate("Bonjour").await?;
        assert!(result.contains("Test unitaire validé avec succès"));
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_llm_client_claude_routing() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let expected_msg = "Test unitaire validé avec succès";
        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: expected_msg.to_string(),
        }));

        let client = LlmClient::new(&manager, sandbox.db.clone(), Some(mock_engine)).await?;

        let result = client
            .ask(LlmBackend::Claude, "System", "User", Clearance::Internal)
            .await?;
        assert!(result.contains(expected_msg));
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_llm_client_gemini_routing() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        // 🎯 ÉTAPE 1 : On crée le "leurre" (Mock) avec le message attendu
        let expected_msg = "Test unitaire validé avec succès";
        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: expected_msg.to_string(),
        }));

        let client = LlmClient::new(&manager, sandbox.db.clone(), Some(mock_engine)).await?;

        let result = client
            .ask(LlmBackend::Gemini, "System", "User", Clearance::Internal)
            .await?;

        assert!(result.contains(expected_msg));
        Ok(())
    }

    /// TEST 1 : Donnée SECRÈTE avec ressource locale disponible -> SUCCÈS.
    #[async_test]
    #[serial_test::serial]
    async fn test_clearance_secret_with_local() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        let expected_msg = "Résultat Confidentiel Local";
        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: expected_msg.to_string(),
        }));

        let client = LlmClient::new(&manager, sandbox.db.clone(), Some(mock_engine)).await?;

        let result = client
            .ask(LlmBackend::Mock, "System", "Prompt", Clearance::Secret)
            .await?;
        assert_eq!(
            result, expected_msg,
            "La requête SECRÈTE doit être traitée localement."
        );
        Ok(())
    }

    /// TEST 2 : Donnée SECRÈTE sans ressource locale -> ÉCHEC BLOQUANT (Maintien de l'Air-Gap).
    #[async_test]
    #[serial_test::serial]
    async fn test_clearance_secret_without_local_blocks_airgap() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        // Aucun moteur local fourni (None)
        let client = LlmClient::new(&manager, sandbox.db.clone(), None).await?;

        let result = client
            .ask(LlmBackend::Mock, "System", "Prompt", Clearance::Secret)
            .await;

        assert!(
            result.is_err(),
            "La requête doit échouer pour protéger la donnée."
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("ERR_SECURITY_AIR_GAP"),
            "L'erreur doit mentionner la protection Air-Gap."
        );
        Ok(())
    }

    /// TEST 3.A : Donnée INTERNE sans ressource locale -> ÉCHEC BLOQUANT (Souveraineté).
    #[async_test]
    #[serial_test::serial]
    async fn test_clearance_internal_without_local_blocks_airgap() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        // Aucun moteur local fourni (None)
        let client = LlmClient::new(&manager, sandbox.db.clone(), None).await?;

        // 🎯 C2-Interne n'est pas compatible Cloud Act, le système doit bloquer !
        let result = client
            .ask(LlmBackend::Mock, "System", "Prompt", Clearance::Internal)
            .await;

        assert!(
            result.is_err(),
            "La requête C2-Interne doit échouer pour protéger la donnée de toute fuite Cloud."
        );
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ERR_SECURITY_AIR_GAP"));
        Ok(())
    }

    /// TEST 3.B : Donnée CLOUD ACT sans ressource locale -> FALLBACK CLOUD RÉUSSI.
    #[async_test]
    #[serial_test::serial]
    async fn test_clearance_cloud_act_triggers_fallback() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        // Aucun moteur local fourni (None)
        let client = LlmClient::new(&manager, sandbox.db.clone(), None).await?;

        // 🎯 C2-CA (InternalCloudAct) autorise le fallback
        let result = client
            .ask(
                LlmBackend::Mock,
                "System",
                "Prompt",
                Clearance::InternalCloudAct,
            )
            .await?;

        assert!(
            result.contains("[CLOUD_MOCK_RESPONSE]"),
            "Le Gatekeeper doit basculer sur le cloud pour les données compatibles Cloud Act."
        );
        Ok(())
    }

    /// TEST 4 : Donnée PUBLIQUE -> CLOUD DIRECT (Bypass du moteur local même si disponible).
    #[async_test]
    #[serial_test::serial]
    async fn test_clearance_public_bypasses_local() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "db");

        let local_msg = "Je suis le GPU Local";
        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: local_msg.to_string(),
        }));

        // Le moteur local EST disponible
        let client = LlmClient::new(&manager, sandbox.db.clone(), Some(mock_engine)).await?;

        let result = client
            .ask(LlmBackend::Mock, "System", "Prompt", Clearance::Public)
            .await?;

        assert!(result.contains("[CLOUD_MOCK_RESPONSE]"), "Le Gatekeeper doit router directement vers le cloud, ignorant le GPU local pour économiser la VRAM.");
        assert!(
            !result.contains(local_msg),
            "Le moteur local ne doit pas avoir été sollicité."
        );
        Ok(())
    }
}
