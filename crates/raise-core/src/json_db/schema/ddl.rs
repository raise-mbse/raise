// FICHIER : crates/raise-core/src/json_db/schema/ddl.rs

use crate::json_db::collections::manager::{CollectionsManager, SystemIndexTx};
use crate::json_db::query::{Condition, FilterOperator, Query, QueryEngine, QueryFilter};

use crate::json_db::schema::SchemaRegistry;
use crate::json_db::storage::file_storage;
use crate::utils::prelude::*;

pub struct DdlHandler<'a> {
    manager: &'a CollectionsManager<'a>,
}

impl<'a> DdlHandler<'a> {
    pub fn new(manager: &'a CollectionsManager<'a>) -> Self {
        Self { manager }
    }

    pub async fn init_db(&self) -> RaiseResult<bool> {
        let app_config = AppConfig::get();
        let sys_domain = &app_config.mount_points.system.domain;
        let sys_db = &app_config.mount_points.system.db;

        let schema_uri = format!(
            "db://{}/{}/schemas/v2/db/system/index_raise.schema.json",
            sys_domain, sys_db
        );
        self.init_db_with_schema(&schema_uri).await
    }

    pub async fn init_db_with_schema(&self, schema_uri: &str) -> RaiseResult<bool> {
        let mgr = self.manager;

        // 1. On prend le verrou global pour protéger le fichier _system.json
        let lock = mgr.storage.get_index_lock(&mgr.space, &mgr.db)?;
        let guard = lock.lock().await;

        // 2. On génère le Jeton (qui contient la preuve du verrou)
        let mut tx = mgr.begin_system_tx(&guard).await?;

        let is_new = tx.document.get("handle").is_none();

        // 3. On initialise l'acte de naissance dans le Jeton
        if is_new {
            tx.document["$schema"] = json_value!(schema_uri);
            tx.document["handle"] = json_value!(format!("{}_{}", mgr.space, mgr.db));
            tx.document["name"] = json_value!(format!("{}_{}", mgr.space, mgr.db));
            tx.document["space"] = json_value!(mgr.space.clone());
            tx.document["domain"] = json_value!(mgr.space.clone());
            tx.document["database"] = json_value!(mgr.db.clone());
        }

        // Validation (On valide l'état temporaire du jeton)
        let reg =
            SchemaRegistry::from_uri(&mgr.storage.config, schema_uri, &mgr.space, &mgr.db).await?;
        let validator =
            crate::json_db::schema::SchemaValidator::compile_with_registry(schema_uri, &reg)?;

        let compute_ctx = crate::rules_engine::compute::ComputeContext {
            document: tx.document.clone(),
            collection_name: "_system".to_string(), // C'est le fichier système !
            db_name: mgr.db.clone(),
            space_name: mgr.space.clone(),
        };

        // On passe le contexte au validateur
        validator
            .compute_then_validate(&mut tx.document, &compute_ctx)
            .await?;

        // Création physique du dossier racine de la base
        let created = crate::json_db::storage::file_storage::create_db(
            &mgr.storage.config,
            &mgr.space,
            &mgr.db,
            &tx.document,
        )
        .await?;

        if is_new || created {
            // 4. On appelle le Bootstrapper en lui PASSANT LE JETON
            use crate::json_db::schema::bootstrapper::SchemaBootstrapper;
            let bootstrapper = SchemaBootstrapper::new(mgr);

            bootstrapper
                .bootstrap_new_database(&mut tx, "v1.0.0")
                .await?;
        }

        // 5. ON SAUVEGARDE ET ON LIBÈRE LE VERROU
        tx.commit().await?;

        Ok(created || is_new)
    }

    pub async fn create_db_with_schema(&self, schema_uri: &str) -> RaiseResult<bool> {
        let created = self.init_db_with_schema(schema_uri).await?;

        if created {
            if let Ok(index_doc) = self.manager.load_index().await {
                let idx_mgr = crate::json_db::indexes::IndexManager::new(
                    self.manager.storage,
                    &self.manager.space,
                    &self.manager.db,
                );
                let _ = idx_mgr.apply_indexes_from_config(&index_doc).await;
            }
            self.register_in_system_governance().await?;
        }
        Ok(created)
    }

