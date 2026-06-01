// FICHIER : crates/raise-core/src/json_db/schema/bootstrapper.rs

use async_recursion::async_recursion;

use crate::json_db::collections::manager::{CollectionsManager, SystemIndexTx};
use crate::json_db::schema::ddl::DdlHandler;
use crate::json_db::schema::SchemaRegistry;
use crate::utils::data::json::replace_uri_in_json;
use crate::utils::prelude::*;

pub struct SchemaBootstrapper<'a> {
    manager: &'a CollectionsManager<'a>,
}

impl<'a> SchemaBootstrapper<'a> {
    pub fn new(manager: &'a CollectionsManager<'a>) -> Self {
        Self { manager }
    }

    /// 🎯 Utilitaires Zéro Dette : Construit dynamiquement le préfixe URI du BIOS
    fn get_bios_uri_prefix() -> String {
        let domain = crate::utils::core::RuntimeEnv::var("RAISE_BOOTSTRAP_DOMAIN")
            .unwrap_or_else(|_| "_system".to_string());
        let db = crate::utils::core::RuntimeEnv::var("RAISE_BOOTSTRAP_DB")
            .unwrap_or_else(|_| "bootstrap".to_string());
        format!("db://{}/{}", domain, db)
    }

    // ========================================================================
    // ORCHESTRATEUR PRINCIPAL
    // ========================================================================

    pub async fn bootstrap_new_database(
        &self,
        tx: &mut SystemIndexTx<'_>,
        initial_version: &str,
    ) -> RaiseResult<()> {
        user_info!("BOOTSTRAP_START", json_value!({ "db": self.manager.db }));

        // 1. Matérialisation des schémas
        self.materialize_local_schemas(tx).await?;

        // 2. Ré-ancrage des schémas vers la base locale
        let bios_prefix = Self::get_bios_uri_prefix();
        self.reanchor_collection_schemas(tx, &bios_prefix).await?;

        // 3. Création des dossiers physiques des collections
        self.sync_physical_collections(tx).await?;

        // 4. Amorçage de la migration
        self.init_migration_log(tx, initial_version, "Bootstrap de l'architecture RAISE")
            .await?;

        Ok(())
    }

    pub async fn materialize_local_schemas(
        &self,
        tx: &mut SystemIndexTx<'_>,
    ) -> RaiseResult<usize> {
        let ddl = DdlHandler::new(self.manager);
        let mut materialized_count = 0;

        let bootstrap_uri = format!(
            "{}/schemas/v2/system/db/index_bootstrap.schema.json",
            Self::get_bios_uri_prefix()
        );

        let global_registry = SchemaRegistry::from_uri(
            &self.manager.storage.config,
            &bootstrap_uri,
            &self.manager.space,
            &self.manager.db,
        )
        .await?;

        let mut schemas_to_clone = UniqueSet::new();

        // 🎯 On lit les collections DIRECTEMENT depuis le jeton (plus d'appel au disque)
        if let Some(cols) = tx.document.get("collections").and_then(|c| c.as_object()) {
            for (_, col_data) in cols {
                if let Some(uri) = col_data.get("schema").and_then(|v| v.as_str()) {
                    if !uri.is_empty() {
                        schemas_to_clone.insert(uri.to_string());
                    }
                }
            }
        }

        for external_uri in schemas_to_clone {
            if let Some(schema_content) = global_registry.get_by_uri(&external_uri) {
                // Parsing robuste via pattern matching
                let rel_path = if let Some(idx) = external_uri.find("/schemas/") {
                    // On prend tout ce qui est après le préfixe "db://domain/db/schemas/"
                    &external_uri[idx + 9..]
                } else {
                    &external_uri
                };

                ddl.create_schema(tx, rel_path, schema_content.clone())
                    .await?;
                materialized_count += 1;
            } else {
                user_warn!(
                    "WRN_BOOTSTRAP_SCHEMA_NOT_FOUND",
                    json_value!({ "uri": external_uri })
                );
            }
        }

        Ok(materialized_count)
    }

