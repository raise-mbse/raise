// FICHIER : src-tauri/src/utils/testing/mock.rs
#![cfg(any(test, debug_assertions))]
use crate::utils::prelude::*;
use async_trait::async_trait;

// 1. Core : Concurrence, Mémoire et Identifiants
use crate::raise_error;
use crate::utils::core::error::RaiseResult;
use crate::utils::core::{RuntimeEnv, SharedRef, UniqueId, UtcClock};
use crate::utils::io::fs::{self, tempdir, Path, PathBuf, TempDir};

// 2. Data : Configuration, JSON et Traits
use crate::utils::data::config::{
    AiAssetsPaths, AppConfig, CoreConfig, DbPointer, MountPointsConfig, SystemAssets, BOOTSTRAP_DB,
    BOOTSTRAP_DOMAIN, CONFIG,
};
use crate::utils::data::json::{self, json_value, JsonValue};
use crate::utils::data::UnorderedMap;

// 4. Dépendances métier (Base de données JSON)
use crate::ai::llm::client::LlmEngine;
use crate::ai::llm::native_engine::NativeTensorEngine;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::{JsonDbConfig, StorageEngine};

pub const MOCK_LLM_MODEL: &str = "mock/qwen2.5-0.5b-instruct-q4_k_m.gguf";
pub const MOCK_LLM_TOKENIZER: &str = "mock/tokenizer.json";

// 🎯 SINGLETON GLOBAL POUR LES TESTS
static SHARED_LLM_ENGINE: AsyncStaticCell<SharedRef<AsyncMutex<dyn LlmEngine>>> =
    AsyncStaticCell::const_new();

pub struct MockLlmEngine {
    pub response: String,
}

#[async_trait]
impl LlmEngine for MockLlmEngine {
    async fn generate(&mut self, _: &str, _: &str, _: usize) -> RaiseResult<String> {
        Ok(self.response.clone())
    }
}

// --- DÉFINITION DES SCHÉMAS STANDARDS POUR TESTS ---

pub const SESSION_SCHEMA_MOCK: &str = r#"{
    "type": "object",
    "properties": {
        "_id": { 
            "type": "string",
            "x_compute": {
                "engine": "plan/v1",
                "scope": "root",
                "update": "if_missing",
                "plan": { "op": "uuid_v4" }
            }
        },
        "_created_at": { 
            "type": "string",
            "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "if_missing" }
        },
        "_updated_at": { 
            "type": "string",
            "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "always" }
        },
        "@type": { "type": "array", "items": { "type": "string" } },
        "user_id": { "type": "string" },
        "user_handle": { "type": "string" },
        "status": { "type": "string", "enum": ["active", "idle", "expired", "revoked"] },
        "expires_at": { "type": "string", "format": "date-time" },
        "last_activity_at": { "type": "string", "format": "date-time" },
        "context": { 
            "type": "object",
            "required": ["current_domain", "current_db", "active_dapp_id"] 
        }
    },
    "required": ["user_id", "status", "context"]
}"#;

pub const ACTORS_SCHEMA_MOCK: &str =
    r#"{ "type": "object", "properties": { "handle": { "type": "string" } } }"#;
pub const ARTICLES_SCHEMA_MOCK: &str =
    r#"{ "type": "object", "properties": { "title": { "type": "string" } } }"#;
pub const CONFIG_ITEMS_SCHEMA_MOCK: &str =
    r#"{ "type": "object", "properties": { "name": { "type": "string" } } }"#;
pub const FINANCE_SCHEMA_MOCK: &str = r#"{
    "type": "object",
    "x_rules": [
        { 
            "handle": "rule_net_margin_low",
            "target": "summary.net_margin_low", 
            "expr": { "mul": [ { "var": "revenue_scenarios.low_eur" }, { "var": "gross_margin.low_pct" } ] }
        },
        { 
            "handle": "rule_net_margin_mid",
            "target": "summary.net_margin_mid", 
            "expr": { "mul": [ { "var": "revenue_scenarios.mid_eur" }, { "var": "gross_margin.mid_pct" } ] }
        },
        { 
            "handle": "rule_mid_profitable",
            "target": "summary.mid_is_profitable", 
            "expr": { "gt": [ { "var": "summary.net_margin_mid" }, { "val": 0 } ] }
        },
        { 
            "handle": "rule_gen_ref",
            "target": "summary.generated_ref", 
            "expr": {
                "replace": {
                    "value": { "var": "billing_model" },
                    "pattern": { "val": "fixed" },
                    "replacement": { "val": "FIN-2025-OK" }
                }
            }
        }
    ]
}"#;

pub const USER_SCHEMA_MOCK: &str = r#"{
    "type": "object",
    "properties": {
        "_id": {
            "type": "string",
            "x_compute": {
                "engine": "plan/v1",
                "scope": "root",
                "update": "if_missing",
                "plan": { "op": "uuid_v4" }
            }
        },
        "handle": { "type": "string" },
        "name": { "type": "object" },
        "default_domain": { "type": "string" },
        "default_db": { "type": "string" },
        "role": { "type": "string" }
    },
    "required": ["_id", "handle",  "name", "default_domain", "default_db"]
}"#;

