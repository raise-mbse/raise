// FICHIER : crates/raise-core/src/ai/orchestrator.rs

use crate::ai::context::{
    conversation_manager::ConversationSession, memory_store::MemoryStore, rag::RagRetriever,
    retriever::SimpleRetriever,
};
use crate::ai::llm::client::{LlmBackend, LlmClient, LlmEngine};
use crate::ai::nlp::parser::CommandType;
use crate::ai::world_model::engine::WorldModelConfig;
use crate::ai::world_model::perception::encoder::HybridEncoder; // 🎯 Import du nouvel encodeur
use crate::ai::world_model::{NeuroSymbolicEngine, WorldAction, WorldTrainer};
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::data::json::Clearance;
use crate::utils::prelude::*;

// --- IMPORTS AGENTS ---
use crate::ai::agents::intent_classifier::IntentClassifier;
use crate::ai::agents::{dynamic_agent::DynamicAgent, Agent, AgentContext, AgentResult};

/// Chef d'orchestre du système IA RAISE.
/// Gère le cycle de vie hybride : RAG sémantique, Inférence LLM et World Model Neuro-Symbolique.
pub struct AiOrchestrator {
    pub rag: RagRetriever,
    pub symbolic: SimpleRetriever,
    pub llm_native: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    pub llm_remote: LlmClient,
    pub session: ConversationSession,
    pub memory_store: MemoryStore,
    pub world_engine: SharedRef<NeuroSymbolicEngine>,
    pub hybrid_encoder: HybridEncoder, // 🎯 Ajout de l'encodeur hybride

    pub space: String,
    pub db_name: String,
    pub storage: SharedRef<StorageEngine>,
}

impl AiOrchestrator {
    pub async fn new(
        model: ProjectModel,
        manager: &CollectionsManager<'_>,
        storage: SharedRef<StorageEngine>,
        native_llm: Option<SharedRef<AsyncMutex<dyn LlmEngine>>>,
    ) -> RaiseResult<Self> {
        // 1. Initialisation des composants RAG (Utilise le NLP sur CPU désormais)
        let rag = RagRetriever::new(manager).await?;
        let symbolic = SimpleRetriever::new(model);

        // 2. Client LLM Distant (Fallback)
        let llm_remote = LlmClient::new(manager, storage.clone(), native_llm.clone()).await?;

        // 3. World Model (Neuro-Symbolique) avec AUTO-HEALING (Zéro Dette)
        let world_engine = match NeuroSymbolicEngine::bootstrap(manager).await {
            Ok(engine) => {
                // 🎯 AUTO-HEALING : Si l'ancien cerveau était dimensionné à 16,
                // il est obsolète face à notre nouvel HybridEncoder (32). On le recrée.
                if engine.config.embedding_dim != 32 {
                    crate::user_warn!(
                        "WRN_OBSOLETE_BRAIN",
                        json_value!({"msg": "Ancien modèle détecté. Recréation en 32 dimensions pour supporter le NLP."})
                    );
                    let mut cfg = engine.config.clone();
                    cfg.embedding_dim = 32;
                    NeuroSymbolicEngine::new_empty(cfg)?
                } else {
                    engine
                }
            }
            Err(e) => {
                crate::user_warn!(
                    "WRN_WORLD_MODEL_LOAD_FAILED",
                    json_value!({ "error": e.to_string(), "hint": "Démarrage avec un modèle vierge." })
                );

                // Récupération de la config mockée/locale avec sécurisation
                // Récupération de la config mockée/locale avec sécurisation
                let wm_config = match AppConfig::get_runtime_settings(
                    manager,
                    "ref:components:handle:ai_world_model",
                )
                .await
                {
                    Ok(settings) => {
                        // 🎯 FIX CLIPPY : unwrap_or au lieu de unwrap_or_else
                        let mut cfg: WorldModelConfig = json::deserialize_from_value(settings)
                            .unwrap_or(WorldModelConfig {
                                vocab_size: 1000,
                                embedding_dim: 32,
                                action_dim: 5,
                                hidden_dim: 64,
                                use_gpu: false,
                            });
                        // 🎯 AUTO-HEALING : On écrase la config de test obsolète
                        if cfg.embedding_dim != 32 {
                            cfg.embedding_dim = 32;
                        }
                        cfg
                    }
                    Err(_) => WorldModelConfig {
                        vocab_size: 1000,
                        embedding_dim: 32,
                        action_dim: 5,
                        hidden_dim: 64,
                        use_gpu: false,
                    },
                };

                NeuroSymbolicEngine::new_empty(wm_config)?
            }
        };

        // 🎯 Initialisation de l'HybridEncoder (On lie ses poids à la varmap du WorldModel !)
        let vb = NeuralWeightsBuilder::from_varmap(
            &world_engine.varmap,
            ComputeType::F32,
            &ComputeHardware::Cpu,
        );

        let hybrid_encoder = match HybridEncoder::new(384, 16, vb) {
            Ok(enc) => enc,
            Err(e) => raise_error!("ERR_HYBRID_ENCODER_INIT", error = e.to_string()),
        };

        // 4. Mémoire conversationnelle
        let memory_store = MemoryStore::new(manager).await?;
        let session = memory_store.load_or_create(manager, "main_session").await?;

        crate::user_info!(
            "MSG_ORCHESTRATOR_INIT_SUCCESS",
            json_value!({
                "native_llm_attached": native_llm.is_some(),
                "space": manager.space
            })
        );

        Ok(Self {
            rag,
            symbolic,
            llm_native: native_llm,
            llm_remote,
            session,
            memory_store,
            world_engine: SharedRef::new(world_engine),
            hybrid_encoder, // 🎯 Injection validée
            space: manager.space.to_string(),
            db_name: manager.db.to_string(),
            storage,
        })
    }

