// FICHIER : src-tauri/src/services/model_service.rs

use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::loader::ModelLoader;
use crate::model_engine::types::ProjectModel;

/// Charge l'intégralité du modèle en mémoire pour analyse.
/// Respecte les points de montage pour la résolution sémantique.
pub async fn load_project_model(
    storage: &StorageEngine,
    space: &str,
    db: &str,
) -> RaiseResult<ProjectModel> {
    let loader = ModelLoader::from_engine(storage, space, db)?;

    match loader.load_full_model().await {
        Ok(model) => Ok(model),
        Err(e) => raise_error!(
            "ERR_MODEL_LOAD_FAIL",
            error = e.to_string(),
            context = json_value!({
                "action": "load_full_project_model",
                "space": space,
                "db": db
            })
        ),
    }
}

/// 📥 Ingeste un tableau JSON généré par l'IA dans le Graphe Arcadia.
/// Utilise le mapping ontologique pour router, en s'appuyant sur le Schéma JSON strict.
pub async fn ingest_arcadia_elements(
    storage: &StorageEngine,
    domain: &str,
    sys_db: &str,
    json_output: &str,
) -> RaiseResult<Vec<String>> {
    // 1. Désérialisation résiliente
    let parsed_json = match json::deserialize_from_str::<JsonValue>(json_output) {
        Ok(j) => j,
        Err(e) => raise_error!("ERR_JSON_PARSE", error = e.to_string()),
    };

    // 2. Extraction du lot d'éléments (Tolérance Array/Object)
    let elements = if let Some(arr) = parsed_json.as_array() {
        arr.clone()
    } else if let Some(obj) = parsed_json.as_object() {
        let found_array = obj
            .get("elements")
            .and_then(|v| v.as_array())
            .or_else(|| obj.values().find_map(|v| v.as_array()));

        match found_array {
            Some(arr) => arr.clone(),
            None => raise_error!(
                "ERR_FORMAT_ELEMENTS_MISSING",
                error = "Aucun tableau d'éléments trouvé dans l'objet JSON."
            ),
        }
    } else {
        raise_error!(
            "ERR_FORMAT_UNRECOGNIZED",
            error = "Le format fourni par l'IA n'est pas un tableau ou un objet valide."
        );
    };

    // 3. Récupération du mapping ontologique via le Manager Système
    let sys_mgr = CollectionsManager::new(storage, domain, sys_db);
    let mapping_doc = match sys_mgr
        .get_document("configs", "ref:configs:handle:ontological_mapping")
        .await
    {
        Ok(Some(doc)) => doc,
        Ok(None) => raise_error!(
            "ERR_ONTOLOGY_MAPPING_NOT_FOUND",
            error = "Document de mapping ontologique manquant en base système."
        ),
        Err(e) => raise_error!("ERR_ONTOLOGY_READ_FAIL", error = e.to_string()),
    };

    let mappings = match mapping_doc.get("mappings").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => raise_error!(
            "ERR_MAPPING_PROPERTY_MISSING",
            error = "La propriété 'mappings' est absente du document de configuration."
        ),
    };

    let mut ingested_ids = Vec::new();

    // 4. Routage et Insertion (Pattern Matching Strict)
    for el in &elements {
        let doc = el.clone();
        let kind = doc
            .get("@type")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown")
            .to_string();

        match mappings.get(&kind) {
            Some(mapping) => {
                let target_layer = mapping["layer"].as_str().unwrap_or(sys_db);
                let target_col_opt = mapping["collection"].as_str();

                match target_col_opt {
                    Some(target_collection) => {
                        let target_mgr = CollectionsManager::new(storage, domain, target_layer);
                        match target_mgr.upsert_document(target_collection, doc).await {
                            Ok(res) => ingested_ids.push(res),
                            Err(e) => user_warn!(
                                "WRN_INGESTION_FAILED",
                                json_value!({"error": e.to_string(), "kind": kind})
                            ),
                        }
                    }
                    None => user_warn!(
                        "WRN_MAPPING_COLLECTION_UNDEFINED",
                        json_value!({ "kind": kind })
                    ),
                }
            }
            None => user_warn!("WRN_UNKNOWN_ONTOLOGY_KIND", json_value!({ "kind": kind })),
        }
    }

    Ok(ingested_ids)
}

// =========================================================================
// INGESTION DE RÉFÉRENCES NORMATIVES (EUR-Lex)
// =========================================================================

