// FICHIER : src-tauri/src/workflow_engine/scheduler.rs
use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use crate::workflow_engine::{
    executor::WorkflowExecutor, state_machine::WorkflowStateMachine, ExecutionStatus,
    WorkflowDefinition, WorkflowInstance,
};

pub struct WorkflowScheduler {
    pub executor: WorkflowExecutor,
    pub definitions: UnorderedMap<String, WorkflowDefinition>,
}

impl WorkflowScheduler {
    pub fn new(executor: WorkflowExecutor) -> Self {
        Self {
            executor,
            definitions: UnorderedMap::new(),
        }
    }

    /// Charge une mission de manière résiliente.
    pub async fn load_mission<'a>(
        &mut self,
        mission_handle: &str,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<()> {
        user_info!(
            "INF_SCHEDULER_LOAD_MISSION",
            json_value!({ "mission": mission_handle })
        );

        // Délégation à l'exécuteur pour la compilation tissée
        let workflow =
            match WorkflowExecutor::load_and_prepare_workflow(manager, mission_handle).await {
                Ok(wf) => wf,
                Err(e) => raise_error!(
                    "ERR_SCHEDULER_LOAD_FAIL",
                    error = e.to_string(),
                    context = json_value!({"mission": mission_handle})
                ),
            };

        // Utilisation du handle sémantique
        self.definitions.insert(workflow.handle.clone(), workflow);
        Ok(())
    }

    /// Crée une instance de workflow persistante.
    pub async fn create_instance<'a>(
        &self,
        mission_id: &str,
        workflow_handle: &str,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<WorkflowInstance> {
        let def = match self.definitions.get(workflow_handle) {
            Some(definition) => definition,
            None => raise_error!(
                "ERR_WF_DEFINITION_NOT_FOUND",
                context = json_value!({"workflow_handle": workflow_handle})
            ),
        };

        let mut instance = WorkflowInstance {
            _id: None,
            handle: format!(
                "inst_{}_{}",
                workflow_handle,
                UtcClock::now().timestamp_millis()
            ),
            mission_id: mission_id.to_string(),
            workflow_id: def.handle.clone(),
            status: ExecutionStatus::Pending,
            current_node_id: None,
            node_states: UnorderedMap::new(),
            context: UnorderedMap::new(),
            xai_traces: Vec::new(),
            logs: vec![format!(
                "Création de l'instance pour le workflow {}",
                def.handle
            )],
            created_at: UtcClock::now().timestamp(),
            updated_at: UtcClock::now().timestamp(),
        };

        self.persist_instance(&mut instance, manager).await?;
        Ok(instance)
    }

    /// Exécute une étape élémentaire du workflow.
    pub async fn run_step<'a>(
        &'a self,
        instance: &mut WorkflowInstance,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<bool> {
        let def = match self.definitions.get(&instance.workflow_id) {
            Some(d) => d,
            None => raise_error!(
                "ERR_WF_INSTANCE_ORPHAN",
                context = json_value!({"instance": instance.handle})
            ),
        };

        let sm = WorkflowStateMachine::new(def);
        let runnable_nodes = sm.next_runnable_nodes(instance).await;

        if runnable_nodes.is_empty() {
            if instance.status == ExecutionStatus::Running {
                instance.status = ExecutionStatus::Completed;
                instance.logs.push("🏁 Exécution terminée.".into());
                self.persist_instance(instance, manager).await?;
            }
            return Ok(false);
        }

        instance.status = ExecutionStatus::Running;
        let mut progress_made = false;

        for node_id in runnable_nodes {
            if let Some(node) = def.nodes.iter().find(|n| n.id == node_id) {
                let status = self
                    .executor
                    .execute_node(node, &mut instance.context, manager)
                    .await?;

                if let Err(e) = sm.transition(instance, &node_id, status) {
                    raise_error!("ERR_WF_STATE_TRANSITION_FAILED", error = e.to_string());
                }

                instance
                    .logs
                    .push(format!("⚙️ Nœud '{}' -> {:?}", node.name, status));
                progress_made = true;

                if status == ExecutionStatus::Paused || status == ExecutionStatus::Failed {
                    instance.status = status;
                    break;
                }
            }
        }

        if progress_made {
            self.persist_instance(instance, manager).await?;
        }

        Ok(progress_made)
    }

    /// Boucle d'exécution automatique jusqu'à complétion ou pause.
    pub async fn execute_instance_loop<'a>(
        &'a self,
        instance_handle: &str,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<ExecutionStatus> {
        let doc = match manager
            .get_document("workflow_instances", instance_handle)
            .await?
        {
            Some(d) => d,
            None => raise_error!(
                "ERR_WF_INSTANCE_NOT_FOUND",
                context = json_value!({"handle": instance_handle})
            ),
        };

        let mut instance: WorkflowInstance = match json::deserialize_from_value(doc) {
            Ok(inst) => inst,
            Err(e) => raise_error!("ERR_WF_DESERIALIZATION", error = e.to_string()),
        };

        loop {
            match self.run_step(&mut instance, manager).await {
                Ok(true) => continue,
                Ok(false) => break,
                Err(e) => return Err(e),
            }
        }

        Ok(instance.status)
    }