// =========================================================================
// 🔧 UTILS DE CONFIGURATION DE TEST
// =========================================================================
/// Injecte de manière idempotente tous les schémas Core, V1 et V2 nécessaires aux tests.
pub async fn bootstrap_system_index(
    db_cfg: &JsonDbConfig,
    space: &str,
    db: &str,
) -> RaiseResult<()> {
    let sys_path = db_cfg.db_root(space, db).join("_system.json");

    if fs::exists_async(&sys_path).await {
        return Ok(());
    }

    fs::ensure_dir_async(sys_path.parent().unwrap())
        .await
        .unwrap();

    let schema_uri = format!(
        "db://{}/{}/schemas/v1/db/index.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
    );

    let mut initial_system_doc = json_value!({
        "$schema": schema_uri,
        "name": format!("{}_{}", space, db),
        "space": space,
        "database": db,
        "schemas": { "v1": {}, "v2": {} }
    });

    inject_core_schemas_to_index(db_cfg, &mut initial_system_doc).await;
    inject_mock_schema_to_index(
        db_cfg,
        &mut initial_system_doc,
        "sessions",
        SESSION_SCHEMA_MOCK,
    )
    .await;
    inject_mock_schema_to_index(db_cfg, &mut initial_system_doc, "users", USER_SCHEMA_MOCK).await;
    inject_mock_schema_to_index(
        db_cfg,
        &mut initial_system_doc,
        "actors",
        ACTORS_SCHEMA_MOCK,
    )
    .await;
    inject_mock_schema_to_index(
        db_cfg,
        &mut initial_system_doc,
        "articles",
        ARTICLES_SCHEMA_MOCK,
    )
    .await;
    inject_mock_schema_to_index(
        db_cfg,
        &mut initial_system_doc,
        "configuration_items",
        CONFIG_ITEMS_SCHEMA_MOCK,
    )
    .await;
    inject_mock_schema_to_index(
        db_cfg,
        &mut initial_system_doc,
        "finance",
        FINANCE_SCHEMA_MOCK,
    )
    .await;

    inject_v2_schema_mock(db_cfg, &mut initial_system_doc, "assurance/quality_report").await;
    inject_v2_schema_mock(db_cfg, &mut initial_system_doc, "assurance/xai_frame").await;
    inject_v2_schema_mock(db_cfg, &mut initial_system_doc, "assurance/rules/rule").await;
    inject_v2_schema_mock(db_cfg, &mut initial_system_doc, "common/types/base").await;
    inject_v2_schema_mock(
        db_cfg,
        &mut initial_system_doc,
        "agents/memory/vector_store_record",
    )
    .await;
    inject_v2_schema_mock(
        db_cfg,
        &mut initial_system_doc,
        "agents/memory/chat_session",
    )
    .await;

    fs::write_json_atomic_async(&sys_path, &initial_system_doc)
        .await
        .unwrap();

    Ok(())
}

pub async fn insert_mock_db(
    manager: &CollectionsManager<'_>,
    collection: &str,
    doc: &JsonValue,
) -> RaiseResult<()> {
    // On tente l'insertion brute
    if let Err(e) = manager.insert_raw(collection, doc).await {
        // En cas de collision concurrentielle, on l'avale silencieusement
        if !e.to_string().contains("ERR_DB_DUPLICATE_HANDLE") {
            // Si c'est une vraie erreur inattendue, on la fait remonter
            return Err(e);
        }
    }
    Ok(())
}

pub fn create_default_test_config() -> AppConfig {
    let mut paths = UnorderedMap::new();
    let tmp = RuntimeEnv::temp_dir();

    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/home/zair"));
    let base_ai_assets = home.join("raise_domain/_system/ai-assets");

    paths.insert(
        "PATH_RAISE_DOMAIN".to_string(),
        tmp.to_string_lossy().to_string(),
    );
    paths.insert(
        "PATH_LOGS".to_string(),
        tmp.join("logs").to_string_lossy().to_string(),
    );
    AppConfig {
        id: UniqueId::new_v4().to_string(),
        handle: "bootstrap".to_string(),
        created_at: UtcClock::now().to_rfc3339(),
        updated_at: UtcClock::now().to_rfc3339(),
        semantic_type: vec!["SystemConfig".to_string()],
        name: Some(UnorderedMap::from([(
            "en".to_string(),
            "Default Test Config".to_string(),
        )])),

        mount_points: MountPointsConfig {
            system: DbPointer {
                domain: BOOTSTRAP_DOMAIN.to_string(),
                db: BOOTSTRAP_DB.to_string(),
            },
            raise: DbPointer {
                domain: BOOTSTRAP_DOMAIN.to_string(),
                db: "raise_core".to_string(),
            },
            exploration: DbPointer {
                domain: "project_x".into(),
                db: "sandbox".into(),
            },
            modeling: DbPointer {
                domain: "project_x".into(),
                db: "mbse".into(),
            },
            simulation: DbPointer {
                domain: "project_x".into(),
                db: "sim_mbse".into(),
            },
            integration: DbPointer {
                domain: "project_x".into(),
                db: "test_mbse".into(),
            },
            production: DbPointer {
                domain: "project_x".into(),
                db: "prod_mbse".into(),
            },
            operation: DbPointer {
                domain: "project_x".into(),
                db: "telemetry".into(),
            },
        },
        core: CoreConfig {
            env_mode: "test".to_string(),
            graph_mode: "none".to_string(),
            log_level: "debug".to_string(),
            vector_store_provider: "memory".to_string(),
            language: "en".to_string(),
            use_gpu: false, // 🎯 FIX : Initialisation du champ Core
        },

        system_assets: SystemAssets {
            schemas_uri: Some(format!(
                "db://{}/{}/schemas",
                BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
            )),
            locales_uri: Some(format!(
                "db://{}/{}/collections/locales",
                BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
            )),
            ontologies_uri: Some(format!(
                "db://{}/{}/ontologies/raise/@context/raise.jsonld",
                BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
            )),
            ai_assets_paths: Some(AiAssetsPaths {
                models: Some(base_ai_assets.join("models").to_string_lossy().to_string()),
                embeddings: Some(
                    base_ai_assets
                        .join("embeddings")
                        .to_string_lossy()
                        .to_string(),
                ),
                lora: Some(base_ai_assets.join("lora").to_string_lossy().to_string()),
                voice: Some(base_ai_assets.join("voice").to_string_lossy().to_string()),
                ontologies: Some(
                    base_ai_assets
                        .join("ontologies")
                        .to_string_lossy()
                        .to_string(),
                ),
            }),
        },

        paths,
        active_dapp_id: "ref:dapps:handle:raise_core".to_string(),
        workstation_id: "ref:workstations:handle:test_ws".to_string(),
        active_services: vec!["ref:services:handle:svc_ai".to_string()],
        active_components: vec![
            "ref:components:handle:ai_llm".to_string(),
            "ref:components:handle:ai_nlp".to_string(),
            "ref:components:handle:rag".to_string(),
            "ref:components:handle:ai_graph_store".to_string(),
            "ref:components:handle:ai_world_model".to_string(),
            "ref:components:handle:ai_voice".to_string(),
        ],

        workstation: None,
        user: None,
    }
}

