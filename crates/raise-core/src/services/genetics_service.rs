use crate::utils::prelude::*;

use crate::genetics::dto::{
    AllocatedSolution, OptimizationProgress, OptimizationRequest, OptimizationResult,
};
use crate::genetics::engine::{GeneticConfig, GeneticEngine};
use crate::genetics::evaluators::architecture::{ArchitectureCostModel, ArchitectureEvaluator};
use crate::genetics::evaluators::constraints::SegregationConstraint;
use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;
use crate::genetics::operators::selection::TournamentSelection;
use crate::genetics::types::{Individual, Population};

pub fn debug_genetics_ping(name: String) -> String {
    println!("🔔 Ping reçu de la part de : {}", name);
    format!("Hello {}, le pont Tauri fonctionne !", name)
}

/// Commande principale pour l'optimisation d'architecture.
pub async fn run_architecture_optimization<F>(
    params: OptimizationRequest,
    on_progress: F,
) -> RaiseResult<OptimizationResult>
where
    // 🎯 FIX : Déclaration des traits requis pour F
    F: Fn(OptimizationProgress) + Send + Sync + 'static,
{
    let start_time = TimeInstant::now();

    // 1. Préparation des données (Mapping IDs -> Index)
    let func_ids: Vec<String> = params.functions.iter().map(|f| f.id.clone()).collect();
    let comp_ids: Vec<String> = params.components.iter().map(|c| c.id.clone()).collect();

    // Mapping des flux (volumes)
    let mut flow_triplets = Vec::new();
    for flow in &params.flows {
        let src_idx = func_ids.iter().position(|id| id == &flow.source_id);
        let tgt_idx = func_ids.iter().position(|id| id == &flow.target_id);
        if let (Some(s), Some(t)) = (src_idx, tgt_idx) {
            flow_triplets.push((s, t, flow.volume));
        }
    }

    let loads: Vec<(usize, f32)> = params
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (i, f.load))
        .collect();
    let capacities: Vec<(usize, f32)> = params
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| (i, c.capacity))
        .collect();

    // 2. Initialisation de l'évaluateur
    // Correction : ArchitectureCostModel attend les dimensions et les vecteurs de données
    let model = ArchitectureCostModel::new(
        func_ids.len(),
        comp_ids.len(),
        &flow_triplets,
        &loads,
        &capacities,
    );
    let mut evaluator = ArchitectureEvaluator::new(model);

    // Ajout des contraintes de ségrégation
    if let Some(conf) = &params.constraints {
        for (id_a, id_b) in &conf.segregations {
            let idx_a = func_ids.iter().position(|id| id == id_a);
            let idx_b = func_ids.iter().position(|id| id == id_b);
            if let (Some(a), Some(b)) = (idx_a, idx_b) {
                evaluator.add_constraint(SegregationConstraint {
                    func_a_idx: a,
                    func_b_idx: b,
                    penalty: 1000.0,
                });
            }
        }
    }

    // 3. Configuration du Moteur
    let config = GeneticConfig {
        population_size: params.population_size,
        max_generations: params.max_generations,
        mutation_rate: params.mutation_rate,
        crossover_rate: params.crossover_rate,
        elitism_count: (params.population_size / 10).max(1),
    };

    let selection = TournamentSelection::new(3);
    let engine = GeneticEngine::new(evaluator, selection, config.clone());

    // 4. Initialisation de la Population
    let mut population = Population::new();
    for _ in 0..config.population_size {
        // Correction : Utilisation de new_random avec les IDs métier
        let genome = SystemAllocationGenome::new_random(func_ids.clone(), comp_ids.clone());
        population.add(Individual::new(genome));
    }

    // 5. Exécution avec Télémétrie (Émissions d'événements Tauri)
    let final_pop = engine
        .run(population, |pop| {
            if let Some(best) = pop.individuals.first() {
                if let Some(fit) = &best.fitness {
                    on_progress(OptimizationProgress {
                        generation: pop.generation,
                        best_fitness: fit.values.clone(),
                        diversity: fit.crowding_distance,
                    });
                }
            }
        })
        .await;

    // 6. Extraction du Front de Pareto
    let pareto_front: Vec<AllocatedSolution> = final_pop
        .individuals
        .into_iter()
        .filter(|ind| ind.fitness.as_ref().map(|f| f.rank == 0).unwrap_or(false))
        .map(|ind| {
            let fitness = ind.fitness.unwrap_or_default();
            AllocatedSolution {
                fitness: fitness.values,
                constraint_violation: fitness.constraint_violation,
                allocation: ind.genome.get_allocations(),
            }
        })
        .collect();

    Ok(OptimizationResult {
        duration_ms: start_time.elapsed().as_millis(),
        pareto_front,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    // L'import est correct ici
    use crate::genetics::dto::{ComponentInfo, DataFlowInfo, FunctionInfo};

    #[test]
    fn test_map_params() {
        let params = OptimizationRequest {
            population_size: 100,
            max_generations: 50,
            mutation_rate: 0.1,
            crossover_rate: 0.8,

            // CORRECTION 1 : Remplacer FunctionDto par FunctionInfo
            functions: vec![FunctionInfo {
                id: "f1".to_string(),
                load: 10.0,
            }],

            // CORRECTION 2 : Remplacer ComponentDto par ComponentInfo
            components: vec![ComponentInfo {
                id: "c1".to_string(),
                capacity: 100.0,
            }],

            flows: vec![DataFlowInfo {
                source_id: "f1".to_string(),
                target_id: "f1".to_string(),
                volume: 10.0,
            }],

            constraints: None,
        };

        // Vérifications simples
        assert_eq!(params.functions.len(), 1);
        assert_eq!(params.components.len(), 1);
    }
}
