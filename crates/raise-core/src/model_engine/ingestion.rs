// FICHIER : crates/raise-core/src/model_engine/ingestion.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::model_engine::arcadia::element_kind::ArcadiaSemantics;
use crate::model_engine::arcadia::element_kind::Layer;
use crate::model_engine::capella::model_reader::CapellaReader;
use crate::model_engine::eurlex::parser::EurlexParser;
use crate::model_engine::transformers::eurlex_to_model::EurlexToModelTransformer;
use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

pub struct ModelIngestionService;

impl ModelIngestionService {
    /// Helper pour déduire l'URI du schéma v2 à partir du vecteur `@type` (kind)
    fn resolve_schema_uri(kinds: &[String]) -> String {
        // Cible validée par la configuration environnementale RAISE_ASSET_DOMAIN / DB
        let base_uri = "db://_system/ai-assets/schemas/v2";

        for kind in kinds {
            match kind.as_str() {
                // Routage exact MBSE Arcadia v2
                "raise:Dapp" | "pa:PhysicalComponent" => {
                    return format!("{}/mbse/pa/physical_component.schema.json", base_uri)
                }
                "raise:Service" | "sa:SystemComponent" => {
                    return format!("{}/mbse/sa/system_component.schema.json", base_uri)
                }
                "raise:Module" | "la:LogicalComponent" => {
                    return format!("{}/mbse/la/logical_component.schema.json", base_uri)
                }
                "la:LogicalFunction" => {
                    return format!("{}/mbse/la/logical_function.schema.json", base_uri)
                }
                "sa:SystemCapability" => {
                    return format!("{}/mbse/sa/system_capability.schema.json", base_uri)
                }
                "arcadia:OperationalActivity" => {
                    return format!("{}/mbse/oa/activity.schema.json", base_uri)
                }
                "arcadia:OperationalEntity" => {
                    return format!("{}/mbse/oa/entity.schema.json", base_uri)
                }
                _ => continue,
            }
        }

        // Fallback de sécurité
        format!("{}/common/types/base.schema.json", base_uri)
    }

