// FICHIER : crates/raise-core/src/services/codegen_service.rs

use crate::utils::prelude::*; // 🎯 Façade Unique RAISE

use crate::code_generator::models::StagedModule;
use crate::code_generator::module_weaver::ModuleWeaver;
use crate::code_generator::CodeGeneratorService;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::model_engine::loader::ModelLoader;
use crate::model_engine::transformers::{get_transformer, TransformationDomain};
use crate::model_engine::types::ProjectModel;

// 🎯 Fonction rendue publique pour que Tauri puisse l'utiliser
pub fn resolve_active_context(model: &ProjectModel) -> (String, String) {
    let config = AppConfig::get();
    let parts: Vec<&str> = model.meta.name.split('/').collect();

    if parts.len() >= 2 {
        (parts[0].to_string(), parts[1].to_string())
    } else {
        (
            config.mount_points.system.domain.clone(),
            config.mount_points.system.db.clone(),
        )
    }
}

/// 🛡️ Aiguilleur Zéro Dette : Redirige vers le Bac à sable si nécessaire
fn resolve_domain_root(is_simulation_or_test: bool) -> PathBuf {
    let config = AppConfig::get();

    if is_simulation_or_test {
        // Bascule sur le dossier de Simulation/Stress-Test défini dans le .env
        config.get_path("PATH_CODE_FILE").unwrap_or_else(|| {
            crate::user_warn!(
                "WRN_SANDBOX_MISSING",
                json_value!({"hint": "PATH_CODE_FILE non défini, fallback risqué sur PATH_RAISE_DOMAIN"})
            );
            config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default()
        })
    } else {
        // Dossier de Production Officiel
        config.get_path("PATH_RAISE_DOMAIN").unwrap_or_default()
    }
}

/// 🛡️ Réécrit le chemin physique absolu vers la Sandbox à la volée
fn redirect_module_to_sandbox(module_doc: &mut JsonValue) {
    let config = AppConfig::get();
    let Some(sandbox_path) = config.get_path("PATH_CODE_FILE") else {
        return;
    };

    // 1. Détection adaptative : Racine ou sous-objet "properties" (JSON-LD)
    // 🎯 FIX CLIPPY 1 : Utilisation de map() au lieu d'un if let manuel
    let old_path_str = if let Some(p) = module_doc.get("path").and_then(|v| v.as_str()) {
        Some(p.to_string())
    } else {
        module_doc
            .get("properties")
            .and_then(|props| props.get("path"))
            .and_then(|v| v.as_str())
            .map(|p| p.to_string())
    };

    if let Some(path_str) = old_path_str {
        let path = Path::new(&path_str);
        let mut is_crates = false;
        let mut relative_crates = PathBuf::new();

        // 2. Extraction déterministe
        for comp in path.components() {
            if comp.as_os_str() == "crates" {
                is_crates = true;
            }
            if is_crates {
                relative_crates.push(comp);
            }
        }

        let new_path = if is_crates {
            sandbox_path
                .join(relative_crates)
                .to_string_lossy()
                .to_string()
        } else {
            // 🎯 FIX CLIPPY 2 : Utilisation directe de as_ref() sans créer une String intermédiaire
            path_str.replace(
                "/home/zair/condorcet-continuum/raise",
                sandbox_path.to_string_lossy().as_ref(),
            )
        };

        // 3. Observabilité stricte
        crate::user_info!(
            "SANDBOX_REDIRECT_ACTIVE",
            json_value!({ "old_path": path_str, "new_path": new_path })
        );

        // 4. Mise à jour chirurgicale du document en mémoire
        if let Some(obj) = module_doc.as_object_mut() {
            if obj.contains_key("path") {
                obj.insert("path".to_string(), json_value!(new_path.clone()));
            }
            if let Some(props) = obj.get_mut("properties").and_then(|p| p.as_object_mut()) {
                if props.contains_key("path") {
                    props.insert("path".to_string(), json_value!(new_path));
                }
            }
        }
    } else {
        crate::user_warn!(
            "SANDBOX_REDIRECT_FAILED",
            json_value!({ "hint": "Aucun champ 'path' trouvé dans le document du module." })
        );
    }
}

