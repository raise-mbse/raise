// FICHIER : crates/raise-core/src/services/training_service.rs

use crate::ai::llm::client::LlmEngine;
use crate::ai::nlp::parser::CommandType;
use crate::ai::orchestrator::AiOrchestrator;
use crate::ai::training::ai_train_domain_native;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::prelude::*;

/// Lance l'entraînement de l'adaptateur LoRA pour le GNN sur un domaine donné.
pub async fn train_domain(
    storage: &StorageEngine,
    space: &str,
    db_name: &str,
    domain: &str,
    epochs: usize,
    lr: f64,
) -> RaiseResult<String> {
    // Instanciation centralisée du manager
    let manager = CollectionsManager::new(storage, space, db_name);

    // Délégation au moteur d'entraînement IA
    ai_train_domain_native(&manager, domain, epochs, lr).await
}

/// Lance l'apprentissage par renforcement du moteur Neuro-Symbolique (World Model).
/// Retourne un tuple (Loss Initiale, Loss Finale).
pub async fn train_world_model(
    storage: SharedRef<StorageEngine>,
    space: &str,
    db_name: &str,
    iterations: usize,
    native_llm: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
) -> RaiseResult<(f64, f64)> {
    let manager = CollectionsManager::new(storage.as_ref(), space, db_name);

    // 1. Instanciation de l'Orchestrateur depuis le service
    let orchestrator = AiOrchestrator::new(
        ProjectModel::default(),
        &manager,
        storage.clone(),
        native_llm,
    )
    .await?;

    // 2. Préparation du scénario d'apprentissage (Transition LA -> PA)
    let state_before = ArcadiaElement {
        handle: "comp_logic_1".try_into()?,
        name: I18nString::default(),
        kind: vec!["la:LogicalComponent".into()],
        properties: UnorderedMap::new(),
        ..Default::default()
    };

    let state_after = ArcadiaElement {
        handle: "comp_phys_1".try_into()?,
        name: I18nString::default(),
        kind: vec!["pa:PhysicalComponent".into()],
        properties: UnorderedMap::new(),
        ..Default::default()
    };

    // 3. Boucle d'entraînement
    let mut initial_loss = 0.0;
    let mut final_loss = 0.0;

    for i in 1..=iterations {
        let loss = orchestrator
            .reinforce_learning(&state_before, CommandType::Create, &state_after)
            .await?;

        if i == 1 {
            initial_loss = loss;
        }

        // On émet un heartbeat de progression technique
        if i == 1 || i % 10 == 0 || i == iterations {
            user_info!(
                "MSG_TRAINING_WM_STEP",
                json_value!({"iteration": i, "loss": loss})
            );
        }
        final_loss = loss;
    }

    Ok((initial_loss, final_loss))
}
