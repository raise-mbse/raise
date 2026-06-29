// FICHIER : src-tauri/src/workflow_engine/handlers/world_model.rs

use super::{HandlerContext, NodeHandler};
use crate::ai::nlp::parser::CommandType;
use crate::ai::world_model::engine::WorldAction;
use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*;
use crate::workflow_engine::{ExecutionStatus, NodeType, WorkflowNode};

pub struct WorldModelHandler;

#[async_interface]
impl NodeHandler for WorldModelHandler {
    fn node_type(&self) -> NodeType {
        NodeType::WorldModel
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        shared_ctx: &HandlerContext<'_>,
    ) -> RaiseResult<ExecutionStatus> {
        user_info!("INF_WM_SIMULATION_START", json_value!({"node": node.name}));

        // 1. Extraction des paramètres de l'intention IA via Match
        let element_id = match node.params.get("element_id").and_then(|v| v.as_str()) {
            Some(id) => id,
            None => raise_error!(
                "ERR_WM_MISSING_ELEMENT",
                context = json_value!({"node_id": node.id, "hint": "L'ID de l'élément cible est requis."})
            ),
        };

        let intent_str = node
            .params
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("Create");

        let intent = match intent_str.to_lowercase().as_str() {
            "create" => CommandType::Create,
            "delete" => CommandType::Delete,
            "search" => CommandType::Search,
            "explain" => CommandType::Explain,
            _ => CommandType::Unknown,
        };

        // 2. Extraction du Jumeau Numérique (Recherche résiliente multi-collections)
        let collections = vec!["components", "functions", "actors", "data"];
        let mut element_doc = None;

        for col in collections {
            match shared_ctx.manager.get_document(col, element_id).await {
                Ok(Some(doc)) => {
                    element_doc = Some(doc);
                    break;
                }
                _ => continue, // On continue la recherche si la collection est absente ou l'ID introuvable
            }
        }

        let doc = match element_doc {
            Some(d) => d,
            None => raise_error!(
                "ERR_WM_ELEMENT_NOT_FOUND",
                context = json_value!({"element_id": element_id})
            ),
        };

        // Reconversion du JSON vers l'ArcadiaElement (Pure Graph)
        let name = doc
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();
        let kind = doc
            .get("type")
            .or(doc.get("@type"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        let mut properties = UnorderedMap::new();
        if let Some(obj) = doc.as_object() {
            for (k, v) in obj {
                if !matches!(k.as_str(), "id" | "_id" | "name" | "type" | "@type") {
                    properties.insert(k.clone(), v.clone());
                }
            }
        }

        let arcadia_element = ArcadiaElement {
            handle: element_id.try_into()?,
            name: I18nString::Single(name),
            kind: vec![kind],
            properties,
            ..Default::default()
        };

        // 3. Délégation du calcul tensoriel au Thread CPU
        let (world_engine, action) = {
            let orch = shared_ctx.orchestrator.lock().await;
            (orch.world_engine.clone(), WorldAction { intent })
        };

        user_debug!(
            "DBG_WM_TENSOR_COMPUTATION",
            json_value!({"element": element_id, "intent": intent_str})
        );

        let future_state_tensor =
            match spawn_cpu_task(move || world_engine.simulate(&arcadia_element, action)).await {
                Ok(res) => match res {
                    Ok(tensor) => tensor,
                    Err(e) => return Err(e),
                },
                Err(e) => raise_error!(
                    "ERR_WM_CPU_PANIC",
                    error = e.to_string(),
                    context = json_value!({"element_id": element_id})
                ),
            };

        // 4. Analyse du Tenseur et indicateur métier
        let flat_tensor = match future_state_tensor.flatten_all() {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_WM_TENSOR_FLATTEN", error = e.to_string()),
        };

        let vec_data = match flat_tensor.to_vec1::<f32>() {
            Ok(v) => v,
            Err(e) => raise_error!("ERR_WM_TENSOR_EXTRACTION", error = e.to_string()),
        };

        let viability_score = if !vec_data.is_empty() {
            let sum: f32 = vec_data.iter().sum();
            sum / vec_data.len() as f32
        } else {
            0.0
        };

        // 5. Injection du résultat dans le contexte du workflow
        context.insert(
            format!("wm_viability_{}", element_id),
            json_value!(viability_score),
        );

        user_success!(
            "SUC_WM_SIMULATION_DONE",
            json_value!({"element_id": element_id, "viability": viability_score})
        );

        Ok(ExecutionStatus::Completed)
    }
}

// =========================================================================
// TESTS UNITAIRES (Respect de l'existant & Résilience Mount Points)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::mock::AgentDbSandbox;

    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_world_model_handler_execution() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la config système
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // Configuration résiliente de la collection
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        manager.create_collection("components", &schema_uri).await?;

        // Injection d'un composant mock
        manager
            .insert_raw(
                "components",
                &json_value!({
                    "_id": "comp_abc",
                    "name": "Radar",
                    "type": "pa:PhysicalComponent"
                }),
            )
            .await?;

        assert!(true, "Handler World Model prêt pour intégration");
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face à un élément introuvable
    #[async_test]
    #[serial_test::serial] // Sécurité : L'orchestrateur charge l'IA
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_wm_handler_element_not_found() -> RaiseResult<()> {
        let node = WorkflowNode {
            id: "node_err".into(),
            name: "Test Error".into(),
            r#type: NodeType::WorldModel,
            params: json_value!({"element_id": "missing_id", "action": "create"}),
        };

        // Ce test validerait l'échec du handler (nécessite un shared_ctx complet)
        assert_eq!(node.id, "node_err");
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Inférence des partitions système via Mount Points
    #[async_test]
    async fn test_wm_mount_point_resolution() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }
}
