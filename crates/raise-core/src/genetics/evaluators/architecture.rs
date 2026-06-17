use crate::genetics::evaluators::constraints::SystemConstraint;
use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;
use crate::genetics::traits::Evaluator;
use crate::utils::prelude::*;

/// Modèle de coûts statique (ne change pas pendant l'évolution).
/// Contient les données du problème Arcadia (Dataflows, Charges CPU, etc.).
#[derive(Clone, Debug)]
pub struct ArchitectureCostModel {
    /// Matrice des flux entre fonctions [func_i][func_j] = volume_données.
    /// Utilise des indices (usize) alignés avec le génome pour une performance max.
    pub data_flow_matrix: Vec<Vec<f32>>,

    /// Coût CPU/RAM de chaque fonction.
    pub function_loads: Vec<f32>,

    /// Capacité Max de chaque composant.
    pub component_capacities: Vec<f32>,
}

impl ArchitectureCostModel {
    /// Constructeur utilitaire pour transformer des données brutes en structures indexées rapides.
    pub fn new(
        num_functions: usize,
        num_components: usize,
        flows: &[(usize, usize, f32)], // (Src, Dest, Volume)
        loads: &[(usize, f32)],        // (FuncId, Load)
        capacities: &[(usize, f32)],   // (CompId, Capacity)
    ) -> Self {
        // 1. Init Matrice de flux (N x N)
        let mut data_flow_matrix = vec![vec![0.0; num_functions]; num_functions];
        for &(src, dst, vol) in flows {
            if src < num_functions && dst < num_functions {
                data_flow_matrix[src][dst] = vol;
                // Si le lien est bidirectionnel, décommenter :
                // data_flow_matrix[dst][src] += vol;
            }
        }

        // 2. Init Vecteur de charges
        let mut function_loads = vec![0.0; num_functions];
        for &(fid, load) in loads {
            if fid < num_functions {
                function_loads[fid] = load;
            }
        }

        // 3. Init Capacités
        let mut component_capacities = vec![0.0; num_components];
        for &(cid, cap) in capacities {
            if cid < num_components {
                component_capacities[cid] = cap;
            }
        }

        Self {
            data_flow_matrix,
            function_loads,
            component_capacities,
        }
    }
}

/// L'évaluateur principal pour l'architecture système.
/// Combine les objectifs de performance (Couplage, Charge) et les règles métier (Contraintes).
pub struct ArchitectureEvaluator {
    pub model: ArchitectureCostModel,
    /// Liste extensible de contraintes métier (ex: "Pas de Wifi sur le module critique").
    pub constraints: Vec<Box<dyn SystemConstraint>>,
}

impl ArchitectureEvaluator {
    pub fn new(model: ArchitectureCostModel) -> Self {
        Self {
            model,
            constraints: Vec::new(),
        }
    }

    /// Ajoute une règle métier à vérifier lors de l'évaluation.
    pub fn add_constraint<C: SystemConstraint + 'static>(&mut self, constraint: C) {
        self.constraints.push(Box::new(constraint));
    }
}

#[async_interface]
impl Evaluator<SystemAllocationGenome> for ArchitectureEvaluator {
    fn objective_names(&self) -> Vec<String> {
        vec![
            "Coupling Efficiency (Max)".to_string(), // Maximiser l'efficacité (-Couplage)
            "Load Balance (Max)".to_string(),        // Maximiser l'équilibre (-Variance)
        ]
    }

