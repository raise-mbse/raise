// FICHIER : crates/raise-core/src/kernel/environment.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::{JsonDbConfig, StorageEngine};
use crate::utils::core::error::RaiseResult;
use crate::utils::core::{FmtCursor, FmtDisplay, FmtResult, RuntimeEnv, SharedRef};
use crate::utils::data::config::AppConfig;
use crate::utils::data::json::{json_value, JsonValue};
use crate::utils::io::fs::{self, Path, PathBuf};
use crate::utils::prelude::*;
use crate::{raise_error, user_info, user_success, user_warn};

// ==============================================================================
// 🧬 TAXONOMIE INDUSTRIELLE (Les 8 Partitions Physiques)
// ==============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndustrialPhase {
    System,
    Raise,
    Exploration,
    Modeling,
    Simulation,
    Integration,
    Production,
    Operation,
}

impl IndustrialPhase {
    pub const ALL: [IndustrialPhase; 8] = [
        Self::System,
        Self::Raise,
        Self::Exploration,
        Self::Modeling,
        Self::Simulation,
        Self::Integration,
        Self::Production,
        Self::Operation,
    ];
}

impl FmtDisplay for IndustrialPhase {
    fn fmt(&self, f: &mut FmtCursor<'_>) -> FmtResult {
        let name = match self {
            Self::System => "system",
            Self::Raise => "raise",
            Self::Exploration => "exploration",
            Self::Modeling => "modeling",
            Self::Simulation => "simulation",
            Self::Integration => "integration",
            Self::Production => "production",
            Self::Operation => "operation",
        };
        write!(f, "{}", name)
    }
}

// ==============================================================================
// 🌍 GESTIONNAIRE DE NŒUD (L'Orchestrateur de Boot)
// ==============================================================================

pub struct NodeEnvironment {
    pub storage: SharedRef<StorageEngine>,
    pub local_domain: String,
}

impl NodeEnvironment {
    pub async fn boot_physical_node() -> RaiseResult<(Self, bool)> {
        let config = AppConfig::get();

        let domain_root = match config.get_path("PATH_RAISE_DOMAIN") {
            Some(path) => path,
            None => raise_error!(
                "ERR_KERNEL_NO_DOMAIN_PATH",
                error = "PATH_RAISE_DOMAIN est introuvable sur ce nœud. Vérifiez votre .env."
            ),
        };

        if config.get_path("PATH_RAISE_ASSET").is_none() {
            raise_error!(
                "ERR_KERNEL_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET est requis dans le .env pour référencer l'usine d'assets externe."
            );
        }

        user_info!(
            "NODE_BOOT_START",
            json_value!({
                "domain": config.mount_points.system.domain,
                "path": domain_root.to_string_lossy().to_string(),
                "assets_reference": "FACTORY_SOURCE"
            })
        );

        let storage_config = JsonDbConfig::new(domain_root.clone());
        let storage = match StorageEngine::new(storage_config) {
            Ok(s) => SharedRef::new(s),
            Err(e) => raise_error!("ERR_KERNEL_STORAGE_INIT", error = e),
        };

        let local_domain = config.mount_points.system.domain.clone();

        let bootstrap_domain = match RuntimeEnv::var("RAISE_BOOTSTRAP_DOMAIN") {
            Ok(d) => d,
            Err(_) => "_system".to_string(),
        };
        let bootstrap_db = match RuntimeEnv::var("RAISE_BOOTSTRAP_DB") {
            Ok(db) => db,
            Err(_) => "master".to_string(),
        };

        let system_manager = CollectionsManager::new(&storage, &bootstrap_domain, &bootstrap_db);
        let bootstrapper = SystemBootstrapper::new(system_manager, bootstrap_domain, bootstrap_db);

        let needs_restart = match bootstrapper.execute_if_needed().await {
            Ok(status) => status,
            Err(e) => raise_error!("ERR_KERNEL_BOOTSTRAP_EXEC", error = e),
        };

        for phase in IndustrialPhase::ALL.iter() {
            match Self::ensure_partition(&storage, &local_domain, phase).await {
                Ok(_) => (),
                Err(e) => raise_error!("ERR_KERNEL_PARTITION_INIT", error = e),
            }
        }

        user_success!(
            "NODE_BOOT_COMPLETE",
            json_value!({
                "status": "Toutes les partitions industrielles sont montées et amorcées."
            })
        );

        Ok((
            Self {
                storage,
                local_domain,
            },
            needs_restart,
        ))
    }