    /// Reprend l'exécution d'un nœud en attente (HITL).
    pub async fn resume_node<'a>(
        &self,
        instance_handle: &str,
        node_id: &str,
        approved: bool,
        manager: &'a CollectionsManager<'a>,
    ) -> RaiseResult<ExecutionStatus> {
        let doc = match manager
            .get_document("workflow_instances", instance_handle)
            .await?
        {
            Some(d) => d,
            None => raise_error!(
                "ERR_WF_INSTANCE_NOT_FOUND",
                context = json_value!({"handle": instance_handle})
            ),
        };

        let mut instance: WorkflowInstance = match json::deserialize_from_value(doc) {
            Ok(inst) => inst,
            Err(e) => raise_error!("ERR_WF_DESERIALIZATION", error = e.to_string()),
        };

        let new_status = if approved {
            ExecutionStatus::Completed
        } else {
            ExecutionStatus::Failed
        };
        instance.node_states.insert(node_id.to_string(), new_status);
        instance.status = ExecutionStatus::Running;

        self.persist_instance(&mut instance, manager).await?;
        Ok(instance.status)
    }

    /// Persistance atomique de l'état de l'instance.
    async fn persist_instance(
        &self,
        instance: &mut WorkflowInstance,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<()> {
        instance.updated_at = UtcClock::now().timestamp();
        let json_val = match json::serialize_to_value(&instance) {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_WF_SERIALIZATION", error = e.to_string()),
        };

        match manager
            .upsert_document("workflow_instances", json_val)
            .await
        {
            Ok(_) => Ok(()),
            Err(e) => raise_error!("ERR_WF_PERSISTENCE_FAIL", error = e.to_string()),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Façade & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::orchestrator::AiOrchestrator;
    use crate::model_engine::types::ProjectModel;
    use crate::plugins::manager::PluginManager;
    use crate::utils::testing::AgentDbSandbox; // 🎯 Ajout de DbSandbox

    async fn setup_test_environment(
        storage: SharedRef<crate::json_db::storage::StorageEngine>,
        config: &AppConfig,
    ) -> RaiseResult<WorkflowScheduler> {
        let manager = CollectionsManager::new(
            &storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        manager
            .create_collection("workflow_instances", &schema_uri)
            .await
            .unwrap();

        let orch = AiOrchestrator::new(ProjectModel::default(), &manager, storage.clone(), None)
            .await
            .unwrap();
        let pm = SharedRef::new(PluginManager::new(&storage, None));
        let executor = WorkflowExecutor::new(SharedRef::new(AsyncMutex::new(orch)), pm);

        Ok(WorkflowScheduler::new(executor))
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_scheduler_create_instance_persistence() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let scheduler = setup_test_environment(sandbox.db.clone(), &sandbox.config).await?;
        let manager = CollectionsManager::new(
            &sandbox.db,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );

        let def = WorkflowDefinition {
            _id: None,
            handle: "wf_test".to_string(),
            entry: "n1".to_string(),
            nodes: vec![],
            edges: vec![],
        };
        let mut scheduler = scheduler;
        scheduler.definitions.insert("wf_test".to_string(), def);

        let instance = scheduler
            .create_instance("mission_1", "wf_test", &manager)
            .await?;
        assert_eq!(instance.workflow_id, "wf_test");

        let doc = manager
            .get_document("workflow_instances", &instance.handle)
            .await?
            .unwrap();
        assert_eq!(doc["workflow_template_id"], "wf_test");
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face au point de montage système
    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_scheduler_mount_point_resilience() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Erreur Match sur instance orpheline
    #[async_test]
    #[serial_test::serial] // 🎯 FIX : Protection CUDA (Bombe à retardement désamorcée)
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_scheduler_orphan_instance_match() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let scheduler = setup_test_environment(sandbox.db.clone(), &sandbox.config).await?;
        let manager = CollectionsManager::new(&sandbox.db, "a", "b");

        let mut instance = WorkflowInstance {
            _id: None,
            handle: "ghost_inst".into(),
            mission_id: "m1".into(),
            workflow_id: "ghost_wf".into(),
            status: ExecutionStatus::Pending,
            current_node_id: None,
            node_states: UnorderedMap::new(),
            context: UnorderedMap::new(),
            xai_traces: Vec::new(),
            logs: Vec::new(),
            created_at: 0,
            updated_at: 0,
        };

        let result = scheduler.run_step(&mut instance, &manager).await;
        match result {
            Err(AppError::Structured(err)) => assert_eq!(err.code, "ERR_WF_INSTANCE_ORPHAN"),
            _ => panic!("Attendu ERR_WF_INSTANCE_ORPHAN"),
        }
        Ok(())
    }
}
