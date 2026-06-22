// FICHIER : src-tauri/src/workflow_engine/state_machine.rs

use super::{ExecutionStatus, WorkflowDefinition, WorkflowInstance};
use crate::utils::prelude::*;
// Intégration du moteur de règles
use crate::rules_engine::ast::Expr;
use crate::rules_engine::evaluator::{Evaluator, NoOpDataProvider};

/// Moteur de règles de transition pour le workflow.
/// C'est lui qui décide quel nœud doit s'exécuter ensuite.
pub struct WorkflowStateMachine<'a> {
    // OPTIMISATION : Utilisation d'une référence pour éviter le clonage coûteux du graphe
    definition: &'a WorkflowDefinition,
}

impl<'a> WorkflowStateMachine<'a> {
    pub fn new(definition: &'a WorkflowDefinition) -> Self {
        Self { definition }
    }

    /// Détermine la liste des ID de nœuds qui peuvent être exécutés maintenant.
    // MODIFICATION : async pour permettre l'évaluation dynamique par le rules_engine
    pub async fn next_runnable_nodes(&self, instance: &WorkflowInstance) -> Vec<String> {
        let mut runnable = Vec::new();

        // Si le workflow est bloqué ou fini, rien ne bouge
        if instance.status == ExecutionStatus::Paused
            || instance.status == ExecutionStatus::Completed
            || instance.status == ExecutionStatus::Failed
        {
            return runnable;
        }

        for node in &self.definition.nodes {
            let node_id = &node.id;

            // 1. Si le nœud est déjà en cours ou traité, on l'ignore
            if let Some(status) = instance.node_states.get(node_id) {
                if *status != ExecutionStatus::Pending && *status != ExecutionStatus::Running {
                    continue;
                }
                if *status == ExecutionStatus::Running {
                    continue;
                }
            }

            // 2. Vérification des Parents (Dépendances)
            let parents = self.get_parents(node_id);

            // Cas spécial : Le nœud de départ n'a pas de parents
            if parents.is_empty() {
                if node_id == &self.definition.entry && !instance.node_states.contains_key(node_id)
                {
                    runnable.push(node_id.clone());
                }
                continue;
            }

            // 3. Logique de Synchronisation (Tous les parents doivent être terminés)
            let mut all_parents_done = true;
            let mut parent_failed = false;

            for parent_id in &parents {
                match instance.node_states.get(parent_id) {
                    Some(ExecutionStatus::Completed) => {
                        // Le parent est OK, mais l'arc a-t-il une condition ?
                        if !self
                            .check_transition_condition(parent_id, node_id, instance)
                            .await
                        {
                            // Parent OK mais condition non remplie => Ce chemin est fermé
                            all_parents_done = false;
                            break;
                        }
                    }
                    Some(ExecutionStatus::Skipped) => {
                        all_parents_done = false;
                        break;
                    }
                    Some(ExecutionStatus::Failed) => {
                        parent_failed = true;
                        all_parents_done = false;
                        break;
                    }
                    _ => {
                        // Parent Pending/Running/Paused
                        all_parents_done = false;
                        break;
                    }
                }
            }

            if parent_failed {
                // Si un parent (ex: Veto) a échoué, ce nœud ne s'exécutera jamais.
                continue;
            }

            if all_parents_done {
                runnable.push(node_id.clone());
            }
        }

        runnable
    }

    /// Applique le changement d'état après l'exécution d'un nœud
    pub fn transition(
        &self,
        instance: &mut WorkflowInstance,
        node_id: &str,
        new_status: ExecutionStatus,
    ) -> RaiseResult<()> {
        instance.node_states.insert(node_id.to_string(), new_status);

        if new_status == ExecutionStatus::Failed {
            tracing::error!("❌ Nœud {} échoué -> Arrêt du Workflow", node_id);
            instance.status = ExecutionStatus::Failed;
            return Ok(());
        }

        // Vérifier si c'était le dernier nœud
        if self.is_end_node(node_id) {
            tracing::info!("🏁 Fin du Workflow atteinte par le nœud {}", node_id);
            instance.status = ExecutionStatus::Completed;
        }

        Ok(())
    }

    // --- Helpers ---

    fn get_parents(&self, node_id: &str) -> Vec<String> {
        self.definition
            .edges
            .iter()
            .filter(|e| e.to == node_id)
            .map(|e| e.from.clone())
            .collect()
    }

    fn is_end_node(&self, node_id: &str) -> bool {
        if let Some(node) = self.definition.nodes.iter().find(|n| n.id == node_id) {
            if matches!(node.r#type, super::NodeType::End) {
                return true;
            }
        }
        !self.definition.edges.iter().any(|e| e.from == node_id)
    }

    async fn check_transition_condition(
        &self,
        from: &str,
        to: &str,
        instance: &WorkflowInstance,
    ) -> bool {
        let edge = self
            .definition
            .edges
            .iter()
            .find(|e| e.from == from && e.to == to);

        if let Some(e) = edge {
            if let Some(condition_script) = &e.condition {
                return self
                    .evaluate_condition(condition_script, &instance.context)
                    .await;
            }
        }

        true
    }