    pub async fn sync_physical_collections(
        &self,
        tx: &mut SystemIndexTx<'_>,
    ) -> RaiseResult<usize> {
        let ddl = DdlHandler::new(self.manager);
        let mut created_count = 0;

        // 🎯 On clone juste les noms et les URIs pour libérer l'emprunt sur `tx`
        let collections_to_sync: Vec<(String, String)> =
            if let Some(cols) = tx.document.get("collections").and_then(|c| c.as_object()) {
                cols.iter()
                    .map(|(k, v)| {
                        let schema = v
                            .get("schema")
                            .and_then(|s| s.as_str())
                            .unwrap_or("")
                            .to_string();
                        (k.clone(), schema)
                    })
                    .collect()
            } else {
                Vec::new()
            };

        for (col_name, schema_uri) in collections_to_sync {
            ddl.create_collection(tx, &col_name, &schema_uri).await?;
            created_count += 1;
        }
        Ok(created_count)
    }

    pub async fn init_migration_log(
        &self,
        tx: &mut SystemIndexTx<'_>,
        version: &str,
        description: &str,
    ) -> RaiseResult<()> {
        let mgr = self.manager;
        let col_name = "_migrations";

        // 🎯 L'URI pointe bien vers la base locale (Étape précédente)
        let migration_schema_uri = format!(
            "db://{}/{}/schemas/v2/system/db/migration.schema.json",
            mgr.space, mgr.db
        );

        // 1. Si la collection n'existe pas dans le jeton, on la crée
        if tx
            .document
            .get("collections")
            .and_then(|c| c.get(col_name))
            .is_none()
        {
            let ddl = DdlHandler::new(mgr);
            ddl.create_collection(tx, col_name, &migration_schema_uri)
                .await?;
        }

        // 2. SÉCURITÉ : On vérifie si une migration a déjà été amorcée dans l'index
        // Cela empêche de générer des UUIDs en boucle si on relance la fonction
        let has_migrations = tx.document["collections"]
            .get(col_name)
            .and_then(|c| c.get("items"))
            .and_then(|i| i.as_array())
            .map(|arr| !arr.is_empty())
            .unwrap_or(false);

        if !has_migrations {
            // 3. 🎯 GÉNÉRATION : Nouvel UUID v4 et définition du handle
            let new_id = crate::utils::prelude::UniqueId::new_v4().to_string();

            // 4. Le nom du fichier DOIT correspondre à l'ID physique
            let filename = format!("{}.json", new_id);
            let col_path = mgr
                .storage
                .config
                .db_collection_path(&mgr.space, &mgr.db, col_name);
            let doc_path = col_path.join(&filename);
            let now = UtcClock::now().to_rfc3339();
            // 5. Création du document aligné avec ton nouveau schéma
            let migration_doc = json_value!({
                "$schema": migration_schema_uri,
                "_id": new_id,
                "handle": version,
                "name": {
                    "fr": format!("Migration {}", version),
                    "en": format!("Migration {}", version)
                },
                "status": "active",
                "version": version,
                "description": description,
                "applied_at": now.clone(),

                // Champs requis par l'héritage de base.schema.json
                "_created_at": now.clone(),
                "_updated_at": now.clone(),
                "_p2p": {
                    "revision": 1,
                    "origin_node": "system_bootstrapper",
                    "last_sync_at": now
                }
            });

            // Écriture PHYSIQUE du document
            fs::write_json_atomic_async(&doc_path, &migration_doc).await?;

            // Inscription LOGIQUE dans le Jeton
            if let Some(col_obj) = tx.document["collections"]
                .get_mut(col_name)
                .and_then(|c| c.as_object_mut())
            {
                if let Some(items) = col_obj.get_mut("items").and_then(|i| i.as_array_mut()) {
                    items.push(json_value!({ "file": filename }));
                }
            }
        }

        Ok(())
    }