pub async fn init_sandbox_workspace() -> RaiseResult<()> {
    let config = AppConfig::get();

    // 🎯 POINT D'ENTRÉE : La racine physique réelle de votre code
    let source_root = Path::new("/home/zair/condorcet-continuum/raise");

    let target_dir = match config.get_path("PATH_CODE_FILE") {
        Some(p) => p,
        None => raise_error!(
            "ERR_NO_SANDBOX",
            error = "PATH_CODE_FILE manquant dans .env"
        ),
    };

    crate::user_info!(
        "SANDBOX_MIRROR_INIT",
        json_value!({
            "action": "Clonage strict du code source vers Sandbox (Zéro DB)",
            "source": source_root.to_string_lossy(),
            "target": target_dir.to_string_lossy()
        })
    );

    // 1. Purge propre
    if crate::utils::io::fs::exists_async(&target_dir).await {
        crate::utils::io::fs::remove_dir_all_async(&target_dir).await?;
    }
    crate::utils::io::fs::create_dir_all_async(&target_dir).await?;

    // 2. Maintien de l'intégrité du Workspace Rust (copie du Cargo.toml racine)
    let root_cargo = source_root.join("Cargo.toml");
    if crate::utils::io::fs::exists_async(&root_cargo).await {
        crate::utils::io::fs::copy_async(&root_cargo, target_dir.join("Cargo.toml")).await?;
    }

    // 3. Copie exclusive du dossier 'crates' contenant le code métier
    let source_crates = source_root.join("crates");
    let target_crates = target_dir.join("crates");

    if crate::utils::io::fs::exists_async(&source_crates).await {
        crate::utils::io::fs::create_dir_all_async(&target_crates).await?;
        copy_workspace_filtered(&source_crates, &target_crates).await?;
    }

    crate::user_success!(
        "SANDBOX_MIRROR_READY",
        json_value!({ "status": "clean_code_only", "path": target_dir.to_string_lossy() })
    );

    Ok(())
}

/// Copie récursive asynchrone avec évitement des artefacts de compilation
async fn copy_workspace_filtered(src: &Path, dst: &Path) -> RaiseResult<()> {
    let mut entries = crate::utils::io::fs::read_dir_async(src).await?;

    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // 🛡️ Zéro Dette : On ignore la compilation et le versioning source
        if file_name_str == "target" || file_name_str == ".git" {
            continue;
        }

        let target_path = dst.join(file_name);

        // Pattern natif RAISE (vu dans environment.rs)
        let is_dir = entry.file_type().await.map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            crate::utils::io::fs::create_dir_all_async(&target_path).await?;
            // Utilisation de Box::pin pour la récursivité asynchrone sécurisée
            Box::pin(copy_workspace_filtered(&path, &target_path)).await?;
        } else {
            crate::utils::io::fs::copy_async(&path, &target_path).await?;
        }
    }
    Ok(())
}

pub async fn generate_source_code(
    element_id: &str,
    target_domain_str: &str, // 🎯 L'axe de transformation (ex: "software")
    domain: &str,            // 🎯 L'espace de travail MBSE (ex: "_system")
    db: &str,
    storage: &StorageEngine,
) -> RaiseResult<JsonValue> {
    let target_domain = match target_domain_str.to_lowercase().as_str() {
        "software" | "code" | "rust" | "cpp" => TransformationDomain::Software,
        "hardware" | "vhdl" | "fpga" | "verilog" => TransformationDomain::Hardware,
        "system" | "overview" | "doc" | "architecture" => TransformationDomain::System,
        _ => {
            raise_error!(
                "ERR_CODEGEN_DOMAIN_UNSUPPORTED",
                error = format!(
                    "Le domaine cible '{}' n'est pas supporté.",
                    target_domain_str
                )
            );
        }
    };

    // 🎯 FIX : On utilise ici le "domain" (l'espace) pour pointer vers la bonne base
    let loader = ModelLoader::new(storage, domain, db)?;
    if let Err(e) = loader.index_project().await {
        raise_error!("ERR_CODEGEN_INDEX_FAILED", error = e.to_string());
    }

    let element = loader.get_element(element_id).await?;
    let element_json = match json::serialize_to_value(&element) {
        Ok(v) => v,
        Err(e) => raise_error!("ERR_CODEGEN_SERIALIZATION_FAILED", error = e.to_string()),
    };

    let transformer = get_transformer(target_domain);
    match transformer.transform(&element_json) {
        Ok(result) => Ok(result),
        Err(e) => raise_error!("ERR_DATA_TRANSFORMATION_FAILED", error = e.to_string()),
    }
}

