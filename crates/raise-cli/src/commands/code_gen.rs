// FICHIER : crates/raise-cli/src/commands/code_gen.rs

use clap::{Args, Subcommand, ValueEnum};
use raise_core::{user_info, user_success, utils::prelude::*};

// 🎯 Imports sémantiques depuis la forge logicielle
use crate::CliContext;
use raise_core::code_generator::models::TargetLanguage;
use raise_core::services::codegen_service;

#[derive(Args, Clone, Debug)]
pub struct CodeGenArgs {
    #[command(subcommand)]
    pub command: CodeGenCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum CodeGenCommands {
    InitSandbox,
    Generate {
        element_id: String,
        #[arg(short, long, value_enum)]
        lang: CliTargetLanguage,
        #[arg(short, long)]
        out_dir: Option<String>,
    },
    AutoTag {
        module_handle: String,
    },
    Ingest {
        module_handle: String,
    },
    LinkModule {
        module_handle: String,
    },
    Stage {
        module_handle: String,
    },
    Commit {
        staged_handle: String,
    },
    Weave {
        module_handle: String,
    },
    /// Génère un module de validation Rust à partir d'une contrainte MBSE
    GenerateValidator {
        constraint_handle: String,
    },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum, Debug)]
pub enum CliTargetLanguage {
    Rust,
    Typescript,
    Cpp,
    Verilog,
    Vhdl,
    Python,
}

impl From<CliTargetLanguage> for TargetLanguage {
    fn from(lang: CliTargetLanguage) -> Self {
        match lang {
            CliTargetLanguage::Rust => TargetLanguage::Rust,
            CliTargetLanguage::Typescript => TargetLanguage::TypeScript,
            CliTargetLanguage::Cpp => TargetLanguage::Cpp,
            CliTargetLanguage::Verilog => TargetLanguage::Verilog,
            CliTargetLanguage::Vhdl => TargetLanguage::Vhdl,
            CliTargetLanguage::Python => TargetLanguage::Python,
        }
    }
}