    /// Exécute un workflow multi-agents complet avec routage d'intention.
    pub async fn execute_workflow(&mut self, user_query: &str) -> RaiseResult<AgentResult> {
        let app_config = AppConfig::get();
        let storage_arc = self.storage.clone();

        // Utilisation des Mount Points pour reconstruire le manager technique
        let _manager = CollectionsManager::new(
            storage_arc.as_ref(),
            &app_config.mount_points.system.domain,
            &app_config.mount_points.system.db,
        );

        let classifier = IntentClassifier::new(self.llm_remote.clone());
        let mut current_intent = classifier.classify(user_query).await;
        let mut current_agent_urn = current_intent.recommended_agent_id().to_string();

        let session_scope = current_intent.default_session_scope();
        let global_session_id =
            AgentContext::generate_default_session_id("orchestrator", session_scope)?;

        let domain_path = match app_config.get_path("PATH_RAISE_DOMAIN") {
            Some(p) => p,
            None => raise_error!(
                "ERR_CONFIG_PATH_MISSING",
                error = "PATH_RAISE_DOMAIN non défini"
            ),
        };
        let dataset_path = app_config
            .get_path("PATH_RAISE_DATASET")
            .unwrap_or_else(|| domain_path.join("dataset"));

        let mut hop_count = 0;
        const MAX_HOPS: i32 = 5;
        let mut accumulated_artifacts = Vec::new();
        let mut accumulated_messages = Vec::new();

        loop {
            if hop_count >= MAX_HOPS {
                accumulated_messages
                    .push("⚠️ Limite de redirections entre agents atteinte.".to_string());
                break;
            }

            let ctx = AgentContext::new(
                &current_agent_urn,
                &global_session_id,
                storage_arc.clone(),
                self.llm_remote.clone(),
                self.world_engine.clone(),
                domain_path.clone(),
                dataset_path.clone(),
            )
            .await?;

            let agent = DynamicAgent::new(&current_agent_urn);
            match agent.process(&ctx, &current_intent).await? {
                Some(res) => {
                    accumulated_artifacts.extend(res.artifacts);
                    accumulated_messages.push(res.message);

                    if let Some(acl_msg) = res.outgoing_message {
                        current_agent_urn = acl_msg.receiver.clone();
                        current_intent = classifier.classify(&acl_msg.content).await;
                        hop_count += 1;
                        continue;
                    } else {
                        break;
                    }
                }
                None => break,
            }
        }

        Ok(AgentResult {
            message: accumulated_messages.join("\n\n---\n\n"),
            artifacts: accumulated_artifacts,
            outgoing_message: None,
            xai_frame: None,
        })
    }

    /// Interface "Ask" optimisée : Priorité au Local (VRAM partagée) -> Fallback Cloud.
    pub async fn ask(&mut self, query: &str) -> RaiseResult<String> {
        self.session.add_user_message(query);
        let app_config = AppConfig::get();
        let manager = CollectionsManager::new(
            self.storage.as_ref(),
            &app_config.mount_points.system.domain,
            &app_config.mount_points.system.db,
        );

        let rag_ctx = self
            .rag
            .retrieve(&manager, query, 3)
            .await
            .unwrap_or_default();
        let arcadia_ctx = self.symbolic.retrieve_context(query);

        let prompt = format!(
            "Contexte MBSE : {}\nContexte Doc : {}\nDemande : {}",
            arcadia_ctx, rag_ctx, query
        );

        let response = if let Some(ref shared_llm) = self.llm_native {
            let mut llm = shared_llm.lock().await;
            llm.generate("Tu es un expert Arcadia.", &prompt, 512)
                .await?
        } else {
            self.llm_remote
                .ask(
                    LlmBackend::GoogleGemini,
                    "Tu es un expert Arcadia.",
                    &prompt,
                    Clearance::Public,
                )
                .await?
        };

        self.session.add_ai_message(&response);
        let _ = self
            .memory_store
            .save_session(&manager, &self.session)
            .await;

        Ok(response)
    }

    /// 🎯 Extrait l'embedding NLP d'un élément (ou retourne un tenseur neutre Zéro Dette)
    pub async fn get_cached_embedding(&self, element: &ArcadiaElement) -> RaiseResult<Vec<f32>> {
        if let Some(val) = element.properties.get("nlp_embedding") {
            if let Ok(vec) = json::deserialize_from_value::<Vec<f32>>(val.clone()) {
                if vec.len() == 384 {
                    return Ok(vec);
                }
            }
        }
        // Fallback sûr : Un tenseur de zéros (384 dimensions pour BGE-Small)
        // Permet au système de tourner même sur des bases de données fraîchement importées
        Ok(vec![0.0f32; 384])
    }

