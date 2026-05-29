// FICHIER : src-tauri/tools/raise-cli/src/commands/ai.rs

use clap::{Args, Subcommand};
use raise_core::{user_error, user_info, user_success, utils::prelude::*};

// --- IMPORTS MÉTIER RAISE ---
use raise_core::ai::agents::intent_classifier::{EngineeringIntent, IntentClassifier};
use raise_core::ai::agents::tools::query_knowledge_graph;
use raise_core::ai::agents::{dynamic_agent::DynamicAgent, Agent, AgentContext};
use raise_core::json_db::collections::manager::CollectionsManager;

use raise_core::ai::context::rag::RagRetriever;
use raise_core::ai::llm::client::LlmClient;
use raise_core::ai::nlp::parser::CommandType;
use raise_core::ai::orchestrator::AiOrchestrator;
use raise_core::ai::training::ai_train_domain_native;
use raise_core::ai::voice::stt::WhisperEngine;
use raise_core::model_engine::types::ProjectModel;
use raise_core::model_engine::types::{ArcadiaElement, NameType};
use raise_core::utils::data::json::Clearance;
use raise_core::utils::io::audio::AudioListener;

use raise_core::ai::agents::prompt_engine::PromptEngine;
use raise_core::ai::agents::tools::extract_json_from_llm;
use raise_core::ai::assurance::health::RaiseHealthEngine;
use raise_core::services::ai_service::validate_arcadia_gnn;
use raise_core::services::model_service::ingest_arcadia_elements;

use crate::CliContext;

#[derive(Args, Debug, Clone)]
pub struct AiArgs {
    #[command(subcommand)]
    pub command: Option<AiCommands>,
}

#[derive(Subcommand, Debug, Clone)]
pub enum AiCommands {
    /// Mode interactif avec le cerveau RAISE
    #[command(visible_alias = "i")]
    Interactive,

    #[command(visible_alias = "l")]
    Listen,

    /// Classifier une intention et éventuellement l'exécuter
    #[command(visible_alias = "x")]
    Classify {
        input: String,
        #[arg(long, short = 'x')]
        execute: bool,
    },

    /// 🔍 Inspecter un agent et son prompt lié
    #[command(visible_alias = "view")]
    Inspect {
        /// Référence de l'agent (ex: 'ref:agents:handle:agent_alpha_planner')
        reference: String,
    },

    /// 🧠 Entraîne un adaptateur LoRA pour un domaine spécifique en local
    #[command(visible_alias = "t")]
    Train {
        /// Forcer le domaine à entraîner (écrase la config utilisateur)
        #[arg(short, long)]
        domain: Option<String>,

        /// Forcer la DB à utiliser
        #[arg(long)]
        db: Option<String>,

        /// Forcer le nombre d'époques (ex: 5)
        #[arg(short, long)]
        epochs: Option<usize>,

        /// Forcer le taux d'apprentissage (ex: 0.001)
        #[arg(short, long)]
        lr: Option<f64>,
    },

    /// 🕸️ Valide mathématiquement une allocation MBSE via le GNN
    #[command(visible_alias = "v")]
    Validate {
        /// URI du premier composant (ex: la:Function_A)
        uri_a: String,
        /// URI du second composant (ex: sa:System_B)
        uri_b: String,
    },

    #[command(visible_alias = "r")]
    Rag {
        #[command(subcommand)]
        action: RagAction,
    },

    #[command(visible_alias = "a")]
    Ask {
        /// La question ou demande directe
        query: String,
    },

    /// 🧠 Déléguer une demande complexe à l'Orchestrateur Multi-Agents
    #[command(visible_alias = "o")]
    Orchestrate {
        /// La demande métier (ex: "Conçois le système de freinage")
        prompt: String,
    },

    /// 🌍 Entraîne le Moteur Neuro-Symbolique (World Model)
    #[command(visible_alias = "tw")]
    TrainWorld {
        /// Nombre d'itérations d'entraînement
        #[arg(short, long, default_value = "50")]
        iterations: usize,
    },