    /// 🛠️ Modifie une propriété de l'index système (_system.json) pour maintenir la cohérence.
    pub async fn alter_db(&self, key: &str, mut value: JsonValue) -> RaiseResult<()> {
        let mgr = self.manager;

        if let Some(s) = value.as_str() {
            if s.starts_with("ref:") || s.starts_with("db://") {
                // resolve_single_reference garantit la récupération de l'_id
                if let Ok(resolved_id) = mgr.resolve_single_reference(s).await {
                    value = json_value!(resolved_id);
                }
            }
        }

        let lock = mgr.storage.get_index_lock(&mgr.space, &mgr.db)?;
        let guard = lock.lock().await;
        let mut tx = mgr.begin_system_tx(&guard).await?;

        tx.document[key] = value.clone();
        tx.commit().await?;

        user_debug!(
            "DB_PROPERTY_ALTERED",
            json_value!({
                "db": mgr.db,
                "key": key,
                "stored_id": value
            })
        );

        Ok(())
    }
    pub async fn drop_db(&self) -> RaiseResult<bool> {
        let mgr = self.manager;
        let db_path = mgr.storage.config.db_root(&mgr.space, &mgr.db);
        if !db_path.exists() {
            return Ok(false);
        }

        file_storage::drop_db(
            &mgr.storage.config,
            &mgr.space,
            &mgr.db,
            file_storage::DropMode::Hard,
        )
        .await?;

        self.unregister_from_system_governance().await?;

        Ok(true)
    }

    async fn register_in_system_governance(&self) -> RaiseResult<()> {
        let app_config = AppConfig::get();
        let raise_domain = &app_config.mount_points.system.domain;
        let raise_db = &app_config.mount_points.system.db;

        // Si on initialise la base système elle-même, on s'arrête (elle s'auto-déclare via 00_init_db)
        if &self.manager.space == raise_domain && &self.manager.db == raise_db {
            return Ok(());
        }

        let sys_mgr = CollectionsManager::new(self.manager.storage, raise_domain, raise_db);

        // 1. Enregistrement du Domaine (Upsert)
        let domain_handle = self.manager.space.clone();
        let domain_doc = json_value!({
            "handle": domain_handle.clone(),
            "name": { "fr": domain_handle.clone(), "en": domain_handle.clone() },
            "status": "active"
        });

        let _ = sys_mgr.upsert_document("domains", domain_doc).await;

        // 2. Enregistrement de la Base de Données (Upsert)
        let db_handle = self.manager.db.clone();
        let db_doc = json_value!({
            "handle": db_handle.clone(),
            "name": { "fr": db_handle.clone(), "en": db_handle.clone() },
            "domain_id": format!("ref:domains:handle:{}", domain_handle),
            "is_system": false,
            "status": "active"
        });

        let _ = sys_mgr.upsert_document("databases", db_doc).await;

        user_info!(
            "MSG_DB_REGISTERED_IN_GOVERNANCE",
            json_value!({ "domain": domain_handle, "db": db_handle })
        );

        Ok(())
    }

    async fn unregister_from_system_governance(&self) -> RaiseResult<()> {
        let app_config = AppConfig::get();
        let raise_domain = &app_config.mount_points.system.domain;
        let raise_db = &app_config.mount_points.system.db;

        // Sécurité : On ne permet pas à la base système de se désinscrire elle-même ici
        if &self.manager.space == raise_domain && &self.manager.db == raise_db {
            return Ok(());
        }

        let sys_mgr = CollectionsManager::new(self.manager.storage, raise_domain, raise_db);

        // 1. On cherche la base de données ciblée dans le catalogue via son handle
        let mut query = Query::new("databases");
        query.filter = Some(QueryFilter {
            operator: FilterOperator::And,
            conditions: vec![Condition::eq("handle", json::json_value!(&self.manager.db))],
        });
        query.limit = Some(1);

        let qe = QueryEngine::new(&sys_mgr);
        if let Ok(res) = qe.execute_query(query).await {
            if let Some(doc) = res.documents.first() {
                // 2. Si on la trouve, on la supprime proprement
                if let Some(id) = doc.get("_id").and_then(|v| v.as_str()) {
                    let _ = sys_mgr.delete_document("databases", id).await;

                    user_info!(
                        "MSG_DB_UNREGISTERED_FROM_GOVERNANCE",
                        json::json_value!({
                            "domain": &self.manager.space,
                            "db": &self.manager.db
                        })
                    );
                }
            }
        }

        Ok(())
    }