    /// Apprentissage par renforcement du World Model Arcadia.
    pub async fn reinforce_learning(
        &self,
        state_before: &ArcadiaElement,
        intent: CommandType,
        state_after: &ArcadiaElement,
    ) -> RaiseResult<f64> {
        let device = ComputeHardware::Cpu;

        // 1. Récupération des embeddings NLP mis en cache
        let nlp_before = self.get_cached_embedding(state_before).await?;
        let nlp_after = self.get_cached_embedding(state_after).await?;

        // 2. Encodage Hybride (Struct + NLP)
        let tensor_before =
            self.hybrid_encoder
                .encode_hybrid(state_before, &nlp_before, &device)?;
        let tensor_after = self
            .hybrid_encoder
            .encode_hybrid(state_after, &nlp_after, &device)?;

        // 3. Transformation de l'action
        let action_obj = WorldAction { intent };
        let action_tensor = action_obj.to_tensor(self.world_engine.config.action_dim)?;

        // 4. Entraînement Mathématique Pur (Totalement agnostique)
        let mut trainer = match WorldTrainer::new(&self.world_engine, 0.01) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_WM_TRAINER_INIT", error = e.to_string()),
        };

        let loss = trainer.train_step(&tensor_before, &action_tensor, &tensor_after)?;

        // 5. Persistance du World Model mis à jour
        let manager = CollectionsManager::new(self.storage.as_ref(), &self.space, &self.db_name);
        match self.world_engine.save(&manager).await {
            Ok(_) => (),
            Err(e) => user_error!("ERR_WM_SAVE_FAIL", json_value!({"error": e.to_string()})),
        }

        Ok(loss)
    }

    pub async fn learn_document(&mut self, content: &str, source: &str) -> RaiseResult<usize> {
        let app_config = AppConfig::get();
        let storage_arc = self.storage.clone();

        let manager = CollectionsManager::new(
            storage_arc.as_ref(),
            &app_config.mount_points.system.domain,
            &app_config.mount_points.system.db,
        );
        self.rag.index_document(&manager, content, source).await
    }

    pub async fn clear_history(&mut self) -> RaiseResult<()> {
        self.session = ConversationSession::new(self.session.id.clone());
        let app_config = AppConfig::get();
        let storage_arc = self.storage.clone();

        let manager = CollectionsManager::new(
            storage_arc.as_ref(),
            &app_config.mount_points.system.domain,
            &app_config.mount_points.system.db,
        );
        let _ = self
            .memory_store
            .save_session(&manager, &self.session)
            .await;
        Ok(())
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation Mount Points & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::NameType;
    use crate::utils::testing::*;

    fn get_hf_lock() -> &'static AsyncMutex<()> {
        static LOCK: StaticCell<AsyncMutex<()>> = StaticCell::new();
        LOCK.get_or_init(|| AsyncMutex::new(()))
    }

    fn make_element(id: &str) -> ArcadiaElement {
        ArcadiaElement {
            id: id.to_string(),
            name: NameType::default(),
            kind: "https://raise.io/ontology/arcadia/la#LogicalFunction".to_string(),
            properties: UnorderedMap::new(),
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_orchestrator_lifecycle() -> RaiseResult<()> {
        let _guard = get_hf_lock().lock().await;

        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // 1. TEST D'INITIALISATION RÉSILIENTE
        let mut orch =
            AiOrchestrator::new(ProjectModel::default(), &manager, sandbox.db.clone(), None)
                .await?;
        assert_eq!(orch.session.id, "main_session");

        // 2. TEST DE L'APPRENTISSAGE RAG (Persistance DB)
        let content = "RAISE fusionne MBSE et Deep Learning.";
        let res = orch.learn_document(content, "doc.txt").await?;
        assert!(res > 0);

        // 3. TEST DU WORLD MODEL (Apprentissage Renforcé)
        let loss = orch
            .reinforce_learning(&make_element("1"), CommandType::Create, &make_element("2"))
            .await?;
        assert!(loss >= 0.0);

        // 4. TEST DE NETTOYAGE D'HISTORIQUE
        orch.session.add_user_message("Test");
        orch.clear_history().await?;
        assert_eq!(orch.session.history.len(), 0);

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_orchestrator_wm_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;

        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // Création d'un fichier Safetensors invalide (corrompu)
        let wm_dir = sandbox
            .db
            .config
            .db_root(
                &config.mount_points.system.domain,
                &config.mount_points.system.db,
            )
            .join("tensors/world_model");
        fs::ensure_dir_async(&wm_dir).await?;
        fs::write_async(wm_dir.join("world_model.safetensors"), b"CORRUPTED_DATA").await?;

        let orch = AiOrchestrator::new(ProjectModel::default(), &manager, sandbox.db.clone(), None)
            .await?;
        assert!(orch.world_engine.config.vocab_size > 0);

        Ok(())
    }
}
