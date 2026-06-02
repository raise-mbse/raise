// FICHIER : src-tauri/tools/raise-cli/src/commands/utils.rs

use clap::{Args, Subcommand};
use raise_core::json_db::collections::manager::CollectionsManager;
use raise_core::json_db::query::{Condition, FilterOperator, Query, QueryEngine, QueryFilter};
use raise_core::utils::prelude::*; // 🎯 Façade Unique RAISE

// 🎯 Import du contexte global CLI
use crate::CliContext;

/// Outils de maintenance et de gestion de session pour RAISE.
#[derive(Args, Clone, Debug)]
pub struct UtilsArgs {
    #[command(subcommand)]
    pub command: UtilsCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum UtilsCommands {
    /// Affiche la configuration active, le statut session et les moteurs
    Info,
    /// Vérifie la connectivité interne (Ping)
    Ping,
    /// Affiche l'identité de l'utilisateur actuellement connecté
    Whoami,
    /// Se connecter avec un identifiant utilisateur (Force une nouvelle session)
    Login { userhandle: String },
    /// Ferme la session actuelle
    Logout,
    /// Gestion de la configuration utilisateur
    Config {
        #[arg(default_value = "show")]
        action: String,
        key: Option<String>,
        value: Option<String>,
    },
    /// Bascule sur un autre domaine
    UseDomain { domain: String },
    /// Bascule sur une autre base de données
    UseDb { db: String },
}

pub async fn handle(args: UtilsArgs, ctx: CliContext) -> RaiseResult<()> {
    // 🎯 Heartbeat de session : Traitement propre de l'erreur
    if let Err(e) = ctx.session_mgr.touch().await {
        user_error!(
            "ERR_SESSION_HEARTBEAT",
            json_value!({"error": e.to_string()})
        );
    }

    match args.command {
        UtilsCommands::Info => {
            user_info!(
                "CLI_INFO_HEADER",
                json_value!({ "header": "RAISE SYSTEM INFO" })
            );

            if let Some(session) = ctx.session_mgr.get_current_session().await {
                user_info!(
                    "CLI_SESSION_ACTIVE",
                    json_value!({
                        "user": session.user_id,
                        "domain": session.current_domain,
                        "db": session.current_db
                    })
                );
            } else {
                user_warn!("CLI_SESSION_INACTIVE", json_value!({}));
            }

            user_info!(
                "APP_VERSION",
                json_value!({
                    "version": env!("CARGO_PKG_VERSION"),
                    "mode": if cfg!(debug_assertions) { "debug" } else { "release" }
                })
            );

            // Vérification du moteur LLM
            let manager = CollectionsManager::new(&ctx.storage, &ctx.active_domain, &ctx.active_db);
            if let Ok(settings) =
                AppConfig::get_runtime_settings(&manager, "ref:components:handle:ai_llm").await
            {
                user_info!(
                    "LLM_ENGINE_STATUS",
                    json_value!({
                        "model": settings.get("rust_model_file").and_then(|v| v.as_str()).unwrap_or("default"),
                        "active": true
                    })
                );
            }
        }

        UtilsCommands::Ping => {
            user_success!("PONG", json_value!({ "timestamp": UtcClock::now() }));
        }

        UtilsCommands::Whoami => {
            if let Some(session) = ctx.session_mgr.get_current_session().await {
                user_info!("CURRENT_USER", json_value!({ "user": session.user_id }));
            } else {
                user_warn!("NO_ACTIVE_SESSION", json_value!({}));
            }
        }

        UtilsCommands::Login { userhandle } => {
            user_info!("AUTH_LOGIN_START", json_value!({ "user": userhandle }));
            match ctx.session_mgr.start_session(&userhandle).await {
                Ok(s) => user_success!("AUTH_SUCCESS", json_value!({ "user": s.user_id })),
                Err(e) => raise_error!("ERR_AUTH_FAILED", error = e.to_string()),
            }
        }

        UtilsCommands::Logout => {
            if ctx.session_mgr.get_current_session().await.is_some() {
                match ctx.session_mgr.end_session().await {
                    Ok(_) => user_success!("AUTH_LOGOUT", json_value!({})),
                    Err(e) => raise_error!("ERR_LOGOUT_FAIL", error = e.to_string()),
                }
            } else {
                user_warn!("LOGOUT_SKIPPED", json_value!({}));
            }
        }

        UtilsCommands::Config { action, key, value } => match action.to_lowercase().as_str() {
            "show" => {
                user_info!(
                    "CLI_CONFIG_SHOW",
                    json_value!({
                        "user": ctx.active_user,
                        "system_domain": ctx.config.mount_points.system.domain
                    })
                );
            }
            "set" => {
                let (k, v) = key.zip(value).ok_or_else(|| {
                    build_error!("ERR_CLI_USAGE", error = "Usage: config set <key> <value>")
                })?;

                let sys_mgr = CollectionsManager::new(
                    &ctx.storage,
                    &ctx.config.mount_points.system.domain,
                    &ctx.config.mount_points.system.db,
                );
                let mut query = Query::new("users");
                query.filter = Some(QueryFilter {
                    operator: FilterOperator::And,
                    conditions: vec![Condition::eq("handle", json_value!(&ctx.active_user))],
                });

                let res = QueryEngine::new(&sys_mgr).execute_query(query).await?;
                if let Some(doc) = res.documents.first() {
                    let id = doc["_id"]
                        .as_str()
                        .ok_or_else(|| build_error!("ERR_DB", error = "User id missing"))?;
                    sys_mgr
                        .update_document("users", id, json_value!({ k.clone(): v.clone() }))
                        .await?;

                    user_success!("CONFIG_UPDATED", json_value!({ "key": k, "value": v }));
                }
            }
            _ => user_warn!("CLI_USAGE", json_value!({ "hint": "Actions: show | set" })),
        },

        UtilsCommands::UseDomain { domain } => {
            let res = ctx.session_mgr.switch_domain(&domain).await?;
            user_success!("DOMAIN_SWITCHED", json_value!(res));
        }

        UtilsCommands::UseDb { db } => {
            let res = ctx.session_mgr.switch_db(&db).await?;
            user_success!("DB_SWITCHED", json_value!(res));
        }
    }
    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Conformité « Zéro Dette »)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use raise_core::utils::testing::{mock::inject_mock_user, DbSandbox};