    pub async fn create_schema(
        &self,
        tx: &mut SystemIndexTx<'_>,
        schema_name: &str,
        schema: JsonValue,
    ) -> RaiseResult<()> {
        let uri = self.manager.build_schema_uri(schema_name).await;
        let (version, rel_key) = self.extract_schema_paths(&uri);

        // Détection de la rigueur
        let db_role = tx
            .document
            .get("db_role")
            .and_then(|v| v.as_str())
            .unwrap_or("exploration");

        let is_strict_role = matches!(
            db_role,
            "system" | "raise" | "integration" | "production" | "operation"
        );

        let path = self
            .manager
            .storage
            .config
            .db_schemas_root(&self.manager.space, &self.manager.db)
            .join(&version)
            .join(&rel_key);

        if let Some(parent) = path.parent() {
            fs::ensure_dir_async(parent).await?;
        }

        let ordered_schema = self.prepare_ordered_schema(&uri, schema, is_strict_role);
        match fs::write_json_atomic_async(&path, &ordered_schema).await {
            Ok(_) => (),
            Err(e) => raise_error!(
                "ERR_DDL_SCHEMA_WRITE_FAILED",
                error = e,
                context = json_value!({"path": path})
            ),
        }

        // Écriture physique du fichier
        let (version, rel_key) = self.extract_schema_paths(&uri);
        let path = self
            .manager
            .storage
            .config
            .db_schemas_root(&self.manager.space, &self.manager.db)
            .join(&version)
            .join(&rel_key);

        fs::ensure_dir_async(path.parent().unwrap()).await?;
        fs::write_json_atomic_async(&path, &ordered_schema).await?;

        // Inscription dans le Jeton
        if tx.document.get("schemas").is_none() {
            tx.document["schemas"] = json_value!({});
        }
        if tx.document["schemas"].get(&version).is_none() {
            tx.document["schemas"][&version] = json_value!({});
        }

        if let Some(v_obj) = tx.document["schemas"][&version].as_object_mut() {
            v_obj.insert(
                rel_key.to_string(),
                json_value!({ "file": format!("{}/{}", version, rel_key) }),
            );
        }

        Ok(())
    }

    pub async fn drop_schema(&self, schema_name: &str) -> RaiseResult<()> {
        let uri = self.manager.build_schema_uri(schema_name).await;
        let (version, rel_key) = self.extract_schema_paths(&uri);

        // 1. Suppression physique
        let path = self
            .manager
            .storage
            .config
            .db_schemas_root(&self.manager.space, &self.manager.db)
            .join(&version)
            .join(&rel_key);
        if path.exists() {
            let _ = crate::utils::io::fs::remove_file_async(&path).await;
        }

        // 2. Mise à jour de l'Index (Ouverture du Jeton)
        let lock = self
            .manager
            .storage
            .get_index_lock(&self.manager.space, &self.manager.db)?;
        let guard = lock.lock().await;
        let mut tx = self.manager.begin_system_tx(&guard).await?;

        if let Some(v_obj) = tx
            .document
            .get_mut("schemas")
            .and_then(|s| s.get_mut(&version))
            .and_then(|v| v.as_object_mut())
        {
            v_obj.remove(&rel_key);
            tx.commit().await?; // On sauvegarde
        }

        Ok(())
    }