    pub async fn run(&self, legacy_space: &str, legacy_db: &str) -> RaiseResult<usize> {
        let config = &self.manager.storage.config;
        let legacy_dir = config.db_schemas_root(legacy_space, legacy_db);

        if !fs::exists_async(&legacy_dir).await {
            return Ok(0);
        }

        let lock = self
            .manager
            .storage
            .get_index_lock(&self.manager.space, &self.manager.db)?;
        let guard = lock.lock().await;
        let mut tx = self.manager.begin_system_tx(&guard).await?;

        if tx.document.get("schemas").is_none() {
            tx.document["schemas"] = json_value!({});
        }

        let source_prefix = format!("db://{}/{}/", legacy_space, legacy_db);
        let target_prefix = format!("db://{}/{}/", self.manager.space, self.manager.db);

        let count = self
            .scan_recursive(
                &mut tx,
                &legacy_dir,
                &legacy_dir,
                &source_prefix,
                &target_prefix,
            )
            .await?;

        if count > 0 {
            tx.commit().await?;
            user_info!(
                "BOOTSTRAP_SCHEMAS_SUCCESS",
                json_value!({"schemas_injected": count})
            );
        }

        Ok(count)
    }

    #[async_recursion]
    async fn scan_recursive(
        &self,
        tx: &mut SystemIndexTx<'_>,
        root_dir: &Path,
        current_dir: &Path,
        source_prefix: &str,
        target_prefix: &str,
    ) -> RaiseResult<usize> {
        let mut count = 0;
        let ddl = DdlHandler::new(self.manager);
        let mut entries = fs::read_dir_async(current_dir).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if entry.file_type().await?.is_dir() {
                // 🎯 On fait transiter le jeton et les préfixes
                count += self
                    .scan_recursive(tx, root_dir, &path, source_prefix, target_prefix)
                    .await?;
            } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                let content = fs::read_to_string_async(&path).await?;
                if let Ok(mut schema_json) = json::deserialize_from_str::<JsonValue>(&content) {
                    replace_uri_in_json(&mut schema_json, source_prefix, target_prefix);
                    if let Ok(rel_path) = path.strip_prefix(root_dir) {
                        let rel_str = rel_path.to_string_lossy().replace('\\', "/");

                        if ddl.create_schema(tx, &rel_str, schema_json).await.is_ok() {
                            count += 1;
                        }
                    }
                }
            }
        }
        Ok(count)
    }

    /// Réécrit les URIs des schémas pour qu'ils pointent vers la base de données locale
    pub async fn reanchor_collection_schemas(
        &self,
        tx: &mut SystemIndexTx<'_>,
        source_prefix: &str,
    ) -> RaiseResult<usize> {
        let mut updated_count = 0;
        let local_prefix = format!("db://{}/{}", self.manager.space, self.manager.db);

        if let Some(cols) = tx
            .document
            .get_mut("collections")
            .and_then(|c| c.as_object_mut())
        {
            for (col_name, col_data) in cols.iter_mut() {
                // On vérifie si l'URI pointe vers le BIOS (bootstrap)
                let current_uri = col_data
                    .get("schema")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");

                if current_uri.starts_with(source_prefix) {
                    let new_uri = current_uri.replace(source_prefix, &local_prefix);

                    // 🎯 On met à jour le Jeton en RAM
                    col_data["schema"] = json_value!(&new_uri);
                    updated_count += 1;

                    user_debug!(
                        "SCHEMA_REANCHORED",
                        json_value!({
                            "collection": col_name,
                            "new_uri": new_uri
                        })
                    );
                }
            }
        }
        Ok(updated_count)
    }
}