    /// Ingestion asynchrone d'un fichier Capella (.aird / .capella)
    pub async fn ingest_capella(
        path: PathBuf,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<usize> {
        user_info!(
            "INF_INGESTION_CAPELLA_START",
            json_value!({"path": path.to_string_lossy()})
        );

        let parse_result = spawn_cpu_task(move || CapellaReader::read_model(&path)).await;

        let model = match parse_result {
            Ok(res) => match res {
                Ok(m) => m,
                Err(e) => raise_error!(
                    "ERR_INGESTION_CAPELLA_PARSE",
                    error = e.to_string(),
                    context = json_value!({"action": "parsing_xml"})
                ),
            },
            Err(e) => raise_error!(
                "ERR_INGESTION_CPU_PANIC",
                error = e.to_string(),
                context = json_value!({"action": "spawn_cpu_task"})
            ),
        };

        Self::persist_model(&model, manager).await
    }

    /// Ingestion asynchrone d'une directive européenne (EUR-Lex XML)
    pub async fn ingest_eurlex(
        path: PathBuf,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<usize> {
        user_info!(
            "INF_INGESTION_EURLEX_START",
            json_value!({"path": path.to_string_lossy()})
        );

        let path_clone = path.clone();
        let parse_result = spawn_cpu_task(move || EurlexParser::parse_xml(&path_clone)).await;

        let parsed_data = match parse_result {
            Ok(res) => match res {
                Ok(data) => data,
                Err(e) => return Err(e),
            },
            Err(e) => raise_error!(
                "ERR_INGESTION_EURLEX_CRITICAL",
                error = e.to_string(),
                context = json_value!({"action": "spawn_cpu_task"})
            ),
        };

        let model = EurlexToModelTransformer::transform_to_model(&parsed_data)?;
        let elements_inserted = Self::persist_model(&model, manager).await?;

        user_success!(
            "SUC_INGESTION_EURLEX_DONE",
            json_value!({
                "message": "Directive ingérée avec succès",
                "elements_inserted": elements_inserted
            })
        );

        Ok(elements_inserted)
    }

    /// Hydratation du Knowledge Graph (JSON-DB) à partir d'un modèle en mémoire.
    pub async fn persist_model(
        model: &ProjectModel,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<usize> {
        let elements = model.all_elements();
        let count = elements.len();

        for el in elements {
            let layer_prefix = match el.get_layer() {
                Layer::OperationalAnalysis => "oa",
                Layer::SystemAnalysis => "sa",
                Layer::LogicalArchitecture => "la",
                Layer::PhysicalArchitecture => "pa",
                Layer::EPBS => "epbs",
                Layer::Data => "data",
                Layer::Transverse => "transverse",
                Layer::Unknown => "other",
            };

            let category_str = format!("{:?}", el.get_category()).to_lowercase();
            // Construction du nom (ex: "pa_component" + "s" = "pa_components")
            let collection_name = format!("{}_{}s", layer_prefix, category_str);

            let schema_uri = Self::resolve_schema_uri(&el.kind);

            match manager
                .create_collection(&collection_name, &schema_uri)
                .await
            {
                Ok(_) => (),
                Err(e) => raise_error!("ERR_INGESTION_COLLECTION_SETUP", error = e.to_string()),
            }

            let doc = match json::serialize_to_value(el) {
                Ok(v) => v,
                Err(e) => raise_error!(
                    "ERR_INGESTION_SERIALIZATION",
                    error = e.to_string(),
                    context = json_value!({"element_handle": el.handle})
                ),
            };

            match manager.upsert_document(&collection_name, doc).await {
                Ok(_) => (),
                Err(e) => raise_error!(
                    "ERR_INGESTION_DB_UPSERT",
                    error = e.to_string(),
                    context =
                        json_value!({"element_handle": el.handle, "collection": collection_name})
                ),
            }
        }

        user_success!(
            "SUC_INGESTION_COMPLETED",
            json_value!({"element_count": count})
        );
        Ok(count)
    }
}

// =========================================================================
// TESTS UNITAIRES (Respect des tests existants & Résilience Mount Points)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ArcadiaElement;
    use crate::utils::testing::mock::AgentDbSandbox;

    /*
        #[async_test]
        async fn test_persist_model_routes_to_correct_collections() -> RaiseResult<()> {
            let sandbox = DbSandbox::new().await?;
            let manager = CollectionsManager::new(
                &sandbox.storage,
                &sandbox.config.mount_points.system.domain,
                &sandbox.config.mount_points.system.db,
            );

            // 1. Enregistrer le schéma pour pa:PhysicalComponent (type reconnu par resolve_schema_uri)
            inject_v2_schema_mock(
                &sandbox.storage.config,
                &mut json_value!({}),
                "mbse/pa/physical_component",
            )
            .await;

            let mut model = ProjectModel::default();
            model.add_element(
                "pa",
                "components",
                ArcadiaElement {
                    handle: "comp_1".try_into()?,
                    kind: vec!["pa:PhysicalComponent".into()],
                    ..Default::default()
                },
            );

            // 2. Persister le modèle
            ModelIngestionService::persist_model(&model, &manager).await?;

            // 3. Vérifier la collection spécifique
            let comp_doc = manager.get_document("pa_components", "comp_1").await?;
            assert!(
                comp_doc.is_some(),
                "Le composant doit être trouvé dans pa_components"
            );

            Ok(())
        }
    */

    #[async_test]
    async fn test_ingestion_mount_point_resilience() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "ghost_partition", "void_db");

        let mut model = ProjectModel::default();
        model.add_element(
            "test",
            "elements",
            ArcadiaElement {
                handle: "err_1".try_into()?,
                name: I18nString::Single("Err".into()),
                kind: vec!["test#Element".into()],
                ..Default::default()
            },
        );

        let result = ModelIngestionService::persist_model(&model, &manager).await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_INGESTION_COLLECTION_SETUP");
                Ok(())
            }
            _ => panic!("L'ingestion aurait dû lever ERR_INGESTION_COLLECTION_SETUP"),
        }
    }
}
