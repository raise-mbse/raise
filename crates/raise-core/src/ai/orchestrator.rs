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

// 🎯 NOUVEAU : Le Runner autonome qui exécute la boucle ACL sans bloquer l'Orchestrateur
#[derive(Clone)]
pub struct SquadRunner {
    pub llm_remote: LlmClient,
    pub world_engine: SharedRef<NeuroSymbolicEngine>,
    pub storage: SharedRef<StorageEngine>,
}

impl SquadRunner {
    pub async fn execute_workflow(&self, user_query: &str) -> RaiseResult<AgentResult> {
        let app_config = AppConfig::get();
        let storage_arc = self.storage.clone();

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
}

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
            Ok(engine) if engine.config.embedding_dim == 36 => engine, // Cas parfait
            Ok(obsolete_engine) => {
                // 🎯 AUTO-GUÉRISON : Le modèle existant est d'une ancienne version
                crate::user_warn!(
                    "WRN_OBSOLETE_BRAIN_DETECTED",
                    json_value!({
                        "detected_dim": obsolete_engine.config.embedding_dim,
                        "expected_dim": 36,
                        "action": "AUTO_HEALING",
                        "hint": "Le modèle existant est obsolète. Purge et réinitialisation de l'espace latent à 36 dimensions."
                    })
                );

                let active_device = AppConfig::device();
                let wm_config = WorldModelConfig {
                    vocab_size: 1000,
                    embedding_dim: 36,
                    action_dim: 5,
                    hidden_dim: 64,
                    use_gpu: active_device.is_cuda() || active_device.is_metal(),
                };

                // On force la création d'un nouveau modèle vierge et sain
                NeuroSymbolicEngine::new_empty(wm_config)?
            }
            Err(e) => {
                // 🎯 AUTO-GUÉRISON : Aucun modèle trouvé ou erreur de lecture
                crate::user_warn!(
                    "WRN_WORLD_MODEL_LOAD_FAILED",
                    json_value!({ "error": e.to_string(), "hint": "Démarrage avec un modèle vierge." })
                );

                let active_device = AppConfig::device();
                let wm_config = WorldModelConfig {
                    vocab_size: 1000,
                    embedding_dim: 36,
                    action_dim: 5,
                    hidden_dim: 64,
                    use_gpu: active_device.is_cuda() || active_device.is_metal(),
                };

                NeuroSymbolicEngine::new_empty(wm_config)?
            }
        };

        // 🎯 Initialisation de l'HybridEncoder (On lie ses poids à la varmap du WorldModel !)
        let active_device = AppConfig::device(); // 🎯 FIX : Résolution dynamique
        let vb = NeuralWeightsBuilder::from_varmap(
            &world_engine.varmap,
            ComputeType::F32,
            active_device,
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

    ///  Fournit une instance isolée pour les Handlers
    pub fn squad_runner(&self) -> SquadRunner {
        SquadRunner {
            llm_remote: self.llm_remote.clone(),
            world_engine: self.world_engine.clone(),
            storage: self.storage.clone(),
        }
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
        // 🎯 FIX ABSOLU : Résolution dynamique du device (au lieu de ComputeHardware::Cpu)
        let device = AppConfig::device();

        // 1. Récupération des embeddings NLP mis en cache
        let nlp_before = self.get_cached_embedding(state_before).await?;
        let nlp_after = self.get_cached_embedding(state_after).await?;

        // 2. Encodage Hybride (Struct + NLP)
        let tensor_before = self
            .hybrid_encoder
            .encode_hybrid(state_before, &nlp_before, device)?;
        let tensor_after = self
            .hybrid_encoder
            .encode_hybrid(state_after, &nlp_after, device)?;

        // 3. Transformation de l'action
        let action_obj = WorldAction { intent };
        let action_tensor = action_obj.to_tensor(self.world_engine.config.action_dim, device)?;

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

    /// Orchestre la génération déterministe d'une architecture via le méta-langage (DSL).
    pub async fn generate_architecture(&mut self, query: &str) -> RaiseResult<usize> {
        user_info!("INF_ARCH_GEN_START", json_value!({"query": query}));

        // 1. FAIL-FAST : Extraction sécurisée du moteur local avant toute opération I/O
        let mut local_llm_guard = match &self.llm_native {
        Some(shared_llm) => shared_llm.lock().await,
        None => raise_error!(
            "ERR_NATIVE_LLM_REQUIRED", 
            error = "La génération d'architecture nécessite le moteur local Air-Gap pour la garantie GBNF."
        ),
    };

        // 2. Le Prompt Système (Gabarit Déterministe)
        // 2. Le Prompt Système (Gabarit Déterministe avec Few-Shot Example)
        let system_prompt = "You are the Intent Translator for the R.A.I.S.E. MBSE System. \
        Translate the user's request into the strictly formatted DSL. \
        RULES: \
        1. NO conversational text. Output ONLY the DSL code block. \
        2. TOPOLOGICAL ISOLATION: Use ONLY logical 'handles' (lowercase, numbers, underscores), NEVER UUIDs. \
        3. Strict Syntax: You MUST separate the block keyword and the handle with a space, and the handle MUST be in double quotes.\n\
        \n\
        EXAMPLE FORMAT:\n\
        dapp \"my_app\" {\n\
            type = \"daemon\"\n\
            pvmt {\n\
                frugality_score = 9\n\
            }\n\
            service \"my_service\" {\n\
                runtime {\n\
                    timeout_ms = 5000\n\
                }\n\
                module \"my_module\" {\n\
                    visibility = \"public\"\n\
                }\n\
            }\n\
        }";

        // 3. Chargement de la Grammaire GBNF depuis les assets
        let app_config = AppConfig::get();
        let grammar_path = match app_config.get_path("PATH_RAISE_GRAMMARS") {
            Some(p) => p,
            None => PathBuf::from("crates/raise-core/src/model_engine/dsl/grammar.gbnf"),
        };

        let grammar_str = match fs::read_to_string_async(&grammar_path).await {
            Ok(g) => g,
            Err(e) => raise_error!("ERR_GRAMMAR_LOAD_FAILED", error = e.to_string()),
        };

        // 4. Inférence sous contrainte (Isolation "Air-Gap" via Native Engine)
        let generated_dsl = match local_llm_guard
            .generate_with_grammar(system_prompt, query, 1024, &grammar_str)
            .await
        {
            Ok(res) => res,
            Err(e) => raise_error!("ERR_ARCH_GENERATION_FAILED", error = e.to_string()),
        };

        user_info!(
            "INF_ARCH_GEN_SUCCESS",
            json_value!({"length": generated_dsl.len()})
        );

        // 5. Parsing et Transformation
        use crate::model_engine::dsl::mapper::DslToArcadiaMapper;
        use crate::model_engine::dsl::parser::parse_dsl_text;

        let parsed_ast = match parse_dsl_text(&generated_dsl) {
            Ok(ast) => ast,
            Err(e) => raise_error!("ERR_DSL_AST_PARSING", error = e.to_string()),
        };

        let file_pair = match parsed_ast.into_iter().next() {
            Some(p) => p,
            None => raise_error!("ERR_DSL_AST_EMPTY", error = "L'AST généré est vide."),
        };

        let manager = CollectionsManager::new(
            self.storage.as_ref(),
            &app_config.mount_points.modeling.domain,
            &app_config.mount_points.modeling.db,
        );

        let mapper = DslToArcadiaMapper::new();
        let staging_model = match mapper.transform(file_pair, &manager).await {
            Ok(m) => m,
            Err(e) => raise_error!("ERR_DSL_TRANSFORMATION", error = e.to_string()),
        };

        // 6. Ingestion dans le Jumeau Numérique
        use crate::model_engine::ingestion::ModelIngestionService;
        let inserted_count =
            match ModelIngestionService::persist_model(&staging_model, &manager).await {
                Ok(c) => c,
                Err(e) => raise_error!("ERR_DSL_PERSISTENCE", error = e.to_string()),
            };

        user_success!(
            "SUC_ARCH_PERSISTED",
            json_value!({"elements_created": inserted_count})
        );

        Ok(inserted_count)
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
    use crate::model_engine::types::SlugString;
    use crate::utils::testing::*;

    fn get_hf_lock() -> &'static AsyncMutex<()> {
        static LOCK: StaticCell<AsyncMutex<()>> = StaticCell::new();
        LOCK.get_or_init(|| AsyncMutex::new(()))
    }

    fn make_element(id: &str) -> RaiseResult<ArcadiaElement> {
        Ok(ArcadiaElement {
            handle: SlugString::new(id)?,
            name: I18nString::default(),
            kind: vec!["la:LogicalFunction".to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        })
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

        let before = make_element("1")?;
        let after = make_element("2")?;
        let loss = orch
            .reinforce_learning(&before, CommandType::Create, &after)
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
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_orchestrator_generate_architecture_missing_llm() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // Instanciation de l'orchestrateur SANS LLM natif (llm_native = None)
        let mut orch =
            AiOrchestrator::new(ProjectModel::default(), &manager, sandbox.db.clone(), None)
                .await?;

        // La génération d'architecture doit échouer brutalement car le moteur Air-Gap est requis
        let result = orch.generate_architecture("Crée un service").await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(
                    err.code, "ERR_NATIVE_LLM_REQUIRED",
                    "Doit exiger le LLM local pour le GBNF."
                );
                Ok(())
            }
            _ => panic!("Le système aurait dû lever ERR_NATIVE_LLM_REQUIRED"),
        }
    }
}
