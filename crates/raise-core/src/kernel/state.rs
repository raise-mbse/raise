// FICHIER : crates/raise-core/src/kernel/state.rs

use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

// Import des moteurs lourds
use crate::ai::llm::client::LlmEngine;
use crate::ai::llm::native_engine::NativeTensorEngine;
use crate::ai::orchestrator::AiOrchestrator;
use crate::code_generator::CodeGeneratorService;

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::types::ProjectModel;

/// Le Cœur du Réacteur RAISE (Single Source of Truth).
/// Propriétaire exclusif des allocations lourdes (GGUF, VRAM, AST).
#[derive(Clone)]
pub struct RaiseKernelState {
    pub orchestrator: Option<SharedRef<AsyncMutex<AiOrchestrator>>>,
    pub native_llm: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    pub code_generator: Option<SharedRef<AsyncMutex<CodeGeneratorService>>>,
}

impl RaiseKernelState {
    /// Séquence de démarrage (Boot) du Kernel.
    /// Ne DOIT être appelée qu'une seule fois au lancement du programme.
    pub async fn boot(storage: SharedRef<StorageEngine>) -> RaiseResult<Self> {
        let config = AppConfig::get();

        // 1. Accès exclusif à la partition Système pour l'initialisation
        let sys_manager = CollectionsManager::new(
            storage.as_ref(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        user_info!(
            "MSG_KERNEL_BOOT_SEQUENCE",
            json_value!({"action": "loading_heavy_engines"})
        );

        // 2. Initialisation du Native LLM
        let native_llm_engine = match NativeTensorEngine::new(&sys_manager).await {
            Ok(engine) => {
                user_success!("SUC_KERNEL_LLM_NATIVE_READY");
                let engine_trait: SharedRef<AsyncMutex<dyn LlmEngine>> =
                    SharedRef::new(AsyncMutex::new(engine));
                Some(engine_trait)
            }
            Err(e) => {
                user_warn!(
                    "WRN_KERNEL_LLM_NATIVE_DEGRADED",
                    json_value!({"error": e.to_string(), "hint": "Le LLM natif ne sera pas disponible."})
                );
                None
            }
        };

        // 3. Initialisation de l'Orchestrateur (Neuro-Symbolique & RAG)
        let orchestrator_engine = match AiOrchestrator::new(
            ProjectModel::default(),
            &sys_manager,
            storage.clone(),
            native_llm_engine.clone(),
        )
        .await
        {
            Ok(orch) => {
                user_success!("SUC_KERNEL_ORCHESTRATOR_READY");
                Some(SharedRef::new(AsyncMutex::new(orch)))
            }
            Err(e) => {
                user_warn!(
                    "WRN_KERNEL_ORCHESTRATOR_DEGRADED",
                    json_value!({"error": e.to_string(), "hint": "L'Orchestrateur IA ne sera pas disponible."})
                );
                None
            }
        };

        // 4. Initialisation du Générateur de Code (AST Weaver)
        let root_path = config
            .get_path("PATH_RAISE_DOMAIN")
            .unwrap_or_else(|| PathBuf::from("./raise_domain"));
        let code_gen_engine = CodeGeneratorService::new(root_path);

        // 5. Retourne l'état global scellé
        Ok(Self {
            orchestrator: orchestrator_engine,
            native_llm: native_llm_engine,
            code_generator: Some(SharedRef::new(AsyncMutex::new(code_gen_engine))),
        })
    }
}

// =========================================================================
// TESTS UNITAIRES ET D'INTÉGRATION (Zéro Dette)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    /// 🎯 TEST 1 : Résilience et Dégradation Gracieuse
    /// Vérifie que le Kernel s'allume même si les fichiers GGUF sont absents de la sandbox.
    /// Il doit simplement mettre l'IA en 'None' sans faire crasher l'application.
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_kernel_boot_graceful_degradation() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db, // 🎯 sandbox.db est notre StorageEngine !
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut llm_doc = manager
            .get_document("service_configs", "cfg_ai_llm_test")
            .await?
            .expect("La config LLM devrait être présente via AgentDbSandbox");

        llm_doc["service_settings"]["rust_model_file"] = json_value!("ghost_model.gguf");
        llm_doc["service_settings"]["rust_tokenizer_file"] = json_value!("ghost_tok.json");
        manager.insert_raw("service_configs", &llm_doc).await?;

        let kernel = RaiseKernelState::boot(sandbox.db.clone()).await?;

        // 1. Vérification : Le Générateur de Code (qui n'a pas de contrainte VRAM) DOIT être allumé
        assert!(
            kernel.code_generator.is_some(),
            "Le générateur de code doit toujours être actif."
        );

        // 2. Vérification : Le moteur IA doit avoir échoué proprement (Graceful Degradation)
        assert!(
            kernel.native_llm.is_none(),
            "Le LLM natif devrait être None car les fichiers GGUF sont introuvables."
        );

        Ok(())
    }

    /// 🎯 TEST 2 : Intégrité des Pointeurs RAM (Singleton garanti)
    /// Prouve que cloner le Kernel pour le passer au frontend Tauri ne copie pas
    /// les objets lourds en mémoire, mais partage bien la référence atomique.
    #[async_test]
    #[serial_test::serial]
    async fn test_kernel_memory_pointer_integrity() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;

        // 🎯 FIX : Utilisation directe de sandbox.db
        let storage_ref = sandbox.db.clone();

        let kernel_primary = RaiseKernelState::boot(storage_ref).await?;

        // C'est ce clone que Tauri utilisera pour chaque fenêtre / commande
        let kernel_clone = kernel_primary.clone();

        match (kernel_primary.code_generator, kernel_clone.code_generator) {
            (Some(cg1), Some(cg2)) => {
                // VERIFICATION CRITIQUE : Les pointeurs mémoires DOIVENT être strictement identiques
                if !SharedRef::ptr_eq(&cg1, &cg2) {
                    raise_error!(
                        "ERR_TEST_MEMORY_DUPLICATION",
                        error = "Alerte : Le clonage du Kernel a dupliqué le moteur en RAM au lieu de partager la référence !"
                    );
                }
            }
            _ => {
                raise_error!(
                    "ERR_TEST_SETUP",
                    error = "Le générateur de code n'a pas pu être initialisé."
                );
            }
        }

        Ok(())
    }
}
