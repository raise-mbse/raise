// FICHIER : crates/raise-core/src/genetics/evaluators/codegen.rs

use crate::genetics::genomes::ast_arch::{AstGenome, AstNode};
use crate::genetics::traits::Evaluator;
use crate::utils::prelude::*; // 🎯 Import STRICT de la façade RAISE

/// Évaluateur dynamique qui interroge le compilateur Rust (rustc)
/// via la façade stricte RAISE (Zero-Debt).
pub struct CodeGenEvaluator {
    pub temp_workspace: PathBuf,
}

impl CodeGenEvaluator {
    pub fn new(temp_workspace: PathBuf) -> Self {
        Self { temp_workspace }
    }

    /// Transpile l'AST génétique en code source Rust valide
    fn generate_source(node: &AstNode) -> String {
        match node {
            AstNode::Block { name, children } => {
                let children_code: Vec<String> =
                    children.iter().map(Self::generate_source).collect();
                if name == "root" {
                    children_code.join("\n\n")
                } else {
                    format!("pub mod {} {{\n{}\n}}", name, children_code.join("\n\n"))
                }
            }
            AstNode::Function { signature, body } => {
                format!("{} {}", signature, body)
            }
        }
    }
}

#[async_interface]
impl Evaluator<AstGenome> for CodeGenEvaluator {
    fn objective_names(&self) -> Vec<String> {
        vec!["Code Conciseness (Max)".to_string()]
    }

    async fn evaluate(&self, genome: &AstGenome) -> (Vec<f32>, f32) {
        let source_code = Self::generate_source(&genome.root);
        let code_size = source_code.len() as f32;

        // 🎯 Génération d'un ID unique garanti par la façade RAISE (Uuid)
        let file_id = UniqueId::new_v4().to_string();
        let file_name = format!("gen_{}.rs", file_id);
        let file_path = self.temp_workspace.join(&file_name);
        let out_path = self.temp_workspace.join(format!("out_{}.rmeta", file_id));

        // 1. Écriture Asynchrone Zéro-Dette (fs::write_async)
        if fs::write_async(&file_path, source_code.as_bytes())
            .await
            .is_err()
        {
            return (vec![-code_size], 1000.0); // Pénalité fatale pour erreur I/O
        }

        let out_str = out_path.to_string_lossy();
        let file_str = file_path.to_string_lossy();
        let args = [
            "--edition=2021",
            "--crate-type=lib",
            "--emit=metadata",
            "-o",
            &out_str,
            &file_str,
        ];

        let mut constraint_violation = 0.0;

        // 2. Vérification Asynchrone via la façade RAISE (os::exec_command_async)
        match os::exec_command_async("rustc", &args, Some(&self.temp_workspace)).await {
            Ok(_) => {
                // Le compilateur a validé le code avec succès (statut 0)
            }
            Err(_) => {
                // RAISE a intercepté une erreur (syntaxe, borrow checker, code non nul).
                // exec_command_async lève automatiquement une erreur structurée.
                constraint_violation += 500.0;
            }
        }

        // 3. Nettoyage Asynchrone (fs::remove_file_async)
        let _ = fs::remove_file_async(&file_path).await;
        let _ = fs::remove_file_async(&out_path).await;

        // Objectif : Minimiser la taille du code => Maximiser l'opposé (-code_size)
        (vec![-code_size], constraint_violation)
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation dynamique par le compilateur)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    async fn test_codegen_evaluator_valid_code() {
        // 🎯 Utilisation de `tempdir` importé depuis le `prelude` RAISE
        let dir = tempdir().expect("Impossible d'allouer le TempDir");
        let evaluator = CodeGenEvaluator::new(dir.path().to_path_buf());

        // Un AST valide : fn hello() { let x = 42; }
        let valid_genome = AstGenome {
            root: AstNode::Block {
                name: "root".to_string(),
                children: vec![AstNode::Function {
                    signature: "pub fn hello()".to_string(),
                    body: "{ let _x = 42; }".to_string(),
                }],
            },
        };

        let (objs, viol) = evaluator.evaluate(&valid_genome).await;

        assert_eq!(
            viol, 0.0,
            "Le code valide ne doit subir aucune violation de contrainte"
        );
        assert!(
            objs[0] < 0.0,
            "L'objectif de taille doit être calculé en négatif"
        );
    }

    #[async_test]
    async fn test_codegen_evaluator_invalid_code() {
        let dir = tempdir().expect("Impossible d'allouer le TempDir");
        let evaluator = CodeGenEvaluator::new(dir.path().to_path_buf());

        // Un AST Invalide : Type incohérent
        let invalid_genome = AstGenome {
            root: AstNode::Block {
                name: "root".to_string(),
                children: vec![AstNode::Function {
                    signature: "pub fn fail() -> i32".to_string(),
                    body: "{ \"Ceci est une string\" }".to_string(), // Ne compile pas
                }],
            },
        };

        let (_, viol) = evaluator.evaluate(&invalid_genome).await;

        assert!(
            viol > 0.0,
            "Le code invalide doit recevoir une pénalité sévère"
        );
    }
}
