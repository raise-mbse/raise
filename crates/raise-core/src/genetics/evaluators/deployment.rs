// FICHIER : crates/raise-core/src/genetics/evaluators/deployment.rs

use crate::genetics::traits::{Evaluator, Genome};
use crate::utils::prelude::*;

/// Le génome manipulé par l'Agent DevOps.
/// Ce sont ces paramètres que le LLM va faire muter en cas d'échec de déploiement.
#[derive(Clone, Debug, Serializable, Deserializable)]
pub struct DeploymentGenome {
    pub binary_path: String,
    pub arguments: Vec<String>,
    // 🎯 ALIGNEMENT STRICT : Utilisation de l'alias sémantique RAISE
    pub env_vars: crate::utils::data::UnorderedMap<String, String>,
}

impl Genome for DeploymentGenome {
    fn random() -> Self {
        Self {
            binary_path: String::new(),
            arguments: vec![],
            // 🎯 ALIGNEMENT STRICT
            env_vars: crate::utils::data::UnorderedMap::new(),
        }
    }

    fn mutate(&mut self, _rate: f32) {
        // La mutation réelle (sémantique) est déléguée au LLM dans process().
    }

    fn crossover(&self, _other: &Self) -> Self {
        self.clone()
    }
}

/// L'évaluateur déterministe pour les services Edge.
pub struct DeploymentEvaluator {
    pub log_workspace: PathBuf,
}

impl DeploymentEvaluator {
    pub fn new(log_workspace: PathBuf) -> Self {
        Self { log_workspace }
    }
}

#[async_interface]
impl Evaluator<DeploymentGenome> for DeploymentEvaluator {
    fn objective_names(&self) -> Vec<String> {
        vec!["Service Stability (Max)".to_string()]
    }

    async fn evaluate(&self, genome: &DeploymentGenome) -> (Vec<f32>, f32) {
        let mut constraint_violation = 0.0;
        let mut stability_score = 100.0;

        let log_file_path = self
            .log_workspace
            .join(format!("{}_edge.log", UniqueId::new_v4()));

        let mut command = AsyncCommand::new(&genome.binary_path);
        command.args(&genome.arguments);
        for (k, v) in &genome.env_vars {
            command.env(k, v);
        }

        match command.spawn() {
            Ok(mut child_process) => {
                sleep_async(TimeDuration::from_millis(500)).await;

                match child_process.try_wait() {
                    Ok(Some(exit_status)) => {
                        constraint_violation += 500.0;
                        stability_score = 0.0;
                        user_warn!(
                            "WARN_SERVICE_CRASH",
                            json_value!({"status": exit_status.code()})
                        );
                    }
                    Ok(None) => {}
                    Err(e) => {
                        constraint_violation += 300.0;
                        user_warn!(
                            "WARN_SERVICE_STATUS_ERR",
                            json_value!({"error": e.to_string()})
                        );
                    }
                }
            }
            Err(e) => {
                constraint_violation += 1000.0;
                stability_score = 0.0;
                user_error!("ERR_SERVICE_SPAWN", json_value!({"error": e.to_string()}));
            }
        }

        if let Ok(logs) = fs::read_to_string_async(&log_file_path).await {
            let lower_logs = logs.to_lowercase();
            if lower_logs.contains("address already in use")
                || lower_logs.contains("panic")
                || lower_logs.contains("segmentation fault")
            {
                constraint_violation += 800.0;
                stability_score = -50.0;
            }
        }

        let _ = fs::remove_file_async(&log_file_path).await;

        (vec![stability_score], constraint_violation)
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation SRE et Auto-Remédiation)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_evaluator() -> DeploymentEvaluator {
        let dir = crate::utils::io::fs::tempdir().expect("Impossible d'allouer le TempDir");
        DeploymentEvaluator::new(dir.path().to_path_buf())
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_deployment_healthy_service() {
        let evaluator = setup_evaluator();

        let genome = DeploymentGenome {
            binary_path: "sh".to_string(),
            arguments: vec![
                "-c".to_string(),
                "echo 'Service démarré' && sleep 2".to_string(),
            ],
            // 🎯 ALIGNEMENT STRICT
            env_vars: crate::utils::data::UnorderedMap::new(),
        };

        let (objs, violation) = evaluator.evaluate(&genome).await;

        assert_eq!(violation, 0.0);
        assert_eq!(objs[0], 100.0);
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_deployment_binary_not_found() {
        let evaluator = setup_evaluator();

        let genome = DeploymentGenome {
            binary_path: "un_binaire_fantome_qui_n_existe_pas".to_string(),
            arguments: vec![],
            env_vars: crate::utils::data::UnorderedMap::new(),
        };

        let (objs, violation) = evaluator.evaluate(&genome).await;

        assert!(violation >= 1000.0);
        assert_eq!(objs[0], 0.0);
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_deployment_immediate_crash() {
        let evaluator = setup_evaluator();

        let genome = DeploymentGenome {
            binary_path: "sh".to_string(),
            arguments: vec!["-c".to_string(), "exit 1".to_string()],
            env_vars: crate::utils::data::UnorderedMap::new(),
        };

        let (objs, violation) = evaluator.evaluate(&genome).await;

        assert_eq!(violation, 500.0);
        assert_eq!(objs[0], 0.0);
    }

    #[async_test]
    #[serial_test::serial]
    async fn test_deployment_env_vars_injection() {
        let evaluator = setup_evaluator();

        let mut envs = crate::utils::data::UnorderedMap::new();
        envs.insert("RAISE_EDGE_PORT".to_string(), "9000".to_string());

        let genome = DeploymentGenome {
            binary_path: "sh".to_string(),
            arguments: vec![
                "-c".to_string(),
                "if [ \"$RAISE_EDGE_PORT\" = \"9000\" ]; then sleep 2; else exit 1; fi".to_string(),
            ],
            env_vars: envs,
        };

        let (objs, violation) = evaluator.evaluate(&genome).await;

        assert_eq!(violation, 0.0);
        assert_eq!(objs[0], 100.0);
    }
}
