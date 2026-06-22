// FICHIER : src-tauri/src/workflow_engine/executor.rs

use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use super::compiler::WorkflowCompiler;
use super::handlers::{
    decision::DecisionHandler, end::EndHandler, hitl::GateHitlHandler, mcp::McpHandler,
    policy::GatePolicyHandler, task::TaskHandler, wasm::WasmHandler, HandlerContext, NodeHandler,
};
use super::tools::AgentTool;
use super::{critic::WorkflowCritic, ExecutionStatus, NodeType, WorkflowDefinition, WorkflowNode};
use crate::plugins::manager::PluginManager;

use crate::ai::orchestrator::AiOrchestrator;
use crate::json_db::collections::manager::CollectionsManager;

/// L'Exécuteur est le routeur principal. Il délègue la logique aux Handlers spécialisés.
/// Assure la résilience du flux de travail MBSE Arcadia.
pub struct WorkflowExecutor {
    pub orchestrator: SharedRef<AsyncMutex<AiOrchestrator>>,
    pub plugin_manager: SharedRef<PluginManager>,
    critic: WorkflowCritic,
    tools: UnorderedMap<String, Box<dyn AgentTool>>,
    handlers: UnorderedMap<NodeType, Box<dyn NodeHandler>>,
}

impl WorkflowExecutor {
    pub fn new(
        orchestrator: SharedRef<AsyncMutex<AiOrchestrator>>,
        plugin_manager: SharedRef<PluginManager>,
    ) -> Self {
        let mut handlers: UnorderedMap<NodeType, Box<dyn NodeHandler>> = UnorderedMap::new();

        // 🎯 ALIGNEMENT MBSE : Utilisation de QualityGate pour la gouvernance
        handlers.insert(NodeType::QualityGate, Box::new(GatePolicyHandler));
        handlers.insert(NodeType::Task, Box::new(TaskHandler));
        handlers.insert(NodeType::Decision, Box::new(DecisionHandler));
        handlers.insert(NodeType::CallMcp, Box::new(McpHandler));
        handlers.insert(NodeType::Wasm, Box::new(WasmHandler));
        handlers.insert(NodeType::GateHitl, Box::new(GateHitlHandler));
        handlers.insert(NodeType::End, Box::new(EndHandler));

        Self {
            orchestrator,
            plugin_manager,
            critic: WorkflowCritic::default(),
            tools: UnorderedMap::new(),
            handlers,
        }
    }

