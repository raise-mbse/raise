// FICHIER : crates/raise-core/src/genetics/evaluators/neuro_symbolic.rs

use crate::genetics::evaluators::architecture::ArchitectureEvaluator;
use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;
use crate::genetics::traits::Evaluator;
use crate::utils::prelude::*;

/// 🎯 Façade "Zéro Dette" pour interroger le modèle GNN.
/// Isole totalement l'algorithme génétique de la logique tensorielle (Candle/CUDA).
#[async_interface]
pub trait GnnResilienceScorer: Send + Sync {
    /// Prédit un score de résilience ou de qualité topologique pour une allocation donnée.
    async fn predict_resilience(&self, genome: &SystemAllocationGenome) -> f32;
}

/// L'Évaluateur Neuro-Symbolique hybride.
/// Combine les règles mathématiques strictes (Symbolique) et l'intuition du réseau de neurones (Neuro).
pub struct NeuroSymbolicEvaluator {
    pub base_evaluator: ArchitectureEvaluator,
    pub gnn_scorer: SharedRef<dyn GnnResilienceScorer>,
}

impl NeuroSymbolicEvaluator {
    pub fn new(
        base_evaluator: ArchitectureEvaluator,
        gnn_scorer: SharedRef<dyn GnnResilienceScorer>,
    ) -> Self {
        Self {
            base_evaluator,
            gnn_scorer,
        }
    }
}

#[async_interface]
impl Evaluator<SystemAllocationGenome> for NeuroSymbolicEvaluator {
    fn objective_names(&self) -> Vec<String> {
        let mut names = self.base_evaluator.objective_names();
        // Ajout du 3ème objectif (GNN) au front de Pareto NSGA-II
        names.push("Neural Resilience (Max)".to_string());
        names
    }

    async fn evaluate(&self, genome: &SystemAllocationGenome) -> (Vec<f32>, f32) {
        // 1. Évaluation Symbolique (Déterministe et ultra-rapide)
        let (mut objs, violation) = self.base_evaluator.evaluate(genome).await;

        // 2. Évaluation Neuronale (Intuition GNN)
        // 🎯 Optimisation : Si le génome viole déjà les contraintes physiques (ex: RAM explosée),
        // inutile de réveiller le GPU ou de faire une passe d'inférence complexe.
        let neural_score = if violation > 0.0 {
            -1000.0 // Pénalité maximale pour élimination naturelle
        } else {
            self.gnn_scorer.predict_resilience(genome).await
        };

        objs.push(neural_score);
        (objs, violation)
    }

    fn is_valid(&self, genome: &SystemAllocationGenome) -> bool {
        self.base_evaluator.is_valid(genome)
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation de la Convergence Neuro-Symbolique)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genetics::evaluators::architecture::ArchitectureCostModel;

    // --- 1. Le Mock du GNN ---
    // Simule une réponse du réseau de neurones sans allouer de tenseurs.
    struct MockGnnScorer {
        pub predicted_resilience: f32,
        pub was_called: std::sync::atomic::AtomicBool,
    }

    impl MockGnnScorer {
        fn new(score: f32) -> Self {
            Self {
                predicted_resilience: score,
                was_called: std::sync::atomic::AtomicBool::new(false),
            }
        }
    }

    #[async_interface]
    impl GnnResilienceScorer for MockGnnScorer {
        async fn predict_resilience(&self, _genome: &SystemAllocationGenome) -> f32 {
            self.was_called
                .store(true, std::sync::atomic::Ordering::SeqCst);
            self.predicted_resilience
        }
    }

    // --- 2. Helper de création d'environnement ---
    fn setup_test_environment(
        mock_gnn_score: f32,
    ) -> (
        NeuroSymbolicEvaluator,
        SystemAllocationGenome,
        SystemAllocationGenome,
    ) {
        // Modèle : 2 fonctions de charge 10.0, 2 composants de capacité 15.0
        let model = ArchitectureCostModel::new(
            2,
            2,
            &[],                     // Pas de flux pour simplifier
            &[(0, 10.0), (1, 10.0)], // Loads: F0=10, F1=10
            &[(0, 15.0), (1, 15.0)], // Caps: C0=15, C1=15
        );

        let base_eval = ArchitectureEvaluator::new(model);
        let gnn_mock = SharedRef::new(MockGnnScorer::new(mock_gnn_score));

        let evaluator = NeuroSymbolicEvaluator::new(base_eval, gnn_mock);

        // Génome 1 (Valide) : F0 sur C0, F1 sur C1 -> Charge=10 par composant (< 15)
        let valid_genome = SystemAllocationGenome {
            genes: vec![0, 1],
            function_ids: vec![],
            available_component_ids: vec![],
        };

        // Génome 2 (Invalide) : F0 et F1 sur C0 -> Charge=20 sur C0 (> 15)
        let invalid_genome = SystemAllocationGenome {
            genes: vec![0, 0],
            function_ids: vec![],
            available_component_ids: vec![],
        };

        (evaluator, valid_genome, invalid_genome)
    }

    // --- 3. Les Cas de Tests ---

    #[async_test]
    async fn test_hybrid_evaluation_valid_allocation() {
        let expected_neural_score = 42.0;
        let (evaluator, valid_genome, _) = setup_test_environment(expected_neural_score);

        let (objs, violation) = evaluator.evaluate(&valid_genome).await;

        // Le génome est valide sur le plan physique
        assert_eq!(violation, 0.0, "Le génome devrait avoir 0.0 violation.");

        // 3 objectifs attendus : [Couplage, Equilibre, Résilience GNN]
        assert_eq!(objs.len(), 3, "Il doit y avoir exactement 3 objectifs.");

        // Le score du GNN a bien été intégré au front de Pareto
        assert_eq!(
            objs[2], expected_neural_score,
            "Le 3ème objectif doit correspondre à l'intuition du GNN."
        );
    }

    #[async_test]
    async fn test_hybrid_evaluation_short_circuits_gnn_on_invalid() {
        let expected_neural_score = 42.0; // Ce score ne devrait jamais être atteint
        let (evaluator, _, invalid_genome) = setup_test_environment(expected_neural_score);

        let (objs, violation) = evaluator.evaluate(&invalid_genome).await;

        // Le génome est physiquement invalide (Capacité dépassée)
        assert!(
            violation > 0.0,
            "Le génome devrait être marqué en violation."
        );

        // Vérification du court-circuit (Zéro Dette)
        assert_eq!(
            objs[2], -1000.0,
            "L'évaluateur doit retourner la pénalité maximale sans appeler le GNN."
        );
    }

    #[test]
    fn test_objective_names_composition() {
        let (evaluator, _, _) = setup_test_environment(0.0);
        let names = evaluator.objective_names();

        assert_eq!(names.len(), 3);
        assert!(names[2].contains("Neural Resilience"));
    }
}
