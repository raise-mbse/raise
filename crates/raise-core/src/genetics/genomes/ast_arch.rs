// FICHIER : crates/raise-core/src/genetics/genomes/ast_arch.rs

use crate::genetics::traits::Genome;
use crate::utils::prelude::*;
use rand::prelude::*;

/// Représentation simplifiée d'un Arbre Syntaxique Abstrait (AST)
#[derive(Clone, Debug, Serializable, Deserializable, PartialEq)]
pub enum AstNode {
    /// Un bloc conteneur (ex: un module `mod { ... }` ou un `impl { ... }`)
    Block {
        name: String,
        children: Vec<AstNode>,
    },
    /// Une feuille logique (ex: le corps d'une fonction `fn foo() { ... }`)
    Function { signature: String, body: String },
}

impl AstNode {
    /// Calcule le nombre total de nœuds dans ce sous-arbre
    pub fn size(&self) -> usize {
        match self {
            AstNode::Block { children, .. } => 1 + children.iter().map(|c| c.size()).sum::<usize>(),
            AstNode::Function { .. } => 1,
        }
    }

    /// Récupère une copie d'un sous-arbre spécifique pour l'extraction
    pub fn get_node(&self, target_idx: usize, current_idx: &mut usize) -> Option<AstNode> {
        if *current_idx == target_idx {
            return Some(self.clone());
        }
        *current_idx += 1;

        if let AstNode::Block { children, .. } = self {
            for child in children {
                if let Some(node) = child.get_node(target_idx, current_idx) {
                    return Some(node);
                }
            }
        }
        None
    }

    /// Récupère une référence mutable d'un sous-arbre pour l'écraser (Mutation/Crossover)
    pub fn get_node_mut<'a>(
        &'a mut self,
        target_idx: usize,
        current_idx: &mut usize,
    ) -> Option<&'a mut AstNode> {
        if *current_idx == target_idx {
            return Some(self);
        }
        *current_idx += 1;

        if let AstNode::Block { children, .. } = self {
            for child in children.iter_mut() {
                if let Some(node) = child.get_node_mut(target_idx, current_idx) {
                    return Some(node);
                }
            }
        }
        None
    }
}

/// Le Génome représentant un fichier ou un module de code complet
#[derive(Clone, Debug, Serializable, Deserializable)]
pub struct AstGenome {
    pub root: AstNode,
}

impl Genome for AstGenome {
    fn random() -> Self {
        Self {
            root: AstNode::Block {
                name: "root".to_string(),
                children: vec![],
            },
        }
    }

    fn mutate(&mut self, rate: f32) {
        let mut rng = rand::rng();
        if rng.random::<f32>() <= rate {
            // La logique de mutation fine (ex: altérer le contenu de `body` d'une fonction)
            // sera branchée ultérieurement sur le LLM local (CodeGen).
        }
    }

    fn crossover(&self, other: &Self) -> Self {
        let mut rng = rand::rng();
        let mut child = self.clone();

        let size_p1 = child.root.size();
        if size_p1 == 0 {
            return child;
        }
        let point_p1 = rng.random_range(0..size_p1);

        let size_p2 = other.root.size();
        if size_p2 == 0 {
            return child;
        }
        let point_p2 = rng.random_range(0..size_p2);

        // 🎯 L'échange de sous-arbres garantit la validité syntaxique
        let mut idx_p2 = 0;
        if let Some(subtree) = other.root.get_node(point_p2, &mut idx_p2) {
            let mut idx_p1 = 0;
            if let Some(target_node) = child.root.get_node_mut(point_p1, &mut idx_p1) {
                *target_node = subtree;
            }
        }

        child
    }
}

