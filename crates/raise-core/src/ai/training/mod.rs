// FICHIER : crates/raise-core/src/ai/training/mod.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*; // 🎯 Façade Unique

// 🎯 IMPORTS GNN ET NLP
use crate::ai::graph_store::engine::GnnEngine;
use crate::ai::graph_store::features::GraphFeatures;
use crate::ai::nlp::embeddings::EmbeddingEngine;

pub mod dataset;
pub mod lora;

/// Entraîne un adaptateur LoRA sur un domaine métier spécifique via le Graphe de Connaissance (GNN).
/// 🎯 Utilise l'échantillonnage de sous-graphes (Sub-graph Seeding) pour éviter l'OOM sur les grands projets.
pub async fn ai_train_domain_native(
    manager: &CollectionsManager<'_>,
    domain: &str,
    epochs: usize,
    lr: f64,
) -> RaiseResult<String> {
    let device = AppConfig::device();

    user_info!(
        "MSG_TRAINING_START",
        json_value!({"domain": domain, "epochs": epochs, "lr": lr, "mode": "GNN+LoRA"})
    );

    // =========================================================================
    // 1. INITIALISATION DU MOTEUR GNN ET INJECTION LORA
    // =========================================================================

    // A. Chargement de l'adjacence et instanciation du GNN (Mode Sparse)
    let mut engine = GnnEngine::new(manager, 384, 128, device).await?;

    // B. 🎯 INJECTION LORA : Modification structurelle à chaud
    // Rank = 4, Alpha = 1.0 (Hyperparamètres optimisés pour le Fine-Tuning topologique)
    engine
        .model
        .inject_lora(4, 1.0, &mut engine.varmap, device)?;

    // C. 🎯 MISE À JOUR DE L'OPTIMISEUR : Il doit prendre en compte les nouvelles variables (A et B) de LoRA
    let opt_config = OptimizerConfigAdamW {
        lr,
        ..Default::default()
    };
    engine.optimizer = match NeuralOptimizerAdamW::new(engine.varmap.all_vars(), opt_config) {
        Ok(opt) => opt,
        Err(e) => raise_error!("ERR_TRAINING_OPT_UPDATE", error = e.to_string()),
    };

    // =========================================================================
    // 2. PRÉPARATION DES VECTEURS SÉMANTIQUES (NLP)
    // =========================================================================

    let texts = GraphFeatures::extract_texts(manager, &engine.adj.index_to_uri).await?;
    let mut embed_engine = EmbeddingEngine::new(manager).await?;

    // Exécution sécurisée de l'inférence sur le pool CPU/Thread isolé
    let vectors =
        match os::execute_native_inference(move || match embed_engine.embed_batch(texts) {
            Ok(v) => Ok(v),
            Err(e) => raise_error!("ERR_TRAINING_EMBED_FAIL", error = e.to_string()),
        })
        .await
        {
            Ok(v) => v,
            Err(e) => return Err(e), // Propagation directe du RaiseResult
        };

    let features = GraphFeatures::build_from_vectors(vectors, device).await?;

    // =========================================================================
    // 3. BOUCLE D'ENTRAÎNEMENT (SUB-GRAPH SAMPLING)
    // =========================================================================

    let mut final_loss = 0.0;

    for epoch in 1..=epochs {
        // lambda_logic = 10.0 (On force le réseau à respecter les règles d'architecture Arcadia)
        // batch_size = 256 nœuds max par sous-graphe pour préserver la VRAM
        let loss = engine.train_step(&features.matrix, 10.0, 256).await?;
        final_loss = loss;

        // Heartbeat tous les 5 epochs ou au dernier epoch
        if epoch % 5 == 0 || epoch == epochs {
            user_info!(
                "MSG_TRAINING_STEP",
                json_value!({
                    "epoch": epoch,
                    "loss": loss
                })
            );
        }
    }

    // =========================================================================
    // 4. SAUVEGARDE DES POIDS (ADAPTATEUR LORA)
    // =========================================================================

    // On utilise le répertoire de domaine sécurisé pour y loger les tenseurs
    let base_path = match AppConfig::get().get_path("PATH_RAISE_DOMAIN") {
        Some(p) => p,
        None => PathBuf::from("."), // Fallback de sécurité
    };

    let out_dir = base_path.join(&manager.space).join("tensors/lora");
    let _ = fs::ensure_dir_async(&out_dir).await;

    let file_path = out_dir.join(format!("{}_gnn_lora.safetensors", domain));
    match engine.varmap.save(&file_path) {
        Ok(_) => (),
        Err(e) => raise_error!("ERR_TRAINING_SAVE_LORA", error = e.to_string()),
    }

    user_success!(
        "MSG_TRAINING_COMPLETE",
        json_value!({
            "domain": domain,
            "final_loss": final_loss,
            "saved_to": file_path.to_string_lossy()
        })
    );

    Ok(file_path.to_string_lossy().to_string())
}

// =========================================================================
// TESTS UNITAIRES (Validation de l'Entraînement Zéro Dette)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    #[async_test]
    #[serial_test::serial] // Sécurité : Isolé pour ne pas saturer le GPU
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_train_domain_native_gnn_lora() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;

        // 1. Initialisation d'une topologie minimale
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );
        manager.create_collection("la", &schema_uri).await?;
        manager
            .insert_raw(
                "la",
                &json_value!({ "_id": "F1", "@id": "la:F1", "name": "Node" }),
            )
            .await?;

        // 2. Exécution d'un mini-entraînement (2 epochs) avec un très fort Learning Rate
        let result = ai_train_domain_native(&manager, "system", 2, 0.01).await;

        assert!(
            result.is_ok(),
            "L'entraînement GNN + LoRA a échoué : {:?}",
            result.err()
        );

        Ok(())
    }
}