    async fn evaluate_condition(
        &self,
        script: &str,
        context: &UnorderedMap<String, JsonValue>,
    ) -> bool {
        let context_value = json::serialize_to_value(context).unwrap_or(json_value!({}));
        let provider = NoOpDataProvider;

        // 1. Tente de lire le script comme un AST JSON pour le rules_engine
        // OPTIMISATION ROBUSTE : On passe par une JsonValue intermédiaire (comme dans l'Executor)
        match json::deserialize_from_str::<JsonValue>(script) {
            Ok(val) => match json::deserialize_from_value::<Expr>(val) {
                Ok(expr) => match Evaluator::evaluate(&expr, &context_value, &provider).await {
                    Ok(res_cow) => {
                        return match res_cow.as_ref() {
                            JsonValue::Bool(b) => *b,
                            _ => false,
                        };
                    }
                    Err(e) => {
                        tracing::error!("❌ Erreur d'évaluation rules_engine: {}", e);
                        return false;
                    }
                },
                Err(ast_err) => {
                    // On loggue L'ERREUR EXACTE de désérialisation pour pouvoir débugger
                    tracing::warn!(
                        "⚠️ Échec du parsing de l'AST JSON : {}. Script reçu : {}",
                        ast_err,
                        script
                    );
                }
            },
            Err(_) => {
                // Ce n'est pas un JSON valide, on passe silencieusement au Legacy
            }
        }

        // 2. Fallback (Legacy)
        if script.contains("==") {
            let parts: Vec<&str> = script.split("==").collect();
            if parts.len() == 2 {
                let key = parts[0].trim();
                let target_val_str = parts[1].trim().replace("'", "").replace("\"", "");

                if let Some(actual_val) = context.get(key) {
                    if let Some(s) = actual_val.as_str() {
                        return s == target_val_str;
                    }
                    if let Some(b) = actual_val.as_bool() {
                        return b.to_string() == target_val_str;
                    }
                    if let Some(n) = actual_val.as_f64() {
                        return n.to_string() == target_val_str;
                    }
                }
            }
        }

        tracing::warn!("⚠️ Condition invalide ou non supportée : {}", script);
        false
    }
}

// =========================================================================
// TESTS UNITAIRES (ROBUSTESSE MAXIMALE)
// =========================================================================