// =========================================================================
// TESTS UNITAIRES (Topologie du Code Source)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper : Crée un petit AST simulant un fichier Rust avec un module et deux fonctions
    fn create_mock_ast() -> AstNode {
        AstNode::Block {
            name: "mod_kernel".to_string(),
            children: vec![
                AstNode::Function {
                    signature: "fn boot()".to_string(),
                    body: "{ println!(\"boot\"); }".to_string(),
                },
                AstNode::Function {
                    signature: "fn halt()".to_string(),
                    body: "{ panic!(\"halt\"); }".to_string(),
                },
            ],
        }
    }

    #[test]
    fn test_ast_size_calculation() {
        let root = create_mock_ast();
        // Le module (1) + fn boot (1) + fn halt (1) = 3 nœuds
        assert_eq!(root.size(), 3, "La taille de l'AST doit être de 3.");

        let empty_root = AstNode::Block {
            name: "empty".to_string(),
            children: vec![],
        };
        assert_eq!(empty_root.size(), 1, "Un bloc vide vaut 1 nœud.");
    }

    #[test]
    fn test_ast_node_retrieval_by_index() {
        let root = create_mock_ast();

        // Index 0 doit être la racine (le bloc)
        let mut idx = 0;
        let node_0 = root.get_node(0, &mut idx).expect("Nœud 0 introuvable");
        match node_0 {
            AstNode::Block { name, .. } => assert_eq!(name, "mod_kernel"),
            _ => panic!("Le nœud 0 devrait être un Block"),
        }

        // Index 1 doit être le premier enfant (fn boot)
        idx = 0;
        let node_1 = root.get_node(1, &mut idx).expect("Nœud 1 introuvable");
        match node_1 {
            AstNode::Function { signature, .. } => assert_eq!(signature, "fn boot()"),
            _ => panic!("Le nœud 1 devrait être une Function"),
        }

        // Index 2 doit être le second enfant (fn halt)
        idx = 0;
        let node_2 = root.get_node(2, &mut idx).expect("Nœud 2 introuvable");
        match node_2 {
            AstNode::Function { signature, .. } => assert_eq!(signature, "fn halt()"),
            _ => panic!("Le nœud 2 devrait être une Function"),
        }

        // Index 3 n'existe pas
        idx = 0;
        let node_3 = root.get_node(3, &mut idx);
        assert!(node_3.is_none(), "L'index hors limite doit retourner None");
    }

    #[test]
    fn test_ast_node_mutation_by_index() {
        let mut root = create_mock_ast();
        let mut idx = 0;

        // On cible la seconde fonction (index 2) et on modifie sa signature
        if let Some(AstNode::Function { signature, .. }) = root.get_node_mut(2, &mut idx) {
            *signature = "async fn halt()".to_string();
        } else {
            panic!("Échec de l'emprunt mutable du nœud 2");
        }

        // Vérification
        idx = 0;
        if let Some(AstNode::Function { signature, .. }) = root.get_node(2, &mut idx) {
            assert_eq!(
                signature, "async fn halt()",
                "La mutation doit persister dans l'arbre principal"
            );
        }
    }

    #[test]
    fn test_ast_genome_crossover_validity() {
        let p1 = AstGenome {
            root: create_mock_ast(),
        };

        // Parent 2 : Un module contenant une fonction d'injection
        let p2 = AstGenome {
            root: AstNode::Block {
                name: "mod_injected".to_string(),
                children: vec![AstNode::Function {
                    signature: "fn hack()".to_string(),
                    body: "{}".to_string(),
                }],
            },
        };

        // Croisement (10 itérations pour couvrir plusieurs tirages aléatoires)
        for _ in 0..10 {
            let child = p1.crossover(&p2);

            // Le croisement ne doit jamais retourner un arbre vide
            let child_size = child.root.size();
            assert!(
                child_size > 0,
                "L'enfant doit toujours avoir une taille > 0"
            );

            // Si le bloc racine a été remplacé par le nœud fonction 'hack' du parent 2,
            // la taille de l'enfant devient 1. Sinon c'est une taille de branche.
            assert!(
                child_size <= 4,
                "L'enfant ne peut pas dépasser la taille mathématique maximale d'insertion"
            );
        }
    }
}