    /// 🚀 Exécuter un prompt stocké dans la base (Data-Driven)
    #[command(visible_alias = "e")]
    Execute {
        /// Le Handle du prompt à exécuter (ex: prompt_mandate2oa_v1)
        prompt_handle: String,

        /// Variables d'injection (format JSON, ex: '{"user_intent": "..."}')
        #[arg(long)]
        vars: Option<String>,

        /// Fichier de sortie optionnel pour sauvegarder la réponse
        #[arg(long)]
        out: Option<String>,

        /// 🎯 NOUVEAU : Ingestion automatique dans la base de données cible
        #[arg(short, long)]
        ingest: bool,
    },

    /// 🔎 Expliquer une décision de l'IA (XAI)
    #[command(visible_alias = "xai")]
    Explain {
        /// L'ID de la trame XAI (XaiFrame) à analyser
        target_id: String,
    },

    /// 🩺 Afficher l'état de santé du moteur IA (Hardware, Assets)
    #[command(visible_alias = "h")]
    Health,

    #[command(visible_alias = "a-check")]
    Audit {
        /// Identifiant du domaine à auditer (ex: 'modeling')
        #[arg(short, long)]
        domain: Option<String>,
    },
}

#[derive(Subcommand, Debug, Clone)]
pub enum RagAction {
    /// 📥 Ingeste un document (texte) dans la base vectorielle locale
    #[command(visible_alias = "i")]
    Ingest {
        /// Chemin vers le fichier à ingérer
        path: String,
    },

    /// 🔍 Interroge la base de connaissances
    #[command(visible_alias = "q")]
    Query {
        /// La question à poser au RAG
        question: String,

        /// Nombre maximum de documents à récupérer pour le contexte (Top K)
        #[arg(short = 'k', long, default_value = "3")]
        top_k: usize,
    },
}

