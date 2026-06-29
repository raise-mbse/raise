// FICHIER : src-tauri/src/workflow_engine/handlers/task.rs
use super::{HandlerContext, NodeHandler};
use crate::ai::assurance::xai::{ExplanationScope, XaiFrame, XaiMethod};
use crate::code_generator::graph_weaver::OntologyWeaver;
use crate::code_generator::toolchains::rust::RustToolchain;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE
use crate::workflow_engine::squad::{Squad, SquadStatus};
use crate::workflow_engine::{ExecutionStatus, NodeType, WorkflowNode};

pub struct TaskHandler;

#[async_interface]
impl NodeHandler for TaskHandler {
    fn node_type(&self) -> NodeType {
        NodeType::Task
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        shared_ctx: &HandlerContext<'_>,
    ) -> RaiseResult<ExecutionStatus> {
        // ====================================================================
        // 1. IDENTIFICATION DE LA MISSION ET DE LA SQUAD
        // ====================================================================
        let mission_handle = match context.get("mission_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => raise_error!(
                "ERR_MISSION_ID_MISSING_IN_CONTEXT",
                context = json_value!({ "node_id": node.id })
            ),
        };

        let mission_doc = match shared_ctx
            .manager
            .get_document("missions", mission_handle)
            .await?
        {
            Some(doc) => doc,
            None => raise_error!(
                "ERR_MISSION_NOT_FOUND",
                context = json_value!({ "mission_handle": mission_handle })
            ),
        };

        let squad_handle = mission_doc["squad_id"].as_str().unwrap_or_default();

        user_info!(
            "INF_SQUAD_ASSIGNED",
            json_value!({"squad_id": squad_handle, "task_id": node.id})
        );

        let squad = Squad::fetch_from_store(shared_ctx.manager, squad_handle).await?;

        if squad.status != SquadStatus::Active {
            user_error!(
                "ERR_SQUAD_NOT_ACTIVE",
                json_value!({"squad_id": squad.handle, "status": format!("{:?}", squad.status)})
            );
            return Ok(ExecutionStatus::Failed);
        }

        let lead_agent_id = squad.lead_agent_id.to_string();

        // ====================================================================
        // 2. FORGEAGE DE L'INTENTION MACRO POUR L'ORCHESTRATEUR
        // ====================================================================
        let rich_mission = format!(
            "OBJECTIF DE PHASE : {}\n\nINSTRUCTIONS SPÉCIFIQUES : {:?}\n\nCONTEXTE JUMEAU NUMÉRIQUE : {:?}\n\nSQUAD LEAD : {}",
            node.name, node.params, context, lead_agent_id
        );

        // ====================================================================
        // 3. EXÉCUTION DE LA SQUAD (BOUCLE ACL)
        // ====================================================================
        let squad_runner = {
            let orch = shared_ctx.orchestrator.lock().await;
            orch.squad_runner()
        };
        let agent_result = squad_runner.execute_workflow(&rich_mission).await?;

        let mut new_artifacts = context
            .get("generated_artifacts")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();

        for artifact in &agent_result.artifacts {
            new_artifacts.push(json::serialize_to_value(artifact).unwrap_or_default());

            // Tissage Ontologique et CodeGen
            if artifact.id.starts_with("code_") || artifact.id.starts_with("module_") {
                let target_path =
                    PathBuf::from("src/generated").join(format!("{}.rs", artifact.id));

                user_info!(
                    "INF_CODEGEN_START",
                    json_value!({"element_id": artifact.id})
                );

                match OntologyWeaver::generate_and_validate(
                    shared_ctx.manager,
                    &artifact.id,
                    target_path,
                    &RustToolchain,
                )
                .await
                {
                    Ok(path) => {
                        user_success!(
                            "SUC_CODEGEN_READY",
                            json_value!({"path": path.to_string_lossy()})
                        );
                    }
                    Err(AppError::Structured(err_box)) => {
                        if err_box.code == "ERR_CODEGEN_TOOLCHAIN_REJECTED" {
                            let feedback = err_box
                                .context
                                .get("xai_feedback")
                                .cloned()
                                .unwrap_or(json_value!("Erreur compilation"));
                            user_warn!(
                                "WRN_CODEGEN_REJECTED",
                                json_value!({"element_id": artifact.id, "feedback": feedback})
                            );
                            return Ok(ExecutionStatus::Failed);
                        }
                        return Err(AppError::Structured(err_box));
                    }
                }
            }
        }
        context.insert(
            "generated_artifacts".to_string(),
            json_value!(new_artifacts),
        );

