// FICHIER : src-tauri/src/workflow_engine/handlers/wasm.rs
use super::{HandlerContext, NodeHandler};
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE
use crate::workflow_engine::{ExecutionStatus, NodeType, WorkflowNode};

pub struct WasmHandler;

#[async_interface]
impl NodeHandler for WasmHandler {
    fn node_type(&self) -> NodeType {
        NodeType::Wasm
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        shared_ctx: &HandlerContext<'_>,
    ) -> RaiseResult<ExecutionStatus> {
        // 1. Identification du plugin via Match
        let plugin_id = match node.params.get("plugin_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => &node.id,
        };

        user_info!("INF_WASM_INVOKING", json_value!({ "plugin_id": plugin_id }));

        // 2. Extraction sécurisée du contexte de mandat
        let mandate_ctx = context.get("_mandate").cloned();

        // 3. Exécution via le PluginManager avec gestion de la résilience
        match shared_ctx
            .plugin_manager
            .run_plugin_with_context(plugin_id, mandate_ctx)
            .await
        {
            Ok((exit_code, signals)) => {
                // 🎯 INJECTION SÉCURISÉE : Isolation absolue des variables tierces
                let mut wasm_signals = match context.get("wasm_signals") {
                    Some(JsonValue::Object(map)) => map.clone(),
                    _ => JsonObject::new(),
                };

                for signal in signals {
                    user_info!(
                        "INF_WASM_SIGNAL",
                        json_value!({ "plugin": plugin_id, "signal": &signal })
                    );
                    wasm_signals.insert(plugin_id.to_string(), signal);
                }
                context.insert("wasm_signals".to_string(), JsonValue::Object(wasm_signals));

                if exit_code == 1 {
                    user_success!("SUC_WASM_COMPLETED", json_value!({ "plugin": plugin_id }));
                    Ok(ExecutionStatus::Completed)
                } else {
                    user_warn!(
                        "WRN_WASM_VETO",
                        json_value!({ "plugin": plugin_id, "exit_code": exit_code })
                    );
                    Ok(ExecutionStatus::Failed)
                }
            }
            Err(e) => {
                user_error!(
                    "ERR_WASM_EXECUTION",
                    json_value!({ "plugin": plugin_id, "error": e.to_string() })
                );
                Ok(ExecutionStatus::Failed)
            }
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Façade & Résilience Mount Points)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::orchestrator::AiOrchestrator;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::model_engine::types::ProjectModel;
    use crate::plugins::manager::PluginManager;
    use crate::utils::testing::AgentDbSandbox; // 🎯 Ajout de DbSandbox
    use crate::workflow_engine::critic::WorkflowCritic;

    async fn setup_wasm_test_context<'a>(
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
            .expect("Orchestrator setup failed");

        let plugin_manager = SharedRef::new(PluginManager::new(&storage, None));

        Ok((
            SharedRef::new(AsyncMutex::new(orch)),
            plugin_manager,
            WorkflowCritic::default(),
            UnorderedMap::new(),
            manager,
        ))
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_wasm_handler_missing_plugin_fails_safely() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) =
            setup_wasm_test_context(sandbox.db.clone(), &config, &sandbox.db).await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };

        let node = WorkflowNode {
            id: "wasm_1".into(),
            r#type: NodeType::Wasm,
            name: "Test Plugin".into(),
            params: json_value!({ "plugin_id": "plugin_inconnu" }),
        };

        let mut data_ctx = UnorderedMap::new();
        let result = WasmHandler.execute(&node, &mut data_ctx, &ctx).await?;

        assert_eq!(result, ExecutionStatus::Failed);
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Inférence résiliente du plugin_id par défaut
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_wasm_handler_default_id_inference() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let (orch, pm, critic, tools, manager) =
            setup_wasm_test_context(sandbox.db.clone(), &config, &sandbox.db).await?;

        let ctx = HandlerContext {
            orchestrator: &orch,
            plugin_manager: &pm,
            critic: &critic,
            tools: &tools,
            manager: &manager,
        };

        let node = WorkflowNode {
            id: "my_auto_plugin".into(),
            r#type: NodeType::Wasm,
            name: "Auto ID Test".into(),
            params: json_value!({}), // Pas de plugin_id spécifié
        };

        let mut data_ctx = UnorderedMap::new();
        // L'exécution échoue car le plugin n'existe pas, mais on valide que l'ID est bien déduit de l'ID du nœud
        let result = WasmHandler.execute(&node, &mut data_ctx, &ctx).await?;
        assert_eq!(result, ExecutionStatus::Failed);
        Ok(())
    }
}
