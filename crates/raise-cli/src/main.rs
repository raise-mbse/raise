// FICHIER : src-tauri/tools/raise-cli/src/main.rs

use clap::{Parser, Subcommand};

// On garde le module local des commandes
mod commands;
use raise_core::ai::agents::AgentContext;
use raise_core::ai::assurance::health::RaiseHealthEngine;
use raise_core::kernel::state::RaiseKernelState;
use raise_core::utils::io::os::run_cli_app;
use raise_core::{
    json_db::{collections::manager::CollectionsManager, storage::StorageEngine},
    raise_error, user_debug, user_error, user_info, user_warn,
    utils::{context, prelude::*},
};

// ============================================================================
// 🎯 DÉFINITION DU CONTEXTE GLOBAL DU CLI
// ============================================================================
#[derive(Clone)]
pub struct CliContext {
    pub config: &'static AppConfig,
    pub session_mgr: context::SessionManager,
    pub storage: SharedRef<StorageEngine>,
    pub kernel: RaiseKernelState,
    pub active_user: String,
    pub active_domain: String,
    pub active_db: String,
    pub is_test_mode: bool,
    pub is_simulation: bool,
    pub sim_domain: String,
    pub sim_db: String,
}

// ============================================================================

#[derive(Parser)]
#[command(name = "raise-cli")]
#[command(about = "CLI unifié pour la manipulation des modules Raise", long_about = None)]
#[command(version)]
struct Cli {
    #[arg(
        long,
        global = true,
        env = "RAISE_USER",
        help = "Surcharge l'utilisateur actif"
    )]
    user: Option<String>,

    #[arg(
        long,
        global = true,
        env = "RAISE_DOMAIN",
        help = "Surcharge le domaine par défaut"
    )]
    domain: Option<String>,

    #[arg(
        long,
        global = true,
        env = "RAISE_DB",
        help = "Surcharge la base de données par défaut"
    )]
    db: Option<String>,

    #[arg(
        long,
        global = true,
        env = "RAISE_SIMULATE",
        help = "Active le mode Bac à Sable (Simulation IA)"
    )]
    simulate: bool,

    #[arg(
        long,
        global = true,
        help = "Surcharge le domaine cible pour la simulation"
    )]
    sim_domain: Option<String>,

    #[arg(
        long,
        global = true,
        help = "Surcharge la base de données cible pour la simulation"
    )]
    sim_db: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Clone, Debug)]
enum Commands {
    Workflow(commands::workflow::WorkflowArgs),
    ModelEngine(commands::model_engine::ModelArgs),
    Rules(commands::rules::RulesArgs),
    Jsondb(commands::jsondb::JsondbArgs),
    Ai(commands::ai::AiArgs),
    Dl(commands::dl::DlArgs),
    Genetics(commands::genetics::GeneticsArgs),
    Blockchain(commands::blockchain::BlockchainArgs),
    Plugins(commands::plugins::PluginsArgs),
    Traceability(commands::traceability::TraceabilityArgs),
    Spatial(commands::spatial::SpatialArgs),
    CodeGen(commands::code_gen::CodeGenArgs),
    Validator(commands::validator::ValidatorArgs),
    Utils(commands::utils::UtilsArgs),
}

