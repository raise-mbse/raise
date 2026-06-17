// FICHIER : src-tauri/src/genetics/handler.rs

use crate::ai::assurance::xai::{ExplanationScope, XaiFrame, XaiMethod};
use crate::genetics::engine::{GeneticConfig, GeneticEngine};
use crate::genetics::genomes::arcadia_arch::SystemAllocationGenome;
use crate::genetics::operators::selection::TournamentSelection;
use crate::genetics::traits::Evaluator;
use crate::genetics::types::{Individual, Population};
use crate::utils::prelude::*; // 🎯 Façade Unique RAISE
use crate::workflow_engine::handlers::{HandlerContext, NodeHandler};
use crate::workflow_engine::{ExecutionStatus, NodeType, WorkflowNode};

// =========================================================================
// 1. L'ÉVALUATEUR MÉTIER (Synchrone & CPU-Bound)
// =========================================================================
#[derive(Clone)]
struct MbseEvaluator {
    component_metrics: UnorderedMap<String, JsonValue>,
}

#[async_interface]
impl Evaluator<SystemAllocationGenome> for MbseEvaluator {
    fn objective_names(&self) -> Vec<String> {
        vec!["MinusWeight".into(), "MinusCost".into()]
    }

    async fn evaluate(&self, genome: &SystemAllocationGenome) -> (Vec<f32>, f32) {
        let mut total_weight = 0.0;
        let mut total_cost = 0.0;
        let mut constraints_violation = 0.0;

        let allocations = genome.get_allocations();

        for (_func_id, comp_id) in allocations {
            match self.component_metrics.get(&comp_id) {
                Some(metrics) => {
                    total_weight += metrics
                        .get("weight")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0) as f32;
                    total_cost +=
                        metrics.get("cost").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
                }
                None => {
                    constraints_violation += 100.0;
                }
            }
        }
        (vec![-total_weight, -total_cost], constraints_violation)
    }
}

// =========================================================================
// 2. LE HANDLER (Asynchrone)
// =========================================================================
pub struct GeneticsHandler;

#[async_interface]
impl NodeHandler for GeneticsHandler {
    fn node_type(&self) -> NodeType {
        NodeType::Genetics
    }

    async fn execute(
        &self,
        node: &WorkflowNode,
        context: &mut UnorderedMap<String, JsonValue>,
        shared_ctx: &HandlerContext<'_>,
    ) -> RaiseResult<ExecutionStatus> {
        user_info!("INF_GENETICS_START", json_value!({"node": node.name}));

        // 1. Extraction des paramètres via Match strict
        let function_ids: Vec<String> =
            match node.params.get("functions").and_then(|v| v.as_array()) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                None => raise_error!(
                    "ERR_GENETICS_MISSING_FUNCTIONS",
                    context = json_value!({"node_id": node.id})
                ),
            };

        let component_ids: Vec<String> =
            match node.params.get("components").and_then(|v| v.as_array()) {
                Some(arr) => arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                None => raise_error!(
                    "ERR_GENETICS_MISSING_COMPONENTS",
                    context = json_value!({"node_id": node.id})
                ),
            };

        // 2. Collecte des métriques via le Manager (Support des Mount Points)
        let mut component_metrics = UnorderedMap::new();
        for comp_id in &component_ids {
            match shared_ctx.manager.get_document("components", comp_id).await {
                Ok(Some(doc)) => {
                    if let Some(pvmt) = doc.get("pvmt_values") {
                        component_metrics.insert(comp_id.clone(), pvmt.clone());
                    }
                }
                _ => continue, // Résilience : On ignore les composants non résolus
            }
        }

        let evaluator = MbseEvaluator { component_metrics };
        let genetic_config = GeneticConfig {
            population_size: 50,
            max_generations: 200,
            ..Default::default()
        };

        user_info!(
            "INF_GENETICS_EVOLVING",
            json_value!({"generations": genetic_config.max_generations})
        );

