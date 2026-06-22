// FICHIER : src-tauri/tools/raise-cli/src/commands/ai.rs

use clap::{Args, Subcommand};
use raise_core::{user_error, user_info, user_success, utils::prelude::*};

// --- IMPORTS MÉTIER RAISE ---
use raise_core::ai::agents::intent_classifier::EngineeringIntent;
use raise_core::ai::agents::AgentResult;
use raise_core::ai::assurance::health::RaiseHealthEngine;
use raise_core::ai::context::rag::RagRetriever;
use raise_core::ai::voice::stt::WhisperEngine;
use raise_core::json_db::collections::manager::CollectionsManager;
use raise_core::services::ai_service::validate_arcadia_gnn;
use raise_core::utils::io::audio::AudioListener;

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

        /// 🎯 Ingestion automatique dans la base de données cible
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

    /// 🧬 Muter un composant de code via l'IA
    #[command(visible_alias = "m")]
    Mutate {
        /// Le handle sémantique cible (ex: fn:missing_file_context)
        #[arg(long)]
        handle: String,
        /// L'instruction de mutation
        #[arg(long)]
        prompt: String,
    },

    /// 💾 Valide une mutation en staging et l'intègre au code de production
    #[command(visible_alias = "c")]
    Commit {
        /// Le handle du module parent en staging (ex: mod_kernel_assets_rs)
        #[arg(long)]
        handle: String,
    },

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

// 🎯 NOUVELLE FONCTION UTILITAIRE (Affichage CLI uniquement)
fn display_agent_result(intent: &EngineeringIntent, agent_urn: &str, result: Option<AgentResult>) {
    user_info!(
        "AI_ANALYZING",
        json_value!({"intent": format!("{:?}", intent)})
    );
    user_info!("AI_AGENT_START", json_value!({ "agent": agent_urn }));

    match result {
        Some(res) => {
            user_success!("AI_RESULT", json_value!({ "message": res.message }));
            for a in res.artifacts {
                user_info!("AI_ARTIFACT_GENERATED", json_value!({ "path": a.path }));
            }
        }
        None => user_info!("AI_SIMULATION_MODE", json_value!({})),
    }
}

