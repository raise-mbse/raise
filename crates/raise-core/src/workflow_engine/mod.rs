// FICHIER : crates/raise-core/src/workflow_engine/mod.rs

pub mod compiler;
pub mod critic;
pub mod executor;
pub mod handlers;
pub mod mandate;
pub mod rbac;
pub mod scheduler;
pub mod squad;
pub mod state_machine;
pub mod tools;

use crate::utils::prelude::*;

// --- RE-EXPORTS (L'API Publique du Moteur) ---
pub use compiler::WorkflowCompiler;
pub use executor::WorkflowExecutor;
pub use mandate::Mandate;
pub use scheduler::WorkflowScheduler;
pub use state_machine::WorkflowStateMachine;

/// Type d'un nœud dans le graphe (aligné avec les besoins MBSE)
#[derive(Debug, Clone, Serializable, Deserializable, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    Task,
    Decision,
    Parallel,
    GateHitl,
    #[serde(alias = "gate_policy")] // 🎯 ALIGNEMENT SCHEMA : Tolérance de désérialisation
    QualityGate,
    Genetics,
    WorldModel,
    CallMcp,
    Wasm,
    Milestone,
    SubProject,
    #[serde(alias = "store_memory")] // 🎯 SCHEMA MAPPING
    StoreMemory,
    #[serde(alias = "emit_event")] // 🎯 SCHEMA MAPPING
    EmitEvent,
    End,
}

/// Statut d'exécution d'une instance ou d'un nœud
#[derive(Debug, Clone, Copy, Serializable, Deserializable, PartialEq)]
#[serde(rename_all = "snake_case")] // 🎯 ALIGNEMENT SCHEMA : lowercase strict (pending, running...)
pub enum ExecutionStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Paused,
    Skipped,
    Aborted, // 🎯 NOUVEAU (Issu du workflow_instance.schema.json)
    Blocked,
    InReview,
}

/// Nœud unitaire du workflow (Le Graphe)
#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct WorkflowNode {
    #[serde(rename = "node_id")] // 🎯 ALIGNEMENT SCHEMA
    pub id: String,
    pub r#type: NodeType,
    pub name: String,
    pub params: JsonValue,
}

/// Lien orienté entre deux nœuds
#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct WorkflowEdge {
    #[serde(rename = "from_node_id")] // 🎯 ALIGNEMENT SCHEMA
    pub from: String,
    #[serde(rename = "to_node_id")] // 🎯 ALIGNEMENT SCHEMA
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>, // 🎯 NOUVEAU : Type de com (U2C, C2A, A2A)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
}

/// Définition statique du Workflow (Le "Template" ou "Plan" compilé)
#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct WorkflowDefinition {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub _id: Option<String>,
    pub handle: String,
    pub nodes: Vec<WorkflowNode>,
    pub edges: Vec<WorkflowEdge>,
    #[serde(rename = "entry_node_id")] // 🎯 ALIGNEMENT SCHEMA
    pub entry: String,
}

/// Instance dynamique (L'Exécution en cours - Jumeau Numérique)
#[derive(Debug, Clone, Serializable, Deserializable)]
// 🎯 SUPPRESSION DU camelCase GLOBAL : Le schéma exige du snake_case strict
pub struct WorkflowInstance {
    #[serde(rename = "_id", skip_serializing_if = "Option::is_none")]
    pub _id: Option<String>,
    pub handle: String,

    #[serde(rename = "workflow_template_id")] // 🎯 ALIGNEMENT SCHEMA
    pub workflow_id: String,

    pub mission_id: String,
    pub status: ExecutionStatus,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_node_id: Option<String>, // 🎯 NOUVEAU : Exigé par le schéma

    /// État de chaque nœud : NodeID -> Status
    pub node_states: UnorderedMap<String, ExecutionStatus>,

    /// Mémoire contextuelle (Jumeau Numérique / Données MBSE partagées)
    pub context: UnorderedMap<String, JsonValue>,

    /// Traces d'explicabilité générées par l'IA (UUIDs vers XaiFrame)
    pub xai_traces: Vec<String>,

    /// Journal d'audit détaillé
    #[serde(rename = "execution_logs")] // 🎯 ALIGNEMENT SCHEMA
    pub logs: Vec<String>,

    pub created_at: i64,
    pub updated_at: i64,
}

impl WorkflowInstance {
    pub fn new(
        handle: &str,
        workflow_id: &str,
        mission_id: &str,
        initial_context: UnorderedMap<String, JsonValue>,
    ) -> Self {
        Self {
            _id: None,
            handle: handle.to_string(),
            workflow_id: workflow_id.to_string(),
            mission_id: mission_id.to_string(),
            status: ExecutionStatus::Pending,
            current_node_id: None, // Initialisation
            node_states: UnorderedMap::new(),
            context: initial_context,
            xai_traces: Vec::new(),
            logs: vec![format!(
                "Création de l'instance pour la mission {}",
                mission_id
            )],
            created_at: UtcClock::now().timestamp(),
            updated_at: UtcClock::now().timestamp(),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Corrigés pour s'aligner sur les nouveaux sérialiseurs)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_node_type_serialization() {
        let t1 = NodeType::GateHitl;
        let json_t1 = json::serialize_to_string(&t1).unwrap();
        assert_eq!(json_t1, "\"gate_hitl\"");

        let t2 = NodeType::QualityGate;
        let json_t2 = json::serialize_to_string(&t2).unwrap();
        assert_eq!(json_t2, "\"quality_gate\"");
    }

    #[test]
    fn test_execution_status_serialization() {
        // 🎯 FIX TEST : Vérifie la sérialisation en snake_case
        let s1 = ExecutionStatus::InReview;
        let json_s1 = json::serialize_to_string(&s1).unwrap();
        assert_eq!(json_s1, "\"in_review\"");

        let s2 = ExecutionStatus::Paused;
        let json_s2 = json::serialize_to_string(&s2).unwrap();
        assert_eq!(json_s2, "\"paused\"");
    }

    #[test]
    fn test_workflow_instance_initialization() {
        let handle = "mission-apollo-11-exec";
        let wf_id = "wf_template_vcycle";
        let mission_id = "uuid-mission-1234";

        let mut initial_ctx = UnorderedMap::new();
        initial_ctx.insert("budget".into(), json_value!(5000));

        let instance = WorkflowInstance::new(handle, wf_id, mission_id, initial_ctx);

        assert_eq!(instance.handle, handle);
        assert_eq!(instance.workflow_id, wf_id);
        assert_eq!(instance.mission_id, mission_id);
        assert_eq!(instance.status, ExecutionStatus::Pending);
        assert!(instance.current_node_id.is_none());

        assert_eq!(
            instance.context.get("budget").unwrap().as_i64().unwrap(),
            5000
        );

        assert!(instance.xai_traces.is_empty());
        assert!(instance.node_states.is_empty());
        assert_eq!(instance.logs.len(), 1);
    }
}