pub async fn handle(args: CodeGenArgs, ctx: CliContext) -> RaiseResult<()> {
    let _ = ctx.session_mgr.touch().await;

    match args.command {
        CodeGenCommands::InitSandbox => {
            codegen_service::init_sandbox_workspace().await?;
        }

        CodeGenCommands::Generate {
            element_id,
            lang,
            out_dir: _,
        } => {
            let target: TargetLanguage = lang.into();

            // 🎯 NOUVEAU : Déduction du domaine cible à partir du langage demandé
            let target_domain_str = match lang {
                CliTargetLanguage::Verilog | CliTargetLanguage::Vhdl => "hardware",
                _ => "software",
            };

            user_info!(
                "FORGE_GENERATE_INIT",
                json_value!({
                    "element_id": element_id,
                    "language": format!("{:?}", target),
                    "target_domain": target_domain_str,
                    "workspace_domain": ctx.active_domain
                })
            );

            match codegen_service::generate_source_code(
                &element_id,
                target_domain_str,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
            )
            .await
            {
                Ok(_) => user_success!("FORGE_SUCCESS", json_value!({"element": element_id})),
                Err(e) => raise_error!("ERR_FORGE_FAILED", error = e),
            }
        }

        CodeGenCommands::AutoTag { module_handle } => {
            // ⚠️ N'oublie pas de changer 'path' en 'module_handle' dans ton enum CodeGenCommands plus haut !
            user_info!(
                "CODE_AUTOTAG_START",
                json_value!({ "module": module_handle })
            );

            match codegen_service::auto_tag_module(
                &module_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(count) => {
                    if count > 0 {
                        user_success!(
                            "CODE_AUTOTAG_SUCCESS",
                            json_value!({ "module": module_handle, "tags_added": count })
                        );
                    } else {
                        user_info!(
                            "CODE_AUTOTAG_SKIPPED",
                            json_value!({ "module": module_handle, "hint": "Déjà synchronisé." })
                        );
                    }
                }
                Err(e) => raise_error!("ERR_AUTOTAG_FAILED", error = e),
            }
        }

        CodeGenCommands::Ingest { module_handle } => {
            user_info!(
                "CODE_INGEST_START",
                json_value!({ "module": module_handle })
            );

            match codegen_service::ingest_module(
                &module_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(count) => user_success!(
                    "CODE_INGEST_SUCCESS",
                    json_value!({ "module": module_handle, "elements_ingested": count })
                ),
                Err(e) => raise_error!("ERR_INGEST_FAILED", error = e),
            }
        }

        CodeGenCommands::LinkModule { module_handle } => {
            user_info!("CODE_LINK_START", json_value!({ "module": module_handle }));

            match codegen_service::link_module(
                &module_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
            )
            .await
            {
                Ok(count) => user_success!(
                    "CODE_LINK_SUCCESS",
                    json_value!({ "relations_resolved": count, "module": module_handle })
                ),
                Err(e) => raise_error!("ERR_CODE_LINK_FAILED", error = e),
            }
        }

        CodeGenCommands::Stage { module_handle } => {
            match codegen_service::stage_module(
                &module_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(path) => user_success!("CODE_STAGE_SUCCESS", json_value!({"path": path})),
                Err(e) => raise_error!(
                    "ERR_STAGE_FAILED",
                    error = e,
                    context = json_value!({"module": module_handle})
                ),
            }
        }

        CodeGenCommands::Commit { staged_handle } => {
            match codegen_service::commit_module(
                &staged_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(final_path) => {
                    user_success!("CODE_COMMIT_SUCCESS", json_value!({"path": final_path}))
                }
                Err(e) => raise_error!(
                    "ERR_COMMIT_FAILED",
                    error = e,
                    context = json_value!({"staged_handle": staged_handle})
                ),
            }
        }

        CodeGenCommands::Weave { module_handle } => {
            user_info!("CODE_WEAVE_START", json_value!({ "module": module_handle }));

            match codegen_service::weave_module(
                &module_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(final_path) => user_success!(
                    "CODE_WEAVE_SUCCESS",
                    json_value!({ "module": module_handle, "final_path": final_path })
                ),
                Err(e) => raise_error!("ERR_WEAVE_FAILED", error = e),
            }
        }

        CodeGenCommands::GenerateValidator { constraint_handle } => {
            user_info!(
                "CODE_GEN_VALIDATOR_START",
                json_value!({ "constraint": constraint_handle })
            );

            // Appel de la fonction que nous avons écrite à l'Étape 5
            match codegen_service::generate_constraint_validator(
                &constraint_handle,
                &ctx.active_domain,
                &ctx.active_db,
                &ctx.storage,
                ctx.is_test_mode,
            )
            .await
            {
                Ok(path) => {
                    user_success!(
                        "CODE_GEN_VALIDATOR_SUCCESS",
                        json_value!({ "constraint": constraint_handle, "path": path })
                    );
                }
                Err(e) => raise_error!("ERR_GEN_VALIDATOR_FAILED", error = e),
            }
        }
    }
    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Conformité & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CliContext;
    use raise_core::json_db::collections::manager::CollectionsManager;
    use raise_core::json_db::storage::StorageEngine;
    use raise_core::kernel::state::RaiseKernelState;
    use raise_core::utils::context::SessionManager;
    use raise_core::utils::testing::{AgentDbSandbox, DbSandbox};

    /// 🎯 Injection stricte dans la partition _system, peu importe le domaine actif
    async fn inject_mock_codegen_config(storage: &SharedRef<StorageEngine>) -> RaiseResult<()> {
        let config = AppConfig::get();
        let sys_manager = CollectionsManager::new(
            storage.as_ref(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );
        let _ = DbSandbox::mock_db(&sys_manager).await;

        let _ = sys_manager
            .create_collection("components", &generic_schema)
            .await;
        let _ = sys_manager
            .create_collection("service_configs", &generic_schema)
            .await;
        let _ = sys_manager
            .create_collection("configs", &generic_schema)
            .await;

        sys_manager.upsert_document("components", json_value!({ "_id": "ref:components:handle:codegen_engine", "handle": "codegen_engine" })).await?;

        sys_manager.upsert_document("service_configs", json_value!({
            "_id": "mock_codegen",
            "component_id": "ref:components:handle:codegen_engine",
            "service_settings": {
                "format_on_save": true,
                "strict_mode": true,
                "semantic_routing": {
                    "software": { "aliases": ["rust", "cpp", "ts", "rs"], "collection": "code_elements", "schema_uri": generic_schema.clone() },
                    "doc": { "aliases": ["md"], "collection": "doc_elements", "schema_uri": generic_schema.clone() }
                }
            }
        })).await?;

        sys_manager
            .upsert_document(
                "configs",
                json_value!({
                    "_id": "ref:configs:handle:ontological_mapping",
                    "search_spaces": [ { "layer": "mock_db", "collection": "components" } ]
                }),
            )
            .await?;

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_codegen_cli_dispatch() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = SessionManager::new(storage.clone());

        let ctx = CliContext::mock(AppConfig::get(), session_mgr, storage.clone());
        let manager = CollectionsManager::new(&ctx.storage, &ctx.active_domain, &ctx.active_db);

        inject_mock_codegen_config(&ctx.storage).await?;
        let _ = DbSandbox::mock_db(&manager).await;

        manager
            .create_collection(
                "components",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let mock_component = json_value!({
            "_id": "sa:Processor_A",
            "handle": "Processor_A",
            "name": "Processor A",
            "type": "SystemComponent"
        });
        manager
            .upsert_document("components", mock_component)
            .await?;

        let test_out_dir = sandbox
            .storage
            .config
            .data_root
            .to_string_lossy()
            .to_string();

        let args = CodeGenArgs {
            command: CodeGenCommands::Generate {
                element_id: "sa:Processor_A".into(),
                lang: CliTargetLanguage::Rust,
                out_dir: Some(test_out_dir),
            },
        };

        handle(args, ctx).await
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_cli_ingest_and_weave_full_cycle() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;

        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = SessionManager::new(storage.clone());
        let ctx = CliContext::mock(AppConfig::get(), session_mgr, storage.clone());
        let manager = CollectionsManager::new(&ctx.storage, &ctx.active_domain, &ctx.active_db);

        inject_mock_codegen_config(&ctx.storage).await?;
        let _ = DbSandbox::mock_db(&manager).await;

        manager
            .create_collection(
                "code_elements",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;
        manager
            .create_collection(
                "staged_contracts",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        let file_path = sandbox.storage.config.data_root.join("test_weave.rs");
        let initial_code = "// @raise-handle: fn:test_fn\npub fn test_fn() { }";
        fs::write_sync(&file_path, initial_code)
            .map_err(|e| build_error!("ERR_TEST_FS", error = e))?;

        manager
            .create_collection(
                "modules",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;

        manager
            .insert_raw(
                "modules",
                &json_value!({
                    "_id": "ref:modules:handle:mod_test_weave",
                    "handle": "mod_test_weave",
                    "path": file_path.to_string_lossy().to_string() // 🎯 Ligne 318 corrigée ici
                }),
            )
            .await?;

        let args_ingest = CodeGenArgs {
            command: CodeGenCommands::Ingest {
                module_handle: "mod_test_weave".to_string(),
            },
        };
        handle(args_ingest, ctx.clone()).await?;

        let query = raise_core::json_db::query::Query::new("code_elements");
        let db_result = raise_core::json_db::query::QueryEngine::new(&manager)
            .execute_query(query)
            .await?;

        if db_result.documents.is_empty() {
            raise_error!(
                "ERR_TEST_EMPTY_DB",
                error = "L'ingestion n'a créé aucun document."
            );
        }

        let mut doc = db_result.documents[0].clone();
        doc["body"] = json_value!("{ println!(\"RAISE_FORGE_OK\"); }");
        manager.upsert_document("code_elements", doc).await?;

        let args_stage = CodeGenArgs {
            command: CodeGenCommands::Stage {
                module_handle: "mod_test_weave".to_string(),
            },
        };
        handle(args_stage, ctx.clone()).await?;

        let args_commit = CodeGenArgs {
            command: CodeGenCommands::Commit {
                staged_handle: "mod_test_weave".to_string(),
            },
        };
        handle(args_commit, ctx.clone()).await?;

        let final_code = fs::read_to_string_sync(&file_path)
            .map_err(|e| build_error!("ERR_TEST_FS", error = e))?;
        if !final_code.contains("RAISE_FORGE_OK") {
            raise_error!(
                "ERR_TEST_FORGE_FAIL",
                error = "Le tissage du code a échoué (le commit n'a pas mis à jour le fichier)."
            );
        }

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_json_schema_workflow_integrity() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 Initialisation de sys_manager pour cibler proprement la DB système
        let sys_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let ctx = CliContext {
            config,
            session_mgr: SessionManager::new(sandbox.db.clone()),
            storage: sandbox.db.clone(),
            kernel: RaiseKernelState {
                orchestrator: None,
                native_llm: None,
                code_generator: None,
            },
            active_user: "tester".to_string(),
            active_domain: config.mount_points.system.domain.clone(),
            active_db: config.mount_points.system.db.clone(),
            is_test_mode: true,
            is_simulation: false,
            sim_domain: "".to_string(),
            sim_db: "".to_string(),
        };

        let schemas_dir = sandbox.domain_root.join("schemas/tools/inputs");
        fs::ensure_dir_async(&schemas_dir).await?;

        let schema_path = schemas_dir.join("blender_input.schema.json");
        let initial_schema = r#"{
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "type": "object",
            "properties": {
                "defect_type": { "type": "string" }
            }
        }"#;
        fs::write_async(&schema_path, initial_schema).await?;

        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );
        sys_manager
            .create_collection("modules", &generic_schema)
            .await?;
        sys_manager
            .create_collection("json_schema_elements", &generic_schema)
            .await?;
        sys_manager
            .create_collection("staged_contracts", &generic_schema)
            .await?;
        sys_manager
            .create_collection("service_configs", &generic_schema)
            .await?;
        sys_manager
            .create_collection("components", &generic_schema)
            .await?;

        sys_manager
            .upsert_document(
                "components",
                json_value!({
                    "_id": "ref:components:handle:codegen_engine",
                    "handle": "codegen_engine"
                }),
            )
            .await?;

        sys_manager
            .upsert_document(
                "service_configs",
                json_value!({
                    "_id": "cfg_codegen_engine_master",
                    "component_id": "ref:components:handle:codegen_engine",
                    "service_settings": {
                        "format_on_save": false,
                        "strict_mode": true,
                        "semantic_routing": {
                            "schema": {
                                "aliases": ["json"],
                                "collection": "json_schema_elements",
                                "schema_uri": &generic_schema
                            }
                        }
                    }
                }),
            )
            .await?;

        let module_handle = "mod_schema_blender_input";
        sys_manager
            .upsert_document(
                "modules",
                json_value!({
                    "_id": format!("ref:modules:handle:{}", module_handle),
                    "handle": module_handle,
                    "path": schema_path.to_string_lossy().to_string(),
                    "domain": "schema"
                }),
            )
            .await?;

        // 4. ÉTAPE 1 : INGESTION
        let args_ingest = CodeGenArgs {
            command: CodeGenCommands::Ingest {
                module_handle: module_handle.to_string(),
            },
        };
        handle(args_ingest, ctx.clone()).await?;

        let query = raise_core::json_db::query::Query::new("json_schema_elements");
        let db_result = raise_core::json_db::query::QueryEngine::new(&sys_manager)
            .execute_query(query)
            .await?;

        if db_result.documents.is_empty() {
            raise_error!(
                "ERR_TEST_EMPTY_DB",
                error = "L'ingestion du schéma JSON a échoué."
            );
        }

        // 5. ÉTAPE 2 : MUTATION IA
        let mut doc = db_result.documents[0].clone();
        doc["content"]["properties"]["resolution_x"] =
            json_value!({ "type": "integer", "minimum": 1 });
        sys_manager
            .upsert_document("json_schema_elements", doc)
            .await?;

        // 6. ÉTAPE 3 : STAGING
        let args_stage = CodeGenArgs {
            command: CodeGenCommands::Stage {
                module_handle: module_handle.to_string(),
            },
        };
        handle(args_stage, ctx.clone()).await?;

        // 7. ÉTAPE 4 : COMMIT
        let args_commit = CodeGenArgs {
            command: CodeGenCommands::Commit {
                staged_handle: module_handle.to_string(),
            },
        };
        handle(args_commit, ctx.clone()).await?;

        // 8. VALIDATION FINALE
        let final_schema_str = fs::read_to_string_async(&schema_path).await?;

        assert!(
            final_schema_str.contains("resolution_x"),
            "Le schéma final ne contient pas la mutation appliquée par l'IA."
        );
        assert!(
            final_schema_str.contains("https://json-schema.org/draft/2020-12/schema"),
            "Le tisseur a perdu la norme JSON Schema en cours de route."
        );

        Ok(())
    }
}
