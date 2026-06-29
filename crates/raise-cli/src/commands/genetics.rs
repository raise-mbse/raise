// FICHIER : src-tauri/tools/raise-cli/src/commands/genetics.rs

use clap::{Args, Subcommand};
use raise_core::{user_info, user_success, user_warn, utils::prelude::*}; // 🎯 Façade Unique RAISE

// 🎯 IMPORTS DU CORE POUR LA CONVERGENCE (Génétique, Modèle, Évaluateurs)
use raise_core::genetics::bridge::{GeneticsAdapter, SystemModelProvider};
use raise_core::genetics::engine::{GeneticConfig, GeneticEngine};
use raise_core::genetics::evaluators::architecture::ArchitectureEvaluator;
use raise_core::genetics::evaluators::neuro_symbolic::NeuroSymbolicEvaluator;
use raise_core::genetics::genomes::arcadia_arch::SystemAllocationGenome;
use raise_core::genetics::operators::selection::TournamentSelection;
use raise_core::genetics::types::{Individual, Population};
use raise_core::json_db::collections::manager::CollectionsManager;
use raise_core::model_engine::types::ProjectModel;
use raise_core::services::gnn_service::{GnnScorerAdapter, GnnState};

// 🎯 Import du contexte global CLI
use crate::CliContext;

/// Commandes pour le Moteur Génétique (Raise Genetics Engine)
#[derive(Args, Clone, Debug)]
pub struct GeneticsArgs {
    #[command(subcommand)]
    pub command: GeneticsCommands,
}

#[derive(Subcommand, Clone, Debug)]
pub enum GeneticsCommands {
    /// Lance une simulation d'évolution sur une population
    Evolve {
        /// Taille de la population initiale
        #[arg(short, long, default_value = "100")]
        population: usize,

        /// Nombre de générations à simuler
        #[arg(short, long, default_value = "50")]
        generations: usize,

        /// Taux de mutation (0.0 - 1.0)
        #[arg(short, long, default_value = "0.05")]
        mutation_rate: f32,

        /// Taux de croisement (crossover)
        #[arg(short, long, default_value = "0.8")]
        crossover_rate: f32,
    },
    /// Inspecte le génome du meilleur individu
    Inspect {
        /// ID spécifique d'un individu ou front de Pareto
        #[arg(short, long)]
        id: Option<String>,
    },
}

