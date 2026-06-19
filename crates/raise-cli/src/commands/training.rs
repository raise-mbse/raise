// FICHIER : crates/raise-cli/src/commands/training.rs

use clap::{Args, Subcommand};
use raise_core::{user_error, user_info, user_success, utils::prelude::*};

// Import du contexte global CLI
use crate::CliContext;
use raise_core::services::training_service;

/// Commandes dédiées à l'entraînement des modèles d'IA (GNN, LoRA, World Model)
#[derive(Args, Debug, Clone)]
pub struct TrainingArgs {
    #[command(subcommand)]
    pub command: TrainingCommands,
}

#[derive(Subcommand, Debug, Clone)]
pub enum TrainingCommands {
    /// 🧠 Entraîne un adaptateur LoRA (GNN) pour un domaine spécifique
    #[command(visible_alias = "lora")]
    Gnn {
        /// Forcer le domaine à entraîner (écrase la config utilisateur)
        #[arg(short, long)]
        domain: Option<String>,

        /// Forcer la DB à utiliser
        #[arg(long)]
        db: Option<String>,

        /// Nombre d'époques d'entraînement
        #[arg(short, long, default_value = "3")]
        epochs: usize,

        /// Taux d'apprentissage (Learning Rate)
        #[arg(long, default_value = "0.001")]
        lr: f64,
    },

    /// 🌍 Entraîne le Moteur Neuro-Symbolique (World Model)
    #[command(visible_alias = "wm")]
    WorldModel {
        /// Nombre d'itérations d'entraînement
        #[arg(short, long, default_value = "50")]
        iterations: usize,
    },
}

pub async fn handle(args: TrainingArgs, ctx: CliContext) -> RaiseResult<()> {
    // 🎯 Heartbeat de session global
    let _ = ctx.session_mgr.touch().await;

    match args.command {
        // =========================================================================
        // 1. ENTRAÎNEMENT GNN + LORA
        // =========================================================================
        TrainingCommands::Gnn {
            domain,
            db: target_db,
            epochs,
            lr,
        } => {
            let final_domain = domain.unwrap_or_else(|| ctx.active_domain.clone());
            let final_db = target_db.unwrap_or_else(|| ctx.active_db.clone());

            user_info!(
                "AI_TRAINING_START",
                json_value!({
                    "engine": "GNN+LoRA", "domain": final_domain, "db": final_db, "lr": lr, "epochs": epochs
                })
            );

            println!(
                "⏳ Lancement de l'entraînement GNN pour le domaine '{}'...",
                final_domain
            );

            // 🎯 L'appel devient une simple ligne vers le service
            match training_service::train_domain(
                ctx.storage.as_ref(),
                &final_domain,
                &final_db,
                &final_domain,
                epochs,
                lr,
            )
            .await
            {
                Ok(out_path) => {
                    user_success!("AI_TRAIN_SUCCESS", json_value!({ "path": &out_path }));
                    println!("✅ Entraînement GNN terminé.");
                    println!("📁 Adaptateur LoRA sauvegardé dans : {}", out_path);
                }
                Err(e) => {
                    user_error!("AI_TRAIN_FAIL", json_value!({ "error": e.to_string() }));
                }
            }
        }

        // =========================================================================
        // 2. ENTRAÎNEMENT WORLD MODEL (NEURO-SYMBOLIQUE)
        // =========================================================================
        TrainingCommands::WorldModel { iterations } => {
            user_info!(
                "AI_WORLD_TRAIN_START",
                json_value!({"iterations": iterations})
            );

            println!("⏳ Réveil de l'Orchestrateur et du Moteur Neuro-Symbolique...");
            println!("\n🌍 --- ENTRAÎNEMENT DU WORLD MODEL ---");
            println!("🧠 Scénario : Apprentissage de la transition d'un composant Logique (LA) vers Physique (PA).");

            // 🎯 L'appel devient une simple ligne vers le service
            match training_service::train_world_model(
                ctx.storage.clone(),
                &ctx.active_domain,
                &ctx.active_db,
                iterations,
                ctx.kernel.native_llm.clone(),
            )
            .await
            {
                Ok((initial_loss, final_loss)) => {
                    println!("\n✅ Entraînement terminé !");
                    println!("   Loss Initiale : {:.6}", initial_loss);
                    println!("   Loss Finale   : {:.6}", final_loss);

                    if initial_loss > 0.0 {
                        let improvement = ((initial_loss - final_loss) / initial_loss) * 100.0;
                        println!("   Amélioration  : {:.2}%", improvement);
                    }

                    user_success!(
                        "AI_WORLD_TRAIN_SUCCESS",
                        json_value!({"final_loss": final_loss})
                    );
                }
                Err(e) => {
                    user_error!(
                        "AI_WORLD_TRAIN_FAIL",
                        json_value!({ "error": e.to_string() })
                    );
                }
            }
        }
    }

    Ok(())
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct TestCli {
        #[command(flatten)]
        args: TrainingArgs,
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_training_gnn_parsing() -> RaiseResult<()> {
        let cli = match TestCli::try_parse_from(vec![
            "test", "gnn", "--domain", "safety", "--epochs", "10",
        ]) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE", error = e.to_string()),
        };

        if let TrainingCommands::Gnn {
            domain,
            epochs,
            db,
            lr,
        } = cli.args.command
        {
            assert_eq!(domain.unwrap(), "safety");
            assert_eq!(epochs, 10);
            assert!(db.is_none());
            assert_eq!(lr, 0.001); // Valeur par défaut
            Ok(())
        } else {
            raise_error!("ERR_TEST_ASSERTION", error = "Mauvais parsing")
        }
    }
}
