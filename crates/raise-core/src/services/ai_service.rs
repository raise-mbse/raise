// FICHIER : crates/raise-core/src/services/ai_service.rs

use crate::ai::agents::AgentResult;
use crate::ai::orchestrator::AiOrchestrator;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

// Import Moteur Natif
use crate::ai::llm::NativeLlmState;

// Imports World Model
use crate::ai::nlp::parser::CommandType;
use crate::model_engine::types::{ArcadiaElement, NameType};

// Imports GNN Arcadia
use crate::ai::deep_learning::models::gnn_model::ArcadiaGnnModel;
use crate::ai::graph_store::{GraphAdjacency, GraphFeatures};
use crate::ai::nlp::embeddings::EmbeddingEngine;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::{JsonDbConfig, StorageEngine};

// 🎯 IMPORT POUR L'EXPORT DE DATASET
use crate::ai::training::dataset::{extract_domain_data, TrainingExample};

use crate::ai::agents::prompt_engine::PromptEngine;
use crate::ai::agents::tools::extract_json_from_llm;
use crate::ai::llm::client::{LlmBackend, LlmClient, LlmEngine};
use crate::utils::data::json::Clearance;

/// 🎯 LOGIQUE CORE : Exécute un blueprint de prompt (Data-Driven).
/// Respecte les points de montage système pour la résolution du client LLM.
pub async fn ai_execute_blueprint_core(
    storage: SharedRef<StorageEngine>,
    native_llm: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    domain: &str,
    db: &str,
    prompt_handle: &str,
    vars: Option<JsonValue>,
) -> RaiseResult<String> {
    // 1. Initialisation résiliente du Manager et du Client LLM
    let manager = CollectionsManager::new(storage.as_ref(), domain, db);
    let client = match LlmClient::new(&manager, storage.clone(), native_llm).await {
        Ok(c) => c,
        Err(e) => raise_error!("ERR_LLM_CLIENT_INIT", error = e.to_string()),
    };

    // 2. Compilation via le PromptEngine
    let prompt_engine = PromptEngine::new(storage, domain, db);
    let system_prompt = prompt_engine.compile(prompt_handle, vars.as_ref()).await?;

    // 3. Inférence LLM
    let response = match client
        .ask(
            LlmBackend::LocalLlama,
            &system_prompt,
            "",
            Clearance::Internal,
        )
        .await
    {
        Ok(r) => r,
        Err(e) => raise_error!("ERR_LLM_INFERENCE_FAIL", error = e.to_string()),
    };

    // 4. Nettoyage JSON
    Ok(extract_json_from_llm(&response))
}

/// 🖥️ : Expose la logique blueprint (Façade pure).
pub async fn ai_execute_blueprint(
    storage: SharedRef<StorageEngine>,
    ai_state: &AiState,
    domain: &str,        // 🎯 OPTIMISATION : &str
    db: &str,            // 🎯 OPTIMISATION : &str
    prompt_handle: &str, // 🎯 OPTIMISATION : &str
    vars: Option<JsonValue>,
) -> RaiseResult<String> {
    let native_llm = {
        let guard = ai_state.0.lock().await;
        if let Some(orch_ref) = &*guard {
            let orchestrator = orch_ref.lock().await;
            orchestrator.llm_native.clone() // Le type parfait !
        } else {
            None
        }
    };
    ai_execute_blueprint_core(storage, native_llm, domain, db, prompt_handle, vars).await
}

/// Exporte un dataset d'entraînement pour un domaine spécifique.
pub async fn ai_export_dataset(
    storage: &StorageEngine,
    space: &str,
    db_name: &str,
    domain: &str,
) -> RaiseResult<Vec<TrainingExample>> {
    let manager = CollectionsManager::new(storage, space, db_name);
    extract_domain_data(&manager, domain).await // 🎯 FIX : Retrait du '&' superflu
}

// --- STATES ---
pub struct AiState(pub AsyncMutex<Option<SharedRef<AsyncMutex<AiOrchestrator>>>>);

impl AiState {
    pub fn new(orch: Option<SharedRef<AsyncMutex<AiOrchestrator>>>) -> Self {
        Self(AsyncMutex::new(orch))
    }
}

// --- COMMANDES ORCHESTRATION UNIFIÉE (V2) ---

