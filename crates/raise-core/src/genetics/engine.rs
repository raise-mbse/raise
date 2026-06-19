use crate::utils::prelude::*;

use crate::genetics::operators::selection::SelectionStrategy;
use crate::genetics::traits::{Evaluator, Genome};
use crate::genetics::types::{Fitness, Individual, Population};
use rand::prelude::*;

/// Configuration du moteur génétique.
#[derive(Clone, Debug)]
pub struct GeneticConfig {
    pub mutation_rate: f32,
    pub crossover_rate: f32,
    pub population_size: usize,
    pub max_generations: usize,
    pub elitism_count: usize,
}

impl Default for GeneticConfig {
    fn default() -> Self {
        Self {
            mutation_rate: 0.05,
            crossover_rate: 0.8,
            population_size: 100,
            max_generations: 50,
            elitism_count: 5,
        }
    }
}

pub struct GeneticEngine<G, E, S>
where
    G: Genome,
    E: Evaluator<G>,
    S: SelectionStrategy<G>,
{
    evaluator: E,
    selection: S,
    config: GeneticConfig,
    _marker: TypeMarker<G>,
}

impl<G, E, S> GeneticEngine<G, E, S>
where
    G: Genome,
    E: Evaluator<G>,
    S: SelectionStrategy<G>,
{
    pub fn new(evaluator: E, selection: S, config: GeneticConfig) -> Self {
        Self {
            evaluator,
            selection,
            config,
            _marker: TypeMarker,
        }
    }

    pub fn initialize_population(&self) -> Population<G> {
        let mut pop = Population::new();
        for _ in 0..self.config.population_size {
            pop.add(Individual::new(G::random()));
        }
        pop
    }

    pub async fn evolve_generation(&self, population: &mut Population<G>) {
        self.evaluate_population(population).await;
        self.fast_non_dominated_sort(population);

        let mut next_gen_individuals = Vec::with_capacity(self.config.population_size);
        let elites = population.get_elites(self.config.elitism_count);
        next_gen_individuals.extend(elites);

        let mut rng = rand::rng();

        while next_gen_individuals.len() < self.config.population_size {
            let parent1 = self.selection.select(&mut rng, population);
            let parent2 = self.selection.select(&mut rng, population);

            let mut child_genome = if rng.random::<f32>() < self.config.crossover_rate {
                parent1.genome.crossover(&parent2.genome)
            } else {
                parent1.genome.clone()
            };

            child_genome.mutate(self.config.mutation_rate);
            next_gen_individuals.push(Individual::new(child_genome));
        }

        population.individuals = next_gen_individuals;
        population.generation += 1;
    }

    pub async fn run<F>(&self, mut population: Population<G>, mut callback: F) -> Population<G>
    where
        F: FnMut(&Population<G>),
    {
        self.evaluate_population(&mut population).await;
        self.fast_non_dominated_sort(&mut population);
        callback(&population);

        for _ in 0..self.config.max_generations {
            self.evolve_generation(&mut population).await;
            callback(&population);
        }

        population
    }

    async fn evaluate_population(&self, population: &mut Population<G>) {
        let mut eval_tasks = Vec::new();

        // 1. Un seul type de bloc async pour toutes les tâches
        for (i, ind) in population.individuals.iter().enumerate() {
            if ind.fitness.is_none() {
                let evaluator = &self.evaluator;
                let genome = &ind.genome;

                eval_tasks.push(async move {
                    if !evaluator.is_valid(genome) {
                        (i, None)
                    } else {
                        let (objs, violation) = evaluator.evaluate(genome).await;
                        (i, Some((objs, violation)))
                    }
                });
            }
        }

        // 2. Exécution concurrente asynchrone
        let results = futures::future::join_all(eval_tasks).await;

        // 3. Application des résultats
        for (i, res) in results {
            match res {
                Some((objs, violation)) => {
                    population.individuals[i].fitness = Some(Fitness::new(objs, violation));
                }
                None => {
                    population.individuals[i].fitness = Some(Fitness::new(vec![], f32::MAX));
                }
            }
        }
    }

    fn fast_non_dominated_sort(&self, population: &mut Population<G>) {
        let n = population.individuals.len();

        // 1. Préparation des données d'entrée pour la façade RAISE
        let indices: Vec<usize> = (0..n).collect();

        // On capture une référence immuable locale pour le contexte multi-thread
        let individuals = &population.individuals;

        // 🎯 FIX ZÉRO DETTE : Utilisation stricte de la façade execute_parallel_map
        let mapped_results = execute_parallel_map(indices, |p| {
            let mut d_list = Vec::new();
            let mut d_count = 0;
            let fit_p = individuals[p].fitness.as_ref().unwrap();

            // 🎯 FIX CLIPPY 1 : Itération idiomatique plutôt que `0..n`
            for (q, ind_q) in individuals.iter().enumerate() {
                if p == q {
                    continue;
                }
                let fit_q = ind_q.fitness.as_ref().unwrap();

                if fit_p.dominates(fit_q) {
                    d_list.push(q);
                } else if fit_q.dominates(fit_p) {
                    d_count += 1;
                }
            }
            (d_list, d_count)
        });

        // 2. Décomposition (Unzip) des résultats retournés par les cœurs CPU
        let (dominates_list, mut dominated_count): (Vec<Vec<usize>>, Vec<usize>) =
            mapped_results.into_iter().unzip();

        let mut fronts: Vec<Vec<usize>> = vec![vec![]];

        // Étape 2 : Extraction du Front de Pareto optimal (Front 0)
        // 🎯 FIX CLIPPY 2 : Itération idiomatique sur dominated_count
        for (p, &count) in dominated_count.iter().enumerate() {
            if count == 0 {
                if let Some(fit) = &mut population.individuals[p].fitness {
                    fit.rank = 0;
                }
                fronts[0].push(p);
            }
        }

        // Étape 3 : Construction itérative des fronts secondaires
        let mut i = 0;
        while i < fronts.len() {
            let mut next_front: Vec<usize> = Vec::new();
            for &p in &fronts[i] {
                for &q in &dominates_list[p] {
                    dominated_count[q] -= 1;
                    if dominated_count[q] == 0 {
                        if let Some(fit) = &mut population.individuals[q].fitness {
                            fit.rank = i + 1;
                        }
                        next_front.push(q);
                    }
                }
            }
            if next_front.is_empty() {
                break;
            }
            fronts.push(next_front);
            i += 1;
        }

        for front in fronts {
            self.calculate_crowding_distance(population, &front);
        }
    }

    fn calculate_crowding_distance(&self, population: &mut Population<G>, front: &[usize]) {
        if front.is_empty() {
            return;
        }
        let l = front.len();

        for &idx in front {
            if let Some(fit) = &mut population.individuals[idx].fitness {
                fit.crowding_distance = 0.0;
            }
        }

        if l <= 2 {
            for &idx in front {
                if let Some(fit) = &mut population.individuals[idx].fitness {
                    fit.crowding_distance = f32::MAX;
                }
            }
            return;
        }

        let num_objectives = population.individuals[front[0]]
            .fitness
            .as_ref()
            .unwrap()
            .values
            .len();

        for m in 0..num_objectives {
            let mut sorted_front = front.to_vec();
            sorted_front.sort_by(|&a, &b| {
                let val_a = population.individuals[a].fitness.as_ref().unwrap().values[m];
                let val_b = population.individuals[b].fitness.as_ref().unwrap().values[m];
                val_a.partial_cmp(&val_b).unwrap_or(FmtOrdering::Equal)
            });

            let first = sorted_front[0];
            let last = sorted_front[l - 1];
            population.individuals[first]
                .fitness
                .as_mut()
                .unwrap()
                .crowding_distance = f32::MAX;
            population.individuals[last]
                .fitness
                .as_mut()
                .unwrap()
                .crowding_distance = f32::MAX;

            let min_obj = population.individuals[first]
                .fitness
                .as_ref()
                .unwrap()
                .values[m];
            let max_obj = population.individuals[last]
                .fitness
                .as_ref()
                .unwrap()
                .values[m];
            let range = max_obj - min_obj;

            if range == 0.0 {
                continue;
            }

            for i in 1..l - 1 {
                let idx = sorted_front[i];
                let next_val = population.individuals[sorted_front[i + 1]]
                    .fitness
                    .as_ref()
                    .unwrap()
                    .values[m];
                let prev_val = population.individuals[sorted_front[i - 1]]
                    .fitness
                    .as_ref()
                    .unwrap()
                    .values[m];

                if let Some(fit) = &mut population.individuals[idx].fitness {
                    if fit.crowding_distance != f32::MAX {
                        fit.crowding_distance += (next_val - prev_val) / range;
                    }
                }
            }
        }
    }
}