    #[async_test]
    #[serial_test::serial]
    async fn test_session_lifecycle() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = crate::context::SessionManager::new(storage.clone());
        let ctx = crate::CliContext::mock(AppConfig::get(), session_mgr.clone(), storage);

        let test_user = "CLI-Tester";
        let db_mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        DbSandbox::mock_db(&db_mgr).await?;
        db_mgr
            .create_collection(
                "users",
                "db://_system/_system/schemas/v1/db/generic.schema.json",
            )
            .await?;
        inject_mock_user(&db_mgr, test_user).await;

        // Login
        handle(
            UtilsArgs {
                command: UtilsCommands::Login {
                    userhandle: test_user.into(),
                },
            },
            ctx.clone(),
        )
        .await?;
        assert_eq!(
            session_mgr.get_current_session().await.unwrap().user_handle,
            test_user
        );

        // Logout
        handle(
            UtilsArgs {
                command: UtilsCommands::Logout,
            },
            ctx,
        )
        .await?;
        assert!(session_mgr.get_current_session().await.is_none());

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_info_execution_integrity() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let ctx = crate::CliContext::mock(
            AppConfig::get(),
            crate::context::SessionManager::new(storage.clone()),
            storage,
        );
        handle(
            UtilsArgs {
                command: UtilsCommands::Info,
            },
            ctx,
        )
        .await
    }
}