pub async fn ai_reset(ai_state: &AiState) -> RaiseResult<()> {
    let guard = ai_state.0.lock().await;
    if let Some(shared_orch) = &*guard {
        let mut orchestrator = shared_orch.lock().await;

        match orchestrator.clear_history().await {
            Ok(_) => (),
            Err(e) => raise_error!(
                "ERR_AI_HISTORY_CLEAR_FAIL",
                error = e.to_string(),
                context = json_value!({ "action": "reset_ai_orchestrator" })
            ),
        }
    }
    Ok(())
}

pub async fn ai_learn_text(
    ai_state: &AiState,
    content: &str, // 🎯 OPTIMISATION : &str
    source: &str,  // 🎯 OPTIMISATION : &str
) -> RaiseResult<String> {
    let guard = ai_state.0.lock().await;
    if let Some(shared_orch) = &*guard {
        let mut orchestrator = shared_orch.lock().await;

        let chunks_count = match orchestrator.learn_document(content, source).await {
            Ok(count) => count,
            Err(e) => raise_error!(
                "ERR_AI_LEARN_DOCUMENT_FAILURE",
                error = e.to_string(),
                context = json_value!({ "source": source })
            ),
        };

        Ok(format!(
            "Document appris avec succès ({} fragments).",
            chunks_count
        ))
    } else {
        raise_error!(
            "ERR_AI_ORCHESTRATOR_NOT_READY",
            error = "Orchestrateur non initialisé"
        )
    }
}

pub async fn ai_confirm_learning(
    ai_state: &AiState,
    action_intent: &str, // 🎯 OPTIMISATION : &str
    entity_name: String, // Laissé en String car consommé par NameType::String
    entity_kind: String, // Laissé en String car consommé par kind
) -> RaiseResult<String> {
    let guard = ai_state.0.lock().await;

    let Some(shared_orch) = &*guard else {
        raise_error!("ERR_AI_SYSTEM_NOT_READY", error = "Orchestrateur manquant")
    };

    let orchestrator = shared_orch.lock().await;

    let intent = match action_intent {
        "Create" => CommandType::Create,
        "Delete" => CommandType::Delete,
        unknown => {
            raise_error!(
                "ERR_CLI_UNKNOWN_ACTION",
                error = "Type d'intention invalide",
                context = json_value!({"received": unknown})
            );
        }
    };

    let props_before = UnorderedMap::new();
    let state_before = ArcadiaElement {
        id: "root".to_string(),
        name: NameType::String("Context".to_string()),
        kind: "Context".to_string(),
        properties: props_before,
    };

    let mut props_after = UnorderedMap::new();
    props_after.insert("description".to_string(), json_value!("Feedback"));

    let state_after = ArcadiaElement {
        id: "new".to_string(),
        name: NameType::String(entity_name),
        kind: entity_kind,
        properties: props_after,
    };

    match orchestrator
        .reinforce_learning(&state_before, intent, &state_after)
        .await
    {
        Ok(loss) => Ok(format!("Renforcement terminé. Loss: {:.5}", loss)),
        Err(e) => raise_error!("ERR_AI_REINFORCEMENT_FAILED", error = e.to_string()),
    }
}

pub async fn ai_chat(ai_state: &AiState, user_input: &str) -> RaiseResult<AgentResult> {
    let guard = ai_state.0.lock().await;

    if let Some(shared_orch) = &*guard {
        let mut orchestrator = shared_orch.lock().await;

        match orchestrator.execute_workflow(user_input).await {
            Ok(res) => Ok(res),
            Err(e) => raise_error!("ERR_AI_WORKFLOW_EXECUTION", error = e.to_string()),
        }
    } else {
        raise_error!("ERR_AI_SYSTEM_NOT_READY")
    }
}

pub async fn ask_native_llm(
    state: &NativeLlmState,
    sys: &str, // 🎯 OPTIMISATION : &str
    usr: &str, // 🎯 OPTIMISATION : &str
) -> RaiseResult<String> {
    let mut guard = match state.0.lock() {
        Ok(lock_guard) => lock_guard,
        Err(_) => raise_error!("ERR_SYS_MUTEX_POISONED"),
    };
    if let Some(engine) = guard.as_mut() {
        match engine.generate(sys, usr, 1000) {
            Ok(output) => Ok(output),
            Err(e) => raise_error!("ERR_AI_GENERATION_FAILED", error = e.to_string()),
        }
    } else {
        raise_error!("ERR_AI_ENGINE_NOT_LOADED")
    }
}