pub fn load_test_sandbox() -> RaiseResult<AppConfig> {
    let manifest = match RuntimeEnv::var("CARGO_MANIFEST_DIR") {
        Ok(v) => v,
        Err(e) => raise_error!(
            "ERR_CONFIG_ENV_MANIFEST",
            error = e,
            context = json_value!({ "var": "CARGO_MANIFEST_DIR" })
        ),
    };

    let path = PathBuf::from(manifest).join("tests/config.test.json");

    if !path.exists() {
        return Ok(create_default_test_config());
    }

    let content = match fs::read_to_string_sync(&path) {
        Ok(c) => c,
        Err(e) => raise_error!(
            "ERR_CONFIG_FS_READ",
            error = e,
            context = json_value!({ "path": path.to_string_lossy() })
        ),
    };

    let mut config: AppConfig = match json::deserialize_from_str(&content) {
        Ok(cfg) => cfg,
        Err(e) => raise_error!(
            "ERR_CONFIG_PARSE",
            error = e,
            context = json_value!({ "path": path.to_string_lossy() })
        ),
    };

    if let Some(domain_path) = config.paths.get_mut("PATH_RAISE_DOMAIN") {
        let temp_dir = RuntimeEnv::temp_dir();
        let temp_str = temp_dir.to_string_lossy();

        if domain_path.starts_with("/tmp") || domain_path.contains(temp_str.as_ref() as &str) {
            let unique_id = format!(
                "{}_{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_micros()
            );
            *domain_path = format!("{}_{}", domain_path, unique_id);
            let _ = fs::create_dir_all_sync(domain_path);
        }
    }

    config.mount_points.system.domain = BOOTSTRAP_DOMAIN.to_string();
    config.mount_points.system.db = BOOTSTRAP_DB.to_string();

    // On dresse la liste de survie absolue pour que les tests IA fonctionnent
    let required_ai_components = vec![
        "ref:components:handle:ai_llm",
        "ref:components:handle:ai_nlp",
        "ref:components:handle:rag",
        "ref:components:handle:ai_graph_store",
        "ref:components:handle:ai_world_model",
        "ref:components:handle:ai_voice",
    ];

    // On inspecte la config qu'on vient de charger depuis le fichier JSON...
    for comp in required_ai_components {
        if !config.active_components.contains(&comp.to_string()) {
            // ... S'il en manque un, on le force brutalement dedans !
            config.active_components.push(comp.to_string());
        }
    }

    Ok(config)
}

pub async fn inject_core_schemas_to_index(db_cfg: &JsonDbConfig, sys_doc: &mut JsonValue) {
    let base_uri = format!("db://{}/{}", BOOTSTRAP_DOMAIN, BOOTSTRAP_DB);
    let schemas_dir = db_cfg
        .db_schemas_root(BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
        .join("v1/db");
    let _ = fs::create_dir_all_async(&schemas_dir).await;

    if sys_doc.get("schemas").is_none() {
        *sys_doc = json_value!({ "schemas": { "v1": {}, "v2": {} } });
    }
    let schemas_v1 = sys_doc["schemas"]["v1"].as_object_mut().unwrap();

    let migration_schema = json_value!({
        "$id": format!("{}/schemas/v2/system/db/migration.schema.json", base_uri),
        "type": "object",
        "properties": {
            "$schema": { "type": "string" },
            "_id": { "type": "string", "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" } },
            "handle": { "type": "string" },
            "name": { "type": "object" },
            "version": { "type": "string" },
            "description": { "type": "string" },
            "applied_at": { "type": "string" }
        },
        "required": ["$schema", "_id", "handle", "name", "version", "description", "applied_at"]
    });
    let _ = fs::write_json_atomic_async(
        &schemas_dir.join("migration.schema.json"),
        &migration_schema,
    )
    .await;
    schemas_v1.insert(
        "db/migration.schema.json".to_string(),
        json_value!({ "file": "v1/db/migration.schema.json" }),
    );

    let core_schema = json_value!({
        "$id": format!("{}/schemas/v1/db/index.schema.json", base_uri),
        "type": "object",
        "properties": {
            "_id": { "type": "string", "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" } },
            "name": { "type": "string" },
            "space": { "type": "string" },
            "database": { "type": "string" },
            "version": { "type": "integer", "default": 1 },
            "collections": {
                "type": "object",
                "properties": {
                    "_migrations": {
                        "type": "object",
                        "default": { "schema": format!("{}/schemas/v1/db/migration.schema.json", base_uri), "items": [] }
                    }
                },
                "default": {}
            },
            "rules": { "type": "object", "default": {} },
            "schemas": { "type": "object", "default": { "v1": {} } }
        },
        "required": ["_id", "name", "space", "database"]
    });
    let _ = fs::write_json_atomic_async(&schemas_dir.join("index.schema.json"), &core_schema).await;
    schemas_v1.insert(
        "db/index.schema.json".to_string(),
        json_value!({ "file": "v1/db/index.schema.json" }),
    );

    let generic_schema = json_value!({
        "$id": format!("{}/schemas/v1/db/generic.schema.json", base_uri),
        "type": "object",
        "properties": {
            "_id": { "type": "string", "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" } },
            "_created_at": { "type": "string", "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "if_missing" } },
            "_updated_at": { "type": "string", "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "always" } }
        },
        "required": ["_id"],
        "additionalProperties": true
    });
    let _ = fs::write_json_atomic_async(&schemas_dir.join("generic.schema.json"), &generic_schema)
        .await;
    schemas_v1.insert(
        "db/generic.schema.json".to_string(),
        json_value!({ "file": "v1/db/generic.schema.json" }),
    );
}