    async fn ensure_partition(
        storage: &SharedRef<StorageEngine>,
        domain: &str,
        phase: &IndustrialPhase,
    ) -> RaiseResult<()> {
        let db_name = phase.to_string();
        let partition_path = storage.config.db_root(domain, &db_name);

        match fs::create_dir_all_async(&partition_path).await {
            Ok(_) => (),
            Err(e) => raise_error!(
                "ERR_KERNEL_MKDIR_PARTITION",
                error = e,
                context = json_value!({"path": partition_path.to_string_lossy().to_string()})
            ),
        }

        let manager = CollectionsManager::new(storage, domain, &db_name);

        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        let schema_filename = match phase {
            IndustrialPhase::Raise => "index_raise.schema.json",
            IndustrialPhase::Modeling => "index_mbse.schema.json",
            IndustrialPhase::Simulation => "index_mbse.schema.json",
            IndustrialPhase::System => "index_bootstrap.schema.json",
            _ => "index_raise.schema.json",
        };

        let sys_domain =
            RuntimeEnv::var("RAISE_BOOTSTRAP_DOMAIN").unwrap_or_else(|_| "_system".to_string());
        let sys_db = RuntimeEnv::var("RAISE_BOOTSTRAP_DB").unwrap_or_else(|_| "master".to_string());

        let schema_uri = format!(
            "db://{}/{}/schemas/{}/system/db/{}",
            sys_domain, sys_db, schema_version, schema_filename
        );

        // 🎯 L'API native du CollectionsManager est respectée
        match manager.create_db_with_schema(&schema_uri).await {
            Ok(_) => Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("already initialized")
                    || err_str.contains("ERR_SCHEMA_NOT_IN_REGISTRY")
                {
                    Ok(())
                } else {
                    raise_error!("ERR_KERNEL_ENSURE_PARTITION", error = e)
                }
            }
        }
    }
}

// ==============================================================================
// 🚀 MOTEUR D'ENSEMENCEMENT (Le "Seed Script" embarqué)
// ==============================================================================

pub struct SystemBootstrapper<'a> {
    manager: CollectionsManager<'a>,
    config: &'static AppConfig,
    domain: String,
    db: String,
}

impl<'a> SystemBootstrapper<'a> {
    pub fn new(manager: CollectionsManager<'a>, domain: String, db: String) -> Self {
        Self {
            manager,
            config: AppConfig::get(),
            domain,
            db,
        }
    }

    fn db_path(&self) -> RaiseResult<PathBuf> {
        let root = match self.config.get_path("PATH_RAISE_DOMAIN") {
            Some(p) => p,
            None => raise_error!(
                "ERR_BOOT_NO_DOMAIN_PATH",
                error = "PATH_RAISE_DOMAIN manquant"
            ),
        };
        Ok(root.join(&self.domain).join(&self.db))
    }

    fn extract_operation_documents(raw: JsonValue) -> Vec<JsonValue> {
        let mut docs = Vec::new();
        if let Some(arr) = raw.as_array() {
            let mut is_ops = true;
            for item in arr {
                if let Some(obj) = item.as_object() {
                    if obj.get("type").and_then(|v| v.as_str()) == Some("upsert")
                        && obj.contains_key("document")
                    {
                        docs.push(obj.get("document").unwrap().clone());
                    } else {
                        is_ops = false;
                        break;
                    }
                } else {
                    is_ops = false;
                    break;
                }
            }
            if is_ops && !docs.is_empty() {
                return docs;
            }
        } else if let Some(obj) = raw.as_object() {
            if obj.get("type").and_then(|v| v.as_str()) == Some("upsert")
                && obj.contains_key("document")
            {
                return vec![obj.get("document").unwrap().clone()];
            }
        }
        vec![raw]
    }