    pub fn register_tool(&mut self, tool: Box<dyn AgentTool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    // ========================================================================
    // LE PONT : Chargement et Compilation Sécurisés
    // ========================================================================

    /// Prépare le workflow en tissant Mission, Mandat et Template.
    /// Utilise les points de montage pour garantir la provenance des données.
    pub async fn load_and_prepare_workflow(
        manager: &CollectionsManager<'_>,
        mission_handle: &str,
    ) -> RaiseResult<WorkflowDefinition> {
        user_info!(
            "INF_WF_COMPILING",
            json_value!({ "mission": mission_handle })
        );

        // Compilation asynchrone via le compilateur résilient
        match WorkflowCompiler::compile(manager, mission_handle).await {
            Ok(workflow) => {
                user_success!(
                    "SUC_WF_COMPILED",
                    json_value!({ "handle": workflow.handle, "nodes": workflow.nodes.len() })
                );
                Ok(workflow)
            }
            Err(e) => raise_error!(
                "ERR_WF_PREPARATION_FAILED",
                error = e.to_string(),
                context = json_value!({ "mission": mission_handle })
            ),
        }
    }

    // ========================================================================
    // EXECUTION DES NOEUDS (ROUTAGE RÉSILIENT)
    // ========================================================================

    /// Exécute un nœud spécifique en routant vers le handler approprié.
    /// Pattern Match strict pour éviter les échecs silencieux.
    pub async fn execute_node<'a>(
        &'a self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<ExecutionStatus> {
        user_info!(
            "INF_WF_NODE_EXEC",
            json_value!({ "name": node.name, "type": format!("{:?}", node.r#type) })
        );

        let shared_ctx = HandlerContext {
            orchestrator: &self.orchestrator,
            plugin_manager: &self.plugin_manager,
            critic: &self.critic,
            tools: &self.tools,
            manager,
        };

        // 🎯 RÉSILIENCE : Match exhaustif sur les exécuteurs
        match self.handlers.get(&node.r#type) {
            Some(handler) => match handler.execute(node, context, &shared_ctx).await {
                Ok(status) => Ok(status),
                Err(e) => raise_error!(
                    "ERR_WF_NODE_FAILURE",
                    error = e.to_string(),
                    context = json_value!({ "node_id": node.id, "node_name": node.name })
                ),
            },
            None => raise_error!(
                "ERR_WF_HANDLER_NOT_FOUND",
                error = "Aucun exécuteur trouvé pour ce type de nœud.",
                context = json_value!({
                    "node_id": node.id,
                    "node_type": format!("{:?}", node.r#type)
                })
            ),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Façade & Résilience Mount Points)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ProjectModel;
    use crate::utils::testing::AgentDbSandbox;
    use crate::workflow_engine::tools::SystemMonitorTool;

    async fn create_test_executor_with_tools(
        storage: SharedRef<crate::json_db::storage::StorageEngine>,
        config: &AppConfig,
    ) -> RaiseResult<WorkflowExecutor> {
        let manager = CollectionsManager::new(
            &storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let orch =
            AiOrchestrator::new(ProjectModel::default(), &manager, storage.clone(), None).await?;
        let plugin_manager = SharedRef::new(PluginManager::new(&storage, None));

        let mut exec = WorkflowExecutor::new(SharedRef::new(AsyncMutex::new(orch)), plugin_manager);
        exec.register_tool(Box::new(SystemMonitorTool));
        Ok(exec)
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_gate_pause() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let executor = create_test_executor_with_tools(sandbox.db.clone(), &config).await?;
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let node = WorkflowNode {
            id: "node_pause".into(),
            r#type: NodeType::GateHitl,
            name: "Human Check".into(),
            params: JsonValue::Null,
        };

        let mut ctx = UnorderedMap::new();
        let status = executor.execute_node(&node, &mut ctx, &manager).await?;
        assert_eq!(status, ExecutionStatus::Paused);
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_bridge_loading_and_compilation() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );

        manager
            .create_collection("workflow_definitions", &schema_uri)
            .await?;
        manager
            .upsert_document(
                "workflow_definitions",
                json_value!({
                    "handle": "tpl_1", "name": "Tpl", "entry_node_id": "start",
                    "nodes": [{"node_id": "start", "type": "task", "name": "Start", "params": {}}],
                    "edges": []
                }),
            )
            .await?;

        manager.create_collection("mandates", &schema_uri).await?;
        manager.upsert_document("mandates", json_value!({
            "handle": "mandate-1", "name": "Mandat",
            "meta": { "mandator_id": "00000000-0000-0000-0000-000000000000", "version": "1.0", "status": "ACTIVE" },
            "governance": { "strategy": "SAFETY_FIRST", "condorcetWeights": {} },
            "hardLogic": { "vetos": [] }, "observability": { "heartbeatMs": 100 }
        })).await?;

        manager.create_collection("missions", &schema_uri).await?;
        manager
            .upsert_document(
                "missions",
                json_value!({
                    "handle": "mission-prod", "name": "Mission",
                    "mandate_id": "mandate-1", "squad_id": "squad_1",
                    "workflow_template_id": "tpl_1", "status": "draft"
                }),
            )
            .await?;

        let wf = WorkflowExecutor::load_and_prepare_workflow(&manager, "mission-prod").await?;
        assert!(wf.handle.contains("mandate-1"));
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face à un Handler manquant
    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_resilience_missing_handler() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let mut exec_mut = create_test_executor_with_tools(sandbox.db.clone(), config).await?;

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let node = WorkflowNode {
            id: "ghost".into(),
            r#type: NodeType::Wasm, // Supposons qu'on ne l'ait pas enregistré
            name: "Ghost".into(),
            params: JsonValue::Null,
        };

        // On retire manuellement le handler pour le test
        exec_mut.handlers.remove(&NodeType::Wasm);

        let mut ctx = UnorderedMap::new();
        let result = exec_mut.execute_node(&node, &mut ctx, &manager).await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_WF_HANDLER_NOT_FOUND");
                Ok(())
            }
            _ => panic!("Le moteur aurait dû lever ERR_WF_HANDLER_NOT_FOUND"),
        }
    }

    /// Validation des Mount Points
    #[async_test]
    async fn test_executor_mount_point_resolution() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;

        let config = AppConfig::get();
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }
}