// 🎯 HANDLE ALLÉGÉ : Délégation totale à ai_service.rs
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

    fs::ensure_dir_async(&domain_path).await?;
    let command = args.command.unwrap_or(AiCommands::Interactive);
    let manager = CollectionsManager::new(&ctx.storage, &ctx.active_domain, &ctx.active_db);

    match command {
        AiCommands::Interactive => run_interactive_mode(&ctx).await?,

        AiCommands::Listen => {
            let health = RaiseHealthEngine::check_engine_health(&manager).await?;
            if !health.acceleration_active {
                user_warn!(
                    "AI_VOICE_PERF_LOW",
                    json_value!({"hint": "Whisper sans GPU risque de bloquer le CLI."})
                );
            }
            run_voice_mode(&ctx, &manager).await?
        }

        AiCommands::Classify { input, execute } => {
            match raise_core::services::ai_service::ai_classify_and_execute(
                ctx.storage.clone(),
                ctx.kernel.native_llm.clone(),
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.active_user,
                &input,
                execute,
            )
            .await
            {
                Ok((intent, agent_urn, agent_result)) => {
                    display_agent_result(&intent, &agent_urn, agent_result)
                }
                Err(e) => raise_error!("ERR_AI_CLASSIFY_FAILED", error = e.to_string()),
            }
        }

        AiCommands::Inspect { reference } => {
            user_info!("AI_INSPECT_START", json_value!({"target": reference}));
            match raise_core::services::ai_service::ai_inspect_agent(
                ctx.storage.clone(),
                &ctx.active_domain,
                &ctx.active_db,
                &reference,
            )
            .await
            {
                Ok(data) => {
                    let persona = data["persona"].as_str().unwrap_or("Inconnu");
                    let directives = data["directives"].as_array().unwrap();
                    user_success!(
                        "AI_PROMPT_RESOLVED",
                        json_value!({"persona": persona, "directives_count": directives.len()})
                    );
                    println!("\n📝 --- INSTRUCTIONS RÉCUPÉRÉES ---");
                    println!("Identité : {}", persona);
                    for (i, d) in directives.iter().enumerate() {
                        println!("  {}. {}", i + 1, d.as_str().unwrap_or(""));
                    }
                }
                Err(e) => raise_error!("ERR_AI_INSPECT_FAILED", error = e.to_string()),
            }
        }

        AiCommands::Validate { uri_a, uri_b } => {
            let root_path_str = domain_path.to_string_lossy().to_string();
            let result = validate_arcadia_gnn(&root_path_str, &uri_a, &uri_b).await?;

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
                println!("Conclusion : La topologie du système renforce le lien entre ces composants (+{:.2}%).", delta * 100.0);
            } else {
                user_warn!(
                    "⚠️ [MBSE] Hypothèse rejetée",
                    json_value!({"improvement_pct": delta * 100.0})
                );
                println!("Conclusion : La structure globale ne justifie pas une allocation forte entre ces composants.");
            }
        }

        AiCommands::Rag { action } => {
            run_rag_action(domain_path.clone(), &manager, action.clone()).await?;
        }

        AiCommands::Ask { query } => {
            let mut orch = orch_ref.lock().await;
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
            let squad_runner = {
                let orch = orch_ref.lock().await;
                orch.squad_runner()
            };

            match squad_runner.execute_workflow(&prompt).await {
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

            match raise_core::services::ai_service::ai_run_audit(
                ctx.storage.clone(),
                &ctx.config.mount_points.system.domain,
                &ctx.config.mount_points.system.db,
                &target_domain,
                &ctx.active_db,
            )
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

        AiCommands::Mutate { handle, prompt } => {
            user_info!(
                "AI_MUTATION_INIT",
                json_value!({"target": handle, "instruction": prompt})
            );

            match raise_core::services::ai_service::ai_mutate_component(
                ctx.storage.clone(),
                ctx.kernel.native_llm.clone(),
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.active_user,
                &handle,
                &prompt,
            )
            .await?
            {
                Some(res) => {
                    user_success!(
                        "AI_MUTATION_SUCCESS",
                        json_value!({"message": res.message, "artifacts": res.artifacts.len()})
                    );
                }
                None => {
                    user_warn!(
                        "AI_MUTATION_SKIPPED",
                        json_value!({"reason": "L'agent n'a rien retourné."})
                    );
                }
            }
        }

        AiCommands::Explain { target_id } => {
            let config = raise_core::utils::data::config::AppConfig::get();
            let sys_manager = CollectionsManager::new(
                &ctx.storage,
                &config.mount_points.system.domain,
                &config.mount_points.system.db,
            );

            match raise_core::ai::assurance::get_xai_frame(&sys_manager, &target_id).await {
                Ok(frame) => {
                    user_success!("AI_EXPLAIN_FOUND", json_value!({"id": target_id}));
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
            let config = raise_core::utils::data::config::AppConfig::get();
            let sys_manager = CollectionsManager::new(
                &ctx.storage,
                &config.mount_points.system.domain,
                &config.mount_points.system.db,
            );

            match RaiseHealthEngine::check_engine_health(&sys_manager).await {
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

        AiCommands::Execute {
            prompt_handle,
            vars,
            out,
            ingest,
        } => {
            user_info!("AI_EXECUTE_START", json_value!({"prompt": prompt_handle}));
            println!("🤖 Inférence RAISE en cours ({})...", prompt_handle);

            match raise_core::services::ai_service::ai_execute_and_ingest(
                ctx.storage.clone(),
                ctx.kernel.native_llm.clone(),
                &ctx.active_domain,
                &ctx.active_db,
                &prompt_handle,
                vars,
                ingest,
            )
            .await
            {
                Ok((clean_json, ingested_ids)) => {
                    if ingest {
                        println!("📥 Routage ontologique et ingestion dans le Graphe Arcadia...");
                        println!(
                            "✅ {} entités validées et sauvegardées avec succès !",
                            ingested_ids.len()
                        );
                    }

                    match out {
                        Some(p) => {
                            let path = std::path::PathBuf::from(&p);
                            raise_core::utils::io::fs::write_async(&path, clean_json).await?;
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
                }
                Err(e) => raise_error!("ERR_AI_EXECUTION_FAILED", error = e.to_string()),
            }
        }

        AiCommands::Commit { handle } => {
            user_info!("AI_COMMIT_START", json_value!({"target": handle}));

            match raise_core::services::codegen_service::commit_module(
                &handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(final_path) => {
                    user_success!("AI_COMMIT_SUCCESS", json_value!({"path": final_path}));
                    println!("\n✅ Mutation fusionnée avec succès !");
                    println!("📁 Fichier de production mis à jour : {}", final_path);
                }
                Err(e) => raise_error!(
                    "ERR_AI_COMMIT_FAILED",
                    error = e,
                    context = json_value!({"handle": handle})
                ),
            }
        }
    }

    Ok(())
}

// =========================================================================
// 🎯 BOUCLES INTERACTIVES MISES À JOUR
// =========================================================================

async fn run_interactive_mode(cli_ctx: &CliContext) -> RaiseResult<()> {
    user_info!("AI_INTERACTIVE_WELCOME", json_value!({}));
    user_info!("AI_LLM_CONNECTED", json_value!({"mode": "local"}));
    let prompt = format!(
        "RAISE-AI [{}@{}/{}]> ",
        cli_ctx.active_user, cli_ctx.active_domain, cli_ctx.active_db
    );

    loop {
        print!("{}", prompt);
        os::flush_stdout()?;
        let input = os::read_stdin_line()?;

        if input.eq_ignore_ascii_case("exit") {
            break;
        }
        if input.is_empty() {
            continue;
        }

        match raise_core::services::ai_service::ai_classify_and_execute(
            cli_ctx.storage.clone(),
            cli_ctx.kernel.native_llm.clone(),
            &cli_ctx.active_domain,
            &cli_ctx.active_db,
            &cli_ctx.active_user,
            &input,
            true,
        )
        .await
        {
            Ok((intent, agent_urn, agent_result)) => {
                display_agent_result(&intent, &agent_urn, agent_result)
            }
            Err(e) => user_error!("AI_EXECUTION_ERROR", json_value!({"error": e.to_string()})),
        }
    }
    Ok(())
}

async fn run_voice_mode(cli_ctx: &CliContext, manager: &CollectionsManager<'_>) -> RaiseResult<()> {
    user_info!("AI_VOICE_INIT", json_value!({"status": "loading"}));
    println!("⏳ Chargement du modèle vocal Whisper (Hors-ligne)...");

    let mut engine = WhisperEngine::new(manager).await?;
    let (_listener, mut rx) = AudioListener::start()?;

    user_success!(
        "AI_VOICE_READY",
        json_value!({"message": "Microphone activé."})
    );
    println!("--------------------------------------------------\n🎤 Le micro est ouvert. Parlez naturellement !\n--------------------------------------------------\n");

    while let Some(audio_chunk) = rx.recv().await {
        print!("⏳ Transcription en cours... ");
        let _ = os::flush_stdout();

        match engine.transcribe(&audio_chunk) {
            Ok(text) => {
                let text = text.trim();
                if text.is_empty() {
                    continue;
                }
                println!("\r🗣️  Vous : {}                    ", text);

                match raise_core::services::ai_service::ai_classify_and_execute(
                    cli_ctx.storage.clone(),
                    cli_ctx.kernel.native_llm.clone(),
                    &cli_ctx.active_domain,
                    &cli_ctx.active_db,
                    &cli_ctx.active_user,
                    text,
                    true,
                )
                .await
                {
                    Ok((intent, agent_urn, agent_result)) => {
                        display_agent_result(&intent, &agent_urn, agent_result)
                    }
                    Err(e) => {
                        user_error!("AI_EXECUTION_ERROR", json_value!({"error": e.to_string()}))
                    }
                }
                println!("\n🎤 Écoute en cours...");
            }
            Err(e) => println!(
                "\r❌ Erreur de transcription : {}               \n\n🎤 Écoute en cours...",
                e
            ),
        }
    }
    Ok(())
}

async fn run_rag_action(
    domain_path: std::path::PathBuf,
    manager: &CollectionsManager<'_>,
    action: RagAction,
) -> RaiseResult<()> {
    let mut rag_engine = RagRetriever::new_internal(domain_path, manager).await?;

    match action {
        RagAction::Ingest { path } => {
            let target_path = std::path::PathBuf::from(&path);
            user_info!("RAG_INGESTION_START", json_value!({"path": path}));

            if !target_path.exists() {
                raise_error!(
                    "RAG_FILE_NOT_FOUND",
                    error = "Le fichier ou dossier spécifié n'existe pas.",
                    context = json_value!({"path": path})
                );
            }

            let content = raise_core::utils::io::fs::read_to_string_async(&target_path).await?;
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

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;
    use raise_core::utils::data::config::AppConfig;
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
            Err(raise_core::utils::core::error::AppError::Structured(err))
                if err.code == "ERR_AI_OFFLINE" =>
            {
                Ok(())
            }
            _ => raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Le handler aurait dû rejeter l'appel car l'orchestrateur est absent."
            ),
        }
    }
}
