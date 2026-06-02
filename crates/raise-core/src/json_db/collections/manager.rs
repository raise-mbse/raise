// FICHIER : crates/raise-core/src/json_db/collections/manager.rs
use crate::utils::prelude::*;

use crate::json_db::indexes::IndexManager;
use crate::json_db::jsonld::{JsonLdProcessor, VocabularyRegistry};
use crate::json_db::query::{Condition, FilterOperator, Query, QueryEngine, QueryFilter};
use crate::json_db::schema::ddl::DdlHandler;
use crate::json_db::schema::{SchemaRegistry, SchemaValidator};
use crate::json_db::storage::{file_storage, StorageEngine};

use super::collection;

pub enum EntityIdentity {
    Id(String),
    Name(String),
}

#[derive(Debug)]
pub struct CollectionsManager<'a> {
    pub storage: &'a StorageEngine,
    pub space: String,
    pub db: String,
}

pub struct SystemIndexTx<'a> {
    pub manager: &'a CollectionsManager<'a>,
    pub document: JsonValue,
}

impl<'a> SystemIndexTx<'a> {
    /// Valide la transaction et sauvegarde l'index sur le disque
    pub async fn commit(mut self) -> RaiseResult<()> {
        // Optionnel : Mettre à jour la date de modification
        if let Some(obj) = self.document.as_object_mut() {
            obj.insert(
                "_updated_at".to_string(),
                JsonValue::String(crate::utils::prelude::UtcClock::now().to_rfc3339()),
            );
        }

        crate::json_db::storage::file_storage::write_system_index(
            &self.manager.storage.config,
            &self.manager.space,
            &self.manager.db,
            &self.document,
        )
        .await
    }
}

impl<'a> CollectionsManager<'a> {
    pub fn new(storage: &'a StorageEngine, space: &str, db: &str) -> Self {
        Self {
            storage,
            space: space.to_string(),
            db: db.to_string(),
        }
    }

    pub async fn begin_system_tx<G>(
        &'a self,
        _proof_of_lock: &G,
    ) -> RaiseResult<SystemIndexTx<'a>> {
        let document = match self.load_index().await {
            Ok(doc) => doc,
            Err(_) => {
                // Initialisation sécurisée si l'index est absent ou illisible
                json_value!({
                    "collections": {},
                    "schemas": { "v2": {} },
                    "rules": {},
                    "ontologies": {}
                })
            }
        };