pub async fn handle(args: AiArgs, ctx: CliContext) -> RaiseResult<()> {
    // 1. GESTION DE SESSION OBLIGATOIRE (Heartbeat global pour toutes les commandes)
    let _ = ctx.session_mgr.touch().await;

    let domain_path = match ctx.config.get_path("PATH_RAISE_DOMAIN") {
        Some(path) => path,
        None => raise_error!(
            "CLI_MISSING_DOMAIN_PATH",
            error = "Le chemin PATH_RAISE_DOMAIN est introuvable !",
            context = json_value!({"required_for": "ai_assets_and_db_access"})
        ),
    };

    let orch_ref = match &ctx.kernel.orchestrator {
        Some(o) => o,
        None => raise_error!(
            "ERR_AI_OFFLINE",
            error = "L'orchestrateur IA n'est pas initialisé.",
            context = json_value!({"hint": "Vérifiez la partition système et les assets IA."})
        ),
    };

    let dataset_path = ctx
        .config
        .get_path("PATH_RAISE_DATASET")
        .unwrap_or_else(|| domain_path.join("dataset"));

    fs::ensure_dir_async(&domain_path).await?;
    let storage = ctx.storage.clone();

    let command = args.command.unwrap_or(AiCommands::Interactive);

    // 2. EXÉCUTION DES COMMANDES SANS LLM (GNN et Entraînement Local)
    match &command {
        AiCommands::Train {
            domain,
            db: target_db,
            epochs,
            lr,
        } => {
            let final_domain = domain.clone().unwrap_or_else(|| ctx.active_domain.clone());
            let final_db = target_db.clone().unwrap_or_else(|| ctx.active_db.clone());
            let final_epochs = epochs.unwrap_or(3);
            let final_lr = lr.unwrap_or(0.001);

            user_info!(
                "AI_TRAINING_START",
                json_value!({ "domain": final_domain, "db": final_db, "lr": final_lr, "epochs": final_epochs })
            );

            let manager = raise_core::json_db::collections::manager::CollectionsManager::new(
                &storage,
                &ctx.active_domain,
                &final_db,
            );

            match ai_train_domain_native(&manager, &final_domain, final_epochs, final_lr).await {
                Ok(msg) => user_success!("AI_TRAIN_SUCCESS", json_value!({ "result": msg })),
                Err(e) => user_error!(
                    "AI_TRAIN_FAIL",
                    json_value!({ "error": e.to_string(), "action": "neural_network_training" })
                ),
            }
            return Ok(());
        }
        AiCommands::Validate { uri_a, uri_b } => {
            run_gnn_validation(&domain_path, uri_a, uri_b).await?;
            return Ok(());
        }
        _ => {}
    }

    // 3. CHARGEMENT TARDIF DU LLM ET DU CONTEXTE AGENT (Mode Interactif & NLP)
    let manager = raise_core::json_db::collections::manager::CollectionsManager::new(
        &storage,
        &ctx.active_domain,
        &ctx.active_db,
    );

    if let AiCommands::Rag { action } = &command {
        run_rag_action(domain_path.clone(), &manager, action.clone()).await?;
        return Ok(());
    }

    let client = LlmClient::new(&manager, storage.clone(), ctx.kernel.native_llm.clone()).await?;

    let current_session = ctx.session_mgr.get_current_session().await;
    let session_id = current_session
        .as_ref()
        .map(|s| s.id.clone())
        .unwrap_or_else(|| "cli_session".to_string());

    // 1. On crée un manager pointant sur la partition système
    let sys_manager = raise_core::json_db::collections::manager::CollectionsManager::new(
        &storage,
        &ctx.config.mount_points.system.domain,
        &ctx.config.mount_points.system.db,
    );
    // 2. On récupère les settings du World Model (Zéro Dette)
    let wm_settings =
        AppConfig::get_runtime_settings(&sys_manager, "ref:components:handle:ai_world_model")
            .await?;
    let wm_config: raise_core::ai::world_model::engine::WorldModelConfig =
        match json::deserialize_from_value(wm_settings) {
            Ok(cfg) => cfg,
            Err(e) => raise_error!("ERR_WM_CONFIG_DESERIALIZE", error = e.to_string()),
        };

    // 🎯 FIX CRITIQUE : Suppression du expect() en production
    let world_engine = match raise_core::ai::world_model::NeuroSymbolicEngine::new_empty(wm_config)
    {
        Ok(engine) => SharedRef::new(engine),
        Err(e) => raise_error!(
            "ERR_WORLD_ENGINE_INIT",
            error = e.to_string(),
            context = json_value!({"action": "initialize_neuro_symbolic_engine"})
        ),
    };

    let agent_ctx = AgentContext::new(
        &ctx.active_user,
        &session_id,
        storage.clone(),
        client.clone(),
        world_engine,
        domain_path.clone(),
        dataset_path,
    )
    .await?;

    // 4. EXÉCUTION DES COMMANDES AGENTS/LLM
    match command {
        AiCommands::Interactive => run_interactive_mode(&agent_ctx, &ctx, client).await?,
        AiCommands::Listen => {
            let health = RaiseHealthEngine::check_engine_health(&manager).await?;
            if !health.acceleration_active {
                user_warn!(
                    "AI_VOICE_PERF_LOW",
                    json_value!({"hint": "Whisper sans GPU risque de bloquer le CLI."})
                );
            }
            run_voice_mode(&agent_ctx, client, &manager).await?
        }
        AiCommands::Classify { input, execute } => {
            process_input(&agent_ctx, &input, client, execute).await
        }
        AiCommands::Inspect { reference } => {
            inspect_agent_logic(&agent_ctx, &reference, &ctx.active_domain, &ctx.active_db).await?;
        }

        AiCommands::Ask { query } => {
            // On verrouille l'orchestrateur partagé injecté par main.rs
            let mut orch = orch_ref.lock().await;

            // Appel à l'interface simplifiée "ask" du noyau
            match orch.ask(&query).await {
                Ok(response) => {
                    println!("\n🤖 RAISE :\n{}", response);
                    user_success!("AI_ASK_SUCCESS");
                }
                Err(e) => raise_error!(
                    "AI_ASK_FAILED",
                    error = e,
                    context = json_value!({"query": &query})
                ),
            }
        }

        AiCommands::Orchestrate { prompt } => {
            let mut orch = orch_ref.lock().await;
            match orch.execute_workflow(&prompt).await {
                Ok(res) => {
                    println!("{}", res.message);
                    user_success!(
                        "AI_ORCHESTRATOR_SUCCESS",
                        json_value!({"artifacts": res.artifacts.len()})
                    );
                }
                Err(e) => raise_error!("AI_ORCHESTRATOR_FAILED", error = e),
            }
        }

        AiCommands::Audit { domain } => {
            let target_domain = domain.unwrap_or_else(|| ctx.active_domain.clone());
            user_info!("AI_AUDIT_START", json_value!({"domain": &target_domain}));

            // 1. Initialisation du manager sur la partition système pour la persistance
            let sys_manager = CollectionsManager::new(
                &ctx.storage,
                &ctx.config.mount_points.system.domain,
                &ctx.config.mount_points.system.db,
            );

            // 2. Création d'un rapport de qualité (utilisant les moteurs du noyau)
            let report =
                raise_core::ai::assurance::QualityReport::new(&target_domain, &ctx.active_db);

            // 3. Persistance du rapport via le module d'assurance
            match raise_core::ai::assurance::persistence::save_quality_report(&sys_manager, &report)
                .await
            {
                Ok(id) => {
                    user_success!(
                        "AI_AUDIT_SUCCESS",
                        json_value!({
                            "report_id": id,
                            "domain": &target_domain
                        })
                    );
                    println!("✅ Rapport d'assurance qualité persisté dans 'quality_reports'.");
                }
                Err(e) => raise_error!("AI_AUDIT_FAILED", error = e),
            }
        }

        AiCommands::Explain { target_id } => {
            let config = AppConfig::get();
            let sys_manager = CollectionsManager::new(
                &ctx.storage,
                &config.mount_points.system.domain,
                &config.mount_points.system.db,
            );

            // 1. Récupération de la trame via le noyau certifié
            match raise_core::ai::assurance::get_xai_frame(&sys_manager, &target_id).await {
                Ok(frame) => {
                    user_success!("AI_EXPLAIN_FOUND", json_value!({"id": target_id}));

                    // 2. Affichage structuré du raisonnement
                    println!("\n🔍 RAISONNEMENT DE L'AGENT :");
                    println!("============================");
                    println!("{}", frame.summarize_for_llm());

                    if !frame.visual_artifacts.is_empty() {
                        println!(
                            "\n🖼️  ARTEFACTS VISUELS DISPONIBLES : {}",
                            frame.visual_artifacts.len()
                        );
                    }
                }
                Err(e) => user_error!("AI_EXPLAIN_FAILED", json_value!({ "error": e.to_string() })),
            }
        }

        AiCommands::Health => {
            let config = AppConfig::get();
            let sys_manager = CollectionsManager::new(
                &ctx.storage,
                &config.mount_points.system.domain,
                &config.mount_points.system.db,
            );

            match raise_core::ai::assurance::health::RaiseHealthEngine::check_engine_health(
                &sys_manager,
            )
            .await
            {
                Ok(report) => {
                    user_success!("AI_HEALTH_REPORT", json_value!(report));
                    println!("\n🩺 RAPPORT DE SANTÉ IA");
                    println!("======================");
                    println!("Matériel     : {}", report.device_type);
                    println!(
                        "Accélération : {}",
                        if report.acceleration_active {
                            "OUI (Active)"
                        } else {
                            "NON (CPU)"
                        }
                    );
                    println!(
                        "Actifs IA    : {}",
                        if report.assets_integrity {
                            "✅ Intègres"
                        } else {
                            "❌ Manquants/Corrompus"
                        }
                    );
                }
                Err(e) => user_error!("AI_HEALTH_FAILED", json_value!({ "error": e.to_string() })),
            }
        }

        AiCommands::TrainWorld { iterations } => {
            run_train_world_action(ctx.storage.clone(), &manager, iterations).await?;
        }

        AiCommands::Execute {
            prompt_handle,
            vars,
            out,
            ingest,
        } => {
            run_execute_action(&ctx, client, &prompt_handle, vars, out, ingest).await?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

async fn run_interactive_mode(
    ctx: &AgentContext,
    cli_ctx: &CliContext,
    client: LlmClient,
) -> RaiseResult<()> {
    user_info!("AI_INTERACTIVE_WELCOME", json_value!({}));
    user_info!("AI_INTERACTIVE_SEPARATOR", json_value!({}));
    user_info!("AI_LLM_CONNECTED", json_value!({"mode": "local"}));
    user_info!(
        "AI_STORAGE_PATH",
        json_value!({ "path": ctx.paths.domain_root })
    );
    user_info!("AI_EXIT_HINT", json_value!({}));
    let prompt = format!(
        "RAISE-AI [{}@{}/{}]> ",
        cli_ctx.active_user, cli_ctx.active_domain, cli_ctx.active_db
    );
    loop {
        print!("{}", prompt);
        os::flush_stdout()?;
        let input = os::read_stdin_line()?;

        if input.eq_ignore_ascii_case("exit") {
            user_info!("AI_GOODBYE", json_value!({}));
            break;
        }
        if input.is_empty() {
            continue;
        }

        process_input(ctx, &input, client.clone(), true).await;
    }
    Ok(())
}

async fn run_rag_action(
    domain_path: PathBuf,
    manager: &raise_core::json_db::collections::manager::CollectionsManager<'_>,
    action: RagAction,
) -> RaiseResult<()> {
    let mut rag_engine = RagRetriever::new_internal(domain_path, manager).await?;

    match action {
        RagAction::Ingest { path } => {
            let target_path = PathBuf::from(&path);
            user_info!("RAG_INGESTION_START", json_value!({"path": path}));

            if !target_path.exists() {
                raise_error!(
                    "RAG_FILE_NOT_FOUND",
                    error = "Le fichier ou dossier spécifié n'existe pas.",
                    context = json_value!({"path": path})
                );
            }

            let content = fs::read_to_string_async(&target_path).await?;
            let source_name = target_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            match rag_engine
                .index_document(manager, &content, &source_name)
                .await
            {
                Ok(chunks_count) => {
                    user_success!(
                        "RAG_INGESTION_SUCCESS",
                        json_value!({ "chunks_indexed": chunks_count, "source": source_name })
                    );
                }
                Err(e) => {
                    user_error!(
                        "RAG_INGESTION_FAILED",
                        json_value!({ "error": e.to_string(), "path": path })
                    );
                }
            }
        }

        RagAction::Query { question, top_k } => {
            user_info!(
                "RAG_QUERY_START",
                json_value!({"question": question, "limit": top_k})
            );

            match rag_engine.retrieve(manager, &question, top_k as u64).await {
                Ok(context_str) => {
                    if context_str.is_empty() {
                        user_warn!(
                            "RAG_NO_CONTEXT_FOUND",
                            json_value!({"hint": "Aucun document pertinent n'atteint le seuil de similarité (0.65)."})
                        );
                    } else {
                        println!("\n{}", context_str);
                        user_success!("RAG_QUERY_SUCCESS", json_value!({}));
                    }
                }
                Err(e) => {
                    user_error!("RAG_QUERY_FAILED", json_value!({ "error": e.to_string() }));
                }
            }
        }
    }

    Ok(())
}

async fn run_voice_mode(
    ctx: &AgentContext,
    client: LlmClient,
    manager: &raise_core::json_db::collections::manager::CollectionsManager<'_>,
) -> RaiseResult<()> {
    user_info!("AI_VOICE_INIT", json_value!({"status": "loading"}));
    println!("⏳ Chargement du modèle vocal Whisper (Hors-ligne)...");

    let mut engine = WhisperEngine::new(manager).await?;
    let (_listener, mut rx) = AudioListener::start()?;

    user_success!(
        "AI_VOICE_READY",
        json_value!({"message": "Microphone activé."})
    );

    println!("--------------------------------------------------");
    println!("🎤 Le micro est ouvert. Parlez naturellement !");
    println!("   (Appuyez sur Ctrl+C pour quitter)");
    println!("--------------------------------------------------\n");

    while let Some(audio_chunk) = rx.recv().await {
        print!("⏳ Transcription en cours... ");
        let _ = os::flush_stdout();

        match engine.transcribe(&audio_chunk) {
            Ok(text) => {
                let text = text.trim();
                if text.is_empty() {
                    println!("\r🎤 Écoute en cours...               ");
                    continue;
                }

                println!("\r🗣️  Vous : {}                    ", text);
                user_info!("AI_VOICE_HEARD", json_value!({"text": text}));

                process_input(ctx, text, client.clone(), true).await;

                println!("\n🎤 Écoute en cours...");
            }
            Err(e) => {
                user_error!("AI_VOICE_ERROR", json_value!({"error": e.to_string()}));
                println!("\r❌ Erreur de transcription : {}               ", e);
                println!("\n🎤 Écoute en cours...");
            }
        }
    }

    Ok(())
}

async fn process_input(ctx: &AgentContext, input: &str, client: LlmClient, execute: bool) {
    let classifier = IntentClassifier::new(client);
    user_info!("AI_ANALYZING", json_value!({"input_length": input.len()}));

    let intent = classifier.classify(input).await;
    let target_agent_urn = intent.recommended_agent_id();

    user_info!(
        "AI_AGENT_START",
        json_value!({ "agent": target_agent_urn, "intent": format!("{:?}", intent) })
    );

    let agent = DynamicAgent::new(target_agent_urn);
    run_agent(agent, ctx, &intent, execute).await;
}

async fn run_agent<A: Agent>(
    agent: A,
    ctx: &AgentContext,
    intent: &EngineeringIntent,
    execute: bool,
) {
    if !execute {
        user_info!("AI_SIMULATION_MODE", json_value!({}));
        return;
    }

    // 🎯 Le CLI ne fait plus aucun traitement : il passe les plats au noyau certifié
    match raise_core::ai::assurance::execute_certified(&agent, ctx, intent).await {
        Ok(Some(res)) => {
            user_success!("AI_RESULT", json_value!({ "message": res.message }));
            for a in res.artifacts {
                user_info!("AI_ARTIFACT_GENERATED", json_value!({ "path": a.path }));
            }
        }
        Ok(None) => user_info!("AI_NO_ACTION", json_value!({})),
        Err(e) => user_error!("AI_AGENT_ERROR", json_value!({ "error": e.to_string() })),
    }
}

async fn run_gnn_validation(domain_path: &Path, uri_a: &str, uri_b: &str) -> RaiseResult<()> {
    let root_path_str = domain_path.to_string_lossy().to_string();

    let result = validate_arcadia_gnn(&root_path_str, uri_a, uri_b).await?;

    let metrics = &result["metrics"];
    let sim_initial = metrics["nlp_similarity"].as_f64().unwrap_or(0.0);
    let sim_final = metrics["gnn_similarity"].as_f64().unwrap_or(0.0);
    let delta = metrics["improvement"].as_f64().unwrap_or(0.0);
    let confirmed = result["hypothesis_confirmed"].as_bool().unwrap_or(false);

    println!("\n📊 --- RÉSULTAT DE L'EXPÉRIENCE GNN ---");
    println!("Composant A : {}", uri_a);
    println!("Composant B : {}", uri_b);
    println!("Similarité Sémantique (NLP pur) : {:.4}", sim_initial);
    println!("Similarité Structurelle (GNN)   : {:.4}", sim_final);

    if confirmed {
        user_success!(
            "✅ [MBSE] Hypothèse confirmée",
            json_value!({"improvement_pct": delta * 100.0})
        );
        println!(
            "Conclusion : La topologie du système renforce le lien entre ces composants (+{:.2}%).",
            delta * 100.0
        );
    } else {
        user_warn!(
            "⚠️ [MBSE] Hypothèse rejetée",
            json_value!({"improvement_pct": delta * 100.0})
        );
        println!("Conclusion : La structure globale ne justifie pas une allocation forte entre ces composants.");
    }

    Ok(())
}

async fn inspect_agent_logic(
    ctx: &AgentContext,
    reference: &str,
    space: &str,
    db: &str,
) -> RaiseResult<()> {
    user_info!(
        "AI_INSPECT_START",
        json_value!({
            "target": reference,
            "space": space,
            "db": db
        })
    );

    let agent_doc = query_knowledge_graph(ctx, reference, false).await?;

    if let Some(prompt_id) = agent_doc["neuro_profile"]["prompt_id"].as_str() {
        let prompt_doc = query_knowledge_graph(ctx, prompt_id, false).await?;

        let directives_len = prompt_doc["directives"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);

        user_success!(
            "AI_PROMPT_RESOLVED",
            json_value!({
                "persona": prompt_doc["identity"]["persona"],
                "directives_count": directives_len
            })
        );

        println!("\n📝 --- INSTRUCTIONS RÉCUPÉRÉES ---");
        println!("Identité : {}", prompt_doc["identity"]["persona"]);
        if let Some(directives) = prompt_doc["directives"].as_array() {
            for (i, d) in directives.iter().enumerate() {
                println!("  {}. {}", i + 1, d.as_str().unwrap_or(""));
            }
        }
    }
    Ok(())
}

async fn run_train_world_action(
    storage: SharedRef<raise_core::json_db::storage::StorageEngine>,
    manager: &raise_core::json_db::collections::manager::CollectionsManager<'_>,
    iterations: usize,
) -> RaiseResult<()> {
    user_info!(
        "AI_WORLD_TRAIN_START",
        json_value!({"iterations": iterations})
    );

    println!("⏳ Réveil de l'Orchestrateur et du Moteur Neuro-Symbolique...");
    let orchestrator = AiOrchestrator::new(ProjectModel::default(), manager, storage, None).await?;

    println!("\n🌍 --- ENTRAÎNEMENT DU WORLD MODEL (NEURO-SYMBOLIQUE) ---");
    println!("🧠 Scénario : Apprentissage de la transition d'un composant Logique (LA) vers Physique (PA).");

    let state_before = ArcadiaElement {
        id: "comp_logic_1".into(),
        name: NameType::default(),
        kind: "https://raise.io/ontology/arcadia/la#LogicalComponent".into(),
        properties: raise_core::utils::data::UnorderedMap::new(),
    };

    let state_after = ArcadiaElement {
        id: "comp_phys_1".into(),
        name: NameType::default(),
        kind: "https://raise.io/ontology/arcadia/pa#PhysicalComponent".into(),
        properties: raise_core::utils::data::UnorderedMap::new(),
    };

    let mut initial_loss = 0.0;
    let mut final_loss = 0.0;

    for i in 1..=iterations {
        let loss = orchestrator
            .reinforce_learning(&state_before, CommandType::Create, &state_after)
            .await?;

        if i == 1 {
            initial_loss = loss;
            println!("📉 Loss initiale (Itération 1) : {:.6}", initial_loss);
        } else if i % 10 == 0 || i == iterations {
            println!("📉 Loss (Itération {:>2}) : {:.6}", i, loss);
        }
        final_loss = loss;
    }

    println!("\n✅ Entraînement terminé !");
    println!("   Loss Initiale : {:.6}", initial_loss);
    println!("   Loss Finale   : {:.6}", final_loss);

    if initial_loss > 0.0 {
        let improvement = ((initial_loss - final_loss) / initial_loss) * 100.0;
        println!("   Amélioration  : {:.2}%", improvement);
    }

    user_success!(
        "AI_WORLD_TRAIN_SUCCESS",
        json_value!({"final_loss": final_loss})
    );

    Ok(())
}

async fn run_execute_action(
    ctx: &CliContext,
    client: LlmClient,
    prompt_handle: &str,
    vars_json: Option<String>,
    out_path: Option<String>,
    ingest: bool,
) -> RaiseResult<()> {
    user_info!("AI_EXECUTE_START", json_value!({"prompt": prompt_handle}));

    let vars: Option<JsonValue> = if let Some(s) = vars_json {
        Some(json::deserialize_from_str(&s)?)
    } else {
        None
    };

    let prompt_engine = PromptEngine::new(ctx.storage.clone(), &ctx.active_domain, &ctx.active_db);

    user_info!(
        "AI_PROMPT_COMPILING",
        json_value!({"handle": prompt_handle})
    );
    let system_prompt = prompt_engine.compile(prompt_handle, vars.as_ref()).await?;

    println!("🤖 Inférence RAISE en cours ({})...", prompt_handle);
    let response = client
        .ask(
            raise_core::ai::llm::client::LlmBackend::LocalLlama,
            &system_prompt,
            "",
            Clearance::Internal,
        )
        .await?;

    let clean_json = extract_json_from_llm(&response);

    if ingest {
        println!("📥 Routage ontologique et ingestion dans le Graphe Arcadia...");
        let ids = ingest_arcadia_elements(
            &ctx.storage,
            &ctx.active_domain,
            &ctx.active_db,
            &clean_json,
        )
        .await?;
        println!(
            "✅ {} entités validées et sauvegardées avec succès !",
            ids.len()
        );
    }

    match out_path {
        Some(p) => {
            let path = PathBuf::from(&p);
            fs::write_async(&path, clean_json).await?;
            user_success!("AI_EXECUTE_SUCCESS", json_value!({"out_file": p}));
            println!("✅ Artefact JSON brut sauvegardé dans : {}", p);
        }
        None => {
            if !ingest {
                println!("\n📦 --- RÉSULTAT DU BLUEPRINT ---");
                println!("{}", clean_json);
            }
            user_success!("AI_EXECUTE_SUCCESS", json_value!({}));
        }
    }

    Ok(())
}

// --- TESTS UNITAIRES ---
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use raise_core::utils::testing::*;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: AiArgs,
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_parsing_robustness() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let cli = match TestCli::try_parse_from(vec!["test"]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };
        assert!(cli.args.command.is_none());

        let cli = match TestCli::try_parse_from(vec![
            "test",
            "classify",
            "créer un composant SA",
            "--execute",
        ]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Classify { input, execute }) = cli.args.command {
            assert_eq!(input, "créer un composant SA");
            assert!(execute);
            Ok(())
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de la commande Classify"
            )
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_intent_dispatch_layers() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let test_cases = vec![
            ("SA", "ref:agents:handle:agent_system"),
            ("PA", "ref:agents:handle:agent_hardware"),
            ("DATA", "ref:agents:handle:agent_data"),
            ("TRANSVERSE", "ref:agents:handle:agent_quality"),
            ("EPBS", "ref:agents:handle:agent_epbs"),
        ];

        for (layer, expected_urn) in test_cases {
            let intent = EngineeringIntent::CreateElement {
                layer: layer.to_string(),
                element_type: "Component".into(),
                name: "TestUnit".into(),
            };

            assert_eq!(intent.recommended_agent_id(), expected_urn);
        }
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_intent_dispatch_software_logic() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let intent_la = EngineeringIntent::CreateElement {
            layer: "LA".into(),
            element_type: "LogicalComponent".into(),
            name: "Test".into(),
        };

        assert_eq!(
            intent_la.recommended_agent_id(),
            "ref:agents:handle:agent_software"
        );
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_business_dispatch() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let intent = EngineeringIntent::DefineBusinessUseCase {
            domain: "Aéronautique".into(),
            process_name: "Gestion Flux".into(),
            description: "Flux passagers".into(),
        };

        assert_eq!(
            intent.recommended_agent_id(),
            "ref:agents:handle:agent_business"
        );
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_train_parsing() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let cli = match TestCli::try_parse_from(vec![
            "test", "train", "--domain", "safety", "--epochs", "10",
        ]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Train {
            domain,
            epochs,
            db,
            lr,
        }) = cli.args.command
        {
            assert_eq!(domain.unwrap(), "safety");
            assert_eq!(epochs.unwrap(), 10);
            assert!(db.is_none());
            assert!(lr.is_none());
            Ok(())
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de la commande Train"
            )
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_listen_parsing() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        let cli = match TestCli::try_parse_from(vec!["test", "listen"]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Listen) = cli.args.command {
            // OK
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de la commande complète 'listen'"
            )
        }

        let cli_alias = match TestCli::try_parse_from(vec!["test", "l"]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Listen) = cli_alias.args.command {
            Ok(())
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de l'alias 'l'"
            )
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_ask_parsing() -> RaiseResult<()> {
        mock::inject_mock_config().await;

        // 1. Test de la commande complète
        let cli = match TestCli::try_parse_from(vec![
            "test",
            "ask",
            "quelle est la charge du moteur ?",
        ]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Ask { query }) = cli.args.command {
            assert_eq!(query, "quelle est la charge du moteur ?");
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de 'ask'"
            );
        }

        // 2. Test de l'alias 'a'
        let cli_alias = match TestCli::try_parse_from(vec!["test", "a", "status"]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let Some(AiCommands::Ask { query }) = cli_alias.args.command {
            assert_eq!(query, "status");
            Ok(())
        } else {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Échec du parsing de l'alias 'a'"
            )
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_ai_ask_execution_offline_safety() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());

        // 🎯 On utilise le mock de CliContext qui initialise orchestrator à None
        let ctx = CliContext::mock(
            AppConfig::get(),
            raise_core::utils::context::SessionManager::new(storage.clone()),
            storage,
        );

        let args = AiArgs {
            command: Some(AiCommands::Ask {
                query: "vériification".into(),
            }),
        };

        // 🎯 L'exécution doit renvoyer l'erreur ERR_AI_OFFLINE de manière structurée
        match handle(args, ctx).await {
            Err(AppError::Structured(err)) if err.code == "ERR_AI_OFFLINE" => Ok(()),
            _ => raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Le handler aurait dû rejeter l'appel car l'orchestrateur est absent."
            ),
        }
    }
}