pub async fn auto_tag_module(
    module_handle: &str,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<usize> {
    let manager = CollectionsManager::new(storage, domain, db);
    let mut module_doc = match manager.get_document("modules", module_handle).await {
        Ok(Some(doc)) => doc,
        Ok(None) => raise_error!(
            "ERR_CODEGEN_MODULE_NOT_FOUND",
            error = format!("Le module '{}' est introuvable.", module_handle)
        ),
        Err(e) => raise_error!("ERR_CODEGEN_MODULE_DB_ERROR", error = e.to_string()),
    };

    if is_test_mode {
        redirect_module_to_sandbox(&mut module_doc);
    }

    let domain_root = resolve_domain_root(is_test_mode);
    let service = CodeGeneratorService::new(domain_root, &manager).await?;

    // 🎯 Appel délégué au service étendu
    service.auto_tag_module(module_doc).await
}

pub async fn ingest_module(
    module_handle: &str,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<usize> {
    let manager = CollectionsManager::new(storage, domain, db);
    let mut module_doc = match manager.get_document("modules", module_handle).await {
        Ok(Some(doc)) => doc,
        Ok(None) => raise_error!(
            "ERR_CODEGEN_MODULE_NOT_FOUND",
            error = format!("Le module '{}' est introuvable.", module_handle)
        ),
        Err(e) => raise_error!("ERR_CODEGEN_MODULE_DB_ERROR", error = e.to_string()),
    };

    if is_test_mode {
        redirect_module_to_sandbox(&mut module_doc);
    }

    let domain_root = resolve_domain_root(is_test_mode);
    let mut service = CodeGeneratorService::new(domain_root, &manager).await?;
    if is_test_mode {
        service = service.with_test_mode();
    }

    // 🎯 Appel délégué au service étendu
    service.ingest_module(module_doc, &manager).await
}

pub async fn weave_module(
    module_handle: &str,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<StagedModule> {
    // 🎯 NOUVEAU RETOUR
    let manager = CollectionsManager::new(storage, domain, db);
    let mut module_doc = match manager.get_document("modules", module_handle).await {
        Ok(Some(doc)) => doc,
        Ok(None) => raise_error!(
            "ERR_CODEGEN_MODULE_NOT_FOUND",
            error = format!("Le module '{}' est introuvable.", module_handle)
        ),
        Err(e) => raise_error!("ERR_CODEGEN_MODULE_DB_ERROR", error = e.to_string()),
    };

    if is_test_mode {
        redirect_module_to_sandbox(&mut module_doc);
    }

    let domain_root = resolve_domain_root(is_test_mode);
    let mut service = CodeGeneratorService::new(domain_root, &manager).await?;
    if is_test_mode {
        service = service.with_test_mode();
    }

    // 🎯 On retourne directement le contrat de Staging (sans faire de match pour préserver l'erreur d'origine)
    service.weave_module(module_doc, &manager).await
}
/// 🏗️ STAGE : Génère et persiste le contrat temporaire (Internalise ModuleWeaver)
pub async fn stage_module(
    module_handle: &str,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<String> {
    // 1. Instanciation du manager de collections
    let manager = CollectionsManager::new(storage, domain, db);

    // 2. Génération via le weaver
    let staged = weave_module(module_handle, domain, db, storage, is_test_mode).await?;

    // 3. Persistance interne en passant la référence du manager
    ModuleWeaver::persist_stage(&manager, &staged, "agent_orchestrator").await?;

    Ok(staged.temp_path.to_string_lossy().to_string())
}

/// 🚀 COMMIT : Charge et intègre le contrat persisté (Internalise ModuleWeaver)
pub async fn commit_module(
    module_handle: &str,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<String> {
    // 1. Instanciation du manager de collections
    let manager = CollectionsManager::new(storage, domain, db);

    // 2. Chargement du contrat en passant la référence du manager
    let staged = ModuleWeaver::load_stage(&manager, module_handle).await?;

    // 3. Intégration
    commit_staged_module(staged, domain, db, storage, is_test_mode).await
}

pub async fn commit_staged_module(
    staged: StagedModule,
    domain: &str,
    db: &str,
    storage: &StorageEngine,
    is_test_mode: bool,
) -> RaiseResult<String> {
    let manager = CollectionsManager::new(storage, domain, db);

    let domain_root = resolve_domain_root(is_test_mode);
    let mut service = CodeGeneratorService::new(domain_root, &manager).await?;
    if is_test_mode {
        service = service.with_test_mode();
    }

    let final_path = service.commit_staged_module(staged, &manager).await?;
    Ok(final_path.to_string_lossy().to_string())
}

pub async fn link_module(
    module_handle: &str, // 🎯 L'argument restreignant l'analyse
    domain: &str,
    db: &str,
    storage: &StorageEngine,
) -> RaiseResult<usize> {
    use crate::code_generator::analyzers::dependency_analyzer::DependencyAnalyzer;

    let manager = CollectionsManager::new(storage, domain, db);
    let analyzer = DependencyAnalyzer::new(&manager);

    let resolved_count = analyzer.link_module("code_elements", module_handle).await?;

    Ok(resolved_count)
}