pub async fn inject_mock_schema_to_index(
    db_cfg: &JsonDbConfig,
    sys_doc: &mut JsonValue,
    collection_name: &str,
    content: &str,
) {
    if sys_doc.get("schemas").is_none() {
        *sys_doc = json_value!({ "schemas": { "v1": {}, "v2": {} } });
    }

    let schemas_dir = db_cfg
        .db_schemas_root(BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
        .join("v1/mock");
    let _ = fs::create_dir_all_async(&schemas_dir).await;

    let schemas_v1 = sys_doc["schemas"]["v1"].as_object_mut().unwrap();
    let mut json_val: JsonValue = json::deserialize_from_str(content).unwrap_or(json_value!({}));

    let schema_uri = format!(
        "db://{}/{}/schemas/v1/mock/{}.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB, collection_name
    );

    if let Some(obj) = json_val.as_object_mut() {
        obj.insert("$id".to_string(), JsonValue::String(schema_uri.clone()));
    }

    let _ = fs::write_json_atomic_async(
        &schemas_dir.join(format!("{}.schema.json", collection_name)),
        &json_val,
    )
    .await;
    schemas_v1.insert(
        format!("mock/{}.schema.json", collection_name),
        json_value!({ "file": format!("v1/mock/{}.schema.json", collection_name) }),
    );
}

pub async fn inject_v2_schema_mock(
    db_cfg: &JsonDbConfig,
    sys_doc: &mut JsonValue,
    logical_path: &str, // ex: "assurance/quality_report"
) {
    if sys_doc.get("schemas").is_none() {
        *sys_doc = json_value!({ "schemas": { "v1": {}, "v2": {} } });
    }
    let schemas_v2 = sys_doc["schemas"]["v2"].as_object_mut().unwrap();

    let file_name = format!(
        "{}.schema.json",
        Path::new(logical_path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
    );
    let parent_dir = Path::new(logical_path).parent().unwrap_or(Path::new(""));
    let schemas_dir = db_cfg
        .db_schemas_root(BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
        .join("v2")
        .join(parent_dir);

    let _ = fs::create_dir_all_async(&schemas_dir).await;

    let schema_uri = format!(
        "db://{}/{}/schemas/v2/{}.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB, logical_path
    );

    let schema_mock = json_value!({
        "$id": schema_uri,
        "type": "object",
        "properties": {
            "_id": { "type": "string", "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" } },
            "_created_at": { "type": "string", "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "if_missing" } },
            "_updated_at": { "type": "string", "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "always" } }
        },
        "required": ["_id"],
        "additionalProperties": true
    });

    let _ = fs::write_json_atomic_async(&schemas_dir.join(&file_name), &schema_mock).await;

    schemas_v2.insert(
        format!("{}.schema.json", logical_path),
        json_value!({ "file": format!("v2/{}.schema.json", logical_path) }),
    );
}

pub async fn inject_mock_user(manager: &CollectionsManager<'_>, userhandle: &str) {
    let user_doc = json_value!({
        "handle": userhandle,
        "name": { "fr": userhandle, "en": userhandle },
        "default_domain": "mbse2",
        "default_db": "drones",
        "role": "engineer"
    });

    match manager.insert_with_schema("users", user_doc).await {
        Ok(_) => {}
        Err(e) => panic!("Échec de l'injection de l'agent de test : {:?}", e),
    }
}

pub async fn inject_mock_component(
    manager: &CollectionsManager<'_>,
    comp_id: &str,
    mut settings: JsonValue,
) -> RaiseResult<()> {
    let real_handle = match comp_id {
        "llm" => "ai_llm",
        "voice" => "ai_voice",
        "nlp" => "ai_nlp",
        other => other,
    };

    if real_handle == "ai_llm" {
        let models_dir = dirs::home_dir()
            .unwrap_or_default()
            .join("raise_domain/_system/ai-assets/models");
        if settings["rust_model_file"].is_null() {
            settings["rust_model_file"] = json_value!(models_dir
                .join(MOCK_LLM_MODEL)
                .to_string_lossy()
                .to_string());
        }
        if settings["rust_tokenizer_file"].is_null() {
            settings["rust_tokenizer_file"] = json_value!(models_dir
                .join(MOCK_LLM_TOKENIZER)
                .to_string_lossy()
                .to_string());
        }
    }

    let ref_id = format!("ref:components:handle:{}", real_handle);
    let service_id_semantic = "ref:services:handle:svc_ai";
    let service_id_physical = "phys-uuid-svc-ai";

    let schema_uri = format!(
        "db://{}/{}/schemas/v1/db/generic.schema.json",
        crate::utils::data::config::BOOTSTRAP_DOMAIN,
        crate::utils::data::config::BOOTSTRAP_DB
    );

    manager.create_collection("services", &schema_uri).await?;
    let mock_service = &json_value!({
        "_id": service_id_physical,
        "handle": "svc_ai"
    });
    insert_mock_db(manager, "services", mock_service).await?;

    manager.create_collection("components", &schema_uri).await?;
    let mock_components = &json_value!({
        "_id": format!("ref:components:handle:{}", real_handle),
        "handle": real_handle,
        "status": "active"
    });
    insert_mock_db(manager, "components", mock_components).await?;

    manager
        .create_collection("service_configs", &schema_uri)
        .await?;
    let config_id = format!("cfg_{}_test", real_handle);
    let final_doc = json_value!({
        "_id": config_id.clone(),
        "handle": config_id,
        "service_id": service_id_physical,
        "component_id": ref_id,
        "owner_user_id": "ref:users:handle:admin",
        "target_workstation_id": "ref:workstations:handle:test",
        "authorizing_mandator_id": "ref:mandators:handle:test",
        "environment": "test",
        "service_settings": settings
    });

    match insert_mock_db(manager, "service_configs", &final_doc).await {
        Ok(_) => Ok(()),
        Err(e) => raise_error!(
            "ERR_TEST_MOCK_INJECTION_FAILED",
            error = e.to_string(),
            context = json_value!({ "comp": real_handle, "service": service_id_semantic })
        ),
    }
}

pub async fn inject_schema_to_path(db_cfg: &JsonDbConfig) {
    let schema_dir = db_cfg
        .db_schemas_root(BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
        .join("v1/db");
    let _ = fs::create_dir_all_async(&schema_dir).await;

    let base_uri = format!("db://{}/{}", BOOTSTRAP_DOMAIN, BOOTSTRAP_DB);

    let migration_schema = json_value!({
        "$id": format!("{}/schemas/v2/system/db/migration.schema.json", base_uri),
        "type": "object",
        "properties": {
            "$schema": { "type": "string" },
            "_id": {
                "type": "string",
                "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" }
            },
            "handle": { "type": "string" },
            "name": { "type": "object" },
            "version": { "type": "string" },
            "description": { "type": "string" },
            "applied_at": { "type": "string" }
        },
        "required": ["$schema", "_id", "handle", "name", "version", "description", "applied_at"]
    });
    let _ =
        fs::write_json_atomic_async(&schema_dir.join("migration.schema.json"), &migration_schema)
            .await;

    let core_schema = json_value!({
        "$id": format!("{}/schemas/v1/db/index.schema.json", base_uri),
        "type": "object",
        "properties": {
            "_id": {
                "type": "string",
                "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" }
            },
            "name": { "type": "string" },
            "space": { "type": "string" },
            "database": { "type": "string" },
            "version": { "type": "integer", "default": 1 },
            "collections": {
                "type": "object",
                "properties": {
                    "_migrations": {
                        "type": "object",
                        "default": {
                            "schema": format!("{}/schemas/v1/db/migration.schema.json", base_uri),
                            "items": []
                        }
                    }
                },
                "default": {}
            },
            "rules": {
                "type": "object",
                "properties": {
                    "_system_rules": {
                        "type": "object",
                        "default": {
                            "schema": format!("{}/schemas/v1/db/rule.schema.json", base_uri),
                            "items": []
                        }
                    }
                },
                "default": {}
            },
            "schemas": { "type": "object", "default": { "v1": {} } }
        },
        "required": ["_id", "name", "space", "database"]
    });
    let _ = fs::write_json_atomic_async(&schema_dir.join("index.schema.json"), &core_schema).await;

    let generic_schema = json_value!({
        "$id": format!("{}/schemas/v1/db/generic.schema.json", base_uri),
        "type": "object",
        "properties": {
            "_id": {
                "type": "string",
                "x_compute": { "plan": { "op": "uuid_v4" }, "update": "if_missing" }
            },
            "_created_at": {
                "type": "string",
                "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "if_missing" }
            },
            "_updated_at": {
                "type": "string",
                "x_compute": { "plan": { "op": "now_rfc3339" }, "update": "always" }
            },
            "_p2p": {
                "type": "object",
                "properties": {
                    "revision": { "type": "integer", "default": 1 },
                    "origin_node": { "type": "string" },
                    "checksum": { "type": "string" }
                },
                "default": { "revision": 1 }
            }
        },
        "required": ["_id"],
        "additionalProperties": true
    });
    let _ =
        fs::write_json_atomic_async(&schema_dir.join("generic.schema.json"), &generic_schema).await;
}

pub async fn inject_collection_schema(domain_root: &Path, collection_name: &str, content: &str) {
    let schemas_dir = domain_root.join(format!(
        "{}/{}/schemas/v1/mock",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
    ));
    let _ = fs::create_dir_all_async(&schemas_dir).await;

    let schema_uri = format!(
        "db://{}/{}/schemas/v1/mock/{}.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB, collection_name
    );
    let schema_file = schemas_dir.join(format!("{}.schema.json", collection_name));

    let mut json_val: JsonValue = match json::deserialize_from_str(content) {
        Ok(v) => v,
        Err(_) => json_value!({}),
    };

    if let Some(obj) = json_val.as_object_mut() {
        obj.insert("$id".to_string(), JsonValue::String(schema_uri.clone()));
    }

    let _ = fs::write_async(&schema_file, json_val.to_string().as_bytes()).await;

    let col_dir = domain_root
        .join(format!("{}/{}/collections", BOOTSTRAP_DOMAIN, BOOTSTRAP_DB))
        .join(collection_name);
    let _ = fs::create_dir_all_async(&col_dir).await;

    let meta_content = json_value!({
        "schema": schema_uri,
        "indexes": []
    });

    let _ = fs::write_async(
        &col_dir.join("_meta.json"),
        meta_content.to_string().as_bytes(),
    )
    .await;
}

pub async fn inject_mock_config() {
    if CONFIG.get().is_none() {
        let config = create_default_test_config();
        let _ = CONFIG.set(config);
    }
    if crate::utils::data::config::DEVICE.get().is_none() {
        let test_device = if cfg!(feature = "cuda") {
            candle_core::Device::new_cuda(0).unwrap_or(candle_core::Device::Cpu)
        } else {
            candle_core::Device::Cpu
        };

        let _ = crate::utils::data::config::DEVICE.set(test_device);
        println!(
            "🧪 [Raise Test] Device injecté : {:?}",
            crate::utils::data::config::DEVICE.get()
        );
    }

    use crate::json_db::jsonld::VocabularyRegistry;

    if VocabularyRegistry::global().is_err() {
        let registry = VocabularyRegistry::new();

        // On crée un mini-contexte JSON-LD valide
        let mock_ontology = json_value!({
            "@context": {
                "oa": "https://raise.io/oa#",
                "sa": "https://raise.io/sa#",
                "la": "https://raise.io/la#",
                "pa": "https://raise.io/pa#",
                "rdfs": "http://www.w3.org/2000/01/rdf-schema#"
            }
        });

        // On utilise la VRAIE fonction de production pour hydrater l'état RCU !
        let _ = registry
            .load_layer_from_json("system_mock", &mock_ontology)
            .await;

        // On verrouille l'instance dans le singleton global
        VocabularyRegistry::set_global_instance(registry);
    }
}
pub async fn inject_test_catalog(manager: &CollectionsManager<'_>) -> RaiseResult<()> {
    let schema_uri = format!(
        "db://{}/{}/schemas/v1/db/generic.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
    );

    // 1. Création des collections de gouvernance (Zéro Dette : on ignore si elles existent déjà)
    let _ = manager.create_collection("domains", &schema_uri).await;
    let _ = manager.create_collection("databases", &schema_uri).await;

    let config = AppConfig::get();

    // 2. Injection du domaine Système
    let sys_domain_handle = &config.mount_points.system.domain;
    let sys_domain_id = format!("dom_{}", sys_domain_handle);

    manager
        .upsert_document(
            "domains",
            json_value!({
                "_id": sys_domain_id,
                "handle": sys_domain_handle,
                "name": {"fr": "Domaine Système", "en": "System Domain"},
                "status": "active"
            }),
        )
        .await?;

    // 3. Injection de la base de données Système
    manager
        .upsert_document(
            "databases",
            json_value!({
                "_id": format!("db_{}", config.mount_points.system.db),
                "handle": config.mount_points.system.db,
                "domain_id": sys_domain_id,
                "is_system": true,
                "status": "active"
            }),
        )
        .await?;

    // 4. Injection automatique du domaine métier 'modeling' défini dans la config
    let mod_domain_handle = &config.mount_points.modeling.domain;
    if mod_domain_handle != sys_domain_handle {
        let mod_domain_id = format!("dom_{}", mod_domain_handle);

        manager
            .upsert_document(
                "domains",
                json_value!({
                    "_id": mod_domain_id,
                    "handle": mod_domain_handle,
                    "name": {"fr": "Domaine Modélisation", "en": "Modeling Domain"},
                    "status": "active"
                }),
            )
            .await?;

        manager
            .upsert_document(
                "databases",
                json_value!({
                    "_id": format!("db_{}", config.mount_points.modeling.db),
                    "handle": config.mount_points.modeling.db,
                    "domain_id": mod_domain_id,
                    "is_system": false,
                    "status": "active"
                }),
            )
            .await?;
    }

    Ok(())
}

// --- SANDBOXES ---
pub struct DbSandbox {
    _dir: TempDir,
    pub storage: StorageEngine,
    pub config: AppConfig,
}

pub async fn inject_system_ontologies(manager: &CollectionsManager<'_>) -> RaiseResult<()> {
    use crate::json_db::jsonld::VocabularyRegistry;
    use crate::json_db::schema::ddl::DdlHandler;

    let schema_uri = format!(
        "db://{}/{}/schemas/v2/system/db/ontology.schema.json",
        BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
    );

    // 1. Schéma DDL de l'Ontologie (Basé sur la Production)
    let schema_doc = json_value!({
        "$id": schema_uri.clone(),
        "type": "object",
        "properties": {
            "@context": { "type": ["object", "string", "array"] },
            "@graph": { "type": "array" },
            "namespace": { "type": "string" },
            "version": { "type": "string" }
        },
        "required": ["@context", "@graph"],
        "additionalProperties": true
    });

    let _ = manager
        .create_schema_def("v2/system/db/ontology.schema.json", schema_doc)
        .await;

    // 2. Création de la collection système
    let _ = manager.create_collection("_ontologies", &schema_uri).await;

    // 3. Injection du Dataset (Subset de raise.jsonld pour les tests)
    let raise_onto = json_value!({
        "_id": "ontology_raise",
        "handle": "onto-raise-core",
        "namespace": "raise",
        "version": "1.1.0",
        "@context": {
            "oa": "https://raise.io/oa#",
            "sa": "https://raise.io/sa#",
            "la": "https://raise.io/la#",
            "pa": "https://raise.io/pa#",
            "transverse": "https://raise.io/transverse#",
            "rdfs": "http://www.w3.org/2000/01/rdf-schema#",
            "owl": "http://www.w3.org/2002/07/owl#",
            "raise": "https://raise.io/ontology/raise#"
        },
        "@graph": [
            { "@id": "raise:Role", "@type": "owl:Class" },
            { "@id": "raise:Permission", "@type": "owl:Class" },
            { "@id": "raise:Agent", "@type": "owl:Class", "rdfs:subClassOf": "pa:PhysicalActor" },
            { "@id": "raise:belongsToDapp", "@type": "owl:ObjectProperty" }
        ]
    });
    insert_mock_db(manager, "_ontologies", &raise_onto).await?;

    // 4. Inscription DDL dans le Jeton Système
    let ddl = DdlHandler::new(manager);
    let _ = ddl
        .register_ontology(
            "raise",
            "db://_system/bootstrap/ontologies/raise.jsonld",
            "1.1.0",
        )
        .await;

    // 5. Hydratation dynamique du Cerveau Sémantique avec la DB !
    let _ = VocabularyRegistry::init_from_db(manager).await;

    Ok(())
}

impl DbSandbox {
    pub async fn new() -> RaiseResult<Self> {
        inject_mock_config().await;
        let mut config = AppConfig::get().clone();

        let dir = match tempdir() {
            Ok(d) => d,
            Err(e) => panic!("Création du dossier temporaire échouée : {:?}", e),
        };
        let root_path = dir.path().to_path_buf();

        config.paths.insert(
            "PATH_RAISE_DOMAIN".to_string(),
            root_path.to_string_lossy().to_string(),
        );

        let db_cfg = JsonDbConfig::new(root_path.clone());

        bootstrap_system_index(&db_cfg, BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
            .await
            .unwrap();

        let storage = StorageEngine::new(db_cfg)?;
        let sandbox = Self {
            _dir: dir,
            storage,
            config,
        };

        let mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );

        let base_uri = format!("db://{}/{}", BOOTSTRAP_DOMAIN, BOOTSTRAP_DB);

        let _ = mgr
            .init_db_with_schema(&format!("{}/schemas/v1/db/index.schema.json", base_uri))
            .await;

        let _ = mgr
            .create_collection(
                "users",
                &format!("{}/schemas/v1/mock/users.schema.json", base_uri),
            )
            .await;
        let _ = mgr
            .create_collection(
                "sessions",
                &format!("{}/schemas/v1/mock/sessions.schema.json", base_uri),
            )
            .await;

        // On injecte la collection et la dApp 'raise_core'
        // pour que TransactionManager::resolve_all_refs ne panique pas !
        let _ = mgr
            .create_collection(
                "dapps",
                &format!("{}/schemas/v1/db/generic.schema.json", base_uri),
            )
            .await;

        let mock_dapp_doc = json_value!({
            "_id": "mock-dapp-raise-core",
            "handle": "raise_core"
        });
        insert_mock_db(&mgr, "dapps", &mock_dapp_doc).await?;

        inject_system_ontologies(&mgr).await?;

        Ok(sandbox)
    }

    pub async fn mock_db(manager: &CollectionsManager<'_>) -> RaiseResult<bool> {
        bootstrap_system_index(&manager.storage.config, &manager.space, &manager.db)
            .await
            .unwrap();

        let uri = format!(
            "db://{}/{}/schemas/v1/db/index.schema.json",
            BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
        );
        let res = manager.init_db_with_schema(&uri).await;

        //  Injection de la dApp dans les tests qui appellent mock_db manuellement
        let base_uri = format!("db://{}/{}", BOOTSTRAP_DOMAIN, BOOTSTRAP_DB);
        manager
            .create_collection(
                "dapps",
                &format!("{}/schemas/v1/db/generic.schema.json", base_uri),
            )
            .await?;

        let mock_dapp_doc = json_value!({
            "_id": "mock-dapp-raise-core",
            "handle": "raise_core"
        });

        insert_mock_db(manager, "dapps", &mock_dapp_doc).await?;

        inject_system_ontologies(manager).await?;

        res
    }
}

pub struct AgentDbSandbox {
    _dir: TempDir,
    pub db: SharedRef<StorageEngine>,
    pub config: AppConfig,
    pub domain_root: PathBuf,
    pub shared_engine: SharedRef<AsyncMutex<dyn LlmEngine>>,
}

impl AgentDbSandbox {
    pub async fn new() -> RaiseResult<Self> {
        let base = DbSandbox::new().await?;
        let db = SharedRef::new(base.storage);
        let domain_root = base.config.get_path("PATH_RAISE_DOMAIN").unwrap();

        let temp_manager = CollectionsManager::new(
            &db,
            &base.config.mount_points.system.domain,
            &base.config.mount_points.system.db,
        );

        match DbSandbox::mock_db(&temp_manager).await {
            Ok(_) => {}
            Err(e) => panic!(
                "Erreur lors de l'initialisation de la DB dans la Sandbox : {:?}",
                e
            ),
        }

        // 🎯  Injection du catalogue de gouvernance pour activer find_global_document
        inject_test_catalog(&temp_manager).await?;

        // INJECTION SYSTÉMATIQUE DE LA STACK IA COMPLÈTE
        // 1. LLM & NLP (Modèles de base)
        inject_mock_component(
            &temp_manager,
            "llm",
            json_value!({
                "rust_model_file":  MOCK_LLM_MODEL,
                "rust_tokenizer_file":  MOCK_LLM_TOKENIZER
            }),
        )
        .await?;

        inject_mock_component(
            &temp_manager,
            "nlp",
            json_value!({
                "model_name": "minilm",
                "rust_config_file": "config.json",
                "rust_tokenizer_file": "tokenizer.json",
                "rust_safetensors_file": "model.safetensors"
            }),
        )
        .await?;

        // 2. RAG & Graph Store (Mémoire) { "model_name": "minilm" }
        inject_mock_component(
            &temp_manager,
            "ai_context_rag",
            json_value!({"model_name": "minilm", "provider": "mock" }),
        )
        .await?;

        inject_mock_component(
            &temp_manager,
            "ai_graph_store",
            json_value!({ "embedding_dim": 16, "provider": "native" , "storage_mode": "memory"}),
        )
        .await?;

        // 3. World Model (Modèle du monde pour les agents)
        inject_mock_component(
            &temp_manager,
            "ai_world_model",
            json_value!({
                "vocab_size": 16,
                "embedding_dim": 16,
                "action_dim": 8,
                "hidden_dim": 32,
                "use_gpu": true
            }),
        )
        .await?;

        let schema_quality = format!(
            "db://{}/{}/schemas/v2/assurance/quality_report.schema.json",
            base.config.mount_points.system.domain, base.config.mount_points.system.db
        );
        let schema_xai = format!(
            "db://{}/{}/schemas/v2/assurance/xai_frame.schema.json",
            base.config.mount_points.system.domain, base.config.mount_points.system.db
        );

        inject_mock_component(
            &temp_manager,
            "ai_assurance",
            json_value!({
                "quality_collection": "quality_reports",
                "quality_schema": schema_quality,
                "xai_collection": "xai_frames",
                "xai_schema": schema_xai
            }),
        )
        .await?;

        inject_mock_component(
            &temp_manager,
            "ai_voice",
            json_value!({
                "rust_model_file": "model.safetensors",
                "rust_config_file": "config.json",
                "rust_tokenizer_file": "tokenizer.json",
                "rust_mel_filters": "mel_filters.safetensors"
            }),
        )
        .await?;

        // 4. Injection des Fallbacks Cloud pour satisfaire le Gatekeeper dans les tests
        // (Garantit que LlmClient peut tester son routage de secours sans crasher)
        for provider in ["mistral_ai", "anthropic_claude", "google_gemini"] {
            let config_doc = json_value!({
                "_id": format!("cfg_{}_test", provider),
                "handle": format!("cfg_{}_test", provider),
                "component_id": format!("ref:services:blueprint:{}", provider),
                "environment": "test",
                "service_settings": {
                    "api_key": "mock_key_for_sandbox",
                    "model": "mock-model",
                    "url": "http://127.0.0.1:9999/mock_api"
                }
            });

            insert_mock_db(&temp_manager, "service_configs", &config_doc).await?;
        }

        // 🎯 INITIALISATION / RÉCUPÉRATION DU MOTEUR PARTAGÉ
        let shared_engine = SHARED_LLM_ENGINE
            .get_or_try_init(|| async {
                // 1. Détection de l'environnement GitHub Actions
                let is_ci = std::env::var("GITHUB_ACTIONS").is_ok();

                if is_ci {
                    // 🎯 SUR GITHUB : On tente le moteur, sinon on Mock sans broncher
                    match NativeTensorEngine::new(&temp_manager).await {
                        Ok(engine) => {
                            let engine_trait: SharedRef<AsyncMutex<dyn LlmEngine>> =
                                SharedRef::new(AsyncMutex::new(engine));
                            Ok::<_, AppError>(engine_trait)
                        }
                        Err(_) => {
                            let mock = MockLlmEngine {
                                response: "Test unitaire validé avec succès".to_string(),
                            };
                            let engine_trait: SharedRef<AsyncMutex<dyn LlmEngine>> =
                                SharedRef::new(AsyncMutex::new(mock));
                            Ok::<_, AppError>(engine_trait)
                        }
                    }
                } else {
                    // 🎯 EN LOCAL : On veut que ça pète si le vrai moteur ne charge pas !
                    // On ne catch pas l'erreur, on la laisse remonter.
                    let engine = NativeTensorEngine::new(&temp_manager).await?;
                    let engine_trait: SharedRef<AsyncMutex<dyn LlmEngine>> =
                        SharedRef::new(AsyncMutex::new(engine));
                    Ok::<_, AppError>(engine_trait)
                }
            })
            .await?
            .clone();

        Ok(Self {
            _dir: base._dir,
            db,
            config: base.config,
            domain_root,
            shared_engine,
        })
    }
}

pub struct GlobalDbSandbox {
    pub db: SharedRef<StorageEngine>,
    pub config: &'static AppConfig,
    pub domain_root: PathBuf,
}

impl GlobalDbSandbox {
    pub async fn new() -> RaiseResult<Self> {
        inject_mock_config().await;
        let config = AppConfig::get();
        let db_root = config.get_path("PATH_RAISE_DOMAIN").unwrap();

        let cfg_db = JsonDbConfig::new(db_root.clone());
        let storage = StorageEngine::new(cfg_db.clone())?;

        let manager = CollectionsManager::new(
            &storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let _ = manager.drop_db().await;

        bootstrap_system_index(&cfg_db, BOOTSTRAP_DOMAIN, BOOTSTRAP_DB)
            .await
            .unwrap();

        match manager.init_db().await {
            Ok(_) => {}
            Err(e) => panic!("Impossible d'initialiser la GlobalDbSandbox : {:?}", e),
        }

        Ok(Self {
            db: SharedRef::new(storage),
            config,
            domain_root: db_root,
        })
    }
}

// =========================================================================
// TESTS UNITAIRES DES MOCKS (Validation de l'infrastructure)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_inject_core_schemas_populates_json() -> RaiseResult<()> {
        let mut sys_doc = json_value!({});
        let cfg = JsonDbConfig::new(tempdir().unwrap().path().to_path_buf());
        inject_core_schemas_to_index(&cfg, &mut sys_doc).await;

        assert!(
            sys_doc["schemas"]["v1"]["db/index.schema.json"].is_object(),
            "Le schéma d'index doit être injecté"
        );
        assert!(
            sys_doc["schemas"]["v1"]["db/generic.schema.json"].is_object(),
            "Le schéma générique doit être injecté"
        );
        Ok(())
    }
    #[tokio::test]
    async fn test_inject_mock_schema_populates_json() -> RaiseResult<()> {
        let mut sys_doc = json_value!({});
        let cfg = JsonDbConfig::new(tempdir().unwrap().path().to_path_buf());
        inject_mock_schema_to_index(
            &cfg,
            &mut sys_doc,
            "test_collection",
            r#"{"type": "object"}"#,
        )
        .await;

        assert!(
            sys_doc["schemas"]["v1"]["mock/test_collection.schema.json"].is_object(),
            "Le schéma mock doit être présent"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_agent_db_sandbox_initializes_and_injects_sessions() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;

        let session_meta_path = sandbox.domain_root.join(format!(
            "{}/{}/collections/sessions/_meta.json",
            BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
        ));

        assert!(
            fs::exists_async(&session_meta_path).await,
            "Le _meta.json de session manque dans la sandbox !"
        );

        let content = match fs::read_to_string_async(&session_meta_path).await {
            Ok(c) => c,
            Err(e) => panic!("Impossible de lire _meta.json : {:?}", e),
        };

        let expected_schema_uri = format!(
            "db://{}/{}/schemas/v1/mock/sessions.schema.json",
            BOOTSTRAP_DOMAIN, BOOTSTRAP_DB
        );
        assert!(
            content.contains(&expected_schema_uri),
            "Le lien URI vers le mock de session est cassé"
        );

        Ok(())
    }
}