    pub async fn add_property(
        &self,
        schema_name: &str,
        prop: &str,
        def: JsonValue,
    ) -> RaiseResult<()> {
        let mut schema = self.load_file(schema_name).await?;
        if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
            props.insert(prop.to_string(), def);
        } else {
            schema["properties"] = json_value!({ prop: def });
        }
        self.save_ordered(schema_name, schema).await
    }

    pub async fn alter_property(
        &self,
        schema_name: &str,
        prop_name: &str,
        definition: JsonValue,
    ) -> RaiseResult<()> {
        let mut schema = self.load_file(schema_name).await?;

        if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
            props.insert(prop_name.to_string(), definition);
            self.save_ordered(schema_name, schema).await
        } else {
            raise_error!(
                "ERR_DDL_PROPERTY_NOT_FOUND",
                error = format!("Propriété '{}' introuvable", prop_name)
            )
        }
    }

    pub async fn drop_property(&self, schema_name: &str, prop_name: &str) -> RaiseResult<()> {
        let mut schema = self.load_file(schema_name).await?;

        if let Some(props) = schema.get_mut("properties").and_then(|p| p.as_object_mut()) {
            props.remove(prop_name);
            self.save_ordered(schema_name, schema).await
        } else {
            Ok(())
        }
    }

    fn prepare_ordered_schema(&self, uri: &str, schema: JsonValue, strict: bool) -> JsonValue {
        let mut ordered = json_value!({});
        let obj = ordered.as_object_mut().unwrap();

        obj.insert(
            "$schema".into(),
            json_value!("https://json-schema.org/draft/2020-12/schema"),
        );
        obj.insert("$id".into(), json_value!(uri));

        if let Some(t) = schema.get("title") {
            obj.insert("title".into(), t.clone());
        }
        obj.insert(
            "type".into(),
            schema.get("type").cloned().unwrap_or(json_value!("object")),
        );

        let unevaluated = if strict {
            json_value!(false)
        } else {
            schema
                .get("unevaluatedProperties")
                .cloned()
                .unwrap_or(json_value!(true))
        };
        obj.insert("unevaluatedProperties".into(), unevaluated);

        if let Some(p) = schema.get("properties") {
            obj.insert("properties".into(), p.clone());
        }

        if let Some(old) = schema.as_object() {
            for (k, v) in old {
                if ![
                    "$schema",
                    "$id",
                    "title",
                    "type",
                    "properties",
                    "unevaluatedProperties",
                ]
                .contains(&k.as_str())
                {
                    obj.insert(k.clone(), v.clone());
                }
            }
        }
        ordered
    }

    async fn load_file(&self, name: &str) -> RaiseResult<JsonValue> {
        let uri = self.manager.build_schema_uri(name).await;
        let (v, k) = self.extract_schema_paths(&uri);
        let path = self
            .manager
            .storage
            .config
            .db_schemas_root(&self.manager.space, &self.manager.db)
            .join(v)
            .join(k);
        crate::utils::io::fs::read_json_async(&path).await
    }

    async fn save_ordered(&self, name: &str, schema: JsonValue) -> RaiseResult<()> {
        // On génère le Jeton car on s'apprête à appeler create_schema
        let lock = self
            .manager
            .storage
            .get_index_lock(&self.manager.space, &self.manager.db)?;
        let guard = lock.lock().await;
        let mut tx = self.manager.begin_system_tx(&guard).await?;

        self.create_schema(&mut tx, name, schema).await?;

        tx.commit().await
    }

    pub fn extract_schema_paths(&self, uri: &str) -> (String, String) {
        let version = uri
            .split("/schemas/")
            .nth(1)
            .and_then(|s| s.split('/').next())
            .unwrap_or("v2")
            .to_string();
        let rel_key = uri
            .split(&format!("/schemas/{}/", version))
            .nth(1)
            .unwrap_or(uri)
            .to_string();
        (version, rel_key)
    }

    /// Crée une collection physique et l'inscrit dans le jeton de transaction
    pub async fn create_collection(
        &self,
        tx: &mut SystemIndexTx<'_>,
        name: &str,
        schema_uri: &str,
    ) -> RaiseResult<()> {
        let mgr = self.manager;

        // 1. Logique physique (Dossiers et Méta-fichiers)
        let final_schema_uri = self.build_schema_uri(schema_uri).await;

        let col_path = mgr
            .storage
            .config
            .db_collection_path(&mgr.space, &mgr.db, name);

        if !col_path.exists() {
            fs::ensure_dir_async(&col_path).await?;
        }

        let meta = json_value!({ "schema": final_schema_uri, "indexes": [] });
        fs::write_json_atomic_async(&col_path.join("_meta.json"), &meta).await?;

        // 2. Logique logique (Modification directe dans la RAM du Jeton)
        if tx.document.get("collections").is_none() {
            tx.document["collections"] = json_value!({});
        }

        if let Some(cols) = tx.document["collections"].as_object_mut() {
            if let Some(existing_col) = cols.get_mut(name).and_then(|c| c.as_object_mut()) {
                existing_col.insert("schema".to_string(), json_value!(&final_schema_uri));
            } else {
                cols.insert(
                    name.to_string(),
                    json_value!({ "schema": final_schema_uri, "items": [] ,"x_indexes": []}),
                );
            }
        }

        // 🎯 Terminé ! Pas de lock, pas de save_system_index. Le Jeton s'en chargera à la fin.
        Ok(())
    }

    pub async fn drop_collection(&self, name: &str) -> RaiseResult<()> {
        crate::json_db::collections::collection::drop_collection(
            &self.manager.storage.config,
            &self.manager.space,
            &self.manager.db,
            name,
        )
        .await?;
        self.remove_collection_from_system_index(name).await?;
        Ok(())
    }

    pub async fn build_schema_uri(&self, schema_name: &str) -> String {
        if schema_name.starts_with("db://") || schema_name.starts_with("http") {
            return schema_name.to_string();
        }
        if schema_name.starts_with("v1/") || schema_name.starts_with("v2/") {
            return format!(
                "db://{}/{}/schemas/{}",
                self.manager.space, self.manager.db, schema_name
            );
        }

        let domain_version = self.get_domain_version().await;
        let relative_path = schema_name
            .trim_start_matches('/')
            .trim_start_matches("schemas/");
        format!(
            "db://{}/{}/schemas/{}/{}",
            self.manager.space, self.manager.db, domain_version, relative_path
        )
    }

    pub async fn get_domain_version(&self) -> String {
        match self.manager.load_index().await {
            Ok(index) => {
                if let Some(uri) = index.get("$schema").and_then(|v| v.as_str()) {
                    if let Some(v) = uri
                        .split("/schemas/")
                        .nth(1)
                        .and_then(|s| s.split('/').next())
                    {
                        return v.to_string();
                    }
                }
                "v2".to_string()
            }
            Err(_) => "v2".to_string(),
        }
    }

    pub async fn resolve_schema_from_index(&self, col_name: &str) -> RaiseResult<String> {
        let sys_json = self.manager.load_index().await?;
        let current_schema = sys_json
            .get("$schema")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        if !current_schema.contains("/index") {
            raise_error!(
                "ERR_DB_INTEGRITY_COMPROMISED",
                error = "L'index utilise un schéma non certifié."
            );
        }

        let col_ptr = format!("/collections/{}/schema", col_name);
        let rule_ptr = format!("/rules/{}/schema", col_name);

        let raw_path = sys_json
            .pointer(&col_ptr)
            .or_else(|| sys_json.pointer(&rule_ptr))
            .and_then(|v| v.as_str());

        let Some(path) = raw_path else {
            raise_error!(
                "ERR_DB_COLLECTION_NOT_FOUND",
                error = "Collection inconnue."
            );
        };

        if path.is_empty() {
            return Ok(String::new());
        }
        Ok(self.build_schema_uri(path).await)
    }

    async fn remove_collection_from_system_index(&self, col_name: &str) -> RaiseResult<()> {
        let lock = self
            .manager
            .storage
            .get_index_lock(&self.manager.space, &self.manager.db)?;
        let _guard = lock.lock().await;

        let mut sys_doc = self.manager.load_index().await?;
        if let Some(cols) = sys_doc
            .get_mut("collections")
            .and_then(|c| c.as_object_mut())
        {
            if cols.remove(col_name).is_some() {
                self.manager.save_system_index(&mut sys_doc).await?;
            }
        }
        Ok(())
    }

    pub async fn import_schemas_from_storage(
        &self,
        source_space: &str,
        source_db: &str,
    ) -> RaiseResult<usize> {
        use crate::json_db::schema::bootstrapper::SchemaBootstrapper;
        let bootstrapper = SchemaBootstrapper::new(self.manager);
        bootstrapper.run(source_space, source_db).await
    }

    /// Enregistre une ontologie fondamentale (DDL) dans l'index de la base de données.
    /// Cela définit les connaissances (Code Génétique) nécessaires au Cerveau Sémantique.
    pub async fn register_ontology(
        &self,
        namespace: &str,
        uri: &str,
        version: &str,
    ) -> RaiseResult<()> {
        let mgr = self.manager;

        // 1. Verrou et Jeton
        let lock = mgr.storage.get_index_lock(&mgr.space, &mgr.db)?;
        let guard = lock.lock().await;
        let mut tx = mgr.begin_system_tx(&guard).await?;

        if tx.document.get("ontologies").is_none() {
            tx.document["ontologies"] = json_value!({});
        }

        if let Some(ontologies) = tx.document["ontologies"].as_object_mut() {
            let new_entry = json_value!({
                "uri": uri,
                "version": version,
                "imports": [] // Résolus dynamiquement par le graphe[cite: 28]
            });

            if let Some(existing) = ontologies.get_mut(namespace) {
                if let Some(arr) = existing.as_array_mut() {
                    // Cas A : C'est déjà un tableau. On ajoute si l'URI n'existe pas.
                    let exists = arr
                        .iter()
                        .any(|item| item.get("uri").and_then(|v| v.as_str()) == Some(uri));
                    if !exists {
                        arr.push(new_entry);
                    }
                } else {
                    // Cas B (Migration) : C'était un objet unique, on le transforme en tableau.
                    let old_entry = existing.clone();
                    let old_uri = old_entry.get("uri").and_then(|v| v.as_str());
                    if old_uri != Some(uri) {
                        *existing = json_value!([old_entry, new_entry]);
                    } else {
                        *existing = json_value!([new_entry]); // Même URI, on convertit juste
                    }
                }
            } else {
                // Cas C : Première insertion pour ce namespace (Création d'un tableau)
                ontologies.insert(namespace.to_string(), json_value!([new_entry]));
            }
        }

        // 2. Commit sur disque[cite: 28]
        tx.commit().await?;

        Ok(())
    }
}

