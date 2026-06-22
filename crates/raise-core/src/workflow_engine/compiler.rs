// FICHIER : src-tauri/src/workflow_engine/compiler.rs
use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use super::mandate::Mandate;
use super::{NodeType, WorkflowDefinition, WorkflowEdge, WorkflowNode};

pub struct WorkflowCompiler;

impl WorkflowCompiler {
    /// 🎯 DATA-DRIVEN : Résout les dépendances techniques depuis la base de données.
    /// Utilise les points de montage pour localiser les configurations système.
    async fn resolve_tool_dependency(
        manager: &CollectionsManager<'_>,
        rule_name: &str,
    ) -> RaiseResult<(String, JsonValue, String)> {
        // 🎯 CORRECTION : Option -> RaiseResult
        match manager
            .get_document("configs", "ref:configs:tool_dependencies")
            .await
        {
            Ok(Some(doc)) => {
                if let Some(mapping) = doc.get("mappings").and_then(|m| m.as_object()) {
                    if let Some(rule_config) = mapping.get(rule_name).and_then(|r| r.as_object()) {
                        let tool_name = rule_config
                            .get("tool_name")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();
                        let arguments = rule_config
                            .get("arguments")
                            .cloned()
                            .unwrap_or(json_value!({}));
                        let output_key = rule_config
                            .get("output_key")
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string();

                        if !tool_name.is_empty() {
                            return Ok((tool_name, arguments, output_key));
                        }
                    }
                }
                raise_error!(
                    "ERR_COMPILER_TOOL_MISSING",
                    error = format!(
                        "Aucune configuration d'outil trouvée pour la règle '{}'.",
                        rule_name
                    ),
                    context = json_value!({"rule": rule_name})
                )
            }
            _ => raise_error!(
                "ERR_COMPILER_TOOL_CONFIG_UNAVAILABLE",
                error = "La configuration système des outils MCP est introuvable."
            ),
        }
    }

