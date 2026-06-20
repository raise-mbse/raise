// FICHIER : crates/raise-cli/src/commands/devops.rs

use clap::{Args, Subcommand};
use raise_core::{raise_error, user_info, user_success, utils::prelude::*};

use crate::CliContext;
use raise_core::services::devops_service::{self, DevopsExecutionContext};

#[derive(Args, Clone, Debug)]
pub struct DevopsArgs {
    #[command(subcommand)]
    pub command: DevopsCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum DevopsCommands {
    /// 🚀 Déploie un artefact sur une cible Edge et valide sa stabilité (Auto-Remédiation)
    Deploy {
        /// Le handle du composant cible (ex: raise-edge-node)
        #[arg(long, short = 't')]
        target_handle: String,

        /// L'architecture matérielle cible
        #[arg(long, short = 'a', default_value = "aarch64-unknown-linux-gnu")]
        target_architecture: String,

        /// L'URI de l'artefact à rapatrier depuis le Knowledge Graph (ex: URN)
        #[arg(long, short = 'u')]
        payload_uri: String,
    },

    /// 🔄 Restaure un service vers un commit stable précédent
    Rollback {
        /// Le handle du composant cible (ex: raise-edge-node)
        #[arg(long, short = 't')]
        target_handle: String,

        /// L'identifiant du commit de restauration
        #[arg(long, short = 'c')]
        fallback_commit: String,
    },

    /// 🩺 Affiche l'état de synchronisation et de télémétrie d'un service Edge
    Status {
        /// Le handle du composant cible
        #[arg(long, short = 't')]
        target_handle: String,
    },

    /// 📋 Récupère les journaux et traces de diagnostic du déploiement
    Logs {
        /// Le handle du composant cible
        #[arg(long, short = 't')]
        target_handle: String,
    },
}

pub async fn handle(args: DevopsArgs, ctx: CliContext) -> RaiseResult<()> {
    let _ = ctx.session_mgr.touch().await;

    let current_session = ctx.session_mgr.get_current_session().await;
    let session_id = current_session
        .as_ref()
        .map(|s| s.id.clone())
        .unwrap_or_else(|| format!("cli_devops_{}", ctx.active_user));

    let devops_ctx = DevopsExecutionContext {
        domain: &ctx.active_domain,
        db: &ctx.active_db,
        storage: ctx.storage.clone(),
        native_llm: ctx.kernel.native_llm.clone(),
        session_id: &session_id,
        is_test_mode: ctx.is_test_mode,
    };

    match args.command {
        DevopsCommands::Deploy {
            target_handle,
            target_architecture,
            payload_uri,
        } => {
            user_info!(
                "CLI_DEVOPS_DEPLOY_INIT",
                json_value!({
                    "target": target_handle,
                    "architecture": target_architecture,
                    "payload": payload_uri
                })
            );

            match devops_service::deploy_edge_artifact(
                &target_handle,
                &target_architecture,
                &payload_uri,
                devops_ctx,
            )
            .await
            {
                Ok(msg) => {
                    println!("\n✅ DÉPLOIEMENT RÉUSSI :");
                    println!("{}", msg);
                    user_success!(
                        "CLI_DEVOPS_DEPLOY_SUCCESS",
                        json_value!({"target": target_handle})
                    );
                }
                Err(e) => raise_error!("ERR_CLI_DEVOPS_DEPLOY_FAILED", error = e),
            }
        }

        DevopsCommands::Rollback {
            target_handle,
            fallback_commit,
        } => {
            user_info!(
                "CLI_DEVOPS_ROLLBACK_INIT",
                json_value!({
                    "target": target_handle,
                    "commit": fallback_commit
                })
            );

            match devops_service::rollback_deployment(&target_handle, &fallback_commit, devops_ctx)
                .await
            {
                Ok(msg) => {
                    println!("\n⏪ RESTAURATION INITIÉE :");
                    println!("{}", msg);
                    user_success!(
                        "CLI_DEVOPS_ROLLBACK_SUCCESS",
                        json_value!({"target": target_handle})
                    );
                }
                Err(e) => raise_error!("ERR_CLI_DEVOPS_ROLLBACK_FAILED", error = e),
            }
        }

        DevopsCommands::Status { target_handle } => {
            match devops_service::get_service_status(&target_handle, devops_ctx).await {
                Ok(status_json) => {
                    println!("\n📊 ÉTAT DE SURFACE DU COMPOSANT EDGE :");
                    // 🎯 FIX : Utilisation du chemin absolu raise_core au lieu de crate
                    println!(
                        "{}",
                        raise_core::utils::data::serialize_to_string_pretty(&status_json)
                            .unwrap_or_default()
                    );
                    user_success!(
                        "CLI_DEVOPS_STATUS_SUCCESS",
                        json_value!({"target": target_handle})
                    );
                }
                Err(e) => raise_error!("ERR_CLI_DEVOPS_STATUS_FAILED", error = e),
            }
        }

        DevopsCommands::Logs { target_handle } => {
            match devops_service::get_service_logs(&target_handle, devops_ctx).await {
                Ok(logs) => {
                    println!("\n📋 JOURNAUX DE DIAGNOSTIC SRE (STAGING) :");
                    println!("--------------------------------------------------");
                    println!("{}", logs);
                    println!("--------------------------------------------------");
                    user_success!(
                        "CLI_DEVOPS_LOGS_SUCCESS",
                        json_value!({"target": target_handle})
                    );
                }
                Err(e) => raise_error!("ERR_CLI_DEVOPS_LOGS_FAILED", error = e),
            }
        }
    }

    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Validation du Parsing CLI)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: DevopsArgs,
    }

    #[test]
    fn test_cli_devops_deploy_parsing() {
        let args = vec![
            "raise",
            "deploy",
            "--target-handle",
            "raise-edge-node",
            "--payload-uri",
            "urn:raise:artifact:v1",
        ];

        let cli = TestCli::parse_from(args);

        match cli.args.command {
            DevopsCommands::Deploy {
                target_handle,
                target_architecture,
                payload_uri,
            } => {
                assert_eq!(target_handle, "raise-edge-node");
                assert_eq!(payload_uri, "urn:raise:artifact:v1");
                assert_eq!(target_architecture, "aarch64-unknown-linux-gnu");
            }
            _ => panic!("Le parsing aurait dû résoudre un DevopsCommands::Deploy"),
        }
    }

    #[test]
    fn test_cli_devops_status_parsing() {
        let args = vec!["raise", "status", "-t", "raise-edge-node"];
        let cli = TestCli::parse_from(args);

        match cli.args.command {
            DevopsCommands::Status { target_handle } => {
                assert_eq!(target_handle, "raise-edge-node");
            }
            _ => panic!("Le parsing aurait dû résoudre un DevopsCommands::Status"),
        }
    }
}
