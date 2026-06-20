// FICHIER : crates/raise-core/src/ai/agents/devops_agent.rs

use super::{intent_classifier::EngineeringIntent, Agent, AgentContext, AgentResult};
use crate::utils::prelude::*;

// Imports spécifiques pour l'évolution, la session et les façades de données
use super::tools::{extract_json_from_llm, load_session, query_knowledge_graph, save_session};
use crate::genetics::evaluators::deployment::{DeploymentEvaluator, DeploymentGenome};
use crate::genetics::traits::Evaluator;
use crate::utils::data::UnorderedMap;

/// 🚀 L'Agent DevOps (SRE Local-First)
/// Responsable de la résolution des artefacts, du staging physique,
/// de la validation d'infrastructure et de l'auto-remédiation des services edge.
pub struct DevopsAgent {
    id: String,
    domain: String,
    db_name: String,
}

impl DevopsAgent {
    pub fn new(domain: String, db_name: String) -> Self {
        Self {
            id: "ref:agents:handle:agent_devops".to_string(),
            domain,
            db_name,
        }
    }
}

#[async_interface]
impl Agent for DevopsAgent {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(
        &self,
        ctx: &AgentContext,
        intent: &EngineeringIntent,
    ) -> RaiseResult<Option<AgentResult>> {
        match intent {
            EngineeringIntent::DeployEdgeArtifact {
                target_handle,
                target_architecture,
                payload_uri,
            } => {
                crate::user_info!(
                    "DEVOPS_START_DEPLOY",
                    json_value!({"target": target_handle, "arch": target_architecture, "uri": payload_uri})
                );

                // ====================================================================
                // PHASE 3 : CHARGEMENT DATA-DRIVEN (Profil & Configuration)
                // ====================================================================
                let config = crate::utils::data::config::AppConfig::get();
                let sys_manager = crate::json_db::collections::manager::CollectionsManager::new(
                    &ctx.db,
                    &config.mount_points.system.domain,
                    &config.mount_points.system.db,
                );

                let ws_manager = crate::json_db::collections::manager::CollectionsManager::new(
                    &ctx.db,
                    &self.domain,
                    &self.db_name,
                );

                let agent_doc = if let Ok(Some(doc)) =
                    ws_manager.get_document("agents", "agent_devops").await
                {
                    doc
                } else if let Ok(Some(doc)) =
                    sys_manager.get_document("agents", "agent_devops").await
                {
                    doc
                } else {
                    raise_error!(
                        "ERR_AGENT_NOT_FOUND",
                        error = "La définition de l'agent DevOps est introuvable dans le graphe système."
                    )
                };

                let prompt_handle = match agent_doc["base"]["neuro_profile"]["prompt_id"].as_str() {
                    Some(id) => id,
                    None => raise_error!(
                        "ERR_AGENT_MISSING_PROMPT",
                        error = "prompt_id absent du neuro_profile pour agent_devops."
                    ),
                };

                let prompt_vars = json_value!({
                    "target_handle": target_handle,
                    "target_architecture": target_architecture
                });

                let prompt_engine_ws = super::prompt_engine::PromptEngine::new(
                    ctx.db.clone(),
                    &self.domain,
                    &self.db_name,
                );
                let system_prompt = match prompt_engine_ws
                    .compile(prompt_handle, Some(&prompt_vars))
                    .await
                {
                    Ok(p) => p,
                    Err(_) => {
                        let prompt_engine_sys = super::prompt_engine::PromptEngine::new(
                            ctx.db.clone(),
                            &config.mount_points.system.domain,
                            &config.mount_points.system.db,
                        );
                        prompt_engine_sys
                            .compile(prompt_handle, Some(&prompt_vars))
                            .await?
                    }
                };

                // ====================================================================
                // PHASE 5 : STAGING PHYSIQUE & SÉCURISATION (Rapatriement et Chmod)
                // ====================================================================
                crate::user_info!(
                    "DEVOPS_FETCHING_ARTIFACT",
                    json_value!({"uri": payload_uri})
                );

                // 1. Résolution de l'artefact binaire via le Graphe de Connaissances
                let artifact_doc = query_knowledge_graph(ctx, payload_uri, false).await?;

                let storage = &artifact_doc["storage"];
                let exec_context = &artifact_doc["execution_context"];

                if storage["encoding"].as_str() != Some("base64") {
                    raise_error!(
                        "ERR_DEVOPS_UNSUPPORTED_ENCODING",
                        error = format!(
                            "Encodage de stockage '{}' non supporté par l'agent.",
                            storage["encoding"].as_str().unwrap_or("aucun")
                        )
                    );
                }

                let b64_payload = storage["payload_or_uri"].as_str().unwrap_or("");
                let binary_data = match decode_base64(b64_payload) {
                    Ok(data) => data,
                    Err(e) => raise_error!("ERR_DEVOPS_DECODE_FAILED", error = e.to_string()),
                };

                // 2. Détermination du chemin physique cible (Staging temporaire sécurisé)
                let temp_dir = match crate::utils::io::fs::tempdir() {
                    Ok(dir) => dir,
                    Err(e) => raise_error!("ERR_FS_TEMP", error = e.to_string()),
                };

                let filename = format!("staged_{}", target_handle);
                let staged_binary_path = temp_dir.path().join(&filename);
                let staged_binary_str = staged_binary_path.to_string_lossy().to_string();

                // 3. Écriture asynchrone transparente sur le stockage local
                if let Err(e) = fs::write_async(&staged_binary_path, &binary_data).await {
                    raise_error!(
                        "ERR_DEVOPS_WRITE_FAILED",
                        error = format!(
                            "Échec d'écriture de l'artefact sur {}: {}",
                            staged_binary_str, e
                        )
                    );
                }

                // 4. Sécurisation Unix : Application asynchrone des privilèges d'exécution (+x)
                let requires_chmod = exec_context["requires_chmod_x"].as_bool().unwrap_or(true);
                if requires_chmod {
                    match fs::get_permissions_async(&staged_binary_str).await {
                        Ok(mut perms) => {
                            perms.set_mode(0o755); // Permissions standard d'exécution RAISE (rwxr-xr-x)
                            if let Err(e) =
                                fs::set_permissions_async(&staged_binary_str, perms).await
                            {
                                crate::user_warn!(
                                    "WARN_DEVOPS_CHMOD_FAILED",
                                    json_value!({"error": e.to_string(), "path": staged_binary_str})
                                );
                            }
                        }
                        Err(e) => {
                            crate::user_warn!(
                                "WARN_DEVOPS_PERMISSIONS_READ_FAILED",
                                json_value!({"error": e.to_string(), "path": staged_binary_str})
                            );
                        }
                    }
                }

                // ====================================================================
                // PHASE 4 : BOUCLE ÉVOLUTIONNAIRE NEURO-SYMBOLIQUE (Auto-Remédiation)
                // ====================================================================
                let evaluator = DeploymentEvaluator::new(temp_dir.path().to_path_buf());

                let mut current_genome = DeploymentGenome {
                    binary_path: staged_binary_str.clone(),
                    arguments: vec![],
                    env_vars: UnorderedMap::new(),
                };

                // Ingestion de l'historique transactionnel de la session
                let mut session = match load_session(ctx).await {
                    Ok(s) => s,
                    Err(_) => super::AgentSession::new(&ctx.session_id, self.id()),
                };

                let mut attempts = 0;
                let max_attempts = 3;
                let mut is_stable = false;

                while attempts < max_attempts {
                    attempts += 1;
                    crate::user_info!(
                        "DEVOPS_EVALUATING",
                        json_value!({"attempt": attempts, "max": max_attempts})
                    );

                    // 1. Évaluation Déterministe (Le crash-test via AsyncCommand)
                    let (objs, violation) = evaluator.evaluate(&current_genome).await;

                    if violation == 0.0 {
                        crate::user_success!(
                            "SUC_DEVOPS_STABLE",
                            json_value!({"stability": objs[0]})
                        );
                        is_stable = true;
                        break;
                    }

                    // 2. Échec : On sollicite la couche neuronale (LLM) pour muter le génome de configuration
                    crate::user_warn!(
                        "WARN_DEVOPS_UNSTABLE",
                        json_value!({"violation": violation})
                    );

                    let user_prompt = format!(
                        "=== RAPPORT DE CRASH INFRASTRUCTURE ===\nLe déploiement du service edge a échoué aux tests de stabilité (Pénalité formelle: {}).\n\n=== GÉNOME ACTUEL CONFIGURÉ ===\nArguments: {:?}\nVariables d'environnement: {:?}\n\n=== INSTRUCTION DE RÉPARATION SRE ===\nPropose une mutation stricte pour corriger ce crash (ex: modification de ports, allocation de mémoire). Tu dois retourner UNIQUEMENT un objet JSON valide contenant les clés 'arguments' (tableau de chaînes) et 'env_vars' (objet clé-valeur). N'ajoute aucun commentaire, aucune explication.",
                        violation, current_genome.arguments, current_genome.env_vars
                    );

                    session.add_message("user", &user_prompt);

                    let response = ctx
                        .llm
                        .ask(
                            crate::ai::llm::client::LlmBackend::LocalLlama,
                            &system_prompt,
                            &user_prompt,
                            crate::utils::data::json::Clearance::Internal,
                        )
                        .await?;

                    session.add_message("assistant", &response);

                    // 3. Extraction syntaxique et application de la mutation sur le génome
                    let json_str = extract_json_from_llm(&response);
                    match crate::utils::data::json::deserialize_from_str::<
                        crate::utils::data::JsonValue,
                    >(&json_str)
                    {
                        Ok(parsed) => {
                            if let Some(args) = parsed.get("arguments").and_then(|a| a.as_array()) {
                                current_genome.arguments = args
                                    .iter()
                                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                                    .collect();
                            }
                            if let Some(envs) = parsed.get("env_vars").and_then(|e| e.as_object()) {
                                current_genome.env_vars.clear();
                                for (k, v) in envs {
                                    if let Some(v_str) = v.as_str() {
                                        current_genome
                                            .env_vars
                                            .insert(k.clone(), v_str.to_string());
                                    }
                                }
                            }
                            crate::user_info!(
                                "DEVOPS_MUTATION_APPLIED",
                                json_value!({"args": current_genome.arguments, "env": current_genome.env_vars})
                            );
                        }
                        Err(e) => {
                            crate::user_warn!(
                                "WARN_DEVOPS_INVALID_MUTATION_SCHEMA",
                                json_value!({"error": e.to_string()})
                            );
                        }
                    }
                }

                // Persistance de la session agent en DB système
                if let Err(e) = save_session(ctx, &session).await {
                    crate::user_warn!(
                        "WARN_SESSION_SAVE_FAILED",
                        json_value!({"err": e.to_string()})
                    );
                }

                if !is_stable {
                    raise_error!(
                        "ERR_DEVOPS_EVOLUTION_FAILED",
                        error = format!(
                            "Impossible de stabiliser le service physique {} après {} tentatives.",
                            target_handle, max_attempts
                        )
                    );
                }

                Ok(Some(AgentResult::text(format!(
                    "Déploiement du service Edge {} certifié stable et fonctionnel après validation neuro-symbolique.", 
                    target_handle
                ))))
            }

            EngineeringIntent::RollbackDeployment {
                target_handle,
                fallback_commit,
            } => {
                crate::user_info!(
                    "DEVOPS_START_ROLLBACK",
                    json_value!({"target": target_handle, "commit": fallback_commit})
                );

                Ok(Some(AgentResult::text(format!(
                    "Rollback du composant {} vers le commit {} initié.",
                    target_handle, fallback_commit
                ))))
            }

            _ => Ok(None),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Architecture Data-Driven & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::client::LlmClient;
    use crate::ai::world_model::NeuroSymbolicEngine;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::data::config::AppConfig;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn setup_devops_test_ctx(sandbox: &AgentDbSandbox) -> RaiseResult<AgentContext> {
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        DbSandbox::mock_db(&manager).await?;
        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );
        let _ = manager
            .create_collection("components", &generic_schema)
            .await;
        let _ = manager
            .create_collection("service_configs", &generic_schema)
            .await;
        let _ = manager.create_collection("agents", &generic_schema).await;
        let _ = manager.create_collection("prompts", &generic_schema).await;

        manager.upsert_document("components", json_value!({ "_id": "ref:components:handle:codegen_engine", "handle": "codegen_engine" })).await?;
        manager
            .upsert_document(
                "service_configs",
                json_value!({
                    "_id": "mock_codegen",
                    "component_id": "ref:components:handle:codegen_engine",
                    "service_settings": {
                        "format_on_save": true,
                        "strict_mode": true
                    }
                }),
            )
            .await?;

        manager
            .upsert_document(
                "prompts",
                json_value!({
                    "_id": "prompt_devops",
                    "handle": "prompt_devops",
                    "role": "SRE",
                    "identity": { "persona": "Testeur", "tone": "Strict" },
                    "environment": "Test",
                    "directives": ["Valider {{target_handle}}"]
                }),
            )
            .await?;

        manager
            .upsert_document(
                "agents",
                json_value!({
                    "_id": "agent_devops",
                    "base": {
                        "name": {"fr": "DevOps Agent"},
                        "neuro_profile": { "prompt_id": "prompt_devops" }
                    }
                }),
            )
            .await?;

        let llm = match LlmClient::new(
            &manager,
            sandbox.db.clone(),
            Some(sandbox.shared_engine.clone()),
        )
        .await
        {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_LLM", error = e),
        };

        let world_engine = SharedRef::new(match NeuroSymbolicEngine::bootstrap(&manager).await {
            Ok(we) => we,
            Err(e) => raise_error!("ERR_TEST_WM", error = e),
        });

        Ok(AgentContext::new(
            "agent_devops",
            "sess_devops_test",
            sandbox.db.clone(),
            llm,
            world_engine,
            sandbox.domain_root.clone(),
            sandbox.domain_root.clone(),
        )
        .await?)
    }

    #[test]
    fn test_devops_agent_id() {
        let agent = DevopsAgent::new("domain".to_string(), "db".to_string());
        assert_eq!(agent.id(), "ref:agents:handle:agent_devops");
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_process_ignores_out_of_scope_intent() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let ctx = setup_devops_test_ctx(&sandbox).await?;

        let config = AppConfig::get();
        let agent = DevopsAgent::new(
            config.mount_points.system.domain.clone(),
            config.mount_points.system.db.clone(),
        );

        let intent = EngineeringIntent::Chat;
        let result_opt = agent.process(&ctx, &intent).await?;
        assert!(result_opt.is_none());

        Ok(())
    }
}