    fn reanchor_semantic_node(&self, doc: &mut JsonValue) {
        let local_ontology = format!("db://{}/{}/ontologies/", self.domain, self.db);
        let local_prefix = format!("db://{}/{}/", self.domain, self.db);

        match doc {
            JsonValue::String(s) => {
                let mut new_s = s.to_string();
                let mut modified = false;

                // 1. Remplacement prioritaire pour les ontologies
                if new_s.contains("db://_system/ontology/")
                    || new_s.contains("db://_system/master/ontologies/")
                    || new_s.contains("db://_system/ai-assets/ontologies/")
                {
                    new_s = new_s
                        .replace("db://_system/ontology/", &local_ontology)
                        .replace("db://_system/master/ontologies/", &local_ontology)
                        .replace("db://_system/ai-assets/ontologies/", &local_ontology);
                    modified = true;
                }

                // 2. Remplacement global des préfixes d'usine pour le reste (schemas, refs, etc.)
                if new_s.starts_with("db://_system/") || new_s.starts_with("db://ai-assets/") {
                    new_s = new_s
                        .replace("db://_system/master/", &local_prefix)
                        .replace("db://_system/bootstrap/", &local_prefix)
                        .replace("db://_system/_system/", &local_prefix)
                        .replace("db://_system/raise/", &local_prefix)
                        .replace("db://_system/ai-assets/", &local_prefix)
                        .replace("db://ai-assets/", &local_prefix);
                    modified = true;
                }

                if modified {
                    *doc = json_value!(new_s);
                }
            }
            // 🔄 Traversée récursive universelle (sans filtrage par clés)
            JsonValue::Object(obj) => {
                for (_, v) in obj.iter_mut() {
                    self.reanchor_semantic_node(v);
                }
            }
            JsonValue::Array(arr) => {
                for v in arr.iter_mut() {
                    self.reanchor_semantic_node(v);
                }
            }
            _ => {}
        }
    }

    pub async fn execute_if_needed(&self) -> RaiseResult<bool> {
        let db_path = match self.db_path() {
            Ok(p) => p,
            Err(e) => raise_error!("ERR_BOOT_GET_PATH", error = e),
        };

        if !fs::exists_async(&db_path).await {
            if let Err(e) = fs::create_dir_all_async(&db_path).await {
                raise_error!(
                    "ERR_BOOT_MKDIR",
                    error = e,
                    context = json_value!({"path": db_path.to_string_lossy()})
                );
            }
        }

        let sys_json_path = db_path.join("_system.json");

        // =========================================================
        // 🔄 CAS 1 : LE NŒUD EST DÉJÀ AMORCÉ (Démarrage Normal)
        // =========================================================
        if fs::exists_async(&sys_json_path).await {
            user_info!(
                "BOOT_SYSTEM_READY",
                json_value!({"status": "already_seeded"})
            );

            // 🧠 RÉVEIL DU CERVEAU SÉMANTIQUE
            if let Err(e) =
                crate::json_db::jsonld::VocabularyRegistry::init_from_db(&self.manager).await
            {
                user_warn!("WRN_VOCABULARY_INIT", json_value!({"error": e.to_string()}));
            }

            return Ok(false);
        }

        // =========================================================
        // 🚀 CAS 2 : PREMIER AMORÇAGE (Démarrage à Froid)
        // =========================================================
        user_info!(
            "BOOT_SYSTEM_START",
            json_value!({"action": "provisioning_db", "db": self.db})
        );

        // 🎯 ÉTAPE 1 & 2 : Bootstrap Natif
        match self.step_1_and_2_native_bootstrap().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_1_2", error = e),
        }