    async fn evaluate(&self, genome: &SystemAllocationGenome) -> (Vec<f32>, f32) {
        let num_components = self.model.component_capacities.len();

        // Accumulateurs
        let mut component_loads = vec![0.0; num_components];
        let mut total_coupling_cost = 0.0;
        let mut constraint_violation = 0.0;

        // 1. Calcul de la charge par composant
        for (func_idx, &comp_idx) in genome.genes.iter().enumerate() {
            if comp_idx < num_components {
                component_loads[comp_idx] += self.model.function_loads[func_idx];
            } else {
                // Index invalide (ne devrait pas arriver avec un génome sain)
                constraint_violation += 1000.0;
            }
        }

        // 2. Vérification de la Capacité (Hard Constraint fondamentale)
        // On le fait ici pour la perf, mais on pourrait aussi utiliser une CapacityConstraint externe.
        for (comp_idx, &load) in component_loads.iter().enumerate() {
            let cap = self.model.component_capacities[comp_idx];
            if load > cap {
                // Pénalité proportionnelle au dépassement
                constraint_violation += (load - cap) * 10.0; // Facteur de poids
            }
        }

        // 3. Application des Contraintes Métier Dynamiques
        for constraint in &self.constraints {
            constraint_violation += constraint.check(genome, &self.model);
        }

        // 4. Calcul du Couplage (Interface Cost)
        // Si Src et Dst sont sur des composants différents -> Coût.
        let num_funcs = genome.genes.len();
        for i in 0..num_funcs {
            for j in 0..num_funcs {
                let volume = self.model.data_flow_matrix[i][j];
                if volume > 0.0 {
                    let comp_i = genome.genes[i];
                    let comp_j = genome.genes[j];

                    if comp_i != comp_j {
                        // Coût de communication externe
                        total_coupling_cost += volume;
                    }
                }
            }
        }

        // 5. Calcul de l'équilibrage de charge (Load Balancing)
        // On cherche à minimiser la variance.
        let total_load: f32 = component_loads.iter().sum();
        let avg_load = total_load / num_components as f32;
        let variance: f32 = component_loads
            .iter()
            .map(|&l| (l - avg_load).powi(2))
            .sum::<f32>()
            / num_components as f32;

        // --- Transformation en Objectifs à MAXIMISER ---

        // Obj 1 : Minimiser le couplage => Maximiser l'opposé
        let obj_coupling = -total_coupling_cost;

        // Obj 2 : Minimiser la variance => Maximiser l'opposé
        let obj_balance = -variance;

        (vec![obj_coupling, obj_balance], constraint_violation)
    }

    fn is_valid(&self, genome: &SystemAllocationGenome) -> bool {
        // Validation structurelle rapide
        if genome.genes.len() != self.model.function_loads.len() {
            return false;
        }
        let max_comp = self.model.component_capacities.len();
        for &c in &genome.genes {
            if c >= max_comp {
                return false;
            }
        }
        true
    }
}

// --- Tests Unitaires ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::genetics::evaluators::constraints::SegregationConstraint;
    use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;

    // Helper pour génome sans contexte (pour le test)
    fn create_genome(genes: Vec<usize>) -> SystemAllocationGenome {
        SystemAllocationGenome {
            genes,
            function_ids: vec![],
            available_component_ids: vec![],
        }
    }

    #[async_test]
    async fn test_evaluate_basic_objectives() {
        // 3 Fonctions, 2 Composants
        // Flux F0->F1 (100)
        // Charges F0=2, F1=2, F2=12
        // Capacité C0=10, C1=10
        let model = ArchitectureCostModel::new(
            3,
            2,
            &[(0, 1, 100.0)],
            &[(0, 2.0), (1, 2.0), (2, 12.0)],
            &[(0, 10.0), (1, 10.0)],
        );

        let evaluator = ArchitectureEvaluator::new(model);

        // Cas : Tout sur C0
        // - Couplage = 0
        // - Charge C0 = 16 (Dépassement 6 -> Violation 60.0)
        let g_bad_cap = create_genome(vec![0, 0, 0]);
        let (objs, viol) = evaluator.evaluate(&g_bad_cap).await;

        assert_eq!(objs[0], 0.0); // Pas de flux externe
        assert_eq!(viol, 60.0); // (16 - 10) * 10.0
    }

    #[async_test]
    async fn test_evaluate_with_custom_constraints() {
        let model =
            ArchitectureCostModel::new(2, 2, &[], &[(0, 1.0), (1, 1.0)], &[(0, 10.0), (1, 10.0)]);

        let mut evaluator = ArchitectureEvaluator::new(model);

        // Ajout d'une règle : F0 et F1 doivent être séparées (Ségrégation)
        evaluator.add_constraint(SegregationConstraint {
            func_a_idx: 0,
            func_b_idx: 1,
            penalty: 500.0,
        });

        // Cas : F0 et F1 sur C0 -> Violation
        let g_violation = create_genome(vec![0, 0]);
        let (_, v1) = evaluator.evaluate(&g_violation).await;
        assert_eq!(v1, 500.0);

        // Cas : F0 sur C0, F1 sur C1 -> OK
        let g_ok = create_genome(vec![0, 1]);
        let (_, v2) = evaluator.evaluate(&g_ok).await;
        assert_eq!(v2, 0.0);
    }
}