// --- Tests ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::genetics::operators::selection::TournamentSelection;

    #[derive(Clone, Debug, Serializable, Deserializable)]
    struct NumberGenome(f32);

    impl Genome for NumberGenome {
        fn random() -> Self {
            NumberGenome(rand::random())
        } // UPDATE
        fn mutate(&mut self, _rate: f32) {
            self.0 += 0.1;
        }
        fn crossover(&self, other: &Self) -> Self {
            NumberGenome((self.0 + other.0) / 2.0)
        }
    }

    struct SimpleEvaluator;

    #[async_interface]
    impl Evaluator<NumberGenome> for SimpleEvaluator {
        fn objective_names(&self) -> Vec<String> {
            vec!["Max".into(), "Min".into()]
        }
        async fn evaluate(&self, g: &NumberGenome) -> (Vec<f32>, f32) {
            (vec![g.0, -g.0], 0.0)
        }
    }

    #[async_test]
    async fn test_engine_workflow() {
        let config = GeneticConfig {
            population_size: 10,
            max_generations: 2,
            ..Default::default()
        };
        let engine = GeneticEngine::new(SimpleEvaluator, TournamentSelection::new(2), config);
        let mut pop = engine.initialize_population();
        engine.evolve_generation(&mut pop).await;
        assert_eq!(pop.generation, 1);
    }
}
