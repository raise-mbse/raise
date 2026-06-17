use crate::utils::prelude::*;

/// Le trait Genome définit la structure manipulable par l'AG.
/// Doit être sérialisable pour le stockage et le passage Frontend/Backend.
pub trait Genome:
    Clone + Send + Sync + FmtDebug + Serializable + for<'de> Deserializable<'de>
{
    /// Génère un individu aléatoire (initialisation)
    fn random() -> Self;

    /// Applique une mutation sur le génome (modification in-place)
    fn mutate(&mut self, rate: f32);

    /// Croise deux génomes pour en produire un nouveau
    fn crossover(&self, other: &Self) -> Self;

    /// (Optionnel) Distance génétique entre deux génomes.
    /// Utile pour la "Fitness Sharing" ou pour mesurer la diversité.
    fn distance(&self, _other: &Self) -> f32 {
        0.0
    }
}

/// Le trait Evaluator fait le lien avec le métier (Arcadia, Règles, etc.).
#[async_interface]
pub trait Evaluator<G: Genome>: Send + Sync {
    /// Retourne les noms des objectifs pour l'affichage (ex: ["Performance", "Coût"]).
    fn objective_names(&self) -> Vec<String>;

    /// Calcule les scores de manière asynchrone pour permettre les I/O.
    /// Retourne : (Vec<valeurs_objectifs>, score_violation_contraintes).
    async fn evaluate(&self, genome: &G) -> (Vec<f32>, f32);

    /// Vérification rapide structurelle (Hard Constraints).
    /// Si false, on peut court-circuiter evaluate() avec une pénalité maximale.
    fn is_valid(&self, _genome: &G) -> bool {
        true
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Architecture Data-Driven & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // Mock minimal pour valider la compilation et l'usage des traits
    #[derive(Clone, Debug, Serializable, Deserializable)]
    struct MockGenome(i32);

    impl Genome for MockGenome {
        fn random() -> Self {
            MockGenome(42)
        }
        fn mutate(&mut self, _rate: f32) {
            self.0 += 1;
        }
        fn crossover(&self, other: &Self) -> Self {
            MockGenome((self.0 + other.0) / 2)
        }
    }

    struct MockEval;

    #[async_interface]
    impl Evaluator<MockGenome> for MockEval {
        fn objective_names(&self) -> Vec<String> {
            vec!["TestObj".into()]
        }
        async fn evaluate(&self, g: &MockGenome) -> (Vec<f32>, f32) {
            (vec![g.0 as f32], 0.0)
        }
    }

    #[async_test]
    async fn test_trait_interaction() {
        let mut g = MockGenome::random();
        assert_eq!(g.0, 42);
        g.mutate(0.1);
        assert_eq!(g.0, 43);

        let eval = MockEval;
        let (res, violation) = eval.evaluate(&g).await;
        assert_eq!(res[0], 43.0);
        assert_eq!(violation, 0.0);
    }
}
