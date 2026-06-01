// FICHIER : crates/raise-core/src/json_db/schema/registry.rs

use crate::json_db::storage::JsonDbConfig;
use crate::utils::prelude::*;

#[derive(Debug, Clone)]
pub struct SchemaRegistry {
    pub(crate) by_uri: UnorderedMap<String, JsonValue>,
    pub base_prefix: String,
    // 🎯 Disparition de `schemas_root` ! Le registre n'a plus besoin de connaître l'arborescence physique.
}

impl Default for SchemaRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SchemaRegistry {
    pub fn new() -> Self {
        Self {
            by_uri: UnorderedMap::new(),
            base_prefix: "db://unknown/unknown/schemas/v2/".to_string(),
        }
    }

    pub async fn from_db(config: &JsonDbConfig, space: &str, db: &str) -> RaiseResult<Self> {
        let base_prefix = format!("db://{}/{}/schemas/", space, db);

        let mut registry = Self {
            by_uri: UnorderedMap::new(),
            base_prefix,
        };

        let app_config = AppConfig::get();

        // 🎯 Chargement en cascade via les Points de Montage stricts
        // Le registre agrège en mémoire tous les catalogues DDL des _system.json

        // 1. Noyau Système
        registry
            .load_domain_schemas(
                config,
                &app_config.mount_points.system.domain,
                &app_config.mount_points.system.db,
            )
            .await?;

        // 2. Raise Core
        registry
            .load_domain_schemas(
                config,
                &app_config.mount_points.raise.domain,
                &app_config.mount_points.raise.db,
            )
            .await?;

        // 3. Workspace MBSE
        registry
            .load_domain_schemas(
                config,
                &app_config.mount_points.simulation.domain,
                &app_config.mount_points.simulation.db,
            )
            .await?;

        // 4. Domaine courant (si différent)
        if space != app_config.mount_points.system.domain || db != app_config.mount_points.system.db
        {
            registry.load_domain_schemas(config, space, db).await?;
        }

        Ok(registry)
    }

    /// 🎯 MOTEUR DE LECTURE : Lit l'index pour trouver les chemins, puis charge les fichiers physiques
    async fn load_domain_schemas(
        &mut self,
        config: &JsonDbConfig,
        space: &str,
        db: &str,
    ) -> RaiseResult<()> {
        use crate::json_db::storage::file_storage;

        if let Ok(Some(sys_doc)) = file_storage::read_system_index(config, space, db).await {
            if let Some(schemas) = sys_doc.get("schemas").and_then(|s| s.as_object()) {
                for (version, v_obj) in schemas {
                    if let Some(obj) = v_obj.as_object() {
                        for (rel_path, _) in obj {
                            let uri =
                                format!("db://{}/{}/schemas/{}/{}", space, db, version, rel_path);
                            let schema_path = config
                                .db_schemas_root(space, db)
                                .join(version)
                                .join(rel_path);

                            // Lecture physique asynchrone
                            if let Ok(schema_json) = fs::read_json_async(&schema_path).await {
                                self.register(uri, schema_json);
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub fn register(&mut self, uri: String, schema: JsonValue) {
        self.by_uri.insert(uri, schema);
    }

    pub async fn from_uri(
        config: &JsonDbConfig,
        uri: &str,
        fallback_space: &str,
        fallback_db: &str,
    ) -> RaiseResult<Self> {
        let mut target_space = fallback_space.to_string();
        let mut target_db = fallback_db.to_string();

        if let Some(without_scheme) = uri.strip_prefix("db://") {
            let parts: Vec<&str> = without_scheme.splitn(3, '/').collect();
            if parts.len() >= 2 {
                target_space = parts[0].to_string();
                target_db = parts[1].to_string();
            }
        }

        Self::from_db(config, &target_space, &target_db).await
    }

    pub fn get_by_uri(&self, uri: &str) -> Option<&JsonValue> {
        // 1. Recherche stricte
        if let Some(schema) = self.by_uri.get(uri) {
            return Some(schema);
        }

        // 2. Fallback intelligent (inchangé)
        if let Some(idx) = uri.find("/schemas/") {
            let remainder = &uri[idx + "/schemas/".len()..];
            let parts: Vec<&str> = remainder.splitn(2, '/').collect();

            if parts.len() == 2 {
                let version = parts[0];
                let relative_path = parts[1];
                let app_config = AppConfig::get();

                let mod_uri = format!(
                    "db://{}/{}/schemas/{}/{}",
                    app_config.mount_points.modeling.domain,
                    app_config.mount_points.modeling.db,
                    version,
                    relative_path
                );
                if let Some(schema) = self.by_uri.get(&mod_uri) {
                    return Some(schema);
                }

                let raise_uri = format!(
                    "db://{}/{}/schemas/{}/{}",
                    app_config.mount_points.raise.domain,
                    app_config.mount_points.raise.db,
                    version,
                    relative_path
                );
                if let Some(schema) = self.by_uri.get(&raise_uri) {
                    return Some(schema);
                }

                let sys_uri = format!(
                    "db://{}/{}/schemas/{}/{}",
                    app_config.mount_points.system.domain,
                    app_config.mount_points.system.db,
                    version,
                    relative_path
                );
                if let Some(schema) = self.by_uri.get(&sys_uri) {
                    return Some(schema);
                }

                let hard_sys_uri =
                    format!("db://_system/_system/schemas/{}/{}", version, relative_path);
                if let Some(schema) = self.by_uri.get(&hard_sys_uri) {
                    return Some(schema);
                }
            }
        }

        None
    }

    pub fn list_uris(&self) -> Vec<String> {
        self.by_uri.keys().cloned().collect()
    }

    pub fn uri(&self, relative_path: &str) -> String {
        format!("{}{}", self.base_prefix, relative_path)
    }
}

// ============================================================================
// TESTS UNITAIRES (Zéro Dette)
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::storage::{file_storage, JsonDbConfig};
    use crate::utils::testing::mock::inject_mock_config;

    #[async_test]
    async fn test_registry_loading_from_index() -> RaiseResult<()> {
        inject_mock_config().await;

        let dir = match tempdir() {
            Ok(d) => d,
            Err(e) => panic!("Erreur tempdir : {:?}", e),
        };

        let config = JsonDbConfig::new(dir.path().to_path_buf());
        let (space, db) = ("s1", "d1");

        let schemas_root = config.db_schemas_root(space, db);
        fs::create_dir_all_async(&schemas_root.join("v2/users")).await?;

        let schema_v2 = json_value!({ "type": "object", "title": "User V2" });
        fs::write_json_atomic_async(&schemas_root.join("v2/users/user.schema.json"), &schema_v2)
            .await?;

        let mock_system_index = json_value!({
            "schemas": {
                "v2": { "users/user.schema.json": { "file": "v2/users/user.schema.json" } }
            }
        });

        file_storage::write_system_index(&config, space, db, &mock_system_index).await?;

        let reg = SchemaRegistry::from_db(&config, space, db).await?;
        let uri_v2 = format!("db://{}/{}/schemas/v2/users/user.schema.json", space, db);

        assert!(reg.get_by_uri(&uri_v2).is_some());
        assert_eq!(reg.get_by_uri(&uri_v2).unwrap()["title"], "User V2");

        Ok(())
    }
}
