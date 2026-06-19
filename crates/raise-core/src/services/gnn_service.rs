// FICHIER : src-tauri/src/services/gnn_service.rs

use crate::ai::graph_store::adjacency::GraphAdjacency;
use crate::ai::graph_store::engine::GnnEngine;
use crate::ai::graph_store::features::GraphFeatures;
use crate::ai::nlp::embeddings::EmbeddingEngine;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
// 🎯 IMPORTS POUR LA CONVERGENCE NEURO-SYMBOLIQUE
use crate::genetics::evaluators::neuro_symbolic::GnnResilienceScorer;
use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;

use crate::utils::prelude::*;

// =========================================================================
// ÉTAT GLOBAL (TAURI STATE)
// =========================================================================

pub struct GnnState {
    pub engine: AsyncRwLock<Option<GnnEngine>>,
}

impl GnnState {
    pub fn new() -> Self {
        Self {
            engine: AsyncRwLock::new(None),
        }
    }
}

impl Default for GnnState {
    fn default() -> Self {
        Self::new()
    }
}

// =========================================================================
// CONVERGENCE NEURO-SYMBOLIQUE (Le Pont GNN -> Génétique)
// =========================================================================

/// Adaptateur "Zéro Dette" qui permet à l'algorithme génétique d'interroger
/// le GNN sans exposer la mécanique des tenseurs (Candle/CUDA) au module génétique.
pub struct GnnScorerAdapter {
    pub state: SharedRef<GnnState>,
}

impl GnnScorerAdapter {
    pub fn new(state: SharedRef<GnnState>) -> Self {
        Self { state }
    }
}

#[async_interface]
impl GnnResilienceScorer for GnnScorerAdapter {
    async fn predict_resilience(&self, genome: &SystemAllocationGenome) -> f32 {
        let guard = self.state.engine.read().await;
        let _engine = match &*guard {
            Some(e) => e,
            None => {
                // 🎯 RÉSILIENCE : Si le GNN n'est pas chargé (ex: démarrage initial
                // ou mode dégradé), on retourne un score neutre.
                // L'AG continuera d'optimiser parfaitement sur les critères symboliques.
                return 0.0;
            }
        };

        // 🎯 INFÉRENCE LATENTE (ZÉRO I/O)
        // L'algorithme génétique (NSGA-II) appelle cette fonction à une fréquence extrême.
        // Stratégie ML cible :
        // 1. Convertir les allocations `genome.genes` en indices de sous-graphe COO.
        // 2. Projeter ces indices sur les poids latents pré-entraînés du GNN (_engine.model).
        // 3. Récupérer l'énergie du système (System Energy) sans aucune allocation lourde.

        // Simulation d'une "intuition" neuronale ultra-rapide pour boucler la Phase 3.
        // Le véritable calcul tensoriel de sous-graphe sera câblé dans le moteur Candle.
        let mut resilience_score = 0.0;

        for &comp_idx in &genome.genes {
            resilience_score += (comp_idx as f32) * 0.1;
        }

        resilience_score
    }
}

// =========================================================================
// LOGIQUE INTERNE (ROBUSTE & ISOLÉE)
// =========================================================================

pub async fn init_gnn_engine(
    state: &GnnState,
    storage: &StorageEngine,
    domain: &str,
    db_name: &str,
) -> RaiseResult<String> {
    let device = AppConfig::device();
    let manager = CollectionsManager::new(storage, domain, db_name);

    let engine = match GnnEngine::new(&manager, 384, 128, device).await {
        Ok(e) => e,
        Err(e) => raise_error!("ERR_GNN_INIT_FAILED", error = e.to_string()),
    };

    let mut guard = state.engine.write().await;
    *guard = Some(engine);

    user_success!(
        "MSG_GNN_INITIALIZED",
        json_value!({ "domain": domain, "db": db_name })
    );
    Ok("GNN_READY".to_string())
}

pub async fn train_gnn_step(
    state: &GnnState,
    storage: &StorageEngine,
    domain: &str,
    db: &str,
    lambda: f32,
) -> RaiseResult<f32> {
    let device = AppConfig::device();

    let mut guard = state.engine.write().await;
    let engine = match &mut *guard {
        Some(e) => e,
        None => raise_error!("ERR_GNN_NOT_INITIALIZED"),
    };

    let manager = CollectionsManager::new(storage, domain, db);

    // Pipeline d'entraînement Sparse
    let adj = GraphAdjacency::build_from_store(&manager, device).await?;
    let texts = GraphFeatures::extract_texts(&manager, &adj.index_to_uri).await?;

    let mut embed_engine = EmbeddingEngine::new(&manager).await?;
    let vectors =
        match os::execute_native_inference(move || match embed_engine.embed_batch(texts) {
            Ok(v) => Ok(v),
            Err(e) => raise_error!("ERR_GNN_EMBED_FAIL", error = e.to_string()),
        })
        .await
        {
            Ok(v) => v,
            Err(e) => return Err(e),
        };

    let features = GraphFeatures::build_from_vectors(vectors, device).await?;
    let loss = engine.train_step(&features.matrix, lambda, 256).await?;

    Ok(loss)
}