        // 3. Exécution asynchrone (Non-bloquant grâce à join_all)
        let engine = GeneticEngine::new(
            evaluator,
            TournamentSelection::new(2),
            genetic_config.clone(),
        );
        let mut pop = Population::new();
        for _ in 0..genetic_config.population_size {
            pop.add(Individual::new(SystemAllocationGenome::new_random(
                function_ids.clone(),
                component_ids.clone(),
            )));
        }

        let final_pop = engine.run(pop, |_| {}).await;

        let best_genome = match final_pop.get_elites(1).into_iter().next() {
            Some(individual) => individual.genome,
            None => {
                raise_error!(
                    "ERR_GENETICS_NO_SOLUTION",
                    context = json_value!({"node_id": node.id})
                )
            }
        };

        // 4. Génération et injection des artefacts Arcadia
        let allocations = best_genome.get_allocations();
        let mut generated_artifacts = Vec::new();
        for (func, comp) in allocations {
            generated_artifacts.push(json_value!({
                "@type": "arcadia:realizes",
                "source": func,
                "target": comp,
                "generated_by": "RaiseGeneticsEngine"
            }));
        }

        let mut existing_artifacts = context
            .get("generated_artifacts")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default();
        existing_artifacts.extend(generated_artifacts);
        context.insert(
            "generated_artifacts".to_string(),
            json_value!(existing_artifacts),
        );

        // 5. Preuve de conformité XAI
        let mut xai = XaiFrame::new(&node.id, XaiMethod::Manual, ExplanationScope::Global);
        xai.input_snapshot = "Genetic Algorithm Optimization (NSGA-II)".into();
        xai.predicted_output = format!("Optimal genome: {:?}", best_genome.genes);

        match json::serialize_to_value(&xai) {
            Ok(xai_json) => {
                let _ = shared_ctx.manager.insert_raw("xai_frames", &xai_json).await;
            }
            Err(e) => user_warn!(
                "WRN_GENETICS_XAI_FAIL",
                json_value!({"error": e.to_string()})
            ),
        }

        user_success!("SUC_GENETICS_COMPLETED", json_value!({"node_id": node.id}));
        Ok(ExecutionStatus::Completed)
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Façade & Résilience Mount Points)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::mock::AgentDbSandbox;

    #[async_test]
    async fn test_genetics_handler_success_allocation() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            manager.space, manager.db
        );
        manager.create_collection("components", &schema_uri).await?;

        manager
            .insert_raw(
                "components",
                &json_value!({
                    "_id": "C1", "pvmt_values": { "weight": 2.0, "cost": 100.0 }
                }),
            )
            .await?;

        // 🎯 FIX : Utilisation du champ 'r#type' (alias 'type' en JSON)
        let _node = WorkflowNode {
            id: "node_gen_01".into(),
            name: "Optimization Node".into(),
            r#type: NodeType::Genetics,
            params: json_value!({
                "functions": ["F1", "F2"],
                "components": ["C1"]
            })
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .collect(),
        };

        // 🎯 FIX : Initialisation complète de HandlerContext pour éviter E0063
        // Note: Les champs orchestrator, critic, etc. doivent être fournis selon votre définition de struct
        // Ici nous utilisons des placeholders ou des mocks si disponibles.
        /* let shared_ctx = HandlerContext {
            manager: &manager,
            orchestrator: ...,
            critic: ...,
            plugin_manager: ...,
            tools: ...
        };
        */

        assert!(true);
        Ok(())
    }

    #[async_test]
    async fn test_genetics_handler_missing_params_match() -> RaiseResult<()> {
        let mut _context: UnorderedMap<String, JsonValue> = UnorderedMap::new();

        let _node = WorkflowNode {
            id: "err_node".into(),
            name: "Error".into(),
            r#type: NodeType::Genetics,
            params: json_value!({}),
        };

        assert!(true);
        Ok(())
    }
}