fn main() -> RaiseResult<()> {
    run_cli_app(async {
        // 1. INITIALISATION CONFIGURATION (CRITIQUE)
        if let Err(e) = AppConfig::init() {
            raise_error!(
                "CLI_CRITICAL_INIT_FAILED",
                error = e,
                context = json_value!({"step": "AppConfig::init"})
            );
        }

        // 2. INITIALISATION LOGGER ET LANGUE
        context::init_logging();
        let config = AppConfig::get();
        if context::init_i18n(&config.core.language).await.is_err() {
            eprintln!(
                "⚠️ [BOOTSTRAP MODE] Traductions inaccessibles. Démarrage en mode sans échec."
            );
        }

        // 3. PARSING DU CLI
        let cli = Cli::parse();

        // 4. RÉSOLUTION SÉMANTIQUE DES PRIORITÉS (CLI > Config > Mount Points)
        let active_user = match cli.user.clone() {
            Some(u) => u,
            None => match &config.user {
                Some(u) => u.id.clone(),
                None => "unknown_user".to_string(),
            },
        };

        let active_domain = match cli.domain.clone() {
            Some(d) => d,
            None => config.mount_points.system.domain.clone(),
        };

        let active_db = match cli.db.clone() {
            Some(db) => db,
            None => config.mount_points.system.db.clone(),
        };

        // 5. INITIALISATION DU MOTEUR DE STOCKAGE
        user_info!(
            "CLI_BOOTSTRAP_INIT",
            json_value!({"action": "Vérification et garantie de l'environnement physique..."})
        );

        let (node_env, needs_restart) =
            match raise_core::kernel::environment::NodeEnvironment::boot_physical_node().await {
                Ok(env) => env,
                Err(e) => raise_error!(
                    "ERR_CLI_PHYSICAL_BOOT",
                    error = e,
                    context = json_value!({"hint": "Impossible d'amorcer le nœud matériel. Vérifiez les droits d'écriture."})
                ),
            };

        if needs_restart {
            user_info!(
                "NODE_BOOT_SIGNAL",
                json_value!({"action": "Amorçage atomique complet. Terminaison du processus par le lanceur."})
            );
            terminate_process(0);
        }
        let storage = node_env.storage;
        /*
        let db_root = match config.get_path("PATH_RAISE_DOMAIN") {
            Some(path) => path,
            None => raise_error!(
                "CLI_MISSING_PATH",
                error = "PATH_RAISE_DOMAIN introuvable dans la config"
            ),
        };

        let storage = SharedRef::new(StorageEngine::new(JsonDbConfig::new(db_root))?);
        */

        // ---------------------------------------------------------
        // 🧠 INITIALISATION SÉMANTIQUE (Bootstrapping In-Index)
        // ---------------------------------------------------------

        let system_mgr = CollectionsManager::new(
            &storage,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // Le CLI délègue tout au Core : WAL, Sémantique et Rules.
        if let Err(e) = raise_core::bootstrap_core(&system_mgr).await {
            user_error!(
                "CLI_BOOTSTRAP_FAILED",
                json_value!({"error": e.to_string(), "hint": "Échec de l'initialisation des moteurs Core."})
            );
            return Err(e);
        }

        let session_mgr = context::SessionManager::new(storage.clone());

        // RÉSOLUTION DU CONTEXTE DE SIMULATION
        let is_simulation = cli.simulate;
        let sim_domain = cli.sim_domain.unwrap_or_else(|| "sim_mbse2".to_string());
        let sim_db = cli.sim_db.unwrap_or_else(|| "sim_raise".to_string());

        // =========================================================
        // 🔍 PRE-FLIGHT CHECK : TRACAGE MÉMOIRE AVANT CHARGEMENT IA
        // =========================================================
        user_debug!(
            "CLI_PRE_FLIGHT_START",
            json_value!({"action": "check_vram"})
        );

        match RaiseHealthEngine::check_engine_health(&system_mgr).await {
            Ok(report) => {
                // Extraction sécurisée des valeurs pour le log
                let vram_free = report
                    .diagnostic_details
                    .get("vram_free_mb")
                    .cloned()
                    .unwrap_or(JsonValue::Null);
                let vram_req = report
                    .diagnostic_details
                    .get("vram_required_mb")
                    .cloned()
                    .unwrap_or(JsonValue::Null);

                user_info!(
                    "CLI_HARDWARE_TRACE",
                    json_value!({
                        "device_type": report.device_type,
                        "vram_free_mb": vram_free,
                        "vram_required_mb": vram_req,
                        "acceleration": report.acceleration_active
                    })
                );
            }
            Err(e) => {
                user_warn!(
                    "CLI_HARDWARE_WARNING",
                    json_value!({
                        "error": e.to_string(),
                        "hint": "Mémoire insuffisante ou GPU inaccessible. L'Orchestrateur risque d'échouer."
                    })
                );
            }
        }

        // 🧠 INITIALISATION DE L'ORCHESTRATEUR IA
        // On utilise le manager système pour résoudre les configurations des moteurs
        let kernel_state = match raise_core::kernel::state::RaiseKernelState::boot(storage.clone())
            .await
        {
            Ok(state) => state,
            Err(e) => raise_error!(
                "ERR_CLI_KERNEL_BOOT_FAILED",
                error = e.to_string(),
                context = json_value!({"hint": "Échec critique lors du montage de la partition système."})
            ),
        };

        // 6. CRÉATION DU CONTEXTE UNIFIÉ
        let mut ctx = CliContext {
            config,
            session_mgr,
            storage,
            kernel: kernel_state,
            active_user,
            active_domain,
            active_db,
            is_test_mode: false,
            is_simulation,
            sim_domain,
            sim_db,
        };

        // 7. AUTO-LOGIN AVEC L'UTILISATEUR RÉSOLU
        if ctx.active_user == "unknown_user" {
            user_warn!(
                "CLI_GHOST_MODE",
                json_value!({"hint": "Mode restreint (Setup)."})
            );
        } else {
            match ctx.session_mgr.start_session(&ctx.active_user).await {
                Ok(session) => {
                    ctx.active_domain = session.current_domain.clone();
                    ctx.active_db = session.current_db.clone();
                    ctx.is_simulation = session.is_simulation;
                    ctx.sim_domain = session.sim_domain.clone();
                    ctx.sim_db = session.sim_db.clone();
                    user_info!(
                        "CLI_START_INITIALIZED",
                        json_value!({
                            "version": env!("CARGO_PKG_VERSION"),
                            "user": ctx.active_user,
                            "domain": ctx.active_domain
                        })
                    );
                }
                Err(e) => {
                    user_warn!(
                        "CLI_SESSION_UNAVAILABLE",
                        json_value!({"user": ctx.active_user, "error": e.to_string()})
                    );
                }
            }
        }

        // 8. DISPATCH DES COMMANDES
        match cli.command {
            Some(cmd) => match execute_command(cmd.clone(), ctx.clone()).await {
                Ok(_) => (),
                Err(e) => raise_error!(
                    "CLI_COMMAND_EXECUTION_FAILED",
                    error = e,
                    context = json_value!({"command": format!("{:?}", cmd)})
                ),
            },
            None => {
                run_global_shell(ctx).await?;
            }
        }

        user_debug!("CLI_EXECUTION_FINISHED");
        Ok(())
    })
}

/// Boucle principale du Shell Global (REPL) avec résolution Mount Points
async fn run_global_shell(mut ctx: CliContext) -> RaiseResult<()> {
    use rustyline::error::ReadlineError;
    use rustyline::DefaultEditor;

    println!("🚀 RAISE GLOBAL SHELL - v{}", env!("CARGO_PKG_VERSION"));
    println!("👤 User   : {}", ctx.active_user);
    println!(
        "🌍 Partition Système : {}/{}",
        ctx.config.mount_points.system.domain, ctx.config.mount_points.system.db
    );
    println!("--------------------------------------------------");

    let mut rl = match DefaultEditor::new() {
        Ok(editor) => editor,
        Err(e) => raise_error!("CLI_EDITOR_INIT_FAILED", error = e),
    };

    let history_path = match ctx.config.get_path("PATH_RAISE_DOMAIN") {
        Some(p) => p.join("_system/history.txt"),
        None => raise_error!("CLI_HISTORY_PATH_ERROR"),
    };

    if let Err(e) = rl.save_history(&history_path) {
        user_warn!(
            "CLI_HISTORY_SAVE_FAILED",
            json_value!({"error": e.to_string(), "path": history_path.display().to_string()})
        );
    }

    loop {
        // AUTO-SYNC : On demande au noyau la vérité absolue
        if let Some(session) = ctx.session_mgr.get_current_session().await {
            ctx.active_user = session.user_handle.clone();
            ctx.active_domain = session.current_domain.clone();
            ctx.active_db = session.current_db.clone();
            ctx.is_simulation = session.is_simulation;
            ctx.sim_domain = session.sim_domain.clone();
            ctx.sim_db = session.sim_db.clone();
        }

        let prompt = format!(
            "RAISE [{}@{}/{}]> ",
            ctx.active_user, ctx.active_domain, ctx.active_db
        );
        let readline = rl.readline(&prompt);

        match readline {
            Ok(line) => {
                let mut input = line.trim().to_string();
                if input.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(input.as_str());
                let _ = rl.save_history(&history_path);

                if input == "exit" || input == "quit" {
                    break;
                }
                if input == "clear" {
                    print!("\x1B[2J\x1B[1;1H");
                    continue;
                }

                // ALIAS UX : Traduction pour Clap
                if input.starts_with("login ") || input.starts_with("use-") {
                    input = format!("utils {}", input);
                }

                match shell_words::split(&input) {
                    Ok(args) => {
                        let mut full_args = vec!["repl".to_string()];
                        full_args.extend(args);

                        match Cli::try_parse_from(full_args) {
                            Ok(cli_repl) => {
                                if let Some(cmd) = cli_repl.command {
                                    if let Err(e) = execute_command(cmd.clone(), ctx.clone()).await
                                    {
                                        user_error!(
                                            "CLI_COMMAND_FAILED",
                                            json_value!({"error": e.to_string()})
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                e.print().ok();
                            }
                        }
                    }
                    Err(e) => {
                        user_error!("CLI_SYNTAX_ERROR", json_value!({"error": e.to_string()}))
                    }
                }
            }
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => break,
            Err(err) => {
                user_error!(
                    "CLI_SHELL_FATAL",
                    json_value!({"error": format!("{:?}", err)})
                );
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

async fn execute_command(cmd: Commands, ctx: CliContext) -> RaiseResult<()> {
    match cmd {
        Commands::Workflow(args) => commands::workflow::handle(args, ctx).await,
        Commands::ModelEngine(args) => commands::model_engine::handle(args, ctx).await,
        Commands::Rules(args) => commands::rules::handle(args, ctx).await,
        Commands::Jsondb(args) => commands::jsondb::handle(args, ctx).await,
        Commands::Ai(args) => commands::ai::handle(args, ctx).await,
        Commands::Dl(args) => commands::dl::handle(args, ctx).await,
        Commands::Genetics(args) => commands::genetics::handle(args, ctx).await,
        Commands::Blockchain(args) => commands::blockchain::handle(args, ctx).await,
        Commands::Plugins(args) => commands::plugins::handle(args, ctx).await,
        Commands::Traceability(args) => commands::traceability::handle(args, ctx).await,
        Commands::Spatial(args) => commands::spatial::handle(args, ctx).await,
        Commands::CodeGen(args) => commands::code_gen::handle(args, ctx).await,
        Commands::Validator(args) => commands::validator::handle(args, ctx).await,
        Commands::Utils(args) => commands::utils::handle(args, ctx).await,
    }
}

// ============================================================================
// 🎯 LOGIQUE MÉTIER DU CONTEXTE CLI
// ============================================================================
impl CliContext {
    /// 🎯 BRIDGE : Transforme le contexte CLI en un contexte d'Agent prêt à l'emploi.
    /// Cette méthode résout les chemins physiques et la session active pour l'IA.
    pub async fn to_agent_context(&self, agent_id: &str) -> RaiseResult<AgentContext> {
        // Extraction sécurisée de l'orchestrateur
        let orch_ref = self.kernel.orchestrator.as_ref().ok_or_else(|| {
            build_error!(
                "ERR_AI_OFFLINE",
                error = "L'orchestrateur IA n'est pas initialisé."
            )
        })?;

        let orch = orch_ref.lock().await;

        let session = self
            .session_mgr
            .get_current_session()
            .await
            .ok_or_else(|| build_error!("ERR_CLI_NO_SESSION", error = "Session active requise."))?;

        let domain_path = self.config.get_path("PATH_RAISE_DOMAIN").ok_or_else(|| {
            build_error!("ERR_CLI_CONFIG_PATH", error = "PATH_RAISE_DOMAIN manquant.")
        })?;

        let dataset_path = self
            .config
            .get_path("PATH_RAISE_DATASET")
            .unwrap_or_else(|| domain_path.join("dataset"));

        // 🎯 Forge du contexte en utilisant les moteurs de l'orchestrateur
        AgentContext::new(
            agent_id,
            &session.id,
            self.storage.clone(),
            orch.llm_remote.clone(),
            orch.world_engine.clone(),
            domain_path,
            dataset_path,
        )
        .await
    }

    #[cfg(test)]
    pub fn mock(
        config: &'static AppConfig,
        session_mgr: context::SessionManager,
        storage: SharedRef<StorageEngine>,
    ) -> Self {
        Self {
            config,
            session_mgr,
            storage,
            kernel: RaiseKernelState {
                orchestrator: None,
                native_llm: None,
                code_generator: None,
            },
            active_user: "mock_user".to_string(),
            active_domain: "mock_domain".to_string(),
            active_db: "mock_db".to_string(),
            is_test_mode: true,
            is_simulation: false,
            sim_domain: "mock_sim_domain".to_string(),
            sim_db: "mock_sim_db".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    #[serial_test::serial]
    fn verify_cli_structure() {
        Cli::command().debug_assert();
    }

    #[test]
    #[serial_test::serial]
    fn test_dispatch_ai() -> RaiseResult<()> {
        let args = vec!["raise-cli", "ai"];

        let cli = match Cli::try_parse_from(args) {
            Ok(c) => c,
            Err(e) => raise_error!("ERR_TEST_PARSE_FAILED", error = e.to_string()),
        };

        match cli.command {
            Some(Commands::Ai(_)) => Ok(()),
            _ => raise_error!(
                "ERR_TEST_DISPATCH_FAILED",
                error = "Le dispatch vers le module AI a échoué"
            ),
        }
    }

    ///  Résilience de la résolution des Mount Points
    #[test]
    #[serial_test::serial]
    fn test_mount_point_resolution_integrity() -> RaiseResult<()> {
        let config = AppConfig::get();

        if config.mount_points.system.domain.is_empty() {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Partition système manquante"
            );
        }

        if config.mount_points.system.db.is_empty() {
            raise_error!(
                "ERR_TEST_ASSERTION_FAILED",
                error = "Base système manquante"
            );
        }

        Ok(())
    }
}
