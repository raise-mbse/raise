// FICHIER : crates/raise-core/src/utils/data/config.rs

// 1. Base de données (AI-Ready Queries)
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::query::{Condition, FilterOperator, Query, QueryEngine, QueryFilter};
// 2. Core : Environnement, Concurrence et Erreurs
use crate::utils::core::error::AppError;
use crate::utils::core::error::RaiseResult;
use crate::utils::core::{RuntimeEnv, StaticCell, UniqueId};
use crate::{kernel_fatal, raise_error};

// 3. I/O : Système de fichiers
use crate::utils::io::fs::{self, PathBuf};

// 4. Data : Traits, Collections sémantiques et JSON
use crate::utils::data::json::{self, json_value, JsonValue};
use crate::utils::data::{
    CustomDeserializerEngine, Deserializable, DeserializationErrorTrait, Serializable, UnorderedMap,
};

/// Singleton global pour la configuration
pub static CONFIG: StaticCell<AppConfig> = StaticCell::new();
pub static DEVICE: StaticCell<candle_core::Device> = StaticCell::new();

/// Constantes Système pour amorcer la première lecture
pub const BOOTSTRAP_DOMAIN: &str = "_system";
pub const BOOTSTRAP_DB: &str = "master";

/// Configuration globale structurée par niveaux de responsabilité
#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct AppConfig {
    #[serde(rename = "_id")]
    pub id: String,

    pub handle: String,

    #[serde(rename = "_created_at")]
    pub created_at: String,

    #[serde(rename = "_updated_at")]
    pub updated_at: String,

    #[serde(rename = "@type", deserialize_with = "deserialize_type_flexible")]
    pub semantic_type: Vec<String>,

    pub name: Option<UnorderedMap<String, String>>,

    // --- LA COLONNE VERTÉBRALE (VITAL) ---
    pub mount_points: MountPointsConfig,
    pub core: CoreConfig,

    #[serde(deserialize_with = "deserialize_paths_flexible")]
    pub paths: UnorderedMap<String, String>,

    // --- IDENTIFIANTS DE BOOT ---
    pub active_dapp_id: String,
    pub workstation_id: String,

    #[serde(default)]
    pub active_services: Vec<String>,
    #[serde(default)]
    pub active_components: Vec<String>,

    // --- SCOPES RUNTIME (DYNAMIQUE) ---
    #[serde(skip)]
    pub dapp: Option<ScopeConfig>,
    #[serde(skip)]
    pub workstation: Option<ScopeConfig>,
    #[serde(skip)]
    pub user: Option<ScopeConfig>,
    #[serde(skip)]
    pub mandator: Option<ScopeConfig>,

    #[serde(default)]
    pub system_assets: SystemAssets,
}

#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct MountPointsConfig {
    pub system: DbPointer,
    pub raise: DbPointer,
    pub exploration: DbPointer, // Incubation
    pub modeling: DbPointer,    // As-Designed
    pub simulation: DbPointer,  // As-Simulated
    pub integration: DbPointer, // V&V Physique
    pub production: DbPointer,  // As-Built
    pub operation: DbPointer,   // As-Operated
}

#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct DbPointer {
    pub domain: String,
    pub db: String,
}

/// Configuration spécifique à un contexte identitaire
#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct ScopeConfig {
    pub id: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serializable, Deserializable, Default, PartialEq)]
pub struct SystemAssets {
    pub schemas_uri: Option<String>,
    pub locales_uri: Option<String>,
    pub ontologies_uri: Option<String>,
    pub ai_assets_paths: Option<AiAssetsPaths>,
}

#[derive(Debug, Clone, Serializable, Deserializable, Default, PartialEq)]
pub struct AiAssetsPaths {
    pub models: Option<String>,
    pub embeddings: Option<String>,
    pub lora: Option<String>,
    pub voice: Option<String>,
    pub ontologies: Option<String>,
}