        // ====================================================================
        // 4. TRAÇABILITÉ (XAI) & AUDITABILITÉ
        // ====================================================================
        let mut xai = XaiFrame::new(
            &node.id,
            XaiMethod::ChainOfThought,
            ExplanationScope::Global,
        );
        xai.predicted_output = agent_result.message.clone();
        xai.input_snapshot = rich_mission;

        use crate::rules_engine::ast::Expr;
        let default_rules = vec![Expr::Contains {
            list: Box::new(Expr::Var("predicted_output".to_string())),
            value: Box::new(Expr::Val(json_value!("JSON"))),
        }];

        let critique = match shared_ctx
            .critic
            .evaluate(&xai, shared_ctx.manager, &default_rules)
            .await
        {
            Ok(c) => c,
            Err(e) => raise_error!(
                "ERR_CRITIC_EXECUTION_FAILED",
                error = e.to_string(),
                context = json_value!({"node_id": node.id})
            ),
        };

        if !critique.is_acceptable {
            user_warn!(
                "WRN_CRITIC_REJECTION",
                json_value!({"reasoning": critique.reasoning, "node_id": node.id})
            );
        }

        // ====================================================================
        // 5. PERSISTANCE (Points de Montage Système)
        // ====================================================================
        let config = AppConfig::get();
        let xai_id = format!(
            "ref:xai_frames:handle:xai_{}_{}",
            node.id,
            UtcClock::now().timestamp_millis()
        );
        let mut xai_json = json::serialize_to_value(&xai).unwrap_or(json_value!({}));

        if let Some(obj) = xai_json.as_object_mut() {
            obj.insert("_id".to_string(), json_value!(xai_id.clone()));
            obj.insert("fidelity_score".to_string(), json_value!(critique.score));
        }

        // 🎯 RÉSILIENCE : Utilisation du schéma système configuré
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        let _ = shared_ctx
            .manager
            .create_collection("xai_frames", &schema_uri)
            .await;
        let _ = shared_ctx
            .manager
            .upsert_document("xai_frames", xai_json)
            .await;

        let mut traces = context
            .get("xai_traces")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        traces.push(json_value!(xai_id));
        context.insert("xai_traces".to_string(), json_value!(traces));

        let output_key = node
            .params
            .get("output_key")
            .and_then(|v| v.as_str())
            .unwrap_or("task_output");
        context.insert(output_key.to_string(), json_value!(agent_result.message));