/// ⚖️ Ingeste une directive européenne (EUR-Lex) et extrait l'ontologie via le moteur d'ingestion.
/// Protégé par la façade R.A.I.S.E (Zéro appel direct à std::).
pub async fn ingest_eurlex_directive(
    storage: &StorageEngine,
    domain: &str,
    db: &str,
    path_str: &str,
) -> RaiseResult<usize> {
    use crate::model_engine::ingestion::ModelIngestionService;

    crate::user_info!("SRV_INGEST_EURLEX_INIT", json_value!({ "path": path_str }));

    // 1. Utilisation stricte de la façade RAISE (crate::utils::prelude::PathBuf et fs::)
    let path = PathBuf::from(path_str);

    if !fs::exists_async(&path).await {
        // La macro gère le return Err() automatiquement
        raise_error!(
            "ERR_FS_NOT_FOUND",
            error = "Le fichier XML de la directive est introuvable.",
            context = json_value!({"path": path_str})
        );
    }

    // 2. Initialisation du Manager sur le Workspace actif
    let manager = CollectionsManager::new(storage, domain, db);

    // 3. Délégation au noyau MBSE
    let elements_inserted = ModelIngestionService::ingest_eurlex(path, &manager).await?;

    crate::user_success!(
        "SRV_INGEST_EURLEX_SUCCESS",
        json_value!({ "elements_inserted": elements_inserted, "path": path_str })
    );

    Ok(elements_inserted)
}

// =========================================================================
// TESTS UNITAIRES (Conformité Façade & Résilience Mount Points)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn inject_mock_mapping(manager: &CollectionsManager<'_>) -> RaiseResult<()> {
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );

        manager.create_collection("configs", &schema_uri).await?;
        manager
            .upsert_document(
                "configs",
                json_value!({
                    "_id": "ref:configs:handle:ontological_mapping",
                    "mappings": {
                        "OperationalActor": { "layer": "oa", "collection": "actors" },
                        "EnvironmentalConstraint": { "layer": "oa", "collection": "constraints" }
                    },
                    "search_spaces": [ { "layer": "oa", "collection": "actors" } ]
                }),
            )
            .await?;
        Ok(())
    }

    #[async_test]
    async fn test_load_project_model_command() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 RÉSILIENCE : Utilisation des nouveaux Mount Points
        let sys_mgr = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        DbSandbox::mock_db(&sys_mgr).await?;
        inject_mock_mapping(&sys_mgr).await?;

        let loader = ModelLoader::from_engine(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )?;

        let result = loader.load_full_model().await?;
        assert_eq!(result.meta.element_count, 0);
        Ok(())
    }

    #[async_test]
    async fn test_ingest_arcadia_elements_success() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let domain = &config.mount_points.system.domain;
        let sys_db = &config.mount_points.system.db;

        // 1. Setup du Système (Mapping Ontologique)
        let sys_mgr = CollectionsManager::new(&sandbox.db, domain, sys_db);
        DbSandbox::mock_db(&sys_mgr).await?;
        inject_mock_mapping(&sys_mgr).await?;

        // 2. Setup de la base cible
        let target_mgr = CollectionsManager::new(&sandbox.db, domain, "oa");
        DbSandbox::mock_db(&target_mgr).await?;
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            domain, sys_db
        );
        target_mgr.create_collection("actors", &schema_uri).await?;

        // 3. Simulation JSON
        let llm_json_output = r#"[
            { "handle": "astronaut", "name": "Astronaute", "@type": "OperationalActor" }
        ]"#;

        // 4. Exécution
        let result = ingest_arcadia_elements(&sandbox.db, domain, sys_db, llm_json_output).await?;
        assert_eq!(result.len(), 1);

        // 5. Vérification Physique
        let doc = target_mgr.get_document("actors", "astronaut").await?;
        assert!(doc.is_some());
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience si le mapping ontologique est corrompu
    #[async_test]
    async fn test_resilience_missing_mapping_document() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // On ne crée pas le document configs:ontological_mapping
        let res = ingest_arcadia_elements(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
            "[]",
        )
        .await;

        match res {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_ONTOLOGY_MAPPING_NOT_FOUND");
                Ok(())
            }
            _ => panic!("Aurait dû lever ERR_ONTOLOGY_MAPPING_NOT_FOUND"),
        }
    }

    #[async_test]
    async fn test_resilience_ingest_eurlex_missing_file() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let res = ingest_eurlex_directive(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
            "/fake/path.xml",
        )
        .await;

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("ERR_FS_NOT_FOUND"));
        Ok(())
    }
}