// ============================================================================
// TESTS UNITAIRES
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::DbSandbox;

    #[async_test]
    async fn test_bootstrapper_syncs_physical_collections() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_sync", "db_sync");

        let index_doc = json_value!({
            "$schema": "db://_system/_system/schemas/v2/system/db/index.schema.json",
            "db_role": "exploration",
            "collections": {
                "users": { "schema": "db://_system/_system/schemas/v2/identity/user.schema.json", "items": [] },
                "_migrations": { "schema": "db://_system/_system/schemas/v2/system/db/migration.schema.json", "items": [] }
            }
        });

        let db_root = sandbox.storage.config.db_root("space_sync", "db_sync");
        crate::utils::io::fs::ensure_dir_async(&db_root).await?;
        crate::json_db::storage::file_storage::write_system_index(
            &sandbox.storage.config,
            "space_sync",
            "db_sync",
            &index_doc,
        )
        .await?;

        // Maintenant, les dossiers n'existent pas. Le Bootstrapper doit faire son travail.
        let bootstrapper = SchemaBootstrapper::new(&manager);

        let lock = manager
            .storage
            .get_index_lock(&manager.space, &manager.db)?;
        let guard = lock.lock().await;
        let mut tx = manager.begin_system_tx(&guard).await?;

        let created_count = bootstrapper.sync_physical_collections(&mut tx).await?;
        tx.commit().await?;

        assert_eq!(
            created_count, 2,
            "Deux collections doivent être créées physiquement"
        );

        let users_dir = sandbox
            .storage
            .config
            .db_collection_path("space_sync", "db_sync", "users");
        assert!(users_dir.exists(), "Le dossier users doit exister");
        assert!(
            users_dir.join("_meta.json").exists(),
            "Le meta.json de users doit exister"
        );

        Ok(())
    }

    #[async_test]
    async fn test_bootstrapper_init_migration_log() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_mig", "db_mig");

        let index_doc = json_value!({
            "$schema": "db://_system/_system/schemas/v2/system/db/index.schema.json",
            "db_role": "exploration",
            "collections": {
                "_migrations": { "schema": "db://_system/_system/schemas/v2/system/db/migration.schema.json", "items": [] }
            }
        });

        // 🎯 FIX 1 : On utilise le chemin complet vers file_storage pour satisfaire le compilateur
        crate::json_db::storage::file_storage::create_db(
            &sandbox.storage.config,
            "space_mig",
            "db_mig",
            &index_doc,
        )
        .await?;
        crate::json_db::storage::file_storage::write_system_index(
            &sandbox.storage.config,
            "space_mig",
            "db_mig",
            &index_doc,
        )
        .await?;

        let bootstrapper = SchemaBootstrapper::new(&manager);

        // 🎯 FIX 2 : On génère le Jeton pour le test
        let lock = manager
            .storage
            .get_index_lock(&manager.space, &manager.db)?;
        let guard = lock.lock().await;
        let mut tx = manager.begin_system_tx(&guard).await?;

        // 🎯 FIX 3 : On passe le Jeton à la fonction
        bootstrapper
            .init_migration_log(&mut tx, "v1.0.0", "Test Bootstrapper")
            .await?;

        // On valide la transaction
        tx.commit().await?;

        let doc = manager.get_document("_migrations", "v1.0.0").await?;
        assert!(
            doc.is_some(),
            "L'enregistrement de migration doit être créé"
        );
        assert_eq!(doc.unwrap()["description"], "Test Bootstrapper");

        Ok(())
    }

    #[async_test]
    async fn test_bootstrapper_legacy_import_zero_debt() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_legacy", "db_legacy");
        DbSandbox::mock_db(&manager).await?;

        let legacy_dir = sandbox
            .storage
            .config
            .db_schemas_root("old_space", "old_db");
        let v2_dir = legacy_dir.join("v2").join("test");
        fs::ensure_dir_async(&v2_dir).await?;

        let fake_schema = json_value!({ "type": "object", "title": "Test Legacy" });
        fs::write_json_atomic_async(&v2_dir.join("legacy.schema.json"), &fake_schema).await?;

        let bootstrapper = SchemaBootstrapper::new(&manager);
        let count = bootstrapper.run("old_space", "old_db").await?;
        assert_eq!(count, 1, "Le schéma legacy doit être importé");

        // Vérification Zéro Dette : L'index ne doit contenir qu'un pointeur
        let sys_doc = manager.load_index().await?;
        let ptr = sys_doc["schemas"]["v2"].get("test/legacy.schema.json");
        assert!(ptr.is_some(), "Le schéma doit être référencé dans l'index");
        assert!(
            ptr.unwrap().get("file").is_some(),
            "La référence doit être un pointeur 'file'"
        );
        assert!(
            ptr.unwrap().get("title").is_none(),
            "L'index NE DOIT PAS contenir le payload complet du schéma"
        );

        Ok(())
    }
}