// =========================================================================
// TESTS UNITAIRES (ROBUSTESSE MAXIMALE)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow_engine::{NodeType, WorkflowEdge, WorkflowNode};

    fn create_sequential_def() -> WorkflowDefinition {
        WorkflowDefinition {
            _id: None,
            handle: "wf_seq".into(),
            entry: "start".into(),
            nodes: vec![
                WorkflowNode {
                    id: "start".into(),
                    r#type: NodeType::Task,
                    name: "S".into(),
                    params: json_value!({}),
                },
                WorkflowNode {
                    id: "mid".into(),
                    r#type: NodeType::Task,
                    name: "M".into(),
                    params: json_value!({}),
                },
                WorkflowNode {
                    id: "end".into(),
                    r#type: NodeType::End,
                    name: "E".into(),
                    params: json_value!({}),
                },
            ],
            edges: vec![
                WorkflowEdge {
                    from: "start".into(),
                    to: "mid".into(),
                    channel: None,
                    condition: None,
                },
                WorkflowEdge {
                    from: "mid".into(),
                    to: "end".into(),
                    channel: None,
                    condition: None,
                },
            ],
        }
    }

    #[async_test]
    async fn test_sequential_flow() {
        let def = create_sequential_def();
        let sm = WorkflowStateMachine::new(&def);
        // 🎯 FIX : On passe les 4 arguments (handle, workflow_id, mission_id, context)
        let mut instance =
            WorkflowInstance::new("test_handle", "wf_seq", "test_mission", UnorderedMap::new());

        // 1. Initial : Start doit être runnable
        let next = sm.next_runnable_nodes(&instance).await;
        assert_eq!(next, vec!["start"]);

        // 2. Start exécuté
        sm.transition(&mut instance, "start", ExecutionStatus::Completed)
            .unwrap();

        // 3. Mid doit être runnable
        let next = sm.next_runnable_nodes(&instance).await;
        assert_eq!(next, vec!["mid"]);

        // 4. Mid exécuté
        sm.transition(&mut instance, "mid", ExecutionStatus::Completed)
            .unwrap();

        // 5. End runnable
        let next = sm.next_runnable_nodes(&instance).await;
        assert_eq!(next, vec!["end"]);
    }

    #[async_test]
    async fn test_end_node_completes_workflow() {
        let def = create_sequential_def();
        let sm = WorkflowStateMachine::new(&def);
        let mut instance =
            WorkflowInstance::new("test_handle", "wf_seq", "test_mission", UnorderedMap::new());

        // L'exécution du nœud "end" (de type End) doit marquer l'instance comme Completed
        sm.transition(&mut instance, "end", ExecutionStatus::Completed)
            .unwrap();

        assert_eq!(instance.status, ExecutionStatus::Completed);
    }

    #[async_test]
    async fn test_ast_conditional_branching_migrated() {
        let def = WorkflowDefinition {
            _id: None,
            handle: "wf_branch".into(),
            entry: "start".into(),
            nodes: vec![
                WorkflowNode {
                    id: "start".into(),
                    r#type: NodeType::Task,
                    name: "S".into(),
                    params: json_value!({}),
                },
                WorkflowNode {
                    id: "path_a".into(),
                    r#type: NodeType::Task,
                    name: "A".into(),
                    params: json_value!({}),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "start".into(),
                to: "path_a".into(),
                channel: None,
                condition: Some(r#"{"eq": [{"var": "status"}, {"val": "ok"}]}"#.into()),
            }],
        };
        let sm = WorkflowStateMachine::new(&def);

        // Cas A : Condition remplie
        let mut ctx_ok = UnorderedMap::new();
        ctx_ok.insert("status".into(), json_value!("ok"));
        let mut inst_ok = WorkflowInstance::new("test_handle", "wf_branch", "test_mission", ctx_ok);
        inst_ok
            .node_states
            .insert("start".into(), ExecutionStatus::Completed);

        assert_eq!(sm.next_runnable_nodes(&inst_ok).await, vec!["path_a"]);

        // Cas B : Condition non remplie
        let mut ctx_ko = UnorderedMap::new();
        ctx_ko.insert("status".into(), json_value!("error"));
        let mut inst_ko = WorkflowInstance::new("test_handle", "wf_branch", "test_mission", ctx_ko);
        inst_ko
            .node_states
            .insert("start".into(), ExecutionStatus::Completed);

        assert!(
            sm.next_runnable_nodes(&inst_ko).await.is_empty(),
            "La branche ne doit pas s'activer"
        );
    }

    #[async_test]
    async fn test_ast_conditional_branching() {
        // CORRECTION : Syntaxe strictement en minuscules comme exigé par rules_engine::ast::Expr
        let ast_condition = json_value!({ "gt": [{"var": "score"}, {"val": 8.0}] }).to_string();

        let def = WorkflowDefinition {
            _id: None,
            handle: "wf_ast".into(),
            entry: "start".into(),
            nodes: vec![
                WorkflowNode {
                    id: "start".into(),
                    r#type: NodeType::Task,
                    name: "S".into(),
                    params: json_value!({}),
                },
                WorkflowNode {
                    id: "path_ast".into(),
                    r#type: NodeType::Task,
                    name: "AST".into(),
                    params: json_value!({}),
                },
            ],
            edges: vec![WorkflowEdge {
                from: "start".into(),
                to: "path_ast".into(),
                channel: None,
                condition: Some(ast_condition),
            }],
        };
        let sm = WorkflowStateMachine::new(&def);

        // Cas A : Condition remplie (10.0 > 8.0)
        let mut ctx_ok = UnorderedMap::new();
        ctx_ok.insert("score".into(), json_value!(10.0));
        let mut inst_ok = WorkflowInstance::new("test_handle", "wf_ast", "test_mission", ctx_ok);
        inst_ok
            .node_states
            .insert("start".into(), ExecutionStatus::Completed);

        assert_eq!(sm.next_runnable_nodes(&inst_ok).await, vec!["path_ast"]);

        // Cas B : Condition non remplie (5.0 n'est pas > 8.0)
        let mut ctx_ko = UnorderedMap::new();
        ctx_ko.insert("score".into(), json_value!(5.0));
        let mut inst_ko = WorkflowInstance::new("test_handle", "wf_ast", "test_mission", ctx_ko);
        inst_ko
            .node_states
            .insert("start".into(), ExecutionStatus::Completed);

        assert!(sm.next_runnable_nodes(&inst_ko).await.is_empty());
    }

    #[async_test]
    async fn test_parent_failure_blocks_execution() {
        let def = create_sequential_def();
        let sm = WorkflowStateMachine::new(&def);
        let mut instance =
            WorkflowInstance::new("test_handle", "wf_seq", "test_mission", UnorderedMap::new());

        // Si start échoue
        sm.transition(&mut instance, "start", ExecutionStatus::Failed)
            .unwrap();

        // L'instance elle-même est marquée comme Failed par transition()
        assert_eq!(instance.status, ExecutionStatus::Failed);

        // Rien ne doit être exécutable
        assert!(sm.next_runnable_nodes(&instance).await.is_empty());
    }

    #[async_test]
    async fn test_instance_status_respected() {
        let def = create_sequential_def();
        let sm = WorkflowStateMachine::new(&def);
        let mut instance =
            WorkflowInstance::new("test_handle", "wf_seq", "test_mission", UnorderedMap::new());

        // On bloque manuellement l'instance
        instance.status = ExecutionStatus::Paused;

        // Même si le premier nœud est prêt, le fait que l'instance soit en pause bloque tout
        assert!(sm.next_runnable_nodes(&instance).await.is_empty());
    }
}