// =========================================================================
// 🤖 FALLBACKS EXPLICITES POUR LA DÉSÉRIALISATION
// =========================================================================
fn deserialize_type_flexible<'de, D>(deserializer: D) -> std::result::Result<Vec<String>, D::Error>
where
    D: CustomDeserializerEngine<'de>,
{
    let v: JsonValue = Deserializable::deserialize(deserializer)?;

    if let Some(s) = v.as_str() {
        // Si c'est une simple chaîne de caractères, on l'enveloppe dans un tableau
        Ok(vec![s.to_string()])
    } else if let Some(arr) = v.as_array() {
        // Si c'est déjà un tableau, on le lit proprement
        let mut types = Vec::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                types.push(s.to_string());
            }
        }
        Ok(types)
    } else {
        Err(DeserializationErrorTrait::custom(
            "Le champ '@type' est invalide. Attendu : String ou Array de Strings.",
        ))
    }
}

fn deserialize_paths_flexible<'de, D>(
    deserializer: D,
) -> std::result::Result<UnorderedMap<String, String>, D::Error>
where
    D: CustomDeserializerEngine<'de>,
{
    let v: JsonValue = Deserializable::deserialize(deserializer)?;

    if let Some(map) = v.as_object() {
        let mut paths = UnorderedMap::new();
        for (key, val) in map {
            if let Some(s) = val.as_str() {
                paths.insert(key.clone(), s.to_string());
            }
        }
        Ok(paths)
    } else if let Some(arr) = v.as_array() {
        let mut paths = UnorderedMap::new();
        for item in arr {
            let id = item.get("id").and_then(|v| v.as_str());
            let val = item.get("value").and_then(|v| v.as_str());
            if let (Some(k), Some(v)) = (id, val) {
                paths.insert(k.to_string(), v.to_string());
            }
        }
        Ok(paths)
    } else {
        Err(DeserializationErrorTrait::custom(
            "Format de 'paths' invalide : attendu JsonObject ou Liste",
        ))
    }
}

// =========================================================================
// SOUS-STRUCTURES DE CONFIGURATION
// =========================================================================

#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub struct CoreConfig {
    pub env_mode: String,
    pub graph_mode: String,
    pub log_level: String,
    pub vector_store_provider: String,
    pub language: String,
    pub use_gpu: bool,
}

// =========================================================================
// IMPLÉMENTATION PRINCIPALE
// =========================================================================

impl AppConfig {
    pub fn init() -> RaiseResult<()> {
        if CONFIG.get().is_some() {
            return Ok(());
        }

        crate::utils::core::env::load_local_env(std::path::Path::new(".env"));

        let target_env = if cfg!(test) || RuntimeEnv::var("RAISE_ENV_MODE").as_deref() == Ok("test")
        {
            "test".to_string()
        } else if let Ok(env_override) = RuntimeEnv::var("RAISE_ENV_MODE") {
            env_override
        } else if cfg!(debug_assertions) {
            "development".to_string()
        } else {
            "production".to_string()
        };

        #[cfg(any(test, debug_assertions))]
        let config = if target_env == "test" {
            crate::utils::testing::mock::load_test_sandbox()?
        } else {
            Self::load_production_config(&target_env)?
        };

        #[cfg(not(any(test, debug_assertions)))]
        let config = Self::load_production_config(&target_env)?;

        if DEVICE.get().is_none() {
            let device = Self::detect_best_device(&config);
            let _ = DEVICE.set(device);
        }

        if CONFIG.set(config).is_err() {
            raise_error!(
                "ERR_CONFIG_INIT_ONCE",
                error = "La configuration est déjà initialisée"
            );
        }

        Ok(())
    }