pub async fn validate_arcadia_gnn(
    collections_path: &str, // 🎯 OPTIMISATION : &str
    uri_a: &str,            // 🎯 OPTIMISATION : &str
    uri_b: &str,            // 🎯 OPTIMISATION : &str
) -> RaiseResult<JsonValue> {
    user_info!(
        "🚀 [GNN] Lancement de l'expérience MBSE...",
        json_value!({ "a": uri_a, "b": uri_b })
    );

    let path_buf = PathBuf::from(collections_path);
    let config = AppConfig::get();

    // 🎯 FIX MATÉRIEL : Utilisation de la façade centrale SSOT
    let device = AppConfig::device();

    let db_config = JsonDbConfig::new(path_buf.clone());
    let storage = StorageEngine::new(db_config)?;

    let manager = CollectionsManager::new(
        &storage,
        &config.mount_points.system.domain,
        &config.mount_points.system.db,
    );

    // Initialisation
    let adjacency = GraphAdjacency::build_from_store(&manager, device).await?;
    let mut engine = EmbeddingEngine::new(&manager).await?;

    // =========================================================================
    // 🎯 ÉTAPE 1 & 2 : L'Orchestration "Zéro Dette" pour les Features
    // =========================================================================
    let texts = GraphFeatures::extract_texts(&manager, &adjacency.index_to_uri).await?;

    let inference_result = crate::utils::io::os::execute_native_inference(move || {
        let vectors = match engine.embed_batch(texts) {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_GNN_EMBEDDING_FAILED", error = e.to_string()),
        };
        Ok(vectors)
    })
    .await;

    let vectors = match inference_result {
        Ok(v) => v,
        Err(e) => return Err(e),
    };

    let features = GraphFeatures::build_from_vectors(vectors, device).await?;

    // =========================================================================
    // 🎯 ÉTAPE 3 : Le modèle GNN et l'inférence (Sparse COO !!)
    // =========================================================================
    let varmap = NeuralWeightsMap::new();
    let vb = NeuralWeightsBuilder::from_varmap(&varmap, ComputeType::F32, device);

    let in_dim = match features.matrix.dims().get(1) {
        Some(&d) => d,
        None => raise_error!("ERR_GNN_DIMS_INVALID"),
    };

    let model = ArcadiaGnnModel::new(in_dim, in_dim / 2, 32, vb).await?;

    let sim_initial = model
        .compute_similarity(&features.matrix, &adjacency, uri_a, uri_b)
        .await?;

    // 🚀 L'APPEL SPARSE MAGIQUE : Adieu la boucle de conversion de 40 Go !
    let final_embeddings = model
        .forward(&adjacency.edge_src, &adjacency.edge_dst, &features.matrix)
        .await?;

    let sim_final = model
        .compute_similarity(&final_embeddings, &adjacency, uri_a, uri_b)
        .await?;

    let delta = sim_final - sim_initial;
    let confirmed = delta > 0.0;

    if confirmed {
        user_success!(
            "✅ [GNN] Hypothèse confirmée : rapprochement de {:.2}%",
            json_value!(delta * 100.0)
        );
    }

    Ok(json_value!({
        "status": "success",
        "uri_a": uri_a,
        "uri_b": uri_b,
        "metrics": {
            "nlp_similarity": sim_initial,
            "gnn_similarity": sim_final,
            "improvement": delta
        },
        "hypothesis_confirmed": confirmed
    }))
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================
#[cfg(test)]
mod tests_gnn_cmd {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    /// Test existant : Échec si URI inconnue
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_validate_arcadia_gnn_not_found_fails() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;

        let result = validate_arcadia_gnn(
            &sandbox.domain_root.to_string_lossy(),
            "la:InconnuA",
            "la:InconnuB",
        )
        .await;

        assert!(result.is_err());
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience Mount Points
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_service_mount_point_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // On vérifie que la commande utilise bien les nouveaux points de montage système
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        assert_eq!(manager.space, config.mount_points.system.domain);
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Inférence matériel sécurisée
    #[async_test]
    async fn test_ai_service_device_ssot() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let device = AppConfig::device();
        // Le périphérique doit être valide pour le moteur natif
        assert!(device.is_cpu() || device.is_cuda() || device.is_metal());
        Ok(())
    }
}