        user_success!(
            "SUC_TASK_COMPLETED",
            json_value!({"task_name": node.name, "node_id": node.id})
        );
        Ok(ExecutionStatus::Completed)
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Façade & Résilience Mount Points)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::client::LlmEngine;
    use crate::ai::orchestrator::AiOrchestrator;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::model_engine::types::ProjectModel;
    use crate::plugins::manager::PluginManager;
    use crate::utils::testing::AgentDbSandbox; // 🎯 Ajout de DbSandbox
    use crate::workflow_engine::critic::WorkflowCritic;

    async fn setup_task_test_context<'a>(
        storage: SharedRef<crate::json_db::storage::StorageEngine>,
        config: &'a AppConfig,
        sandbox_db: &'a crate::json_db::storage::StorageEngine,
        engine: SharedRef<AsyncMutex<dyn LlmEngine>>,
    ) -> RaiseResult<(
        SharedRef<AsyncMutex<AiOrchestrator>>,
        SharedRef<PluginManager>,
        WorkflowCritic,
        UnorderedMap<String, Box<dyn crate::workflow_engine::tools::AgentTool>>,
        CollectionsManager<'a>,
    )> {
        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la config système
        let manager = CollectionsManager::new(
            sandbox_db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let orch = AiOrchestrator::new(
            ProjectModel::default(),
            &manager,
            storage.clone(),
            Some(engine),
        )
        .await
        .unwrap();
        Ok((
            SharedRef::new(AsyncMutex::new(orch)),
            SharedRef::new(PluginManager::new(&storage, None)),
            WorkflowCritic::default(),
            UnorderedMap::new(),
            manager,
        ))
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_task_handler_squad_delegation() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) = setup_task_test_context(
            sandbox.db.clone(),
            &config,
            &sandbox.db,
            sandbox.shared_engine.clone(),
        )
        .await?;

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        let lead_agent_uuid = "10000000-0000-0000-0000-000000000001";

        let mock_agent = |id: &str| {
            let handle = id.replace("ref:agents:handle:", "");
            json_value!({
                "_id": id, "handle": handle, "name": handle, "status": "active",
                "neuroProfile": { "promptId": "ref:prompts:handle:dummy" },
                "base": { "neuro_profile": { "prompt_id": "ref:prompts:handle:dummy" } }
            })
        };

        // Configuration résiliente des collections
        let collections = vec![
            "prompts",
            "agents",
            "missions",
            "squads",
            "configs",
            "session_agents",
        ];
        for coll in collections {
            let _ = manager.create_collection(coll, &schema_uri).await;
        }

        // 🎯 FIX 1 : 'environment' DOIT être un String selon prompt_engine.rs (ligne 83)
        manager
            .upsert_document(
                "prompts",
                json_value!({
                    "_id": "ref:prompts:handle:dummy",
                    "handle": "dummy",
                    "role": "Test",
                    "identity": { "persona": "Test", "tone": "pro" },
                    "directives": ["Go"],
                    "environment": "Environnement de test sécurisé"
                }),
            )
            .await?;

        // 🎯 FIX 2 : On injecte l'agent_system que le classificateur d'intention réclame
        manager
            .upsert_document("agents", mock_agent(lead_agent_uuid))
            .await?;
        manager
            .upsert_document("agents", mock_agent("ref:agents:handle:agent_software"))
            .await?;
        manager
            .upsert_document("agents", mock_agent("ref:agents:handle:agent_system"))
            .await?;

        manager
            .upsert_document("agents", mock_agent("ref:agents:handle:agent_dispatcher"))
            .await?;

        // 🎯 FIX 3 : Un seul upsert de squad avec 'status' en minuscules ("active")
        manager
            .upsert_document(
                "squads",
                json_value!({
                    "_id": "squad_01",
                    "handle": "squad-01",
                    "name": "Squad Alpha",
                    "lead_agent_id": lead_agent_uuid,
                    "members": [],
                    "status": "active"
                }),
            )
            .await?;

        manager.upsert_document("missions", json_value!({
            "_id": "mission_123", "handle": "mission-123", "squad_id": "squad-01", "status": "running"
        })).await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };
        let node = WorkflowNode {
            id: "task_1".into(),
            r#type: NodeType::Task,
            name: "Task Test".into(),
            params: json_value!({ "output_key": "la_report" }),
        };

        let mut data_ctx = UnorderedMap::new();
        data_ctx.insert("mission_id".to_string(), json_value!("mission-123"));

        let result = TaskHandler.execute(&node, &mut data_ctx, &ctx).await?;
        assert_eq!(result, ExecutionStatus::Completed);
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face à une mission introuvable via Match
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_task_handler_missing_mission_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) = setup_task_test_context(
            sandbox.db.clone(),
            &config,
            &sandbox.db,
            sandbox.shared_engine.clone(),
        )
        .await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };
        let node = WorkflowNode {
            id: "err".into(),
            r#type: NodeType::Task,
            name: "Err".into(),
            params: json_value!({}),
        };

        let mut data_ctx = UnorderedMap::new();
        data_ctx.insert("mission_id".to_string(), json_value!("ghost-mission"));

        let result = TaskHandler.execute(&node, &mut data_ctx, &ctx).await;
        match result {
            Err(AppError::Structured(err)) => assert_eq!(err.code, "ERR_MISSION_NOT_FOUND"),
            _ => panic!("Le moteur aurait dû lever ERR_MISSION_NOT_FOUND via Match"),
        }
        Ok(())
    }
}
