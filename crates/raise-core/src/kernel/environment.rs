// FICHIER : crates/raise-core/src/kernel/environment.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::{JsonDbConfig, StorageEngine};
use crate::utils::core::error::RaiseResult;
use crate::utils::core::{FmtCursor, FmtDisplay, FmtResult, RuntimeEnv, SharedRef};
use crate::utils::data::config::AppConfig;
use crate::utils::data::json::{json_value, JsonValue};
use crate::utils::io::fs::{self, Path, PathBuf};
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
    pub async fn boot_physical_node() -> RaiseResult<Self> {
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

        match bootstrapper.execute_if_needed().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_KERNEL_BOOTSTRAP_EXEC", error = e),
        }

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

        Ok(Self {
            storage,
            local_domain,
        })
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
        let ddl = crate::json_db::schema::ddl::DdlHandler::new(&manager);

        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        // 🎯 L'AIGUILLAGE SÉMANTIQUE : Le Polymorphisme des Partitions
        // On assigne le bon ADN (schéma) en fonction de la vocation de la base
        let schema_filename = match phase {
            IndustrialPhase::Raise => "index_raise.schema.json",
            IndustrialPhase::Modeling => "index_mbse.schema.json",
            IndustrialPhase::Simulation => "index_mbse.schema.json",
            IndustrialPhase::System => "index_bootstrap.schema.json",
            // Par défaut, les autres partitions utilisent l'index générique
            _ => "index_raise.schema.json",
        };

        // L'URI pointera dynamiquement vers le bon fichier
        let schema_uri = format!(
            "db://{}/{}/schemas/{}/system/db/{}",
            domain, db_name, schema_version, schema_filename
        );

        // 🎯 ACTION : On crée la base avec son schéma spécifique, ET on l'inscrit dans la gouvernance de 'master'
        match ddl.create_db_with_schema(&schema_uri).await {
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

    fn get_assets_root(&self) -> RaiseResult<PathBuf> {
        match self.config.get_path("PATH_RAISE_ASSET") {
            Some(p) => Ok(p),
            None => raise_error!(
                "ERR_BOOT_NO_ASSET_PATH",
                error = "PATH_RAISE_ASSET manquant"
            ),
        }
    }

    // 🎯 UTILITAIRE : Extrait de manière robuste le "document" d'une opération Seed (Array ou Object)
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

    // 🎯 MUTATEUR SÉMANTIQUE CHIRURGICAL : Navigue dans l'arbre et corrige uniquement les URIs cibles
    fn reanchor_semantic_node(&self, doc: &mut JsonValue) {
        let local_ontology = format!("db://{}/{}/ontologies/", self.domain, self.db);
        let local_prefix = format!("db://{}/{}/", self.domain, self.db);

        match doc {
            JsonValue::Object(obj) => {
                let keys_to_check = ["@context", "@id", "$schema", "$ref"];

                for key in keys_to_check {
                    if let Some(val) = obj.get_mut(key) {
                        if let Some(s) = val.as_str() {
                            if key == "@context" {
                                let new_s = s
                                    .replace("db://_system/ontology/", &local_ontology)
                                    .replace("db://_system/ai-assets/ontologies/", &local_ontology)
                                    .replace("db://_system/master/ontologies/", &local_ontology);
                                *val = json_value!(new_s);
                            } else {
                                if s.starts_with("db://_system/")
                                    || s.starts_with("db://ai-assets/")
                                {
                                    let new_s = s
                                        .replace("db://_system/master/", &local_prefix)
                                        .replace("db://_system/bootstrap/", &local_prefix)
                                        .replace("db://_system/_system/", &local_prefix)
                                        .replace("db://_system/raise/", &local_prefix)
                                        .replace("db://_system/ai-assets/", &local_prefix);
                                    *val = json_value!(new_s);
                                }
                            }
                        }
                    }
                }

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

    pub async fn execute_if_needed(&self) -> RaiseResult<()> {
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

            // 🧠 RÉVEIL DU CERVEAU SÉMANTIQUE (Avec l'emprunt &)
            if let Err(e) =
                crate::json_db::jsonld::VocabularyRegistry::init_from_db(&self.manager).await
            {
                user_warn!("WRN_VOCABULARY_INIT", json_value!({"error": e.to_string()}));
            }

            return Ok(());
        }

        // =========================================================
        // 🚀 CAS 2 : PREMIER AMORÇAGE (Démarrage à Froid)
        // =========================================================
        user_info!(
            "BOOT_SYSTEM_START",
            json_value!({"action": "provisioning_db", "db": self.db})
        );

        match self.step_1_import_schemas().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_1", error = e),
        }
        match self.step_2_create_db().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_2", error = e),
        }

        // 🧬 Les Ontologies sont écrites sur le disque
        match self.step_3_import_and_register_ontologies().await {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_STEP_3", error = e),
        }

        // 🧠 RÉVEIL DU CERVEAU SÉMANTIQUE : On charge l'ADN en RAM (Avec l'emprunt &)
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
        Ok(())
    }

    async fn step_1_import_schemas(&self) -> RaiseResult<()> {
        let assets_root = self.get_assets_root()?;
        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        let source_path = assets_root.join("schemas").join(&schema_version);
        let target_path = self.db_path()?.join("schemas").join(&schema_version);

        if !fs::exists_async(&source_path).await {
            raise_error!(
                "ERR_BOOT_SCHEMA_MISSING",
                error = "Schémas d'amorçage introuvables.",
                context = json_value!({"path": source_path.to_string_lossy()})
            );
        }

        user_info!(
            "BOOT_COPY_SCHEMAS",
            json_value!({"source": source_path.to_string_lossy(), "target": target_path.to_string_lossy()})
        );

        let mut schemas_index = json_value!({});
        let mut queue = vec![(source_path.clone(), target_path.clone())];

        while let Some((curr_src, curr_tgt)) = queue.pop() {
            if !fs::exists_async(&curr_tgt).await {
                if let Err(e) = fs::create_dir_all_async(&curr_tgt).await {
                    raise_error!("ERR_BOOT_MKDIR", error = e);
                }
            }

            let mut entries = match fs::read_dir_async(&curr_src).await {
                Ok(e) => e,
                Err(e) => raise_error!("ERR_BOOT_READDIR", error = e),
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();
                let target_file = curr_tgt.join(entry.file_name());

                if path.is_dir() {
                    queue.push((path, target_file.clone()));
                } else if path.extension().and_then(|s| s.to_str()) == Some("json") {
                    let content = match fs::read_to_string_async(&path).await {
                        Ok(c) => c,
                        Err(e) => raise_error!(
                            "ERR_BOOT_READ_FILE",
                            error = e,
                            context = json_value!({"file": path.to_string_lossy()})
                        ),
                    };

                    let mut schema_json =
                        match crate::utils::data::json::deserialize_from_str::<JsonValue>(&content)
                        {
                            Ok(j) => j,
                            Err(e) => raise_error!(
                                "ERR_BOOT_PARSE_JSON",
                                error = format!("Erreur de parsing JSON : {}", e),
                                context = json_value!({"file": path.to_string_lossy()})
                            ),
                        };

                    // Application chirurgicale du Mutateur
                    self.reanchor_semantic_node(&mut schema_json);

                    if let Some(obj) = schema_json.as_object_mut() {
                        if let Ok(rel_path) = path.strip_prefix(&assets_root) {
                            let rel_str = rel_path.to_string_lossy().replace('\\', "/");
                            let expected_uri =
                                format!("db://{}/{}/{}", self.domain, self.db, rel_str);
                            obj.insert("$id".to_string(), json_value!(expected_uri));
                        }
                    }

                    if let Err(e) = fs::write_json_atomic_async(&target_file, &schema_json).await {
                        raise_error!(
                            "ERR_BOOT_WRITE_JSON",
                            error = e,
                            context = json_value!({"file": target_file.to_string_lossy()})
                        );
                    }

                    if let Ok(rel_path) = target_file.strip_prefix(&target_path) {
                        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
                        if let Some(obj) = schemas_index.as_object_mut() {
                            obj.insert(
                                rel_str.clone(),
                                json_value!({ "file": format!("{}/{}", schema_version, rel_str) }),
                            );
                        }
                    }
                } else {
                    if let Err(e) = fs::copy_async(&path, &target_file).await {
                        raise_error!(
                            "ERR_BOOT_COPY_FILE",
                            error = e,
                            context = json_value!({"file": target_file.to_string_lossy()})
                        );
                    }
                }
            }
        }

        let sys_path = self.db_path()?.join("_system.json");
        let seed_index = json_value!({
            "collections": {},
            "ontologies": {},
            "rules": {},
            "schemas": {
                schema_version: schemas_index
            }
        });

        if let Err(e) = fs::write_json_atomic_async(&sys_path, &seed_index).await {
            raise_error!("ERR_BOOT_WRITE_SEED", error = e);
        }

        Ok(())
    }

    async fn step_2_create_db(&self) -> RaiseResult<()> {
        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());

        let schema_uri = format!(
            "db://{}/{}/schemas/{}/system/db/index_bootstrap.schema.json",
            self.domain, self.db, schema_version
        );

        match self.manager.init_db_with_schema(&schema_uri).await {
            Ok(_) => (),
            Err(e) => {
                let err_str = e.to_string();
                if !err_str.contains("already initialized") {
                    raise_error!("ERR_BOOT_INIT_DB", error = e);
                }
            }
        }

        if let Err(e) = self
            .manager
            .alter_db("db_role", json_value!("system"))
            .await
        {
            raise_error!("ERR_BOOT_ALTER_DB_ROLE", error = e);
        }

        Ok(())
    }

    async fn step_3_import_and_register_ontologies(&self) -> RaiseResult<()> {
        let schema_version =
            RuntimeEnv::var("PATH_RAISE_SCHEMA_VERSION").unwrap_or_else(|_| "v2".to_string());
        let assets_root = self.get_assets_root()?;

        let onto_schema = format!(
            "db://{}/{}/schemas/{}/system/db/ontology.schema.json",
            self.domain, self.db, schema_version
        );

        match self
            .manager
            .create_collection("_ontologies", &onto_schema)
            .await
        {
            Ok(_) => (),
            Err(e) => raise_error!("ERR_BOOT_CREATE_ONTO_COLL", error = e),
        }

        let assets_base = assets_root.join("ontologies");

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

        for (namespace, rel_path, handle, version) in ontology_files {
            let file_path = assets_base.join(rel_path);

            if fs::exists_async(&file_path).await {
                let raw_doc: JsonValue = match fs::read_json_async(&file_path).await {
                    Ok(d) => d,
                    Err(e) => raise_error!(
                        "ERR_BOOT_ONTO_READ_PARSE",
                        error = e,
                        context = json_value!({"file": file_path.to_string_lossy().to_string()})
                    ),
                };

                let docs_to_insert = Self::extract_operation_documents(raw_doc);

                for mut doc in docs_to_insert {
                    self.reanchor_semantic_node(&mut doc);

                    match self.manager.upsert_document("_ontologies", doc).await {
                        Ok(_) => (),
                        Err(e) => raise_error!("ERR_BOOT_ONTO_UPSERT", error = e),
                    }

                    let uri = format!(
                        "db://{}/{}/collections/_ontologies/handle/{}",
                        self.domain, self.db, handle
                    );

                    match self
                        .manager
                        .register_ontology(namespace, &uri, version)
                        .await
                    {
                        Ok(_) => (),
                        Err(e) => raise_error!("ERR_BOOT_ONTO_REGISTER", error = e),
                    }
                }
            } else {
                user_warn!(
                    "BOOT_ONTO_MISSING",
                    json_value!({"file": file_path.to_string_lossy().to_string()})
                );
            }
        }

        if let Err(e) = self
            .manager
            .alter_db(
                "@context",
                json_value!("ref:_ontologies:handle:onto-raise-core"),
            )
            .await
        {
            raise_error!("ERR_BOOT_ALTER_DB_CONTEXT", error = e);
        }

        Ok(())
    }

    async fn step_4_import_locales(&self) -> RaiseResult<()> {
        let assets_root = self.get_assets_root()?;
        let locales_dir = assets_root.join("locales");
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
        let assets_root = self.get_assets_root()?;
        let system_seeds_dir = assets_root.join("seeds").join(&self.db);

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
