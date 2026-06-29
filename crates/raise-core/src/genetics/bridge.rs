// FICHIER : src-tauri/src/genetics/bridge.rs

use crate::genetics::dto::AllocatedSolution;
use crate::genetics::evaluators::architecture::ArchitectureCostModel;
use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

// --- Interfaces d'Entrée ---

pub trait SystemModelProvider {
    fn get_functions(&self) -> Vec<ModelFunction>;
    fn get_components(&self) -> Vec<ModelComponent>;
    fn get_exchanges(&self) -> Vec<ModelExchange>;
}

#[derive(Debug, Clone)]
pub struct ModelFunction {
    pub id: String,
    pub name: String,
    pub complexity: f32,
}

#[derive(Debug, Clone)]
pub struct ModelComponent {
    pub id: String,
    pub name: String,
    pub capacity_limit: f32,
}

#[derive(Debug, Clone)]
pub struct ModelExchange {
    pub source_id: String,
    pub target_id: String,
    pub data_volume: f32,
}

// --- Implémentation du Pont sur Arcadia ProjectModel ---

impl SystemModelProvider for ProjectModel {
    fn get_functions(&self) -> Vec<ModelFunction> {
        // 🎯 PURE GRAPH : Utilisation de get_collection
        self.get_collection("la", "functions")
            .iter()
            .map(|f| {
                let complexity = f
                    .properties
                    .get("complexity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(10.0) as f32;
                let id = f
                    .properties
                    .get("handle")
                    .and_then(|v| v.as_str())
                    .unwrap_or(f.handle.as_str())
                    .to_string();

                ModelFunction {
                    id,
                    name: f.name.as_str().to_string(),
                    complexity,
                }
            })
            .collect()
    }

    fn get_components(&self) -> Vec<ModelComponent> {
        // 🎯 PURE GRAPH : Utilisation de get_collection
        self.get_collection("pa", "components")
            .iter()
            .map(|c| {
                let capacity = c
                    .properties
                    .get("capacity")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(100.0) as f32;

                let id = c
                    .properties
                    .get("handle")
                    .and_then(|v| v.as_str())
                    .unwrap_or(c.handle.as_str())
                    .to_string();

                ModelComponent {
                    id,
                    name: c.name.as_str().to_string(),
                    capacity_limit: capacity,
                }
            })
            .collect()
    }

    fn get_exchanges(&self) -> Vec<ModelExchange> {
        // 🎯 PURE GRAPH : Utilisation de get_collection
        self.get_collection("la", "exchanges")
            .iter()
            .map(|e| {
                let source_id = e
                    .properties
                    .get("source_handle")
                    .or_else(|| e.properties.get("source"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let target_id = e
                    .properties
                    .get("target_handle")
                    .or_else(|| e.properties.get("target"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                let volume = e
                    .properties
                    .get("volume")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(1.0) as f32;

                ModelExchange {
                    source_id,
                    target_id,
                    data_volume: volume,
                }
            })
            .collect()
    }
}

// --- L'adaptateur (Adapter) ---

pub struct GeneticsAdapter {
    func_idx_to_id: Vec<String>,
    comp_idx_to_id: Vec<String>,
    func_id_to_idx: UnorderedMap<String, usize>,
}

impl GeneticsAdapter {
    pub fn new(provider: &impl SystemModelProvider) -> Self {
        let functions = provider.get_functions();
        let components = provider.get_components();

        let mut func_id_to_idx = UnorderedMap::new();
        let mut func_idx_to_id = Vec::with_capacity(functions.len());
        for (i, f) in functions.iter().enumerate() {
            func_id_to_idx.insert(f.id.clone(), i);
            func_idx_to_id.push(f.id.clone());
        }

        let comp_idx_to_id = components.iter().map(|c| c.id.clone()).collect();

        Self {
            func_idx_to_id,
            comp_idx_to_id,
            func_id_to_idx,
        }
    }

    pub fn build_cost_model(&self, provider: &impl SystemModelProvider) -> ArchitectureCostModel {
        let functions = provider.get_functions();
        let components = provider.get_components();
        let exchanges = provider.get_exchanges();

        let loads: Vec<(usize, f32)> = functions
            .iter()
            .enumerate()
            .map(|(i, f)| (i, f.complexity))
            .collect();

        let capacities: Vec<(usize, f32)> = components
            .iter()
            .enumerate()
            .map(|(i, c)| (i, c.capacity_limit))
            .collect();

        let mut flow_triplets: Vec<(usize, usize, f32)> = Vec::new();
        for ex in exchanges {
            let src_idx = self.func_id_to_idx.get(&ex.source_id);
            let tgt_idx = self.func_id_to_idx.get(&ex.target_id);

            if let (Some(&s), Some(&t)) = (src_idx, tgt_idx) {
                flow_triplets.push((s, t, ex.data_volume));
            }
        }

        ArchitectureCostModel::new(
            self.func_idx_to_id.len(),
            self.comp_idx_to_id.len(),
            &flow_triplets,
            &loads,
            &capacities,
        )
    }

    pub fn convert_solution(
        &self,
        raw_fitness: Vec<f32>,
        raw_violation: f32,
        raw_genes: &[usize],
    ) -> AllocatedSolution {
        let allocation_map: Vec<(String, String)> = raw_genes
            .iter()
            .enumerate()
            .map(|(func_idx, &comp_idx)| {
                let f_id = self
                    .func_idx_to_id
                    .get(func_idx)
                    .cloned()
                    .unwrap_or_default();
                let c_id = self
                    .comp_idx_to_id
                    .get(comp_idx)
                    .cloned()
                    .unwrap_or_default();
                (f_id, c_id)
            })
            .collect();

        AllocatedSolution {
            fitness: raw_fitness,
            constraint_violation: raw_violation,
            allocation: allocation_map,
        }
    }
}

// --- TESTS UNITAIRES ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::{ArcadiaElement, ProjectModel};

    fn make_element(id: &str, name: &str, kind: &str) -> RaiseResult<ArcadiaElement> {
        Ok(ArcadiaElement {
            handle: id.try_into()?,
            name: I18nString::Single(name.to_string()),
            kind: vec![kind.to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        })
    }

    fn create_mock_project() -> RaiseResult<ProjectModel> {
        let mut model = ProjectModel::default();

        // 1. Navigation (F1)
        let mut f1 = make_element("F1", "Navigation", "LogicalFunction")?;
        f1.properties.insert("handle".into(), json_value!("fn_nav"));
        f1.properties.insert("complexity".into(), json_value!(20.0));
        // On insère f1 dans le modèle APRÈS toutes les modifs
        model.add_element("la", "functions", f1);

        // 2. Radio (F2)
        let mut f2 = make_element("F2", "Radio", "LogicalFunction")?;
        f2.properties
            .insert("handle".into(), json_value!("fn_radio"));
        f2.properties.insert("complexity".into(), json_value!(10.0));
        model.add_element("la", "functions", f2);

        // 3. CPU (C1)
        let mut c1 = make_element("C1", "MainCPU", "PhysicalComponent")?;
        c1.properties
            .insert("handle".into(), json_value!("comp_cpu"));
        c1.properties.insert("capacity".into(), json_value!(100.0));
        model.add_element("pa", "components", c1);

        // 4. DataLink (E1)
        let mut ex = make_element("E1", "DataLink", "FunctionalExchange")?;
        ex.properties
            .insert("source_handle".into(), json_value!("fn_nav"));
        ex.properties
            .insert("target_handle".into(), json_value!("fn_radio"));
        ex.properties.insert("volume".into(), json_value!(50.0));
        model.add_element("la", "exchanges", ex);

        Ok(model)
    }

    #[test]
    fn test_cost_model_building_corrected() -> RaiseResult<()> {
        let model = create_mock_project()?;
        let adapter = GeneticsAdapter::new(&model);
        let cost_model = adapter.build_cost_model(&model);

        assert_eq!(cost_model.function_loads.len(), 2);
        assert_eq!(cost_model.component_capacities.len(), 1);
        assert_eq!(cost_model.data_flow_matrix[0][1], 50.0);
        Ok(())
    }

    #[test]
    fn test_solution_conversion_back_to_ids() -> RaiseResult<()> {
        let model = create_mock_project()?;
        let adapter = GeneticsAdapter::new(&model);
        let raw_genes = vec![0, 0];
        let sol = adapter.convert_solution(vec![1.0], 0.0, &raw_genes);

        assert_eq!(
            sol.allocation[0],
            ("fn_nav".to_string(), "comp_cpu".to_string())
        );
        Ok(())
    }
}