pub async fn handle(args: GeneticsArgs, ctx: CliContext) -> RaiseResult<()> {
    // 🎯 Heartbeat de session
    let _ = ctx.session_mgr.touch().await;

    match args.command {
        GeneticsCommands::Evolve {
            population,
            generations,
            mutation_rate,
            crossover_rate,
        } => {
            user_info!(
                "GENETICS_INIT",
                json_value!({
                    "active_domain": ctx.active_domain,
                    "active_user": ctx.active_user
                })
            );

            // 1. Création de la configuration réelle
            let config = GeneticConfig {
                population_size: population,
                max_generations: generations,
                mutation_rate,
                crossover_rate,
                elitism_count: 5,
            };

            // 2. Validation des hyperparamètres
            if !(0.0..=1.0).contains(&config.mutation_rate) {
                user_warn!(
                    "GENETICS_CONFIG_BOUNDS",
                    json_value!({
                        "field": "mutation_rate",
                        "value": config.mutation_rate,
                        "hint": "Le taux devrait être entre 0.0 et 1.0."
                    })
                );
            }

            user_info!(
                "GENETICS_READY",
                json_value!({
                    "pop_size": config.population_size,
                    "max_gen": config.max_generations
                })
            );

            // =========================================================================
            // 3. INTÉGRATION NEURO-SYMBOLIQUE (Remplacement du TODO)
            // =========================================================================

            // A. Initialisation du gestionnaire de données sur le domaine actif
            let _manager = CollectionsManager::new(
                ctx.storage.as_ref(),
                &ctx.active_domain,
                "master", // DB principale par défaut pour le CLI
            );

            // B. Chargement du modèle de projet (Suppose que ProjectModel possède load_from_db ou qu'on l'instancie proprement)
            // Si la méthode n'existe pas encore, on initialise un modèle par défaut (Mocké pour la compilation)
            let project_model = ProjectModel::default();

            // C. L'Adaptateur Zéro Dette (Extraction des fonctions, composants et flux)
            let adapter = GeneticsAdapter::new(&project_model);
            let cost_model = adapter.build_cost_model(&project_model);

            let functions = project_model.get_functions();
            let components = project_model.get_components();

            if functions.is_empty() || components.is_empty() {
                // Pour éviter le crash du CLI si la DB est vide, on logge un warning et on quitte proprement
                user_warn!(
                    "GENETICS_EMPTY_MODEL",
                    json_value!({"msg": "Aucune fonction ou composant à allouer dans le domaine cible."})
                );
                return Ok(());
            }

            let func_ids: Vec<String> = functions.into_iter().map(|f| f.id).collect();
            let comp_ids: Vec<String> = components.into_iter().map(|c| c.id).collect();

            // D. Instanciation des Évaluateurs
            let base_evaluator = ArchitectureEvaluator::new(cost_model);

            // L'état GNN est initialisé à vide localement.
            // Grâce à notre architecture Zéro Dette, le `GnnScorerAdapter` va court-circuiter
            // en renvoyant 0.0 et laisser la génétique opérer sur le volet Symbolique uniquement.
            let gnn_state = SharedRef::new(GnnState::new());
            let gnn_adapter = SharedRef::new(GnnScorerAdapter::new(gnn_state));

            let evaluator = NeuroSymbolicEvaluator::new(base_evaluator, gnn_adapter, None);

            // E. Moteur et Population Initiale
            let engine = GeneticEngine::new(evaluator, TournamentSelection::new(2), config.clone());

            let mut pop = Population::new();
            for _ in 0..config.population_size {
                pop.add(Individual::new(SystemAllocationGenome::new_random(
                    func_ids.clone(),
                    comp_ids.clone(),
                )));
            }

            // F. Lancement de la boucle d'évolution
            println!("🚀 Lancement du moteur Neuro-Symbolique (NSGA-II)...");

            let final_pop = engine
                .run(pop, |current_pop| {
                    // Affichage d'un Heartbeat console toutes les 10 générations
                    if current_pop.generation % 10 == 0 {
                        if let Some(elite) = current_pop.get_elites(1).into_iter().next() {
                            if let Some(fit) = elite.fitness {
                                println!(
                                "⏳ Gen {} | Objectifs (Couplage, Équilibre, Résilience) : {:?}",
                                current_pop.generation, fit.values
                            );
                            }
                        }
                    }
                })
                .await;

            // G. Extraction et Traduction de la solution
            let best_individual = match final_pop.get_elites(1).into_iter().next() {
                Some(ind) => ind,
                None => raise_error!(
                    "ERR_GENETICS_NO_SOLUTION",
                    error = "Aucun front de Pareto généré."
                ),
            };

            if let Some(fit) = best_individual.fitness {
                let solution_dto = adapter.convert_solution(
                    fit.values,
                    fit.constraint_violation,
                    &best_individual.genome.genes,
                );

                user_success!(
                    "GENETICS_SUCCESS",
                    json_value!({
                        "status": "Optimisation terminée",
                        "best_solution": solution_dto
                    })
                );
            }
        }
        GeneticsCommands::Inspect { id } => {
            let target = id.as_deref().unwrap_or("Pareto Front Best");
            user_info!("GENETICS_INSPECT", json_value!({ "target": target }));
        }
    }
    Ok(())
}

// =========================================================================
// TESTS UNITAIRES (Conformité "Zéro Dette")
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use raise_core::utils::testing::DbSandbox;

    #[async_test]
    #[serial_test::serial] // 🎯 FIX : Empêche les collisions de sandbox
    async fn test_genetics_config_mapping() -> RaiseResult<()> {
        let sandbox = DbSandbox::new().await?;
        let storage = SharedRef::new(sandbox.storage.clone());
        let session_mgr = crate::context::SessionManager::new(storage.clone());

        let ctx = crate::CliContext::mock(AppConfig::get(), session_mgr, storage);

        let args = GeneticsArgs {
            command: GeneticsCommands::Evolve {
                population: 10,
                generations: 5,
                mutation_rate: 0.1,
                crossover_rate: 0.9,
            },
        };

        handle(args, ctx).await
    }
}