    /// 🎯 NOUVEAU : Compile dynamiquement un workflow à partir d'une Mission.
    /// Gère le tissage (Weaving) sécurisé entre les couches MBSE et la Gouvernance.
    pub async fn compile(
        manager: &CollectionsManager<'_>,
        mission_handle: &str,
    ) -> RaiseResult<WorkflowDefinition> {
        // 1. Charger la Mission via Match
        let mission_doc = match manager.get_document("missions", mission_handle).await? {
            Some(doc) => doc,
            None => raise_error!(
                "ERR_MISSION_NOT_FOUND",
                error = "La mission spécifiée est introuvable.",
                context = json_value!({"mission_id": mission_handle})
            ),
        };

        let template_handle = mission_doc["workflow_template_id"]
            .as_str()
            .unwrap_or_default();
        let mandate_handle = mission_doc["mandate_id"].as_str().unwrap_or_default();

        // 2. Charger le WorkflowTemplate
        let template_doc = match manager
            .get_document("workflow_definitions", template_handle)
            .await?
        {
            Some(doc) => doc,
            None => raise_error!(
                "ERR_TEMPLATE_NOT_FOUND",
                error = "Le template de workflow est manquant.",
                context = json_value!({"template_id": template_handle})
            ),
        };

        let mut workflow: WorkflowDefinition = match json::deserialize_from_value(template_doc) {
            Ok(wf) => wf,
            Err(e) => raise_error!("ERR_WORKFLOW_DESERIALIZATION", error = e.to_string()),
        };

        // 3. Charger le Mandat (Règles de gouvernance)
        let mandate = Mandate::fetch_from_store(manager, mandate_handle).await?;

        // 4. "Weaving" (Tissage) : Injection résiliente des vetos
        let original_entry = workflow.entry.clone();
        let mut previous_node_id = original_entry.clone();

        // Isolation des arêtes de départ pour injection
        let entry_edges: Vec<WorkflowEdge> = workflow
            .edges
            .iter()
            .filter(|e| e.from == original_entry)
            .cloned()
            .collect();
        workflow.edges.retain(|e| e.from != original_entry);

        for (i, veto) in mandate.hard_logic.vetos.iter().enumerate() {
            if veto.active {
                // 🎯 Résolution Tooling MCP stricte
                let (tool_name, args, output_key) =
                    Self::resolve_tool_dependency(manager, &veto.rule).await?;

                let tool_node_id = format!("tool_read_{}_{}", veto.rule.to_lowercase(), i);
                workflow.nodes.push(WorkflowNode {
                    id: tool_node_id.clone(),
                    r#type: NodeType::CallMcp,
                    name: format!("Lecture pour {}", veto.rule),
                    params: json_value!({
                        "tool_name": tool_name,
                        "arguments": args,
                        "output_key": output_key
                    }),
                });
                workflow.edges.push(WorkflowEdge {
                    from: previous_node_id.clone(),
                    to: tool_node_id.clone(),
                    channel: None,
                    condition: None,
                });
                previous_node_id = tool_node_id;

                // Nœud de contrôle QualityGate
                let node_id = format!("quality_gate_{}_{}", veto.rule.to_lowercase(), i);
                let mut params = json_value!({
                    "rule": veto.rule,
                    "action": veto.action
                });

                if let Some(ast) = &veto.ast {
                    if let Some(obj) = params.as_object_mut() {
                        obj.insert("ast".to_string(), ast.clone());
                    }
                }

                workflow.nodes.push(WorkflowNode {
                    id: node_id.clone(),
                    r#type: NodeType::QualityGate,
                    name: format!("Vérification: {}", veto.rule),
                    params,
                });

                workflow.edges.push(WorkflowEdge {
                    from: previous_node_id.clone(),
                    to: node_id.clone(),
                    channel: None,
                    condition: None,
                });
                previous_node_id = node_id;
            }
        }

        // Re-bouclage vers le reste du graphe original
        for mut edge in entry_edges {
            edge.from = previous_node_id.clone();
            workflow.edges.push(edge);
        }

        // Signature unique de l'instance compilée
        workflow._id = None;
        workflow.handle = format!(
            "wf_compiled_{}_{}",
            mandate_handle,
            UtcClock::now().timestamp_millis()
        );

        Ok(workflow)
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité & Résilience Mount Points)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    #[async_test]
    async fn test_compiler_mission_weaving() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la config système
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );

        manager.create_collection("configs", &schema_uri).await?;
        manager
            .upsert_document(
                "configs",
                json_value!({
                    "handle": "tool_dependencies",
                    "_id": "ref:configs:tool_dependencies",
                    "mappings": {
                        "ISO_26262_CHK": {
                            "tool_name": "safety_checker",
                            "arguments": {},
                            "output_key": "safety_report"
                        }
                    }
                }),
            )
            .await?;

        // 1. Setup Workflow Template
        manager
            .create_collection("workflow_definitions", &schema_uri)
            .await?;
        manager
            .upsert_document(
                "workflow_definitions",
                json_value!({
                    "handle": "tpl_mbse_v1",
                    "entry_node_id": "start",  
                    "nodes": [
                        { "node_id": "start", "type": "task", "name": "Start", "params": {} },  
                        { "node_id": "task_1", "type": "task", "name": "Phase LA", "params": {} }  
                    ],
                    "edges": [{ "from_node_id": "start", "to_node_id": "task_1", "condition": null }]  
                }),
            )
            .await?;

        // 2. Setup Mandat
        manager.create_collection("mandates", &schema_uri).await?;
        manager.upsert_document("mandates", json_value!({
            "handle": "mandate-123",
            "name": "Mandat de Sécurité",
            "meta": { "mandator_id": "00000000-0000-0000-0000-000000000000", "version": "1.0", "status": "ACTIVE" },
            "governance": { "strategy": "SAFETY_FIRST", "condorcetWeights": {} },
            "hardLogic": {
                "vetos": [{ "rule": "ISO_26262_CHK", "active": true, "action": "STOP", "ast": {"Eq": [{"Var": "x"}, {"Val": 1}]} }]
            },
            "observability": { "heartbeatMs": 100 }
        })).await?;

        // 3. Setup Mission
        manager.create_collection("missions", &schema_uri).await?;
        manager
            .upsert_document(
                "missions",
                json_value!({
                    "handle": "mission_alpha",
                    "name": "Mission Alpha",
                    "mandate_id": "mandate-123",
                    "squad_id": "squad_arch",
                    "workflow_template_id": "tpl_mbse_v1",
                    "status": "draft"
                }),
            )
            .await?;

        let wf = WorkflowCompiler::compile(&manager, "mission_alpha").await?;

        // 🎯 FIX : Le graphe passe de 3 à 4 nœuds car le compilateur
        // a automatiquement tissé un nœud CallMcp *avant* le QualityGate.
        assert_eq!(wf.nodes.len(), 4);

        // On vérifie que l'outil de collecte et la porte de qualité sont bien présents
        assert!(wf.nodes.iter().any(|n| n.r#type == NodeType::CallMcp));
        assert!(wf.nodes.iter().any(|n| n.r#type == NodeType::QualityGate));

        // Le routage final vers la porte de qualité doit toujours exister
        assert!(wf
            .edges
            .iter()
            .any(|e| e.to == "quality_gate_iso_26262_chk_0"));

        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face aux points de montage invalides
    #[async_test]
    async fn test_compiler_mount_point_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        // Manager pointant sur une partition système inexistante
        let manager = CollectionsManager::new(&sandbox.db, "ghost_partition", "void_db");

        let result = WorkflowCompiler::compile(&manager, "any_mission").await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_MISSION_NOT_FOUND");
                Ok(())
            }
            _ => panic!("Le compilateur aurait dû lever ERR_MISSION_NOT_FOUND"),
        }
    }

    /// 🎯 NOUVEAU TEST : Inférence résiliente des dépendances Tooling
    #[async_test]
    async fn test_resolve_tool_dependency_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test", "test");

        // 🎯 FIX : Le compilateur est désormais "Fail-Fast".
        // Il DOIT lever une erreur si la configuration système est absente.
        let result = WorkflowCompiler::resolve_tool_dependency(&manager, "DUMMY_RULE").await;

        match result {
            Err(crate::utils::core::error::AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_COMPILER_TOOL_CONFIG_UNAVAILABLE");
                Ok(())
            }
            _ => panic!("Le compilateur aurait dû lever ERR_COMPILER_TOOL_CONFIG_UNAVAILABLE"),
        }
    }
}