    pub fn get() -> &'static AppConfig {
        CONFIG
            .get()
            .expect("❌ AppConfig non initialisé ! Appelez AppConfig::init() au démarrage.")
    }

    pub fn is_test_env(&self) -> bool {
        self.core.env_mode == "test"
    }

    pub fn get_path(&self, key: &str) -> Option<PathBuf> {
        // 1. Priorité absolue à l'environnement OS (le fichier .env chargé en RAM)
        let raw_path = match RuntimeEnv::var(key) {
            Ok(v) => v,
            Err(_) => {
                // 2. Fallback sur la configuration interne (JSON)
                match self.paths.get(key) {
                    Some(p) => p.to_string(),
                    None => return None,
                }
            }
        };

        // 3. ANCRAGE UTILISATEUR (Tilde) -> /home/user/...
        if raw_path.starts_with("~/") {
            let home_dir = dirs::home_dir()?;
            let clean_path = raw_path.trim_start_matches("~/");
            return Some(home_dir.join(clean_path));
        }

        let path = PathBuf::from(raw_path);

        // 4. ANCRAGE SYSTÈME ABSOLU (ex: /var/lib/raise)
        if path.is_absolute() {
            return Some(path);
        }

        // 5. ANCRAGE BINAIRE (PORTABLE) -> Résolu par rapport à l'exécutable
        match RuntimeEnv::current_exe() {
            Ok(mut exe_path) => {
                exe_path.pop(); // Remonte au dossier parent de l'exécutable
                Some(exe_path.join(path))
            }
            Err(_) => None,
        }
    }

    pub async fn get_runtime_settings(
        manager: &CollectionsManager<'_>,
        target_ref: &str,
    ) -> RaiseResult<JsonValue> {
        let config = AppConfig::get();

        // 1. RÉSOLUTION DYNAMIQUE DU DOMAINE CIBLE (Cross-Domain Ready)
        let (target_domain, target_db, _) =
            config.resolve_system_uri(Some(&target_ref.to_string()), "service_configs");

        let target_manager = CollectionsManager::new(manager.storage, &target_domain, &target_db);

        // 2. RÉSOLUTION DE L'ID
        let id_to_query = match target_manager.resolve_single_reference(target_ref).await {
            Ok(uuid) => uuid,
            Err(_) => target_ref.to_string(), // Si on a déjà passé l'UUID direct
        };

        // 3. REQUÊTE STRICTE
        let join_field = if target_ref.contains("components:") {
            "component_id"
        } else {
            "service_id"
        };

        let mut query = Query::new("service_configs");
        query.filter = Some(QueryFilter {
            operator: FilterOperator::And,
            conditions: vec![Condition::eq(join_field, json_value!(id_to_query.clone()))],
        });
        query.limit = Some(1);

        let result = QueryEngine::new(&target_manager)
            .execute_query(query)
            .await?;

        // 4. RETOUR OU ERREUR FATALE (Pas de fallback silencieux !)
        if let Some(doc) = result.documents.into_iter().next() {
            if let Some(settings) = doc.get("service_settings") {
                return Ok(settings.clone());
            } else {
                raise_error!(
                    "ERR_CONFIG_INVALID_SETTINGS",
                    error = "Le document a été trouvé mais 'service_settings' est absent.",
                    context = json_value!({ "target": target_ref })
                );
            }
        }

        // 🚨 ERREUR STRICTE : La config n'est pas trouvée
        raise_error!(
            "ERR_CONFIG_NOT_FOUND",
            error = "Configuration introuvable pour ce composant dans la base de données cible.",
            context = json_value!({
                "target": target_ref,
                "queried_id": id_to_query,
                "domain": target_domain,
                "db": target_db
            })
        )
    }

    fn get_bootstrap_pointers() -> (String, String) {
        let domain = RuntimeEnv::var("RAISE_BOOTSTRAP_DOMAIN")
            .unwrap_or_else(|_| BOOTSTRAP_DOMAIN.to_string());
        let db = RuntimeEnv::var("RAISE_BOOTSTRAP_DB").unwrap_or_else(|_| BOOTSTRAP_DB.to_string());
        (domain, db)
    }

    // =====================================================================
    // 🧬 SÉQUENCE D'AMORÇAGE CONTEXTUEL BOTTOM-UP OPTIMISÉE (IaD)
    // =====================================================================
    fn load_production_config(env: &str) -> RaiseResult<Self> {
        // 🟢 ÉTAPE 1 : WORKSTATION (La Fondation Matérielle)
        let target_hostname = RuntimeEnv::var("RAISE_WORKSTATION")
            .or_else(|_| RuntimeEnv::var("HOSTNAME"))
            .or_else(|_| RuntimeEnv::var("COMPUTERNAME"))
            .unwrap_or_else(|_| "localhost".to_string());

        let ws_lookup = Self::load_collection_doc_with_meta("workstations", |v| {
            v.get("hostname").and_then(|h| h.as_str()) == Some(target_hostname.as_str())
        });

        let mut current_ws_id = String::new();
        let mut current_ws_handle = String::new();
        let mut current_owner_id = String::new();
        let mut ws_language = None;

        if let Some((_, _, ws_doc)) = ws_lookup {
            current_ws_handle = ws_doc
                .get("handle")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // On récupère le VRAI _id UUID pour l'intersection
            current_ws_id = ws_doc
                .get("_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            current_owner_id = ws_doc
                .get("owner_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            ws_language = ws_doc
                .get("language")
                .and_then(|v| v.as_str())
                .map(String::from);
        }

        // 🟢 ÉTAPE 2 : DAPP (Le Contexte Logiciel)
        let exe_path = match std::env::current_exe() {
            Ok(p) => p,
            Err(e) => raise_error!(
                "ERR_CONFIG_CURRENT_EXE",
                error = "Impossible de déterminer l'exécutable en cours d'exécution.",
                context = json_value!({"details": e.to_string()})
            ),
        };

        let exe_name = match exe_path.file_stem() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => raise_error!(
                "ERR_CONFIG_EXE_NAME",
                error = "Le binaire exécuté n'a pas de nom de fichier valide.",
                context = json_value!({"path": exe_path.to_string_lossy()})
            ),
        };

        let normalized_exe = exe_name.replace("-", "_");
        let target_dapp = RuntimeEnv::var("RAISE_DAPP").unwrap_or(normalized_exe);

        let dapp_lookup = Self::load_collection_doc_with_meta("dapps", |v| {
            v.get("plugin_config")
                .and_then(|pc| pc.get("rust_package_name"))
                .and_then(|n| n.as_str())
                == Some(target_dapp.as_str())
                || v.get("handle").and_then(|h| h.as_str()) == Some(target_dapp.as_str())
        });

        let mut current_dapp_id = String::new();
        let mut current_dapp_handle = String::new();

        if let Some((_, _, dapp_doc)) = dapp_lookup {
            current_dapp_handle = dapp_doc
                .get("handle")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            // On récupère le VRAI _id UUID
            current_dapp_id = dapp_doc
                .get("_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
        }

        // 🟢 ÉTAPE 3 : CONFIG (L'Intersection)
        let profile_override = RuntimeEnv::var("RAISE_PROFILE").ok();

        let system_match = Self::load_collection_doc_with_meta("configs", |v| {
            let is_active = v.get("status").and_then(|s| s.as_str()) != Some("inactive");
            if !is_active {
                return false;
            }

            if let Some(ref profile) = profile_override {
                v.get("handle").and_then(|h| h.as_str()) == Some(profile.as_str())
                    || v.get("_id").and_then(|id| id.as_str()) == Some(profile.as_str())
            } else {
                // 🎯 On compare avec les VRAIS UUID résolus dans la BDD !
                let match_ws = v.get("workstation_id").and_then(|id| id.as_str())
                    == Some(current_ws_id.as_str());
                let match_dapp = v.get("active_dapp_id").and_then(|id| id.as_str())
                    == Some(current_dapp_id.as_str());
                match_ws && match_dapp
            }
        });

        let Some((config_path, raw_json, _json_val)) = system_match else {
            crate::user_warn!(
                "WRN_BOOTSTRAP_MODE",
                json_value!({
                    "environment": env,
                    "hint": format!("Aucune configuration liant Workstation '{}' et DApp '{}' trouvée.", current_ws_handle, current_dapp_handle)
                })
            );
            return Ok(Self::generate_bootstrap_config());
        };

        let mut config: AppConfig = match json::deserialize_from_str(&raw_json) {
            Ok(c) => c,
            Err(e) => {
                let AppError::Structured(data) = &e;
                let detail_technique = data
                    .context
                    .get("technical_error")
                    .and_then(|v| v.as_str())
                    .unwrap_or("Détail technique absent")
                    .to_string();

                kernel_fatal!(
                    "Désérialisation du Schéma de Configuration",
                    config_path.display(),
                    detail_technique
                );

                raise_error!(
                    "ERR_CONFIG_SCHEMA_INVALID",
                    error = e.to_string(),
                    context = json_value!({ "file": config_path.to_string_lossy() })
                )
            }
        };

        if !current_ws_handle.is_empty() {
            config.workstation = Some(ScopeConfig {
                id: current_ws_handle,
                language: ws_language,
            });
        }
        if !current_dapp_handle.is_empty() {
            config.dapp = Some(ScopeConfig {
                id: current_dapp_handle,
                language: None,
            });
        }

        // 🟢 ÉTAPE 4 : USER (Le Contexte Identitaire)
        let os_user = RuntimeEnv::var("USER")
            .or_else(|_| RuntimeEnv::var("USERNAME"))
            .unwrap_or_else(|_| "unknown".to_string());

        let mut current_user_id = String::new(); // 🎯 L'UUID résolu par jsondb
        let user_json = Self::load_collection_doc_with_meta("users", |v| {
            let match_os = v.get("handle").and_then(|u| u.as_str()) == Some(os_user.as_str());
            let match_owner = !current_owner_id.is_empty()
                && v.get("_id").and_then(|u| u.as_str()) == Some(current_owner_id.as_str());
            match_os || match_owner
        })
        .or_else(|| {
            Self::load_collection_doc_with_meta("users", |v| {
                v.get("handle").and_then(|u| u.as_str()) == Some("admin")
            })
        });

        if let Some((_, _, user_doc)) = user_json {
            let handle = user_doc
                .get("handle")
                .and_then(|v| v.as_str())
                .unwrap_or("admin");
            // 🎯 On stocke le véritable _id de l'utilisateur
            current_user_id = user_doc
                .get("_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();

            config.user = Some(ScopeConfig {
                id: handle.to_string(),
                language: user_doc
                    .get("preferences")
                    .and_then(|p| p.get("language"))
                    .and_then(|v| v.as_str())
                    .map(String::from),
            });
        }

        // 🟢 ÉTAPE 5 : MANDATOR (Le Contexte d'Autorité)
        if !current_user_id.is_empty() {
            if let Some((_, _, mandator_doc)) =
                Self::load_collection_doc_with_meta("mandators", |v| {
                    if let Some(user_ids) = v.get("user_ids").and_then(|arr| arr.as_array()) {
                        // 🎯 On compare avec l'UUID réel !
                        user_ids
                            .iter()
                            .any(|id| id.as_str() == Some(&current_user_id))
                    } else {
                        false
                    }
                })
            {
                config.mandator = Some(ScopeConfig {
                    id: mandator_doc
                        .get("handle")
                        .and_then(|v| v.as_str())
                        .unwrap_or_default()
                        .to_string(),
                    language: None,
                });
            }
        }

        Ok(config)
    }

    /// Génère une configuration matérielle minimale en RAM pour autoriser l'amorçage
    /// d'une station vierge. Les vrais paramètres seront forgés plus tard par le Bootstrapper.
    fn generate_bootstrap_config() -> Self {
        let (domain, db) = Self::get_bootstrap_pointers();
        let mut paths = UnorderedMap::new();
        if let Ok(home) = RuntimeEnv::var("HOME") {
            paths.insert(
                "PATH_RAISE_DOMAIN".to_string(),
                format!("{}/production_domain", home),
            );
            paths.insert(
                "PATH_RAISE_DATASET".to_string(),
                format!("{}/production_dataset", home),
            );
        }
        Self {
            id: UniqueId::new_v4().to_string(),
            handle: "bootstrap".to_string(),
            created_at: String::new(),
            updated_at: String::new(),
            semantic_type: vec!["Configuration".to_string()],
            name: None,
            mount_points: MountPointsConfig {
                system: DbPointer {
                    domain: domain.clone(),
                    db: db.clone(),
                },
                raise: DbPointer {
                    domain: domain.clone(),
                    db: "raise_core".to_string(),
                },
                exploration: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "exploration".to_string(),
                },
                modeling: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "modeling".to_string(),
                },
                simulation: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "simulation".to_string(),
                },
                integration: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "integration".to_string(),
                },
                production: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "production".to_string(),
                },
                operation: DbPointer {
                    domain: "sandbox".to_string(),
                    db: "operation".to_string(),
                },
            },
            core: CoreConfig {
                env_mode: "development".to_string(),
                graph_mode: "none".to_string(),
                log_level: "info".to_string(),
                vector_store_provider: "memory".to_string(),
                language: "en".to_string(),
                use_gpu: false,
            },
            paths: UnorderedMap::new(),
            active_dapp_id: "bootstrap".to_string(),
            workstation_id: "bootstrap_ws".to_string(),
            active_services: vec![],
            active_components: vec![],
            workstation: None,
            user: None,
            dapp: None,
            mandator: None,
            system_assets: SystemAssets::default(),
        }
    }

    /// 🎯 RETOURNE LE CHEMIN, LE TEXTE BRUT ET L'ARBRE PARSÉ
    fn load_collection_doc_with_meta<F>(
        collection_name: &str,
        predicate: F,
    ) -> Option<(PathBuf, String, JsonValue)>
    where
        F: Fn(&JsonValue) -> bool,
    {
        // 🎯 On instancie une config "sonde" pour utiliser get_path !
        let probe_config = Self::generate_bootstrap_config();

        let base_domain = probe_config.get_path("PATH_RAISE_DOMAIN")?;

        let (bios_domain, bios_db) = Self::get_bootstrap_pointers();

        let collection_dir = base_domain
            .join(bios_domain)
            .join(bios_db)
            .join("collections")
            .join(collection_name);

        if !collection_dir.exists() {
            return None;
        }

        if let Ok(entries) = fs::read_dir_sync(collection_dir) {
            for entry in entries.flatten() {
                if entry.path().extension().and_then(|s| s.to_str()) == Some("json") {
                    if let Ok(content) = fs::read_to_string_sync(&entry.path()) {
                        if let Ok(doc) = json::deserialize_from_str::<JsonValue>(&content) {
                            if predicate(&doc) {
                                return Some((entry.path(), content, doc));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    fn detect_best_device(config: &AppConfig) -> candle_core::Device {
        // 1. Respect de la frugalité : priorité au CPU si demandé
        if !config.core.use_gpu {
            return candle_core::Device::Cpu;
        }

        // 2. Accélération CUDA (Linux/Windows)
        #[cfg(feature = "cuda")]
        {
            // On tente l'index 0 (ta RTX 5060 physique validée par nvidia-smi)
            if let Ok(dev) = candle_core::Device::new_cuda(0) {
                return dev;
            }
        }

        // 3. Accélération Metal (Mac)
        #[cfg(feature = "metal")]
        {
            if let Ok(dev) = candle_core::Device::new_metal(0) {
                return dev;
            }
        }

        // 4. Fallback universel vers le CPU si aucune accélération n'est disponible
        candle_core::Device::Cpu
    }

    pub fn device() -> &'static candle_core::Device {
        DEVICE.get().expect("Device non initialisé")
    }

    /// Résout une URI système (ex: "db://_system/bootstrap/collections/locales")
    /// Retourne un tuple propre : (domain, db, target_name)
    pub fn resolve_system_uri(
        &self,
        uri_opt: Option<&String>,
        fallback_target: &str,
    ) -> (String, String, String) {
        if let Some(uri) = uri_opt {
            if let Some(path) = uri.strip_prefix("db://") {
                let parts: Vec<&str> = path.split('/').collect();
                if parts.len() >= 3 {
                    let domain = parts[0].to_string();
                    let db = parts[1].to_string();
                    let target = parts[2..].join("/");

                    // Nettoyage intelligent : si l'URI inclut "collections/", on l'enlève pour le SGBD
                    let clean_target = if target.starts_with("collections/") {
                        target.replace("collections/", "")
                    } else {
                        target
                    };
                    return (domain, db, clean_target);
                }
            }
        }

        // 🛡️ Fallback sur la partition système par défaut
        (
            self.mount_points.system.domain.clone(),
            self.mount_points.system.db.clone(),
            fallback_target.to_string(),
        )
    }

    /// Résout un chemin physique absolu (ex: modèles IA) avec un fallback relatif
    pub fn resolve_asset_path(
        &self,
        asset_path_opt: Option<&String>,
        fallback_suffix: &str,
    ) -> RaiseResult<PathBuf> {
        if let Some(absolute_path) = asset_path_opt {
            Ok(PathBuf::from(absolute_path))
        } else {
            // 🛡️ Fallback : On utilise le dossier centralisé des assets
            let asset_root_path = match self.get_path("PATH_RAISE_ASSET") {
                Some(p) => p,
                None => raise_error!(
                    "ERR_CONFIG_ASSET_PATH_MISSING",
                    error = "La variable 'PATH_RAISE_ASSET' est absente de la configuration (JSON ou .env)."
                ),
            };
            let clean_suffix = fallback_suffix
                .strip_prefix("ai-assets/")
                .unwrap_or(fallback_suffix);
            Ok(asset_root_path.join(clean_suffix))
        }
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::core::async_test;

    #[test]
    fn test_scope_config_structure() {
        let scope = ScopeConfig {
            id: "dev-machine".to_string(),
            language: Some("fr".to_string()),
        };
        assert_eq!(scope.id, "dev-machine");
        assert_eq!(scope.language.as_deref(), Some("fr"));
    }

    #[test]
    fn test_deserialize_app_config_with_mount_points() {
        // 🎯 On crée un JSON qui respecte strictement la structure V2
        let json_data = json_value!({
            "_id": "cfg_test",
            "handle": "cfg_test_handle",
            "_created_at": "2026-01-01T00:00:00Z",
            "_updated_at": "2026-01-01T00:00:00Z",
            "@type": ["Configuration"],
            "active_dapp_id": "ref:dapps:handle:raise_core",
            "workstation_id": "ref:workstations:handle:condorcet",
            "mount_points": {
                "system": { "domain": "_sys_domain", "db": "_sys_db" },
                "raise": { "domain": "_sys_domain", "db": "_raise_core" },
                "exploration": { "domain": "proj1", "db": "sandbox" },
                "modeling": { "domain": "proj1", "db": "mbse" },
                "simulation": { "domain": "proj1", "db": "sim" },
                "integration": { "domain": "proj1", "db": "test" },
                "production": { "domain": "proj1", "db": "prod" },
                "operation": { "domain": "proj1", "db": "ops" }
            },
            "core": {
                "env_mode": "test",
                "graph_mode": "none",
                "log_level": "debug",
                "vector_store_provider": "memory",
                "language": "en",
                "use_gpu": false
            },
            "paths": { "PATH_TEST": "/tmp" }
        });

        let config: AppConfig =
            json::deserialize_from_value(json_data).expect("Désérialisation échouée");

        // ✅ Vérification des Mount Points
        assert_eq!(config.mount_points.system.domain, "_sys_domain");
        assert_eq!(config.mount_points.modeling.db, "mbse");

        // ✅ Vérification des Identifiants (Strings)
        assert_eq!(config.active_dapp_id, "ref:dapps:handle:raise_core");
        assert_eq!(config.workstation_id, "ref:workstations:handle:condorcet");

        // 💡 Note : config.workstation sera None ici car il est marqué #[serde(skip)].
        // C'est normal, il est peuplé plus tard par load_production_config().
        assert!(config.workstation.is_none());
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_get_runtime_settings_resolves_correctly() -> RaiseResult<()> {
        use crate::utils::testing::mock::DbSandbox;

        let sandbox = DbSandbox::new().await?;
        let manager = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );

        DbSandbox::mock_db(&manager).await?;

        let generic_schema = "db://_system/bootstrap/schemas/v1/db/generic.schema.json";
        manager
            .create_collection("services", generic_schema)
            .await?;

        let expected_uuid = "phys-uuid-gemini";
        manager
            .insert_raw(
                "services",
                &json_value!({ "_id": expected_uuid, "blueprint": "google_gemini" }),
            )
            .await?;

        manager
            .create_collection("service_configs", generic_schema)
            .await?;

        let mock_service_id = "ref:services:blueprint:google_gemini";

        let mock_doc = json_value!({
            "_id": "cfg_test_gemini",
            "handle": "cfg_test_gemini",
            "service_id": mock_service_id,
            "environment": "test",
            "service_settings": {
                "api_key": "TEST_KEY_123",
                "model": "gemini-test-model"
            }
        });

        manager.upsert_document("service_configs", mock_doc).await?;

        // 🎯 ON TESTE LE GATEKEEPER (get_runtime_settings)
        let settings = AppConfig::get_runtime_settings(&manager, mock_service_id).await?;

        assert_eq!(settings["api_key"], "TEST_KEY_123");
        assert_eq!(settings["model"], "gemini-test-model");

        Ok(())
    }
}