        // 🧬 Les Ontologies sont écrites sur le disque
        match self.step_3_import_and_register_ontologies().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_3", error = e),
        }

        // 🧠 RÉVEIL DU CERVEAU SÉMANTIQUE
        user_info!(
            "BOOT_SEMANTIC_INIT",
            json_value!({"action": "Chargement du VocabularyRegistry"})
        );

        if let Err(e) =
            crate::json_db::jsonld::VocabularyRegistry::init_from_db(&self.manager).await
        {
            raise_error!("ERR_BOOT_VOCABULARY_INIT", error = e);
        }

        // ⚙️ Poursuite de l'amorçage
        match self.step_4_import_locales().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_4", error = e),
        }
        match self.step_5_import_seeds().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_5", error = e),
        }

        if let Ok(index_doc) = self.manager.load_index().await {
            let idx_mgr = crate::json_db::indexes::IndexManager::new(
                self.manager.storage,
                &self.domain,
                &self.db,
            );
            let _ = idx_mgr.apply_indexes_from_config(&index_doc).await;
        }

        user_success!(
            "BOOT_SYSTEM_COMPLETE",
            json_value!({"status": "success", "db": self.db})
        );
        Ok(true)
    }

    // =========================================================================
    // 🎯 ÉTAPES 1 & 2 : Importation Cross-Domain et Émancipation Native
    // =========================================================================
    async fn step_1_and_2_native_bootstrap(&self) -> RaiseResult<()> {
        let asset_domain =
            RuntimeEnv::var("RAISE_ASSET_DOMAIN").unwrap_or_else(|_| "_system".to_string());
        let asset_db =
            RuntimeEnv::var("RAISE_ASSET_DB").unwrap_or_else(|_| "ai-assets".to_string());
        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        let sys_json_path = self.db_path()?.join("_system.json");

        // =====================================================================
        // 0. ADOUBEMENT DE L'USINE (Répare l'usine si $schema est manquant)
        // =====================================================================
        let factory_path = match self.config.get_path("PATH_RAISE_ASSET") {
            Some(p) => p,
            None => raise_error!(
                "ERR_BOOT_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET manquant"
            ),
        };
        let factory_index_path = factory_path.join("_system.json");

        // On regénère l'index de l'usine s'il est manquant ou s'il n'a pas de $schema (ancienne version)
        let mut needs_factory_index = true;
        if fs::exists_async(&factory_index_path).await {
            if let Ok(idx) = fs::read_json_async::<JsonValue>(&factory_index_path).await {
                if idx.get("$schema").is_some() {
                    needs_factory_index = false;
                }
            }
        }

        if needs_factory_index {
            user_info!(
                "BOOT_FACTORY_INDEX",
                json_value!({"action": "Génération dynamique de l'index de l'usine"})
            );
            let schemas_dir = factory_path.join("schemas").join(&schema_version);
            let mut schemas_map = crate::utils::data::json::JsonObject::new();

            if fs::exists_async(&schemas_dir).await {
                let mut queue = vec![schemas_dir.clone()];
                while let Some(dir) = queue.pop() {
                    let mut entries = match fs::read_dir_async(&dir).await {
                        Ok(e) => e,
                        Err(_) => continue,
                    };
                    while let Ok(Some(entry)) = entries.next_entry().await {
                        let path = entry.path();
                        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);

                        if is_dir {
                            queue.push(path);
                        } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                            if let Ok(rel_path) = path.strip_prefix(&schemas_dir) {
                                let rel_str = rel_path.to_string_lossy().replace('\\', "/");
                                let file_str = format!("{}/{}", schema_version, rel_str);
                                schemas_map.insert(rel_str, json_value!({ "file": file_str }));
                            }
                        }
                    }
                }
            }

            let factory_schema_uri = format!(
                "db://{}/{}/schemas/{}/system/db/index_bootstrap.schema.json",
                asset_domain, asset_db, schema_version
            );

            let factory_index = json_value!({
                "$schema": factory_schema_uri,
                "handle": format!("{}_{}", asset_domain, asset_db),
                "name": "Factory Assets",
                "space": asset_domain,
                "database": asset_db,
                "collections": {},
                "schemas": { schema_version.clone(): schemas_map },
                "ontologies": {},
                "rules": {},
                "db_role": "factory"
            });
            fs::write_json_atomic_async(&factory_index_path, &factory_index).await?;
        }

        // =====================================================================
        // 1. CRÉATION DE L'EMBRYON MASTER (Avec $schema certifié)
        // =====================================================================
        if !fs::exists_async(&sys_json_path).await {
            user_info!(
                "BOOT_MASTER_INIT",
                json_value!({"action": "Génération de l'embryon Master"})
            );
            fs::create_dir_all_async(self.db_path()?).await?;

            let local_schema_uri = format!(
                "db://{}/{}/schemas/{}/system/db/index_bootstrap.schema.json",
                self.domain, self.db, schema_version
            );

            let embryon = json_value!({
                "$schema": local_schema_uri,
                "handle": format!("{}_{}", self.domain, self.db),
                "name": "Master Database",
                "space": self.domain,
                "database": self.db,
                "collections": {},
                "schemas": {},
                "ontologies": {},
                "rules": {},
                "db_role": "system"
            });
            fs::write_json_atomic_async(&sys_json_path, &embryon).await?;
        }

        // =====================================================================
        // 2. TRANSFERT DES SCHÉMAS (Via 2 Managers, Zéro Dette)
        // =====================================================================
        user_info!(
            "BOOT_IMPORT_SCHEMAS",
            json_value!({"action": "Importation cross-domain", "source": asset_db})
        );

        let factory_root = factory_path
            .parent()
            .and_then(|p| p.parent())
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| factory_path.clone());
        let factory_storage = match StorageEngine::new(JsonDbConfig::new(factory_root)) {
            Ok(s) => s,
            Err(e) => raise_error!("ERR_BOOT_FACTORY_STORAGE", error = e),
        };

        let factory_mgr = CollectionsManager::new(&factory_storage, &asset_domain, &asset_db);
        let factory_index = match factory_mgr.load_index().await {
            Ok(idx) => idx,
            Err(e) => raise_error!("ERR_BOOT_FACTORY_INDEX", error = e),
        };

        if let Some(schemas) = factory_index["schemas"]
            .get(&schema_version)
            .and_then(|v| v.as_object())
        {
            for (schema_key, _) in schemas {
                let schema_uri = format!(
                    "db://{}/{}/schemas/{}/{}",
                    asset_domain, asset_db, schema_version, schema_key
                );
                if let Ok(mut schema_json) = factory_mgr.get_schema_def(&schema_uri).await {
                    // 🎯 MAGIE : On réécrit l'ADN pour qu'il pointe sur master AVANT de le sauvegarder
                    self.reanchor_semantic_node(&mut schema_json);

                    if let Err(e) = self
                        .manager
                        .create_schema_def(schema_key, schema_json)
                        .await
                    {
                        user_warn!(
                            "WRN_BOOT_CREATE_SCHEMA",
                            json_value!({"key": schema_key, "error": e.to_string()})
                        );
                    }
                }
            }
        }

        // =====================================================================
        // 3. L'ADOUBEMENT FINAL ET CALCUL SÉMANTIQUE (Le réveil du DDL)
        // =====================================================================
        user_info!(
            "BOOT_EMANCIPATION",
            json_value!({"action": "Autonomisation de Master et génération des collections"})
        );

        let local_schema_uri = format!(
            "db://{}/{}/schemas/{}/system/db/index_bootstrap.schema.json",
            self.domain, self.db, schema_version
        );

        if let Err(e) = self.manager.init_db_with_schema(&local_schema_uri).await {
            if !e.to_string().contains("already initialized") {
                raise_error!("ERR_BOOT_EMANCIPATION", error = e);
            }
        }

        // =====================================================================
        // 4. NETTOYAGE ULTIME & SYNCHRONISATION PHYSIQUE (Lazy Sync)
        // =====================================================================
        let lock = self
            .manager
            .storage
            .get_index_lock(&self.domain, &self.db)?;
        let guard = lock.lock().await;
        let mut tx = self.manager.begin_system_tx(&guard).await?;

        // 🎯 FIX ULTIME : Le DDL vient d'injecter les collections par défaut depuis le schéma source.
        // On repasse le mutateur sémantique sur l'intégralité du jeton pour écraser les restes de 'ai-assets' !
        self.reanchor_semantic_node(&mut tx.document);

        let bootstrapper =
            crate::json_db::schema::bootstrapper::SchemaBootstrapper::new(&self.manager);
        bootstrapper.sync_physical_collections(&mut tx).await?;
        tx.commit().await?;

        Ok(())
    }

    // =========================================================================
    // 🎯 ÉTAPE 3 : Alignement strict sur le CLI (Importation des Ontologies)
    // =========================================================================
    async fn step_3_import_and_register_ontologies(&self) -> RaiseResult<()> {
        let factory_path = match self.config.get_path("PATH_RAISE_ASSET") {
            Some(p) => p,
            None => raise_error!(
                "ERR_BOOT_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET manquant"
            ),
        };
        let ontologies_dir = factory_path.join("ontologies");

        user_info!(
            "BOOT_ONTOLOGIES",
            json_value!({"action": "Importation pure déléguée au moteur json_db"})
        );

        let ontology_files = vec![
            (
                "raise",
                "raise/@context/raise.jsonld",
                "onto-raise-core",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/arcadia.jsonld",
                "onto-arcadia-core",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/data.jsonld",
                "onto-arcadia-data",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/transverse.jsonld",
                "onto-arcadia-transverse",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/oa.jsonld",
                "onto-arcadia-oa",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/sa.jsonld",
                "onto-arcadia-sa",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/la.jsonld",
                "onto-arcadia-la",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/pa.jsonld",
                "onto-arcadia-pa",
                "1.1.0",
            ),
            (
                "arcadia",
                "arcadia/@context/epbs.jsonld",
                "onto-arcadia-epbs",
                "1.1.0",
            ),
        ];

        // =========================================================
        // PHASE 1 : jsondb import (Délégation totale au CollectionsManager)
        // =========================================================
        for (_, rel_path, handle, _) in &ontology_files {
            let file_path = ontologies_dir.join(rel_path);

            if fs::exists_async(&file_path).await {
                let json: JsonValue = fs::read_json_async(&file_path).await?;

                let docs = if let Some(arr) = json.as_array() {
                    arr.to_vec()
                } else {
                    vec![json]
                };

                let count = docs.len();

                for mut doc in docs {
                    self.reanchor_semantic_node(&mut doc);
                    match self.manager.upsert_document("_ontologies", doc).await {
                        Ok(_) => {}
                        Err(e) => {
                            user_warn!(
                                "JSONDB_IMPORT_PARTIAL_FAIL",
                                json_value!({
                                    "error": e.to_string(),
                                    "hint": "L'import de ce document a échoué. Le script continue."
                                })
                            );
                        }
                    }
                }

                user_success!(
                    "JSONDB_IMPORT_SUCCESS",
                    json_value!({ "count": count, "handle": handle })
                );

                // jsondb alter-db
                if *handle == "onto-raise-core" {
                    self.manager
                        .alter_db(
                            "@context",
                            json_value!("ref:_ontologies:handle:onto-raise-core"),
                        )
                        .await?;
                }
            } else {
                user_warn!(
                    "BOOT_ONTO_MISSING",
                    json_value!({"file": file_path.to_string_lossy().to_string()})
                );
            }
        }

        // =========================================================
        // PHASE 2 : jsondb register-ontology
        // =========================================================
        for (namespace, rel_path, handle, version) in &ontology_files {
            let file_path = ontologies_dir.join(rel_path);

            if fs::exists_async(&file_path).await {
                let uri = format!(
                    "db://{}/{}/collections/_ontologies/handle/{}",
                    self.domain, self.db, handle
                );

                // 🎯 ALIGNEMENT STRICT : Appel métier suivi du log exact du CLI
                self.manager
                    .register_ontology(namespace, &uri, version)
                    .await?;

                user_success!(
                    "JSONDB_ONTOLOGY_REGISTERED",
                    json_value!({ "namespace": namespace })
                );
            }
        }

        Ok(())
    }

    async fn step_4_import_locales(&self) -> RaiseResult<()> {
        // 🎯 FIX ZÉRO DETTE : On lit l'usine externe depuis le .env
        let factory_path = match self.config.get_path("PATH_RAISE_ASSET") {
            Some(p) => p,
            None => raise_error!(
                "ERR_BOOT_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET manquant"
            ),
        };
        let locales_dir = factory_path.join("locales");
        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        let locale_schema = format!(
            "db://{}/{}/schemas/{}/system/configs/locale.schema.json",
            self.domain, self.db, schema_version
        );

        match self
            .manager
            .create_collection("locales", &locale_schema)
            .await
        {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_CREATE_LOCALE_COLL", error = e),
        }

        for lang in &["fr", "en", "es", "de", "it"] {
            let file_path = locales_dir.join(format!("{}.json", lang));

            if fs::exists_async(&file_path).await {
                let raw_doc: JsonValue = match fs::read_json_async(&file_path).await {
                    Ok(d) => d,
                    Err(e) => raise_error!("ERR_BOOT_LOCALE_READ_PARSE", error = e),
                };

                let docs_to_insert = Self::extract_operation_documents(raw_doc);

                for mut doc in docs_to_insert {
                    self.reanchor_semantic_node(&mut doc);

                    match self.manager.upsert_document("locales", doc).await {
                        Ok(_) => (),
                        Err(e) => raise_error!("ERR_BOOT_LOCALE_UPSERT", error = e),
                    }
                }
            }
        }

        Ok(())
    }

    async fn step_5_import_seeds(&self) -> RaiseResult<()> {
        // 🎯 FIX ZÉRO DETTE : On lit l'usine externe depuis le .env
        let factory_path = match self.config.get_path("PATH_RAISE_ASSET") {
            Some(p) => p,
            None => raise_error!(
                "ERR_BOOT_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET manquant"
            ),
        };
        let system_seeds_dir = factory_path.join("seeds").join(&self.db);

        if fs::exists_async(&system_seeds_dir).await {
            user_info!(
                "BOOT_SYSTEM_SEEDS",
                json_value!({"action": "Injection de l'ADN de base"})
            );
            self.apply_seeds_from_dir(&system_seeds_dir).await?;
        }

        if let Some(dataset_path) = self.config.get_path("PATH_RAISE_DATASET") {
            let env_seeds_dir = dataset_path.join("_setup").join(&self.db).join("seeds");

            if fs::exists_async(&env_seeds_dir).await {
                user_info!(
                    "BOOT_ENV_SEEDS",
                    json_value!({"action": "Injection du contexte local"})
                );
                self.apply_seeds_from_dir(&env_seeds_dir).await?;
            }
        } else {
            user_warn!(
                "BOOT_NO_DATASET",
                json_value!({"hint": "PATH_RAISE_DATASET non défini."})
            );
        }

        Ok(())
    }

    async fn apply_seeds_from_dir(&self, seeds_dir: &Path) -> RaiseResult<()> {
        let mut entries = match fs::read_dir_async(seeds_dir).await {
            Ok(e) => e,
            Err(err) => raise_error!("ERR_BOOT_SEEDS_READDIR", error = err),
        };

        let mut files = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                files.push(path);
            }
        }
        files.sort();

        for file_path in files {
            let file_name = file_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            user_info!("BOOT_SEEDING", json_value!({"file": file_name}));

            let mut operations: Vec<JsonValue> = match fs::read_json_async(&file_path).await {
                Ok(ops) => ops,
                Err(e) => raise_error!(
                    "ERR_BOOT_SEED_READ_PARSE",
                    error = e,
                    context = json_value!({"file": file_name})
                ),
            };

            for op in operations.iter_mut() {
                if let Some(op_obj) = op.as_object_mut() {
                    let op_type = op_obj
                        .get("type")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let collection = op_obj
                        .get("collection")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();

                    if op_type == "upsert" {
                        if let Some(document) = op_obj.get_mut("document") {
                            self.reanchor_semantic_node(document);

                            match self
                                .manager
                                .upsert_document(&collection, document.clone())
                                .await
                            {
                                Ok(_) => (),
                                Err(e) => raise_error!(
                                    "ERR_BOOT_SEED_UPSERT",
                                    error = e,
                                    context =
                                        json_value!({"collection": collection, "file": file_name})
                                ),
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

// ==============================================================================
// 🧪 TESTS UNITAIRES
// ==============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::core::async_test;
    use crate::utils::testing::mock::DbSandbox;

    #[test]
    fn test_industrial_phase_taxonomy() {
        assert_eq!(IndustrialPhase::System.to_string(), "system");
        assert_eq!(IndustrialPhase::Raise.to_string(), "raise");
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_ensure_partition_idempotency() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let test_domain = "test_boot_domain";
        let phase = IndustrialPhase::Exploration;

        let shared_storage = SharedRef::new(sandbox.storage);
        NodeEnvironment::ensure_partition(&shared_storage, test_domain, &phase).await?;

        let result = NodeEnvironment::ensure_partition(&shared_storage, test_domain, &phase).await;
        assert!(result.is_ok());

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_bootstrapper_early_exit_zero_debt() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let shared_storage = SharedRef::new(sandbox.storage);

        let test_domain = "bootstrap_test_domain".to_string();
        let test_db = "master_test".to_string();

        let manager = CollectionsManager::new(&shared_storage, &test_domain, &test_db);
        let bootstrapper = SystemBootstrapper::new(manager, test_domain.clone(), test_db.clone());

        let db_path = bootstrapper.db_path()?;
        fs::create_dir_all_async(&db_path).await?;

        let sys_json_path = db_path.join("_system.json");
        fs::write_json_atomic_async(&sys_json_path, &json_value!({"db_role": "system"})).await?;

        let result = bootstrapper.execute_if_needed().await;
        assert!(result.is_ok());

        Ok(())
    }
}
