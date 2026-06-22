// FICHIER : src-tauri/src/workflow_engine/handlers/decision.rs
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use super::{HandlerContext, NodeHandler};
use crate::workflow_engine::{ExecutionStatus, NodeType, WorkflowNode};

pub struct DecisionHandler;

#[async_interface]
impl NodeHandler for DecisionHandler {
    fn node_type(&self) -> NodeType {
        NodeType::Decision
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        _shared_ctx: &HandlerContext<'_>,
    ) -> RaiseResult<ExecutionStatus> {
        user_info!("INF_DECISION_START", json_value!({ "node": node.name }));

        // 1. Extraction sécurisée des poids via Match
        let weights = match node.params.get("weights").and_then(|v| v.as_object()) {
            Some(obj) => obj,
            None => {
                user_warn!(
                    "WRN_DECISION_DEFAULT_WEIGHTS",
                    json_value!({ "node": node.id })
                );
                &JsonObject::default()
            }
        };

        let w_security = weights
            .get("agent_security")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);
        let w_finance = weights
            .get("agent_finance")
            .and_then(|v| v.as_f64())
            .unwrap_or(1.0);

        // 2. Récupération des candidats avec validation de résilience
        let candidates = match context.get("candidates").and_then(|v| v.as_array()) {
            Some(list) if list.len() > 1 => list,
            Some(_) => {
                user_warn!(
                    "WRN_DECISION_SINGLE_CANDIDATE",
                    json_value!({ "node": node.id })
                );
                return Ok(ExecutionStatus::Completed);
            }
            None => {
                raise_error!(
                    "ERR_DECISION_MISSING_CANDIDATES",
                    context = json_value!({ "node_id": node.id, "hint": "Le contexte doit contenir une liste 'candidates'." })
                );
            }
        };

        // 3. Algorithme de Condorcet pondéré
        let mut wins = vec![0.0; candidates.len()];

        for i in 0..candidates.len() {
            for j in (i + 1)..candidates.len() {
                let cand_a = &candidates[i].to_string();
                let cand_b = &candidates[j].to_string();

                let len_a = cand_a.len();
                let len_b = cand_b.len();

                // Vote Sécurité (préférence au plus court/simple)
                if len_a < len_b {
                    wins[i] += w_security;
                } else {
                    wins[j] += w_security;
                }

                // Vote Finance (préférence au plus détaillé/long)
                if len_a > len_b {
                    wins[i] += w_finance;
                } else {
                    wins[j] += w_finance;
                }
            }
        }

        // 4. Détermination du vainqueur avec gestion d'erreur
        let winner_idx = match wins
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(FmtOrdering::Equal))
        {
            Some((idx, _)) => idx,
            None => raise_error!("ERR_DECISION_COMPUTATION_FAILED"),
        };

        let winner = candidates[winner_idx].clone();
        context.insert("condorcet_winner".into(), winner.clone());

        user_success!(
            "SUC_DECISION_WINNER",
            json_value!({ "winner": winner.to_string() })
        );
        Ok(ExecutionStatus::Completed)
    }
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Façade & Résilience Mount Points)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::orchestrator::AiOrchestrator;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::model_engine::types::ProjectModel;
    use crate::plugins::manager::PluginManager;
    use crate::utils::testing::AgentDbSandbox;
    use crate::workflow_engine::critic::WorkflowCritic;

    async fn setup_dummy_context<'a>(
        storage: SharedRef<crate::json_db::storage::StorageEngine>,
        config: &'a AppConfig,
        sandbox_db: &'a crate::json_db::storage::StorageEngine,
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

        let orch = AiOrchestrator::new(ProjectModel::default(), &manager, storage.clone(), None)
            .await
            .expect("Setup Orchestrator failed");

        let plugin_manager = SharedRef::new(PluginManager::new(&storage, None));
        let critic = WorkflowCritic::default();
        let tools = UnorderedMap::new();

        Ok((
            SharedRef::new(AsyncMutex::new(orch)),
            plugin_manager,
            critic,
            tools,
            manager,
        ))
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_decision_handler_condorcet_evaluation() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) =
            setup_dummy_context(sandbox.db.clone(), &config, &sandbox.db).await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };

        let handler = DecisionHandler;
        let node = WorkflowNode {
            id: "dec_1".into(),
            r#type: NodeType::Decision,
            name: "Vote Final".into(),
            params: json_value!({ "weights": { "agent_security": 5.0, "agent_finance": 1.0 } }),
        };

        let mut data_ctx = UnorderedMap::from([(
            "candidates".into(),
            json_value!(["Option A (Courte)", "Option B (Très très longue)"]),
        )]);

        let result = handler.execute(&node, &mut data_ctx, &ctx).await?;

        assert_eq!(result, ExecutionStatus::Completed);
        assert!(data_ctx.contains_key("condorcet_winner"));
        Ok(())
    }

    ///   Résilience face à l'absence de candidats
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_resilience_missing_candidates() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) =
            setup_dummy_context(sandbox.db.clone(), &config, &sandbox.db).await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };

        let node = WorkflowNode {
            id: "dec_err".into(),
            r#type: NodeType::Decision,
            name: "Fail Vote".into(),
            params: json_value!({}),
        };

        let mut data_ctx = UnorderedMap::new(); // Contexte vide
        let result = DecisionHandler.execute(&node, &mut data_ctx, &ctx).await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_DECISION_MISSING_CANDIDATES")
            }
            _ => panic!("Le moteur aurait dû lever ERR_DECISION_MISSING_CANDIDATES"),
        }
        Ok(())
    }

    /// Validation de la partition système via Mount Points
    #[async_test]
    async fn test_decision_mount_point_resolution() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }
}