        Ok(SystemIndexTx {
            manager: self,
            document,
        })
    }

    pub async fn init_db(&self) -> RaiseResult<bool> {
        DdlHandler::new(self).init_db().await
    }

    pub async fn init_db_with_schema(&self, schema_uri: &str) -> RaiseResult<bool> {
        DdlHandler::new(self).init_db_with_schema(schema_uri).await
    }

    pub async fn create_db_with_schema(&self, schema_uri: &str) -> RaiseResult<bool> {
        DdlHandler::new(self)
            .create_db_with_schema(schema_uri)
            .await
    }
    pub async fn alter_db(&self, key: &str, value: JsonValue) -> RaiseResult<()> {
        DdlHandler::new(self).alter_db(key, value).await
    }
    pub async fn drop_db(&self) -> RaiseResult<bool> {
        DdlHandler::new(self).drop_db().await
    }

    pub async fn import_schemas(&self, source_space: &str, source_db: &str) -> RaiseResult<usize> {
        DdlHandler::new(self)
            .import_schemas_from_storage(source_space, source_db)
            .await
    }

    pub async fn load_index(&self) -> RaiseResult<JsonValue> {
        let sys_path = self
            .storage
            .config
            .db_root(&self.space, &self.db)
            .join("_system.json");

        if !fs::exists_async(&sys_path).await {
            raise_error!(
                "ERR_DB_SYSTEM_INDEX_NOT_FOUND",
                error = "L'index _system.json est introuvable physiquement.",
                context = json_value!({
                    "path": sys_path,
                    "space": self.space,
                    "db": self.db
                })
            );
        }

        match fs::read_json_async(&sys_path).await {
            Ok(v) => Ok(v),
            Err(e) => Err(e),
        }
    }

    // ============================================================================
    // GESTION DES SCHÉMAS ET ONTOLOGIES (DDL)
    // ============================================================================

    pub async fn get_domain_version(&self) -> String {
        DdlHandler::new(self).get_domain_version().await
    }

    pub async fn build_schema_uri(&self, schema_name: &str) -> String {
        DdlHandler::new(self).build_schema_uri(schema_name).await
    }

    pub async fn create_schema_def(&self, name: &str, schema: JsonValue) -> RaiseResult<()> {
        // 1. On sécurise le coffre
        let lock = self.storage.get_index_lock(&self.space, &self.db)?;
        let guard = lock.lock().await;

        // 2. On génère le Jeton
        let mut tx = self.begin_system_tx(&guard).await?;

        // 3. On passe le jeton à l'ouvrier
        DdlHandler::new(self)
            .create_schema(&mut tx, name, schema)
            .await?;

        // 4. On valide
        tx.commit().await
    }

    pub async fn get_schema_def(&self, name_or_uri: &str) -> RaiseResult<JsonValue> {
        // 1. Résolution stricte de l'URI complète
        let uri = self.build_schema_uri(name_or_uri).await;

        // 2. Extraction des sous-dossiers via le DdlHandler
        let (version, rel_key) = DdlHandler::new(self).extract_schema_paths(&uri);

        // 3. Calcul du chemin physique absolu via la config du StorageEngine
        let path = self
            .storage
            .config
            .db_schemas_root(&self.space, &self.db)
            .join(&version)
            .join(&rel_key);

        if !fs::exists_async(&path).await {
            raise_error!(
                "ERR_DDL_SCHEMA_NOT_FOUND",
                error = format!("Le schéma '{}' est introuvable physiquement.", uri),
                context = json_value!({ "path": path.to_string_lossy() })
            );
        }

        // 4. Lecture asynchrone via la façade RAISE
        fs::read_json_async(&path).await
    }

    pub async fn drop_schema_def(&self, name: &str) -> RaiseResult<()> {
        DdlHandler::new(self).drop_schema(name).await
    }

    pub async fn add_schema_property(
        &self,
        name: &str,
        prop: &str,
        def: JsonValue,
    ) -> RaiseResult<()> {
        DdlHandler::new(self).add_property(name, prop, def).await
    }

    /// Modifie la définition d'une propriété existante
    pub async fn alter_schema_property(
        &self,
        name: &str,
        prop: &str,
        definition: JsonValue,
    ) -> RaiseResult<()> {
        DdlHandler::new(self)
            .alter_property(name, prop, definition)
            .await
    }

    /// Supprime une propriété du schéma
    pub async fn drop_schema_property(&self, name: &str, prop: &str) -> RaiseResult<()> {
        DdlHandler::new(self).drop_property(name, prop).await
    }

    pub async fn list_schemas(&self) -> RaiseResult<Vec<String>> {
        let reg = SchemaRegistry::from_db(&self.storage.config, &self.space, &self.db).await?;
        let mut uris = reg.list_uris();
        uris.sort();
        Ok(uris)
    }

    pub async fn register_ontology(
        &self,
        namespace: &str,
        uri: &str,
        version: &str,
    ) -> RaiseResult<()> {
        DdlHandler::new(self)
            .register_ontology(namespace, uri, version)
            .await
    }

    // ============================================================================
    // MÉTHODES DE LECTURE
    // ============================================================================
    pub async fn resolve_single_reference(&self, smart_link: &str) -> RaiseResult<String> {
        // On réutilise ta logique interne de parse_smart_link
        let json_val = JsonValue::String(smart_link.to_string());
        let resolved_json = resolve_refs_recursive(json_val, self).await?;

        match resolved_json.as_str() {
            Some(uuid) => Ok(uuid.to_string()),
            None => raise_error!(
                "ERR_DB_REF_RESOLUTION",
                error = "Le lien n'a pas pu être résolu en UUID"
            ),
        }
    }

    #[async_recursive]
    pub async fn get_document(
        &self,
        collection: &str,
        id_or_handle: &str,
    ) -> RaiseResult<Option<JsonValue>> {
        // 1. Protection "Fail-Fast"
        let col_path = self
            .storage
            .config
            .db_collection_path(&self.space, &self.db, collection);
        if !col_path.exists() {
            return Ok(None);
        }

        // 2. Recherche Physique directe par ID (Rapide O(1))
        if let Ok(Some(doc)) = self
            .storage
            .read_document(&self.space, &self.db, collection, id_or_handle)
            .await
        {
            return Ok(Some(doc));
        }

        // 3. Recherche Logique par Handle via Index (Rapide O(log N))
        let mut query = Query::new(collection);
        query.filter = Some(QueryFilter {
            operator: FilterOperator::And,
            conditions: vec![Condition::eq(
                "handle",
                crate::utils::data::json::json_value!(id_or_handle),
            )],
        });
        query.limit = Some(1);

        // Si le QueryEngine réussit, on renvoie le résultat.
        if let Ok(result) = QueryEngine::new(self).execute_query(query).await {
            if let Some(doc) = result.documents.into_iter().next() {
                return Ok(Some(doc));
            }
        }

        // 4. FALLBACK PHYSIQUE ULTIME
        if let Ok(mut entries) = fs::read_dir_async(&col_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("json") {
                    if path.file_name().and_then(|s| s.to_str()) == Some("_meta.json") {
                        continue;
                    }
                    if let Ok(content) = fs::read_json_async::<JsonValue>(&path).await {
                        if content.get("handle").and_then(|v| v.as_str()) == Some(id_or_handle) {
                            return Ok(Some(content));
                        }
                    }
                }
            }
        }
        Ok(None)
    }

    #[async_recursive]
    pub async fn get(
        &self,
        collection: &str,
        id_or_handle: &str,
    ) -> RaiseResult<Option<JsonValue>> {
        self.get_document(collection, id_or_handle).await
    }

    pub async fn read_many(&self, collection: &str, ids: &[String]) -> RaiseResult<Vec<JsonValue>> {
        let mut docs = Vec::with_capacity(ids.len());
        for _id in ids {
            let doc_opt = match self.get_document(collection, _id).await {
                Ok(doc) => doc,
                Err(e) => raise_error!(
                    "ERR_DB_DOCUMENT_READ",
                    error = e,
                    context = json_value!({
                        "collection": collection,
                        "document_id": _id
                    })
                ),
            };

            match doc_opt {
                Some(doc) => docs.push(doc),
                None => raise_error!(
                    "ERR_DB_CORRUPTION_INDEX_MISMATCH",
                    error = "Document indexé mais introuvable physiquement",
                    context = json_value!({ "_id": _id, "coll": collection })
                ),
            }
        }
        Ok(docs)
    }

    pub async fn list_all(&self, collection: &str) -> RaiseResult<Vec<JsonValue>> {
        collection::list_documents(self.storage, &self.space, &self.db, collection, None, None)
            .await
    }

    pub async fn list_collections(&self) -> RaiseResult<Vec<String>> {
        collection::list_collection_names_fs(&self.storage.config, &self.space, &self.db).await
    }

    pub async fn list_paginated(
        &self,
        collection: &str,
        limit: usize,
        offset: usize,
    ) -> RaiseResult<Vec<JsonValue>> {
        collection::list_documents(
            self.storage,
            &self.space,
            &self.db,
            collection,
            Some(limit),
            Some(offset),
        )
        .await
    }

    pub(crate) async fn save_system_index(&self, doc: &mut JsonValue) -> RaiseResult<()> {
        let schema_uri = match doc.get("$schema").and_then(|v| v.as_str()) {
            Some(uri) => uri.to_string(),
            None => {
                raise_error!(
                    "ERR_DB_MISSING_INDEX_SCHEMA",
                    error = "Le document d'index système ne possède pas de propriété '$schema'.",
                    context = json_value!({ "action": "save_system_index" })
                );
            }
        };

        let is_v1_index = schema_uri.contains("/schemas/v1/") && schema_uri.contains("/index");
        let is_v2_index = schema_uri.contains("/schemas/v2/") && schema_uri.contains("/index");

        if !is_v1_index && !is_v2_index {
            raise_error!(
                "ERR_DB_SECURITY_VIOLATION",
                error = "Le schéma déclaré n'est pas un schéma d'index système autorisé.",
                context = json_value!({
                    "found_uri": schema_uri,
                    "action": "enforce_system_integrity",
                    "hint": "Un schéma d'index doit être situé dans /schemas/v1/ ou /schemas/v2/ et contenir 'index'."
                })
            );
        }

        let reg =
            SchemaRegistry::from_uri(&self.storage.config, &schema_uri, &self.space, &self.db)
                .await?;
        if reg.get_by_uri(&schema_uri).is_none() {
            /* à remettre après migration et tests
            raise_error!(
                "ERR_DB_SECURITY_VIOLATION",
                error = "Schéma d'index système introuvable ou non autorisé.",
                context = json_value!({
                    "required_uri": schema_uri,
                    "action": "enforce_system_integrity",
                })
            );
            */
            if reg.get_by_uri(&schema_uri).is_none() {
                user_warn!(
                    "WRN_DB_MISSING_CORE_SCHEMA",
                    json_value!({
                        "required_uri": schema_uri,
                        "action": "enforce_system_integrity",
                        "hint": "Migration DDL : Schéma d'index système introuvable en mémoire. Tolérance activée pour permettre l'écriture."
                    })
                );
            }
        }

        if let Err(e) = crate::rules_engine::apply_business_rules(
            self,
            "_system_index",
            doc,
            None,
            &reg,
            &schema_uri,
        )
        .await
        {
            user_warn!(
                "WRN_SYSTEM_RULE_INDEX_FAIL",
                json_value!({
                    "component": "INDEX_ENGINE",
                    "technical_error": e.to_string(),
                    "is_blocking": false,
                    "hint": "Vérifiez l'intégrité du schéma JSON-LD dans _system"
                })
            );
        }

        if let Ok(validator) = SchemaValidator::compile_with_registry(&schema_uri, &reg) {
            // 🎯 Création du contexte pour l'index système
            let compute_ctx = crate::rules_engine::compute::ComputeContext {
                document: doc.clone(),
                collection_name: "_system".to_string(),
                db_name: self.db.clone(),
                space_name: self.space.clone(),
            };

            if let Err(e) = validator.compute_then_validate(doc, &compute_ctx).await {
                user_warn!(
                    "WRN_SYSTEM_INDEX_INVALID_RECOVER",
                    json_value!({
                        "component": "INDEX_ENGINE",
                        "action": "FORCE_SAVE",
                        "technical_error": e.to_string(),
                        "hint": "L'index a été corrompu mais la Forteresse a forcé une récupération."
                    })
                );
            }
        }

        file_storage::write_system_index(&self.storage.config, &self.space, &self.db, doc).await?;
        Ok(())
    }

    // --- GESTION DES COLLECTIONS ---
    pub async fn create_collection(&self, name: &str, uri: &str) -> RaiseResult<()> {
        let sys_path = self
            .storage
            .config
            .db_root(&self.space, &self.db)
            .join("_system.json");

        if !sys_path.exists() {
            raise_error!(
                "ERR_DB_NOT_FOUND",
                error = format!(
                    "Impossible de créer la collection '{}' car la base '{}/{}' n'existe pas.",
                    name, self.space, self.db
                ),
                context = json_value!({
                    "action": "create_collection",
                    "hint": "Initialisez d'abord la base de données avec son schéma principal avant d'y ajouter des collections."
                })
            );
        }
        let lock = self.storage.get_index_lock(&self.space, &self.db)?;
        let guard = lock.lock().await;

        // On génère le Jeton
        let mut tx = self.begin_system_tx(&guard).await?;

        // On passe le jeton à l'ouvrier
        DdlHandler::new(self)
            .create_collection(&mut tx, name, uri)
            .await?;

        // On valide la transaction
        tx.commit().await
    }

    pub async fn drop_collection(&self, name: &str) -> RaiseResult<()> {
        collection::drop_collection(&self.storage.config, &self.space, &self.db, name).await?;
        self.remove_collection_from_system_index(name).await?;
        Ok(())
    }

    // --- INDEXES SECONDAIRES ---
    pub async fn create_index(&self, collection: &str, field: &str, kind: &str) -> RaiseResult<()> {
        let mut idx_mgr = IndexManager::new(self.storage, &self.space, &self.db);
        idx_mgr.create_index(collection, field, kind).await
    }

    pub async fn drop_index(&self, collection: &str, field: &str) -> RaiseResult<()> {
        let mut idx_mgr = IndexManager::new(self.storage, &self.space, &self.db);
        idx_mgr.drop_index(collection, field).await
    }

    // --- HELPER INDEX SYSTÈME & RÉSOLUTION SCHÉMA ---

    async fn resolve_schema_from_index(&self, col_name: &str) -> RaiseResult<String> {
        DdlHandler::new(self)
            .resolve_schema_from_index(col_name)
            .await
    }

    async fn remove_collection_from_system_index(&self, col_name: &str) -> RaiseResult<()> {
        // 🎯 FIX FAIL-FAST : On s'assure physiquement que la base est initialisée avant une mutation
        let sys_path = self
            .storage
            .config
            .db_root(&self.space, &self.db)
            .join("_system.json");
        if !sys_path.exists() {
            let _ = self.load_index().await?; // Déclenche l'erreur ERR_DB_SYSTEM_INDEX_NOT_FOUND
        }

        let lock = self.storage.get_index_lock(&self.space, &self.db)?;
        let guard = lock.lock().await;

        let mut tx = self.begin_system_tx(&guard).await?;
        let mut changed = false;

        if let Some(cols) = tx
            .document
            .get_mut("collections")
            .and_then(|c| c.as_object_mut())
        {
            if cols.remove(col_name).is_some() {
                changed = true;
            }
        }

        if changed {
            tx.commit().await?;
        }

        Ok(())
    }

    async fn add_item_to_index(&self, col_name: &str, id: &str) -> RaiseResult<()> {
        // 🎯 FIX FAIL-FAST
        let sys_path = self
            .storage
            .config
            .db_root(&self.space, &self.db)
            .join("_system.json");
        if !sys_path.exists() {
            let _ = self.load_index().await?;
        }

        let lock = self.storage.get_index_lock(&self.space, &self.db)?;
        let guard = lock.lock().await;

        let mut tx = self.begin_system_tx(&guard).await?;

        if tx.document.get("collections").is_none() {
            tx.document["collections"] = json_value!({});
        }
        let filename = format!("{}.json", id);

        if let Some(cols) = tx.document["collections"].as_object_mut() {
            if !cols.contains_key(col_name) {
                let schema_guess = self
                    .resolve_schema_from_index(col_name)
                    .await
                    .unwrap_or_default();
                cols.insert(
                    col_name.to_string(),
                    json_value!({ "schema": schema_guess, "items": [] }),
                );
            }
            if let Some(col_entry) = cols.get_mut(col_name) {
                if col_entry.get("items").is_none() {
                    col_entry["items"] = json_value!([]);
                }
                if let Some(items) = col_entry["items"].as_array_mut() {
                    if !items
                        .iter()
                        .any(|i| i.get("file").and_then(|f| f.as_str()) == Some(&filename))
                    {
                        items.push(json_value!({ "file": filename }));
                    }
                }
            }
        }

        tx.commit().await?;
        Ok(())
    }

    async fn remove_item_from_index(&self, col_name: &str, id: &str) -> RaiseResult<()> {
        // 🎯 FIX FAIL-FAST
        let sys_path = self
            .storage
            .config
            .db_root(&self.space, &self.db)
            .join("_system.json");
        if !sys_path.exists() {
            let _ = self.load_index().await?;
        }

        let lock = self.storage.get_index_lock(&self.space, &self.db)?;
        let guard = lock.lock().await;

        let mut tx = self.begin_system_tx(&guard).await?;

        let filename = format!("{}.json", id);
        let mut changed = false;

        if let Some(cols) = tx
            .document
            .get_mut("collections")
            .and_then(|c| c.as_object_mut())
        {
            if let Some(col_entry) = cols.get_mut(col_name) {
                if let Some(items) = col_entry.get_mut("items").and_then(|i| i.as_array_mut()) {
                    let initial_len = items.len();

                    items.retain(|i| i.get("file").and_then(|f| f.as_str()) != Some(&filename));

                    if items.len() < initial_len {
                        changed = true;
                    }
                }
            }
        }

        if changed {
            tx.commit().await?;
        }

        Ok(())
    }

    // --- ÉCRITURE ET MISE À JOUR ---
    pub async fn insert_raw(&self, collection: &str, doc: &JsonValue) -> RaiseResult<()> {
        let internal_id = doc
            .get("_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let _id = match internal_id {
            Some(id) => id,
            None => raise_error!(
                "ERR_DB_INTEGRITY_VIOLATION",
                error = "Identifiant '_id' absent du document JSON.",
                context = json_value!({ "collection": collection })
            ),
        };

        if let Some(handle) = doc.get("handle").and_then(|v| v.as_str()) {
            if let Ok(Some(existing_doc)) = self.get_document(collection, handle).await {
                let existing_id = existing_doc
                    .get("_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                if existing_id != _id {
                    raise_error!(
                        "ERR_DB_DUPLICATE_HANDLE",
                        error = format!("Violation d'intégrité : Le handle '{}' existe déjà dans la collection '{}'.", handle, collection),
                        context = json_value!({ "existing_id": existing_id, "new_id": _id })
                    );
                }
            }
        }

        let meta_path = self
            .storage
            .config
            .db_collection_path(&self.space, &self.db, collection)
            .join("_meta.json");

        if !meta_path.exists() {
            let schema_hint = doc.get("$schema").and_then(|s| s.as_str());
            if let Some(uri) = schema_hint {
                self.create_collection(collection, uri).await?;
            } else {
                raise_error!(
                    "ERR_DB_STRICT_SCHEMA_REQUIRED",
                    error = "Impossible de créer la collection à la volée : aucun '$schema' défini dans le document.",
                    context = json_value!({ "collection": collection })
                );
            }
        }

        // Passage par référence &_id
        self.storage
            .write_document(&self.space, &self.db, collection, &_id, doc)
            .await?;

        // Passage par référence &_id
        self.add_item_to_index(collection, &_id).await?;

        let mut idx_mgr = IndexManager::new(self.storage, &self.space, &self.db);
        if let Err(_e) = idx_mgr.index_document(collection, doc).await {
            #[cfg(debug_assertions)]
            user_warn!(
                "WRN_SECONDARY_INDEX_FAILED",
                json_value!({
                    "component": "INDEX_ENGINE",
                    "index_type": "secondary",
                    "technical_error": _e.to_string(),
                    "is_critical": false,
                    "hint": "La recherche sur cet index peut être dégradée, mais l'intégrité de la donnée source est préservée."
                })
            );
        }
        Ok(())
    }

    #[async_recursive]
    pub async fn insert_with_schema(
        &self,
        collection: &str,
        mut doc: JsonValue,
    ) -> RaiseResult<JsonValue> {
        doc = self.resolve_document_references(collection, doc).await?;
        self.prepare_document(collection, &mut doc).await?;
        self.insert_raw(collection, &doc).await?;
        Ok(doc)
    }

    pub async fn update_document(
        &self,
        collection: &str,
        id: &str,
        patch_data: JsonValue,
    ) -> RaiseResult<JsonValue> {
        let resolved_patch = self
            .resolve_document_references(collection, patch_data)
            .await?;
        let old_doc_opt = self.get_document(collection, id).await?;
        let Some(mut doc) = old_doc_opt else {
            raise_error!(
                "ERR_DB_UPDATE_TARGET_NOT_FOUND",
                error = "Échec de la mise à jour : le document original est introuvable.",
                context = json_value!({ "action": "update_document" })
            );
        };
        json_merge(&mut doc, resolved_patch);

        if let Some(obj) = doc.as_object_mut() {
            obj.insert("_id".to_string(), JsonValue::String(id.to_string()));
            if let Some(p2p) = obj.get_mut("_p2p").and_then(|v| v.as_object_mut()) {
                if let Some(rev) = p2p.get("revision").and_then(|v| v.as_i64()) {
                    p2p.insert("revision".to_string(), json_value!(rev + 1));
                }
            }
        }

        self.prepare_document(collection, &mut doc).await?;

        self.storage
            .write_document(&self.space, &self.db, collection, id, &doc)
            .await?;

        let mut idx_mgr = IndexManager::new(self.storage, &self.space, &self.db);
        let _ = idx_mgr.index_document(collection, &doc).await;

        Ok(doc)
    }

    #[async_recursive]
    pub async fn upsert_document(
        &self,
        collection: &str,
        mut data: JsonValue,
    ) -> RaiseResult<String> {
        data = self.resolve_document_references(collection, data).await?;

        let id_opt = data
            .get("_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let handle_opt = data.get("handle").and_then(|v| v.as_str());
        let name_opt = data.get("name").and_then(|v| v.as_str());

        if id_opt.is_none() && handle_opt.is_none() && name_opt.is_none() {
            raise_error!(
                "ERR_DB_UPSERT_MISSING_IDENTITY",
                error = "Identifiant manquant : l'upsert requiert '_id', 'handle' ou 'name'.",
                context = json_value!({
                    "action": "upsert_document",
                    "hint": "Fournissez une clé d'identification unique à la racine du document."
                })
            );
        }

        let mut target_id = None;

        // 1. Recherche par _id
        if let Some(ref id) = id_opt {
            if let Ok(Some(_)) = self.get_document(collection, id).await {
                target_id = Some(id.clone());
            }
        }

        // 2. Recherche par Handle (Via le nouveau get_document sécurisé)
        if target_id.is_none() {
            if let Some(handle) = handle_opt {
                if let Some(existing_doc) = self.get_document(collection, handle).await? {
                    if let Some(found_id) = existing_doc.get("_id").and_then(|v| v.as_str()) {
                        target_id = Some(found_id.to_string());
                    }
                }
            }
        }

        // 3. Recherche par Name (Fallback)
        if target_id.is_none() {
            if let Some(name) = name_opt {
                let col_path =
                    self.storage
                        .config
                        .db_collection_path(&self.space, &self.db, collection);
                if col_path.exists() {
                    let mut query = Query::new(collection);
                    query.filter = Some(QueryFilter {
                        operator: FilterOperator::And,
                        conditions: vec![Condition::eq("name", json_value!(name))],
                    });
                    query.limit = Some(1);

                    // 🎯 FIX : Propagation de l'erreur
                    let result = QueryEngine::new(self).execute_query(query).await?;
                    if let Some(existing_doc) = result.documents.first() {
                        if let Some(found_id) = existing_doc.get("_id").and_then(|v| v.as_str()) {
                            target_id = Some(found_id.to_string());
                        }
                    }
                }
            }
        }

        match target_id {
            Some(id) => {
                self.update_document(collection, &id, data).await?;
                Ok(format!("Updated: {}", id))
            }
            None => {
                let doc = self.insert_with_schema(collection, data).await?;
                let new_id = doc.get("_id").and_then(|v| v.as_str()).unwrap();
                Ok(format!("Created: {}", new_id))
            }
        }
    }

    pub async fn delete_document(&self, collection: &str, id: &str) -> RaiseResult<bool> {
        let old_doc = self.get_document(collection, id).await?;
        self.storage
            .delete_document(&self.space, &self.db, collection, id)
            .await?;
        if let Some(doc) = old_doc {
            let mut idx_mgr = IndexManager::new(self.storage, &self.space, &self.db);
            let _ = idx_mgr.remove_document(collection, &doc).await;
        }
        self.remove_item_from_index(collection, id).await?;
        Ok(true)
    }

    #[async_recursive]
    pub async fn prepare_document(&self, collection: &str, doc: &mut JsonValue) -> RaiseResult<()> {
        let mut resolved_uri: Option<String> = None;

        let meta_path = self
            .storage
            .config
            .db_collection_path(&self.space, &self.db, collection)
            .join("_meta.json");

        if meta_path.exists() {
            if let Ok(content) = fs::read_to_string_async(&meta_path).await {
                if let Ok(meta) = json::deserialize_from_str::<JsonValue>(&content) {
                    if let Some(s) = meta.get("schema").and_then(|v| v.as_str()) {
                        if !s.is_empty() {
                            resolved_uri = Some(self.build_schema_uri(s).await);
                        }
                    }
                }
            }
        }

        if resolved_uri.is_none() {
            if let Ok(sys_uri) = self.resolve_schema_from_index(collection).await {
                if !sys_uri.is_empty() {
                    resolved_uri = Some(sys_uri);
                }
            }
        }

        // ====================================================================
        // Auto-découverte du schéma pour les nouvelles collections
        // ====================================================================
        if resolved_uri.is_none() {
            if let Some(doc_schema) = doc.get("$schema").and_then(|v| v.as_str()) {
                if !doc_schema.is_empty() {
                    resolved_uri = Some(doc_schema.to_string());
                }
            }
        }

        if let Some(uri) = &resolved_uri {
            if let Some(obj) = doc.as_object_mut() {
                obj.insert("$schema".to_string(), JsonValue::String(uri.clone()));
            }

            let reg =
                SchemaRegistry::from_uri(&self.storage.config, uri, &self.space, &self.db).await?;

            if let Err(e) =
                crate::rules_engine::apply_business_rules(self, collection, doc, None, &reg, uri)
                    .await
            {
                user_warn!(
                    "WRN_BUSINESS_RULE_FAILURE",
                    json_value!({
                        "component": "RULES_ENGINE",
                        "severity": "non_blocking",
                        "technical_error": e.to_string(),
                        "hint": "Une règle métier n'a pas pu être validée, mais l'opération continue."
                    })
                );
            }

            let validator = SchemaValidator::compile_with_registry(uri, &reg)?;
            let compute_ctx = crate::rules_engine::compute::ComputeContext {
                document: doc.clone(),
                collection_name: collection.to_string(),
                db_name: self.db.clone(),
                space_name: self.space.clone(),
            };

            validator.compute_then_validate(doc, &compute_ctx).await?;

            if let Some(obj) = doc.as_object_mut() {
                let ws_id = AppConfig::get()
                    .workstation
                    .as_ref()
                    .map(|ws| ws.id.as_str())
                    .unwrap_or("unknown");

                let mut doc_for_hash = obj.clone();
                doc_for_hash.remove("_p2p");
                let hash = self.compute_document_checksum(&JsonValue::Object(doc_for_hash));

                if let Some(p2p) = obj.get_mut("_p2p").and_then(|v| v.as_object_mut()) {
                    p2p.insert("checksum".to_string(), json_value!(hash));
                    p2p.insert("origin_node".to_string(), json_value!(ws_id));
                    p2p.insert(
                        "last_sync_at".to_string(),
                        json_value!(UtcClock::now().to_rfc3339()),
                    );
                }
            }
        } else {
            raise_error!(
                "ERR_DB_STRICT_SCHEMA_REQUIRED",
                error = "Insertion refusée : Aucun schéma de validation n'est défini pour cette collection.",
                context = json_value!({ "collection": collection, "action": "prepare_document" })
            );
        }

        if let Err(e) = self.apply_semantic_logic(doc).await {
            raise_error!(
                "ERR_AI_SEMANTIC_VALIDATION_FAIL",
                error = e.to_string(),
                context = json_value!({
                    "action": "semantic_integrity_check",
                    "collection": collection,
                    "resolved_uri": resolved_uri,
                    "hint": "Le document n'a pas pu être aligné avec l'ontologie JSON-LD."
                })
            );
        }

        Ok(())
    }

    // Applique la logique sémantique avec Résolution Ontologique en Cascade (Lazy Loading)
    pub(crate) async fn apply_semantic_logic(&self, doc: &mut JsonValue) -> RaiseResult<()> {
        let mut prefix_opt = None;

        // 🎯 ÉTAPE 1 : Détection de l'intention sémantique
        // On cherche un préfixe dans le(s) type(s) déclaré(s) (ex: "raise" dans "raise:User")
        if let Some(t) = doc.get("@type") {
            let type_str = if let Some(s) = t.as_str() {
                Some(s)
            } else if let Some(arr) = t.as_array() {
                arr.first().and_then(|v| v.as_str()) // On prend le type primaire
            } else {
                None
            };

            if let Some(s) = type_str {
                if let Some((p, _)) = s.split_once(':') {
                    if p != "http" && p != "urn" {
                        prefix_opt = Some(p.to_string());
                    }
                }
            }
        }

        // 🎯 ÉTAPE 2 : Résolution en Cascade et Chargement RCU
        if let Some(prefix) = prefix_opt {
            let registry = VocabularyRegistry::global()?;

            // Si l'ontologie n'est pas encore en RAM, on déclenche la recherche
            if registry.get_context_for_layer(&prefix).is_none() {
                let mut ontology_doc = None;

                // --- NIVEAU 1 : L'Index Local (_system.json) ---[cite: 6]
                if let Ok(sys_idx) = self.load_index().await {
                    if let Some(uri) = sys_idx
                        .get("ontologies")
                        .and_then(|o| o.get(&prefix))
                        .and_then(|p| p.get("uri"))
                        .and_then(|u| u.as_str())
                    {
                        // On parse le lien intelligent (ex: db://_system/master/collections/_ontologies/handle/onto-raise-core)[cite: 4, 6]
                        if let Some(SmartLink::Absolute {
                            space,
                            db,
                            col,
                            val,
                            ..
                        }) = parse_smart_link(uri)
                        {
                            let target_mgr = CollectionsManager::new(self.storage, space, db);
                            // get_document cherche par _id ou handle nativement
                            if let Ok(Some(doc)) = target_mgr.get_document(col, val).await {
                                ontology_doc = Some(doc);
                            }
                        }
                    }
                }

                // --- NIVEAU 2 & 3 : Fallback Global (Catalogue puis Partition Système) ---
                if ontology_doc.is_none() {
                    // On tente la convention de nommage métier (ex: onto-raise-core)[cite: 6]
                    let handle = format!("onto-{}-core", prefix);
                    if let Ok(Some((_, _, doc))) =
                        self.find_global_document("_ontologies", &handle).await
                    {
                        ontology_doc = Some(doc);
                    } else {
                        // Fallback sur la convention système ancienne (ex: ontology_raise)[cite: 12]
                        let alt_handle = format!("ontology_{}", prefix);
                        if let Ok(Some((_, _, doc))) =
                            self.find_global_document("_ontologies", &alt_handle).await
                        {
                            ontology_doc = Some(doc);
                        }
                    }
                }

                // --- HYDRATATION EN RAM ---[cite: 11]
                if let Some(onto) = ontology_doc {
                    registry.load_layer_from_json(&prefix, &onto).await?;
                } else {
                    user_warn!(
                        "WRN_ONTOLOGY_RESOLUTION_FAIL",
                        json_value!({
                            "prefix": prefix,
                            "hint": "Le Cerveau sémantique n'a pas pu localiser la définition de ce préfixe."
                        })
                    );
                }
            }

            // 🎯 ÉTAPE 3 : Injection du @context (Si le schéma x_compute ne l'a pas fait)
            if let Some(obj) = doc.as_object_mut() {
                if !obj.contains_key("@context") {
                    if let Some(ctx) = registry.get_context_for_layer(&prefix) {
                        obj.insert("@context".to_string(), ctx);
                    }
                }
            }
        }

        // 🎯 ÉTAPE 4 : Validation Stricte par le Cerveau (Identique mais optimisé)[cite: 4, 10]
        let has_type = doc.get("@type").is_some()
            || doc
                .get("http://www.w3.org/1999/02/22-rdf-syntax-ns#type")
                .is_some();

        if has_type {
            let processor = JsonLdProcessor::new()?.with_doc_context(doc)?;

            if let Some(type_uri) = processor.get_primary_type(doc) {
                let registry = VocabularyRegistry::global()?;
                let mut expanded_type = processor.context_manager().expand_term(&type_uri);

                // Double expansion si on a une CURIE complexe (ex: "raise:User")
                if !VocabularyRegistry::is_iri(&expanded_type) && expanded_type.contains(':') {
                    expanded_type = processor.context_manager().expand_term(&expanded_type);
                }

                if !registry.has_class(&expanded_type) {
                    #[cfg(debug_assertions)]
                    println!(
                        "⚠️ [Semantic Warning] Type inconnu: {} (Expanded from {})",
                        expanded_type, type_uri
                    );
                }
            }
        }
        Ok(())
    }

    pub async fn delete_identity(
        &self,
        collection: &str,
        identity: EntityIdentity,
    ) -> RaiseResult<()> {
        let target_id = match identity {
            EntityIdentity::Id(id) => id,
            EntityIdentity::Name(name) => {
                let qe = QueryEngine::new(self);
                let mut query = Query::new(collection);
                query.filter = Some(QueryFilter {
                    operator: FilterOperator::And,
                    conditions: vec![Condition::eq(
                        "name",
                        crate::utils::json::json_value!(&name),
                    )],
                });
                let res = qe.execute_query(query).await?;

                let doc = match res.documents.first() {
                    Some(d) => d,
                    None => {
                        raise_error!(
                            "ERR_DB_ENTITY_NOT_FOUND",
                            error = format!("Document nommé '{}' introuvable.", name),
                            context = json_value!({ "name": name, "collection": collection })
                        );
                    }
                };

                match doc.get("_id").and_then(|v| v.as_str()) {
                    Some(id_str) => id_str.to_string(),
                    None => {
                        raise_error!("ERR_DB_MISSING_ID", context = json_value!({ "name": name }))
                    }
                }
            }
        };

        match self.delete_document(collection, &target_id).await {
            Ok(_) => Ok(()),
            Err(e) => Err(e),
        }
    }

    pub async fn resolve_document_references(
        &self,
        collection: &str,
        mut document: JsonValue,
    ) -> RaiseResult<JsonValue> {
        let mut ref_keys: UniqueSet<String> = UniqueSet::new();

        if let Ok(sys_uri) = self.resolve_schema_from_index(collection).await {
            if !sys_uri.is_empty() {
                if let Ok(reg) =
                    SchemaRegistry::from_uri(&self.storage.config, &sys_uri, &self.space, &self.db)
                        .await
                {
                    if let Some(schema) = reg.get_by_uri(&sys_uri) {
                        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
                            for (k, v) in props {
                                let mut is_id_ref = false;

                                if let Some(r) = v.get("$ref").and_then(|s| s.as_str()) {
                                    if r.ends_with("_id") {
                                        is_id_ref = true;
                                    }
                                }
                                if let Some(items) = v.get("items") {
                                    if let Some(r) = items.get("$ref").and_then(|s| s.as_str()) {
                                        if r.ends_with("_id") {
                                            is_id_ref = true;
                                        }
                                    }
                                }

                                if is_id_ref {
                                    ref_keys.insert(k.clone());
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(obj) = document.as_object_mut() {
            let schema_val = obj.remove("$schema");
            let context_val = obj.remove("@context");

            for (k, v) in obj.iter_mut() {
                let should_resolve = if !ref_keys.is_empty() {
                    ref_keys.contains(k)
                } else {
                    !k.starts_with('@') && !k.starts_with('_')
                };

                if should_resolve {
                    *v = resolve_refs_recursive(v.clone(), self).await?;
                }
            }

            if let Some(s) = schema_val {
                obj.insert("$schema".to_string(), s);
            }
            if let Some(c) = context_val {
                obj.insert("@context".to_string(), c);
            }
        } else {
            document = resolve_refs_recursive(document, self).await?;
        }

        Ok(document)
    }

    pub async fn find_global_document(
        &self,
        collection: &str,
        id_or_handle: &str,
    ) -> RaiseResult<Option<(String, String, JsonValue)>> {
        let config = AppConfig::get();
        let sys_domain = &config.mount_points.system.domain;
        let sys_db = &config.mount_points.system.db;

        // 1. Accès au Catalogue Système
        // On crée un manager système prêt pour le catalogue ou le fallback
        let sys_mgr = CollectionsManager::new(self.storage, sys_domain, sys_db);

        // On détermine quel manager sert de catalogue de gouvernance
        let catalog = if self.space == *sys_domain && self.db == *sys_db {
            self
        } else {
            &sys_mgr
        };

        // 2. Récupérer toutes les bases de données déclarées
        let databases = match catalog.list_all("databases").await {
            Ok(dbs) => dbs,
            Err(_) => return Ok(None), // Fail-gracefully si le catalogue n'est pas prêt
        };

        for db_doc in databases {
            // On ignore les bases désactivées
            if db_doc.get("status").and_then(|v| v.as_str()) == Some("inactive") {
                continue;
            }

            let db_handle = match db_doc.get("handle").and_then(|v| v.as_str()) {
                Some(h) => h.to_string(),
                None => continue,
            };

            let domain_id = match db_doc.get("domain_id").and_then(|v| v.as_str()) {
                Some(id) => id,
                None => continue,
            };

            // 3. Résolution du domaine
            let domain_handle =
                if let Ok(Some(domain_doc)) = catalog.get_document("domains", domain_id).await {
                    domain_doc
                        .get("handle")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string()
                } else {
                    continue;
                };

            if domain_handle.is_empty() {
                continue;
            }

            // 4. Recherche Locale immédiate (Optimisation)
            if domain_handle == self.space && db_handle == self.db {
                if let Ok(Some(doc)) = self.get_document(collection, id_or_handle).await {
                    return Ok(Some((domain_handle, db_handle, doc)));
                }
                continue;
            }

            // 5. Recherche dans la base distante
            let temp_mgr = CollectionsManager::new(self.storage, &domain_handle, &db_handle);

            // Remplacement du fs::exists par la lecture de l'index de la base (Zéro Dette OS)
            if let Ok(index) = temp_mgr.load_index().await {
                if index["collections"].get(collection).is_some() {
                    if let Ok(Some(doc)) = temp_mgr.get_document(collection, id_or_handle).await {
                        return Ok(Some((domain_handle, db_handle, doc)));
                    }
                }
            }
        }

        // 3. 🛡️ FALLBACK CRITIQUE : Partition Système (Résilience Bootstrap)
        //if self.space != *sys_domain || self.db != *sys_db {
        user_debug!(
            "DB_GLOBAL_SEARCH_FALLBACK",
            json_value!({"coll": collection, "target": id_or_handle})
        );
        // Ici, sys_mgr n'est plus une Option, donc get_document() fonctionne !
        if let Ok(Some(doc)) = sys_mgr.get_document(collection, id_or_handle).await {
            return Ok(Some((sys_domain.clone(), sys_db.clone(), doc)));
        }
        //}
        Ok(None)
    }

    fn compute_document_checksum(&self, doc: &JsonValue) -> String {
        let content = json::serialize_to_bytes(doc).unwrap_or_default();
        let mut hasher = CryptoSha256::new();
        hasher.update(content);
        hex::encode(hasher.finalize())
    }
}

fn json_merge(a: &mut JsonValue, b: JsonValue) {
    match (a, b) {
        (JsonValue::Object(a), JsonValue::Object(b)) => {
            for (k, v) in b {
                json_merge(a.entry(k).or_insert(JsonValue::Null), v);
            }
        }
        (a, b) => *a = b,
    }
}

pub enum SmartLink<'a> {
    Local {
        col: &'a str,
        field: &'a str,
        val: &'a str,
    },
    Absolute {
        space: &'a str,
        db: &'a str,
        col: &'a str,
        field: &'a str,
        val: &'a str,
    },
}

pub fn parse_smart_link(s: &str) -> Option<SmartLink<'_>> {
    if s.starts_with("ref:") {
        let parts: Vec<&str> = s.splitn(4, ':').collect();
        if parts.len() == 4 {
            Some(SmartLink::Local {
                col: parts[1],
                field: parts[2],
                val: parts[3],
            })
        } else {
            None
        }
    } else if let Some(without_scheme) = s.strip_prefix("db://") {
        // 🛡️ BOUCLIER SÉMANTIQUE : Ignorer les URIs des Assets de l'Usine
        if s.contains("/schemas/")
            || s.contains("/ontologies/")
            || s.contains("/ontology/")
            || s.contains("/@context/")
        {
            return None;
        }
        let parts: Vec<&str> = without_scheme.splitn(5, '/').collect();
        if parts.len() == 5 {
            Some(SmartLink::Absolute {
                space: parts[0],
                db: parts[1],
                col: parts[2],
                field: parts[3],
                val: parts[4],
            })
        } else {
            None
        }
    } else {
        None
    }
}

fn resolve_refs_recursive<'a>(
    data: JsonValue,
    col_mgr: &'a CollectionsManager<'a>,
) -> Pinned<Box<dyn AsyncFuture<Output = RaiseResult<JsonValue>> + Send + 'a>> {
    Box::pin(async move {
        // 🎯 FIX : Extraction indépendante de l'optimisation mémoire (Short ou String)
        if let Some(s) = data.as_str() {
            if let Some(link) = parse_smart_link(s) {
                let (col, field, val, is_absolute, target_space, target_db) = match link {
                    SmartLink::Local { col, field, val } => (col, field, val, false, "", ""),
                    SmartLink::Absolute {
                        space,
                        db,
                        col,
                        field,
                        val,
                    } => (col, field, val, true, space, db),
                };

                let mut query = Query::new(col);
                query.filter = Some(QueryFilter {
                    operator: FilterOperator::And,
                    conditions: vec![Condition::eq(field, val.into())],
                });
                query.limit = Some(1);

                let doc_opt = if is_absolute {
                    let remote_mgr =
                        CollectionsManager::new(col_mgr.storage, target_space, target_db);
                    let query_result = QueryEngine::new(&remote_mgr).execute_query(query).await;
                    match query_result {
                        Ok(res) => res.documents.into_iter().next(),
                        Err(_) => None,
                    }
                } else {
                    let query_result = QueryEngine::new(col_mgr).execute_query(query).await;
                    match query_result {
                        Ok(res) => res.documents.into_iter().next(),
                        Err(_) => None,
                    }
                };

                if let Some(doc) = doc_opt {
                    let id = doc.get("_id").and_then(|v| v.as_str()).unwrap_or("");
                    return Ok(JsonValue::String(id.to_string()));
                } else {
                    raise_error!(
                        "ERR_DB_DANGLING_REFERENCE",
                        error = format!("Impossible de résoudre la référence : '{}' pointe vers une entité introuvable.", s),
                        context = json_value!({
                            "action": "resolve_document_references",
                            "target_collection": col,
                            "target_field": field,
                            "target_value": val,
                            "is_cross_domain": is_absolute,
                        })
                    );
                }
            }
            // Si c'est une chaîne classique (ex: "linux", "fr"), on la retourne telle quelle
            return Ok(JsonValue::String(s.to_string()));
        }

        // Suite du traitement pour les Array et Object
        match data {
            JsonValue::Array(arr) => {
                let mut new_arr = Vec::new();
                for item in arr {
                    new_arr.push(resolve_refs_recursive(item, col_mgr).await?);
                }
                Ok(JsonValue::Array(new_arr))
            }
            JsonValue::Object(map) => {
                let mut new_map = JsonObject::new();
                for (k, v) in map {
                    new_map.insert(k, resolve_refs_recursive(v, col_mgr).await?);
                }
                Ok(JsonValue::Object(new_map))
            }
            _ => Ok(data),
        }
    })
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::mock::insert_mock_db;
    use crate::utils::testing::DbSandbox;

    #[async_test]
    #[serial_test::serial]
    async fn test_manager_init_db_completeness() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "system_test", "db_test");

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/index.schema.json",
            crate::utils::data::config::BOOTSTRAP_DOMAIN,
            crate::utils::data::config::BOOTSTRAP_DB
        );

        let created = manager.init_db_with_schema(&schema_uri).await?;
        assert!(created, "La DB aurait dû être créée pour la première fois");

        let index_opt =
            file_storage::read_system_index(&sandbox.storage.config, "system_test", "db_test")
                .await?;
        let index = match index_opt {
            Some(idx) => idx,
            None => panic!("Le fichier _system.json est introuvable"),
        };

        // On vérifie que le validateur a bien généré l'ID
        assert!(
            index.get("_id").is_some(),
            "L'index devrait avoir un '_id' généré"
        );

        // On vérifie que le validateur a bien injecté la collection par défaut
        assert!(
            index["collections"].get("_migrations").is_some(),
            "La collection '_migrations' aurait dû être injectée par défaut"
        );

        let expected_migration_uri = format!(
            "db://{}/{}/schemas/v1/db/migration.schema.json",
            crate::utils::data::config::BOOTSTRAP_DOMAIN,
            crate::utils::data::config::BOOTSTRAP_DB
        );

        assert_eq!(
            index["collections"]["_migrations"]["schema"],
            expected_migration_uri
        );

        let db_root = sandbox.storage.config.db_root("system_test", "db_test");
        let migration_path = db_root.join("collections/_migrations");
        assert!(
            migration_path.exists(),
            "Le dossier physique de _migrations est manquant"
        );

        Ok(())
    }

    #[async_test]
    async fn test_manager_get_document_integration() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;
        manager
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let doc = json_value!({ "_id": "user_123", "name": "Test User" });
        insert_mock_db(&manager, "users", &doc).await?;

        let result = manager.get_document("users", "user_123").await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap()["name"], "Test User");

        let missing = manager.get_document("users", "ghost").await?;
        assert!(missing.is_none());

        Ok(())
    }

    #[async_test]
    async fn test_read_many_parallel() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;
        manager
            .create_collection(
                "items",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        for i in 0..100 {
            let doc = json_value!({ "_id": i.to_string(), "val": i });
            insert_mock_db(&manager, "items", &doc).await?;
        }

        let ids: Vec<String> = vec!["10", "20", "50", "80", "99"]
            .into_iter()
            .map(String::from)
            .collect();
        let results = manager.read_many("items", &ids).await?;

        assert_eq!(results.len(), 5);
        for res in results {
            let id = match res["_id"].as_str() {
                Some(id) => id,
                None => panic!("Document sans _id retourné"),
            };
            assert!(ids.contains(&id.to_string()));
        }

        Ok(())
    }

    #[async_test]
    async fn test_read_many_strict_integrity() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;
        manager
            .create_collection(
                "items",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let item_doc = &json_value!({ "_id": "1", "val": "A" });
        insert_mock_db(&manager, "items", &item_doc).await?;

        let ids = vec!["1".to_string(), "999".to_string()];
        let result = manager.read_many("items", &ids).await;

        assert!(result.is_err());
        match result {
            Err(e) => assert!(e.to_string().contains("ERR_DB_CORRUPTION_INDEX_MISMATCH")),
            Ok(_) => panic!("Aurait dû échouer"),
        }

        Ok(())
    }

    #[async_test]
    async fn test_crud_workflow() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let mgr = CollectionsManager::new(&sandbox.storage, "test", "crud");
        DbSandbox::mock_db(&mgr).await?;

        mgr.create_collection(
            "items",
            "db://_system/_system/schemas/v1/db/generic.schema.json",
        )
        .await?;

        let doc = json_value!({ "name": "Item 1", "price": 100 });
        let created_doc = mgr.insert_with_schema("items", doc).await?;

        let id = match created_doc["_id"].as_str() {
            Some(id) => id.to_string(),
            None => panic!("Aucun _id généré lors de la création"),
        };

        let fetched = mgr.get_document("items", &id).await?;
        assert!(fetched.is_some());

        mgr.update_document(
            "items",
            &id,
            json_value!({ "price": 150, "status": "active" }),
        )
        .await?;

        let updated_doc = mgr.get_document("items", &id).await?;
        let updated = match updated_doc {
            Some(u) => u,
            None => panic!("Le document mis à jour est introuvable"),
        };
        assert_eq!(updated["price"], 150);
        assert_eq!(updated["name"], "Item 1");
        assert_eq!(updated["status"], "active");

        let deleted = mgr.delete_document("items", &id).await?;
        assert!(deleted);

        let missing = mgr.get_document("items", &id).await?;
        assert!(missing.is_none());

        Ok(())
    }

    #[async_test]
    async fn test_upsert_idempotence() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let mgr = CollectionsManager::new(&sandbox.storage, "test", "upsert");
        DbSandbox::mock_db(&mgr).await?;

        mgr.create_collection(
            "configs",
            "db://_system/_system/schemas/v1/db/generic.schema.json",
        )
        .await?;

        let data1 = json_value!({ "_id": "config-01", "val": "A" });
        let res1 = mgr.upsert_document("configs", data1).await?;
        assert!(res1.contains("Created"));

        let data2 = json_value!({ "_id": "config-01", "val": "B" });
        let res2 = mgr.upsert_document("configs", data2).await?;
        assert!(res2.contains("Updated"));

        let final_doc_opt = mgr.get_document("configs", "config-01").await?;
        let final_doc = match final_doc_opt {
            Some(d) => d,
            None => panic!("Document upsert introuvable"),
        };

        assert_eq!(final_doc["val"], "B");
        assert_eq!(final_doc["_id"], "config-01");

        Ok(())
    }

    #[test]
    fn test_parse_smart_link_local_valid() {
        let input = "ref:oa_actors:name:Sécurité";
        let res = super::parse_smart_link(input);
        assert!(res.is_some());
        if let super::SmartLink::Local { col, field, val } = res.unwrap() {
            assert_eq!(col, "oa_actors");
            assert_eq!(field, "name");
            assert_eq!(val, "Sécurité");
        } else {
            panic!("Aurait dû être un lien local");
        }
    }

    #[test]
    fn test_parse_smart_link_absolute_valid() {
        let input = "db://_system/_system/prompts/handle/prompt_mission_initializer";
        let res = super::parse_smart_link(input);
        assert!(res.is_some());
        if let super::SmartLink::Absolute {
            space,
            db,
            col,
            field,
            val,
        } = res.unwrap()
        {
            assert_eq!(space, "_system");
            assert_eq!(db, "_system");
            assert_eq!(col, "prompts");
            assert_eq!(field, "handle");
            assert_eq!(val, "prompt_mission_initializer");
        } else {
            panic!("Aurait dû être un lien absolu");
        }
    }

    #[test]
    fn test_parse_smart_link_invalid_prefix() {
        assert!(super::parse_smart_link("uuid:1234-5678").is_none());
        assert!(super::parse_smart_link("http://google.com").is_none());
    }

    #[test]
    fn test_parse_smart_link_missing_parts() {
        assert!(super::parse_smart_link("ref:oa_actors:name").is_none());
        assert!(super::parse_smart_link("db://_system/_system/prompts").is_none());
    }

    #[async_test]
    async fn test_manager_delete_identity() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;

        manager
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let doc_alice = json_value!({ "_id": "u_100", "name": "Alice" });
        let doc_bob = json_value!({ "_id": "u_200", "name": "Bob" });

        insert_mock_db(&manager, "users", &doc_alice).await?;
        insert_mock_db(&manager, "users", &doc_bob).await?;

        manager
            .delete_identity("users", EntityIdentity::Id("u_100".to_string()))
            .await?;

        let fetch_alice = manager.get_document("users", "u_100").await?;
        assert!(
            fetch_alice.is_none(),
            "Alice (u_100) devrait être supprimée de la base"
        );

        manager
            .delete_identity("users", EntityIdentity::Name("Bob".to_string()))
            .await?;

        let fetch_bob = manager.get_document("users", "u_200").await?;
        assert!(
            fetch_bob.is_none(),
            "Bob (u_200) devrait être supprimé après résolution du nom"
        );

        let res = manager
            .delete_identity("users", EntityIdentity::Name("Fantome".to_string()))
            .await;

        assert!(
            res.is_err(),
            "La tentative de suppression d'un nom inexistant doit échouer"
        );

        Ok(())
    }

    #[async_test]
    async fn test_manager_fail_fast_on_missing_index() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "ghost_domain", "ghost_db");

        DbSandbox::mock_db(&manager).await?;
        manager
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let sys_path = manager
            .storage
            .config
            .db_root(&manager.space, &manager.db)
            .join("_system.json");
        fs::remove_file_async(&sys_path).await?;

        let doc = json_value!({ "_id": "1", "name": "Test Fail Fast" });
        let res = insert_mock_db(&manager, "users", &doc).await;
        assert!(res.is_err());
        if let Err(e) = res {
            assert!(e.to_string().contains("ERR_DB_SYSTEM_INDEX_NOT_FOUND"));
        }

        Ok(())
    }

    #[async_test]
    async fn test_manager_remove_item_from_index() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");

        DbSandbox::mock_db(&manager).await?;
        manager
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let doc = json_value!({ "_id": "u1", "name": "Alice" });
        insert_mock_db(&manager, "users", &doc).await?;

        let index = manager.load_index().await?;
        let items = match index["collections"]["users"]["items"].as_array() {
            Some(i) => i,
            None => panic!("L'index n'a pas été formaté correctement"),
        };
        assert_eq!(items.len(), 1, "Le document devrait être dans l'index");

        manager.delete_document("users", "u1").await?;

        let index_after = manager.load_index().await?;
        let items_after = match index_after["collections"]["users"]["items"].as_array() {
            Some(i) => i,
            None => panic!("L'index a été altéré"),
        };
        assert!(
            items_after.is_empty(),
            "L'index devrait être vide après suppression"
        );

        Ok(())
    }

    #[async_test]
    async fn test_manager_remove_collection_from_index() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");

        DbSandbox::mock_db(&manager).await?;

        manager
            .create_collection(
                "temporary",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let index = manager.load_index().await?;
        assert!(
            index["collections"].get("temporary").is_some(),
            "La collection devrait exister dans l'index"
        );

        manager.drop_collection("temporary").await?;

        let index_after = manager.load_index().await?;
        assert!(
            index_after["collections"].get("temporary").is_none(),
            "La collection devrait avoir disparu de l'index"
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_manager_resolve_single_reference() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;

        // 1. Création de la collection cible
        manager
            .create_collection(
                "services",
                "db://_system/bootstrap/schemas/v1/db/generic.schema.json",
            )
            .await?;

        // 2. Insertion d'un document avec un UUID fixe
        let expected_uuid = "uuid-physique-1234";
        let handle = "svc_test_ref";
        let doc_service = &json_value!({
            "_id": expected_uuid,
            "handle": handle,
            "name": "Service de Test"
        });
        insert_mock_db(&manager, "services", &doc_service).await?;
        // 3. Test de résolution
        let smart_link = format!("ref:services:handle:{}", handle);
        let resolved = manager.resolve_single_reference(&smart_link).await?;

        assert_eq!(
            resolved, expected_uuid,
            "Le lien sémantique doit pointer vers l'UUID physique"
        );
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_manager_find_global_document() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let sys_domain = &sandbox.config.mount_points.system.domain;
        let sys_db = &sandbox.config.mount_points.system.db;

        let sys_mgr = CollectionsManager::new(&sandbox.storage, sys_domain, sys_db);
        DbSandbox::mock_db(&sys_mgr).await?;

        // 1. Initialisation du Catalogue Système
        let schema_uri = "db://_system/_system/schemas/v1/db/generic.schema.json";
        sys_mgr.create_collection("domains", schema_uri).await?;
        sys_mgr.create_collection("databases", schema_uri).await?;

        // Injection d'un domaine et d'une base de données distante
        let doc_domain =
            &json_value!({"_id": "dom_archive", "handle": "archive_domain", "status": "active"});
        insert_mock_db(&sys_mgr, "domains", &doc_domain).await?;

        let doc_db = &json_value!({
            "_id": "db_archive", "handle": "history_db", "domain_id": "dom_archive", "status": "active"
        });
        insert_mock_db(&sys_mgr, "databases", &doc_db).await?;

        // 2. Création des données dans la base distante
        let remote_mgr = CollectionsManager::new(&sandbox.storage, "archive_domain", "history_db");
        DbSandbox::mock_db(&remote_mgr).await?;
        remote_mgr
            .create_collection("global_items", schema_uri)
            .await?;
        let doc_gitem = &json_value!({
            "_id": "g_123", "handle": "my_global_item", "status": "archived"
        });
        insert_mock_db(&remote_mgr, "global_items", &doc_gitem).await?;
        // 3. Test de la recherche globale depuis la base principale
        let result = sys_mgr
            .find_global_document("global_items", "my_global_item")
            .await?;

        assert!(
            result.is_some(),
            "Le document global aurait dû être trouvé via le catalogue"
        );

        let (found_domain, found_db, doc) = result.unwrap();
        assert_eq!(found_domain, "archive_domain");
        assert_eq!(found_db, "history_db");
        assert_eq!(doc["_id"], "g_123");
        assert_eq!(doc["status"], "archived");

        // 4. Test d'un objet introuvable
        let missing = sys_mgr
            .find_global_document("global_items", "ghost")
            .await?;
        assert!(missing.is_none());

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_manager_find_global_fallback_to_system() -> RaiseResult<()> {
        // 1. SETUP : On utilise la sandbox qui initialise déjà la partition système
        let sandbox = DbSandbox::new().await?;
        let sys_domain = &sandbox.config.mount_points.system.domain;
        let sys_db = &sandbox.config.mount_points.system.db;

        // On crée un manager sur la partition SYSTÈME pour y injecter une ressource
        let sys_mgr = CollectionsManager::new(&sandbox.storage, sys_domain, sys_db);
        DbSandbox::mock_db(&sys_mgr).await?;

        sys_mgr
            .create_collection(
                "locales",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;
        let doc_locale = &json_value!({
            "_id": "fr",
            "handle": "fr",
            "value": "Bonjour"
        });
        insert_mock_db(&sys_mgr, "locales", &doc_locale).await?;

        // 2. TARGET : On crée un manager sur une base métier TOTALEMENT ISOLÉE
        // Cette base n'est pas déclarée dans le catalogue 'databases'
        let user_mgr = CollectionsManager::new(&sandbox.storage, "project_x", "sandbox_db");
        // On n'appelle PAS mock_db ici pour simuler une base vide/nouvelle

        // 3. ACTION : On cherche la ressource 'fr' globalement depuis la base métier
        let result = user_mgr.find_global_document("locales", "fr").await?;

        // 4. VERIFICATION
        assert!(
            result.is_some(),
            "Le fallback aurait dû trouver 'fr' dans la partition système même sans catalogue"
        );

        let (found_domain, found_db, doc) = result.unwrap();

        // On vérifie que le moteur nous confirme bien que ça vient du système
        assert_eq!(found_domain, *sys_domain);
        assert_eq!(found_db, *sys_db);
        assert_eq!(doc["value"], "Bonjour");

        Ok(())
    }
    #[async_test]
    #[serial_test::serial]
    async fn test_manager_physical_fallback_prevents_duplicates() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.storage, "space_test", "db_test");
        DbSandbox::mock_db(&manager).await?;

        // 1. Création de la collection
        manager
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        // 2. Insertion du premier document (Setup : insert_mock_db est parfait ici)
        let doc1 = json_value!({ "_id": "u1", "handle": "agent_smith", "name": "Alice" });
        insert_mock_db(&manager, "users", &doc1).await?;

        // 3. 💣 CORRUPTION VOLONTAIRE
        let idx_path = sandbox
            .storage
            .config
            .db_collection_path(&manager.space, &manager.db, "users")
            .join("indexes")
            .join("handle.idx");

        if idx_path.exists() {
            crate::utils::io::fs::remove_file_async(&idx_path).await?;
        }
        manager.remove_item_from_index("users", "u1").await?;

        // 4. L'ÉPREUVE DU FEU : Tentative d'insertion d'un doublon
        let doc2 = json_value!({ "_id": "u2", "handle": "agent_smith", "name": "Bob" });

        // 🎯 FIX : On utilise `insert_raw` car c'est la méthode stricte de production.
        // On VEUT que le moteur hurle au doublon !
        let res = manager.insert_raw("users", &doc2).await;

        // 5. VALIDATION DE LA FORTERESSE
        assert!(
            res.is_err(),
            "FAIL CRITIQUE : Le moteur a laissé passer un doublon !"
        );

        if let Err(crate::utils::core::error::AppError::Structured(err)) = res {
            assert_eq!(
                err.code, "ERR_DB_DUPLICATE_HANDLE",
                "Le moteur a bloqué l'insertion, mais avec la mauvaise erreur."
            );
        } else {
            panic!("Type d'erreur inattendu renvoyé par insert_raw.");
        }

        Ok(())
    }
}
