// FICHIER : crates/raise-core/src/model_engine/loader.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::jsonld::JsonLdProcessor;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::types::{ArcadiaElement, ProjectMeta, ProjectModel};
use crate::rules_engine::evaluator::DataProvider;
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

/// Index de localisation : Document_ID -> (Couche_DB, Nom_Collection)
type LocationIndex = UnorderedMap<String, (String, String)>;

pub struct ModelLoader<'a> {
    pub manager: CollectionsManager<'a>,
    /// Index partagé protégé par un verrou asynchrone
    index: SharedRef<AsyncRwLock<LocationIndex>>,
    processor: JsonLdProcessor,
}

impl<'a> ModelLoader<'a> {
    pub fn new(storage: &'a StorageEngine, space: &str, db: &str) -> RaiseResult<Self> {
        Self::from_engine(storage, space, db)
    }

    pub fn from_engine(storage: &'a StorageEngine, space: &str, db: &str) -> RaiseResult<Self> {
        Ok(Self {
            manager: CollectionsManager::new(storage, space, db),
            index: SharedRef::new(AsyncRwLock::new(UnorderedMap::new())),
            processor: JsonLdProcessor::new()?,
        })
    }

    pub fn new_with_manager(manager: CollectionsManager<'a>) -> RaiseResult<Self> {
        Ok(Self {
            manager,
            index: SharedRef::new(AsyncRwLock::new(UnorderedMap::new())),
            processor: JsonLdProcessor::new()?,
        })
    }
    /// Analyse la structure du projet sur disque via le mapping ontologique.
    /// Utilise les points de montage système pour localiser les configurations.
    pub async fn index_project(&self) -> RaiseResult<usize> {
        let mut idx = self.index.write().await;
        idx.clear();

        let config = AppConfig::get();

        // 🎯 RÉSILIENCE MOUNT POINTS : Utilisation dynamique de la partition système
        let sys_mgr = CollectionsManager::new(
            self.manager.storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // Lecture du mapping pour connaître les collections à scanner
        let mapping_doc = match sys_mgr
            .get_document("configs", "ref:configs:handle:ontological_mapping")
            .await?
        {
            Some(doc) => doc,
            None => return Ok(0), // Si pas de mapping, index vide
        };

        let search_spaces = match mapping_doc.get("search_spaces").and_then(|v| v.as_array()) {
            Some(arr) => arr,
            None => raise_error!(
                "ERR_INVALID_ONTOLOGY_MAPPING",
                error =
                    "Le champ 'search_spaces' est manquant ou invalide dans le Jumeau Numérique."
            ),
        };

        let mut count = 0;
        for space_def in search_spaces {
            let layer_db = space_def
                .get("layer")
                .and_then(|v| v.as_str())
                .unwrap_or("raise");
            let col = space_def
                .get("collection")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            // Scan physique via Match
            let ids = match self.fetch_document_ids(layer_db, col).await {
                Ok(list) => list,
                Err(e) => {
                    user_warn!(
                        "WRN_LOADER_SCAN_FAIL",
                        json_value!({"db": layer_db, "col": col, "error": e.to_string()})
                    );
                    continue;
                }
            };

            for id in ids {
                idx.insert(id.clone(), (layer_db.to_string(), col.to_string()));
                count += 1;
            }
        }
        Ok(count)
    }

    /// Récupère la liste des IDs de documents présents dans une collection physique.
    async fn fetch_document_ids(&self, db: &str, col: &str) -> RaiseResult<Vec<String>> {
        let col_path = self
            .manager
            .storage
            .config
            .db_collection_path(&self.manager.space, db, col);
        let mut ids = Vec::new();

        if fs::exists_async(&col_path).await {
            let mut entries = match fs::read_dir_async(&col_path).await {
                Ok(e) => e,
                Err(err) => raise_error!("ERR_LOADER_IO", error = err.to_string()),
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        if !stem.starts_with('_') {
                            ids.push(stem.to_string());
                        }
                    }
                }
            }
        }
        Ok(ids)
    }

    /// Charge un élément spécifique par son ID.
    pub async fn get_element(&self, id: &str) -> RaiseResult<ArcadiaElement> {
        let location = {
            let idx = self.index.read().await;
            idx.get(id).cloned()
        };

        match location {
            Some((db, col)) => {
                let target_mgr =
                    CollectionsManager::new(self.manager.storage, &self.manager.space, &db);

                let doc = match target_mgr.get_document(&col, id).await? {
                    Some(d) => d,
                    None => raise_error!(
                        "ERR_DB_INDEX_OUT_OF_SYNC",
                        error = format!(
                            "L'ID '{}' est dans l'index mais introuvable sur le disque.",
                            id
                        )
                    ),
                };

                self.json_to_element(doc, Some(&db))
            }
            None => raise_error!(
                "ERR_DB_UNKNOWN_IDENTITY",
                error = format!("ID '{}' non répertorié dans le World Model local.", id)
            ),
        }
    }

    /// Transforme un document JSON en ArcadiaElement Pure Graph
    fn json_to_element(
        &self,
        doc: JsonValue,
        layer_hint: Option<&str>,
    ) -> RaiseResult<ArcadiaElement> {
        let handle = doc
            .get("_id")
            .or(doc.get("id"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let name_val = doc
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("Sans nom");

        let raw_type = doc
            .get("type")
            .or(doc.get("@type"))
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown");

        let kind = if let Some(layer) = layer_hint {
            let mut local_proc = self.processor.clone();
            let _ = local_proc.load_layer_context(layer);
            vec![local_proc.context_manager().expand_term(raw_type)]
        } else {
            vec![raw_type.to_string()]
        };

        let mut properties = UnorderedMap::new();
        if let Some(obj) = doc.as_object() {
            for (k, v) in obj {
                if !matches!(
                    k.as_str(),
                    "id" | "_id" | "name" | "type" | "@type" | "@context"
                ) {
                    properties.insert(k.clone(), v.clone());
                }
            }
        }

        Ok(ArcadiaElement {
            handle: handle.as_str().try_into()?,
            name: I18nString::Single(name_val.to_string()),
            kind,
            properties,
            ..Default::default()
        })
    }

    /// Charge l'intégralité du modèle en mémoire.
    pub async fn load_full_model(&self) -> RaiseResult<ProjectModel> {
        let count = self.index_project().await?;
        let index_snapshot = { self.index.read().await.clone() };

        let mut model = ProjectModel {
            meta: ProjectMeta {
                name: format!("{}/{}", self.manager.space, self.manager.db),
                element_count: count,
            },
            ..Default::default()
        };

        for (id, (layer, col)) in index_snapshot {
            match self.get_element(&id).await {
                Ok(el) => model.add_element(&layer, &col, el),
                Err(e) => user_warn!(
                    "WRN_LOADER_ELEMENT_SKIP",
                    json_value!({"id": id, "error": e.to_string()})
                ),
            }
        }

        Ok(model)
    }
}

#[async_interface]
impl<'a> DataProvider for ModelLoader<'a> {
    async fn get_value(&self, _collection: &str, id: &str, field: &str) -> Option<JsonValue> {
        match self.get_element(id).await {
            Ok(el) => el.properties.get(field).cloned(),
            Err(_) => None,
        }
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::AgentDbSandbox;

    #[async_test]
    async fn test_loader_json_to_element_pure_graph() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let loader = ModelLoader::from_engine(&sandbox.db, "space", "db")?;

        let doc = json_value!({
            "_id": "el_1",
            "name": "Moteur",
            "type": "Component",
            "description": "Un moteur puissant",
            "mass": 450
        });

        let element = loader.json_to_element(doc, None)?;

        assert_eq!(element.handle.as_str(), "el_1");
        assert_eq!(element.name.as_str(), "Moteur");
        assert_eq!(
            element.properties.get("mass").and_then(|v| v.as_i64()),
            Some(450)
        );
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Résilience face à un mapping ontologique manquant
    #[async_test]
    async fn test_loader_resilience_missing_mapping() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let loader = ModelLoader::from_engine(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )?;

        // Indexation sans document de mapping en base
        let count = loader.index_project().await?;
        assert_eq!(
            count, 0,
            "L'index doit être vide si aucune config n'existe."
        );
        Ok(())
    }

    /// 🎯 NOUVEAU TEST : Validation Mount Points System
    #[async_test]
    async fn test_loader_mount_point_resolution() -> RaiseResult<()> {
        let _sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        assert!(!config.mount_points.system.domain.is_empty());
        assert!(!config.mount_points.system.db.is_empty());
        Ok(())
    }
}