// =========================================================================
// TESTS UNITAIRES (DDL Handler)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::io::fs;
    use crate::utils::testing::mock::insert_mock_db;
    use crate::utils::testing::mock::DbSandbox;

    /// 🧪 TEST 1 : Création et Suppression d'un Schéma (Disque + Index)
    #[async_test]
    async fn test_ddl_create_and_drop_schema() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        let ddl = DdlHandler::new(&manager);

        let schema_name = "test_entities/robot.schema.json";
        let initial_schema = json_value!({
            "type": "object",
            "properties": { "serial_number": { "type": "string" } }
        });

        // 🎯 FIX : On limite la durée de vie du verrou avec un bloc `{}`
        {
            let lock = manager
                .storage
                .get_index_lock(&manager.space, &manager.db)?;
            let guard = lock.lock().await;
            let mut tx = manager.begin_system_tx(&guard).await?;
            ddl.create_schema(&mut tx, schema_name, initial_schema)
                .await?;
            tx.commit().await?;
        } // 🛡️ Ici, guard est détruit et le verrou est rendu au système !

        let version = ddl.get_domain_version().await;
        let sys_doc = manager.load_index().await?;
        let ptr = sys_doc["schemas"][&version].get("test_entities/robot.schema.json");
        assert!(
            ptr.is_some(),
            "Le pointeur du schéma doit être dans l'index"
        );

        let uri = ddl.build_schema_uri(schema_name).await;
        let (v, rel_key) = ddl.extract_schema_paths(&uri);
        let path = manager
            .storage
            .config
            .db_schemas_root(&manager.space, &manager.db)
            .join(&v)
            .join(&rel_key);
        assert!(path.exists(), "Le fichier physique du schéma doit exister");

        // 2. DROP (Maintenant c'est safe car l'ancien verrou est détruit)
        ddl.drop_schema(schema_name).await?;

        assert!(!path.exists(), "Le fichier physique doit être supprimé");
        let sys_doc_after = manager.load_index().await?;
        assert!(sys_doc_after["schemas"][&version]
            .get("test_entities/robot.schema.json")
            .is_none());

        Ok(())
    }

    /// 🧪 TEST 2 : Manipulation fine des Propriétés
    #[async_test]
    async fn test_ddl_property_manipulation() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        let ddl = DdlHandler::new(&manager);
        let schema_name = "test_props.schema.json";

        {
            let lock = manager
                .storage
                .get_index_lock(&manager.space, &manager.db)?;
            let guard = lock.lock().await;
            let mut tx = manager.begin_system_tx(&guard).await?;
            ddl.create_schema(&mut tx, schema_name, json_value!({ "type": "object" }))
                .await?;
            tx.commit().await?;
        }

        let get_props = || async {
            let uri = ddl.build_schema_uri(schema_name).await;
            let (v, k) = ddl.extract_schema_paths(&uri);
            let path = manager
                .storage
                .config
                .db_schemas_root(&manager.space, &manager.db)
                .join(v)
                .join(k);
            let schema: JsonValue = fs::read_json_async(&path).await.unwrap();
            schema["properties"].clone()
        };

        // Les fonctions de modification génèrent leurs propres jetons en interne
        ddl.add_property(schema_name, "age", json_value!({ "type": "integer" }))
            .await?;
        let props_after_add = get_props().await;
        assert_eq!(props_after_add["age"]["type"], "integer");

        ddl.alter_property(
            schema_name,
            "age",
            json_value!({ "type": "number", "minimum": 0 }),
        )
        .await?;
        let props_after_alter = get_props().await;
        assert_eq!(props_after_alter["age"]["type"], "number");

        ddl.drop_property(schema_name, "age").await?;
        let props_after_drop = get_props().await;
        assert!(props_after_drop.get("age").is_none());

        Ok(())
    }

    /// 🧪 TEST 3 : Gestion du cycle de vie des Collections
    #[async_test]
    async fn test_ddl_collections_lifecycle() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        let ddl = DdlHandler::new(&manager);

        let col_name = "test_robots";
        let schema_name = "robot.schema.json";
        let schema_uri = ddl.build_schema_uri(schema_name).await;

        // 🎯 FIX : Création de la collection enfermée dans son scope
        {
            let lock = manager
                .storage
                .get_index_lock(&manager.space, &manager.db)?;
            let guard = lock.lock().await;
            let mut tx = manager.begin_system_tx(&guard).await?;

            ddl.create_schema(&mut tx, schema_name, json_value!({ "type": "object" }))
                .await?;
            ddl.create_collection(&mut tx, col_name, &schema_uri)
                .await?;

            tx.commit().await?;
        } // Le garde meurt ici. On est libéré !

        let col_path =
            manager
                .storage
                .config
                .db_collection_path(&manager.space, &manager.db, col_name);
        assert!(
            col_path.exists(),
            "Le dossier de la collection doit exister"
        );

        let meta_path = col_path.join("_meta.json");
        assert!(meta_path.exists(), "Le fichier _meta.json doit exister");

        let sys_doc = manager.load_index().await?;
        assert!(sys_doc["collections"].get(col_name).is_some());

        // 2. DROP Collection (Maintenant ça ne bloquera plus)
        ddl.drop_collection(col_name).await?;

        assert!(!col_path.exists());
        let sys_doc_after = manager.load_index().await?;
        assert!(sys_doc_after["collections"].get(col_name).is_none());

        Ok(())
    }

    /// 🧪 TEST 4 : Enregistrement d'Ontologies dans l'ADN (DDL)
    #[async_test]
    async fn test_ddl_register_ontology() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        let ddl = DdlHandler::new(&manager);

        // 🎯 FIX : Utiliser des espaces de noms fictifs pour ne pas interférer avec le bootstrap de la Sandbox
        let test_ns = "test_family";

        // 1. Enregistrement initial (Création du tableau)
        ddl.register_ontology(test_ns, "db://_system/test/onto-core.jsonld", "1.0.0")
            .await?;

        let mut sys_doc = manager.load_index().await?;
        let entries = sys_doc["ontologies"][test_ns]
            .as_array()
            .expect("Doit être un tableau");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["uri"], "db://_system/test/onto-core.jsonld");

        // 2. Test d'étanchéité : Espace de noms différent
        ddl.register_ontology("test_other", "db://_system/test/onto-other.jsonld", "1.0.0")
            .await?;

        // 3. 🎯 Ajout d'une ontologie fille dans la MÊME famille
        ddl.register_ontology(test_ns, "db://_system/test/onto-extended.jsonld", "1.0.0")
            .await?;

        // 🔍 Vérification finale
        sys_doc = manager.load_index().await?;

        let final_entries = sys_doc["ontologies"][test_ns].as_array().unwrap();
        assert_eq!(
            final_entries.len(),
            2,
            "La famille de test doit contenir 2 ontologies"
        );
        assert_eq!(
            final_entries[1]["uri"],
            "db://_system/test/onto-extended.jsonld"
        );

        let other_final = sys_doc["ontologies"]["test_other"].as_array().unwrap();
        assert_eq!(other_final.len(), 1);

        Ok(())
    }

    /// 🧪 TEST 5 : Cycle de vie complet d'une Base de Données (Gouvernance système)
    #[async_test]
    #[serial_test::serial]
    async fn test_ddl_database_lifecycle_with_governance() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;

        let target_domain = "project_omega";
        let target_db = "flight_software";

        // 🎯 FIX : Initialiser les collections de gouvernance dans la base système pour le test
        let sys_mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        let generic_schema: &str = "db://_system/bootstrap/schemas/v1/db/generic.schema.json";
        sys_mgr.create_collection("domains", generic_schema).await?;
        sys_mgr
            .create_collection("databases", generic_schema)
            .await?;

        // 1. Initialisation du manager pour la NOUVELLE base
        let target_mgr = CollectionsManager::new(&sandbox.storage, target_domain, target_db);
        let ddl = DdlHandler::new(&target_mgr);

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/index.schema.json",
            crate::utils::data::config::BOOTSTRAP_DOMAIN,
            crate::utils::data::config::BOOTSTRAP_DB
        );

        // 2. Création de la DB (DDL)
        let created = ddl.create_db_with_schema(&schema_uri).await?;
        assert!(created, "La base de données aurait dû être créée.");

        // 3. Vérification Physique
        let db_path = sandbox.storage.config.db_root(target_domain, target_db);
        assert!(
            db_path.exists(),
            "Le dossier racine de la base doit exister."
        );

        // 4. Vérification de la Gouvernance (Catalogue Système)
        // La méthode get_document() sait chercher par 'handle' grâce à son fallback interne
        let domain_doc = sys_mgr.get_document("domains", target_domain).await?;
        assert!(
            domain_doc.is_some(),
            "Le domaine doit être inscrit dans le catalogue système"
        );

        let db_doc = sys_mgr.get_document("databases", target_db).await?;
        assert!(
            db_doc.is_some(),
            "La base de données doit être inscrite dans le catalogue système"
        );

        // 5. Suppression de la DB (DDL)
        let dropped = ddl.drop_db().await?;
        assert!(dropped, "La base de données aurait dû être supprimée.");

        // 6. Vérification du Nettoyage Physique
        assert!(
            !db_path.exists(),
            "Le dossier racine de la base doit avoir été supprimé."
        );

        // 7. Vérification du Nettoyage de la Gouvernance
        let db_doc_after = sys_mgr.get_document("databases", target_db).await?;
        assert!(
            db_doc_after.is_none(),
            "La base de données doit être désinscrite du catalogue système"
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_ddl_alter_db_properties() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let mgr = CollectionsManager::new(&sandbox.storage, "test_alter", "db_alter");
        let ddl = DdlHandler::new(&mgr);

        // 1. Initialisation de la base avec le schéma v1 garanti par la Sandbox
        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/index.schema.json",
            crate::utils::data::config::BOOTSTRAP_DOMAIN,
            crate::utils::data::config::BOOTSTRAP_DB
        );
        let created = ddl.create_db_with_schema(&schema_uri).await?;
        assert!(created, "La base de données doit être créée");

        // Création explicite de la collection _ontologies avant l'insertion
        mgr.create_collection(
            "_ontologies",
            "db://_system/_system/schemas/v1/db/generic.schema.json",
        )
        .await?;

        // 2. On insère une ontologie cible avec un ID physique fixe
        let expected_uuid = "uuid-physique-core-001";
        let doc_onto = &json_value!({
            "_id": expected_uuid,
            "handle": "onto-raise-core",
            "name": "Ontologie RAISE Core"
        });
        insert_mock_db(&mgr, "_ontologies", doc_onto).await?;

        // 3. ACTION : On tente de modifier @context avec un SmartLink (chaîne brute)
        let input_link = "ref:_ontologies:handle:onto-raise-core";
        ddl.alter_db("@context", json_value!(input_link)).await?;

        // 4. VERIFICATION DE LA CHIRURGIE
        let index = mgr.load_index().await?;
        let stored_val = index.get("@context").and_then(|v| v.as_str());

        assert_eq!(
            stored_val,
            Some(expected_uuid),
            "ECHEC CRITIQUE : L'index contient encore la référence '{}' au lieu de l'ID physique '{}'",
            stored_val.unwrap_or("null"),
            expected_uuid
        );

        Ok(())
    }
}