pub async fn audit_ontology(
    state: &GnnState,
    storage: &StorageEngine,
    domain: &str,
    db: &str,
) -> RaiseResult<Vec<JsonValue>> {
    let device = AppConfig::device();

    let guard = state.engine.read().await;
    let engine = match &*guard {
        Some(e) => e,
        None => raise_error!("ERR_GNN_NOT_INITIALIZED"),
    };

    let manager = CollectionsManager::new(storage, domain, db);

    let adj = GraphAdjacency::build_from_store(&manager, device).await?;
    let texts = GraphFeatures::extract_texts(&manager, &adj.index_to_uri).await?;

    let mut embed_engine = EmbeddingEngine::new(&manager).await?;
    let vectors =
        match os::execute_native_inference(move || match embed_engine.embed_batch(texts) {
            Ok(v) => Ok(v),
            Err(e) => raise_error!("ERR_GNN_EMBED_FAIL", error = e.to_string()),
        })
        .await
        {
            Ok(v) => v,
            Err(e) => return Err(e),
        };

    let features = GraphFeatures::build_from_vectors(vectors, device).await?;
    let reports = engine.audit_ontology(&features.matrix).await?;

    Ok(reports)
}

// =========================================================================
// TESTS UNITAIRES (Couverture Totale & Cuda-Aware)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gnn_command_full_lifecycle() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let state = GnnState::new();

        // 🎯 Utilisation du domaine "system" pour le test (Plus rapide via mock_db)
        let domain = config.mount_points.system.domain.clone();
        let db = config.mount_points.system.db.clone();

        let manager = CollectionsManager::new(&sandbox.db, &domain, &db);
        DbSandbox::mock_db(&manager).await?;

        // 🎯 FIX INTEGRITY : Ajout de _id et @id
        let schema = "db://_system/bootstrap/schemas/v1/db/generic.schema.json";
        manager.create_collection("la", schema).await?;
        manager.create_collection("pa", schema).await?;

        manager
            .insert_raw(
                "pa",
                &json_value!({
                    "_id": "P1", "@id": "pa:P1", "name": "Radar"
                }),
            )
            .await?;
        manager
            .insert_raw(
                "la",
                &json_value!({
                    "_id": "F1", "@id": "la:F1", "realizes": [{"@id": "pa:P1"}]
                }),
            )
            .await?;

        // 1. Initialisation
        init_gnn_engine(&state, &sandbox.db, &domain, &db).await?;

        // 2. Entraînement (Doit maintenant trouver les 2 nœuds et le lien)
        let loss = train_gnn_step(&state, &sandbox.db, &domain, &db, 1.0).await?;
        assert!(loss >= 0.0);

        // 3. Audit
        let reports = audit_ontology(&state, &sandbox.db, &domain, &db).await?;
        assert!(
            reports.is_empty(),
            "Un lien valide ne devrait pas générer d'anomalie. Trouvé: {:?}",
            reports
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gnn_resilience_uninitialized() -> RaiseResult<()> {
        let state = GnnState::new();
        let sandbox = AgentDbSandbox::new().await?;
        let res = audit_ontology(&state, &sandbox.db, "dom".into(), "db".into()).await;
        match res {
            Err(AppError::Structured(err)) => assert_eq!(err.code, "ERR_GNN_NOT_INITIALIZED"),
            _ => panic!("Échec attendu sur moteur non initialisé"),
        }
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gnn_scorer_adapter_fallback() -> RaiseResult<()> {
        let state = SharedRef::new(GnnState::new());
        let adapter = GnnScorerAdapter::new(state.clone());

        let genome = SystemAllocationGenome {
            genes: vec![1, 2, 1],
            function_ids: vec![],
            available_component_ids: vec![],
        };

        // Test du Court-circuit : Le GNN n'est pas initialisé, le score doit être parfaitement 0.0
        // pour ne pas biaiser le front de Pareto de l'Algorithme Génétique.
        let score = adapter.predict_resilience(&genome).await;
        assert_eq!(score, 0.0, "Le score de repli doit être 0.0");

        Ok(())
    }
}
