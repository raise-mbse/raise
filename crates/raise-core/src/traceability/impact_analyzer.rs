// FICHIER : src-tauri/src/traceability/impact_analyzer.rs

use super::tracer::Tracer;
use crate::utils::prelude::*;

#[derive(Debug, Serializable)]
pub struct ImpactReport {
    pub root_element_id: String,
    pub impacted_elements: Vec<ImpactedItem>,
}

#[derive(Debug, Serializable)]
pub struct ImpactedItem {
    pub element_id: String,
    pub distance: usize,
}

pub struct ImpactAnalyzer {
    tracer: Tracer,
}

impl ImpactAnalyzer {
    pub fn new(tracer: Tracer) -> Self {
        Self { tracer }
    }

    pub fn analyze(&self, element_id: &str, max_depth: usize) -> RaiseResult<ImpactReport> {
        let mut visited = UniqueSet::new();
        let mut impacted = Vec::new();

        if self.tracer.get_downstream_ids(element_id).is_empty()
            && self.tracer.get_upstream_ids(element_id).is_empty()
        {
            raise_error!(
                "ERR_IMPACT_ROOT_NOT_FOUND",
                context = json_value!({"id": element_id})
            );
        }

        self.traverse(element_id, 0, max_depth, &mut visited, &mut impacted)?;

        Ok(ImpactReport {
            root_element_id: element_id.to_string(),
            impacted_elements: impacted,
        })
    }

    fn traverse(
        &self,
        id: &str,
        depth: usize,
        max: usize,
        visited: &mut UniqueSet<String>,
        results: &mut Vec<ImpactedItem>,
    ) -> RaiseResult<()> {
        if depth > max || !visited.insert(id.to_string()) {
            return Ok(());
        }
        if depth > 0 {
            results.push(ImpactedItem {
                element_id: id.to_string(),
                distance: depth,
            });
        }
        for next_id in self.tracer.get_downstream_ids(id) {
            self.traverse(&next_id, depth + 1, max, visited, results)?;
        }
        for next_id in self.tracer.get_upstream_ids(id) {
            self.traverse(&next_id, depth + 1, max, visited, results)?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::json_db::jsonld::VocabularyRegistry;
    use crate::model_engine::types::{ArcadiaElement, ProjectModel};
    use crate::utils::testing::mock::DbSandbox;

    async fn init_test_env() -> RaiseResult<DbSandbox> {
        let sandbox = DbSandbox::new().await?;
        let mgr = CollectionsManager::new(
            &sandbox.storage,
            &sandbox.config.mount_points.system.domain,
            &sandbox.config.mount_points.system.db,
        );
        VocabularyRegistry::init_from_db(&mgr).await?;
        Ok(sandbox)
    }

    #[async_test]
    async fn test_impact_propagation_pure_graph() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;

        let mut model = ProjectModel::default();

        // 1. A doit posséder la propriété "allocatedTo" pointant vers B
        let mut p_a = UnorderedMap::new();
        p_a.insert("allocatedTo".into(), json_value!("B"));

        model.add_element(
            "sa",
            "functions",
            ArcadiaElement {
                handle: "A".try_into()?,
                kind: vec!["SystemFunction".into()],
                properties: p_a, // La relation est ICI
                ..Default::default()
            },
        );

        // 2. B n'a pas besoin de propriétés pour être une cible
        model.add_element(
            "sa",
            "functions",
            ArcadiaElement {
                handle: "B".try_into()?,
                kind: vec!["SystemFunction".into()],
                ..Default::default()
            },
        );

        let tracer = Tracer::from_legacy_model(&model)?;

        // 🎯 Vérification : A doit maintenant être vu comme parent de B
        assert!(
            !tracer.get_downstream_ids("A").is_empty(),
            "A doit avoir des éléments en aval"
        );

        let analyzer = ImpactAnalyzer::new(tracer);
        let report = analyzer.analyze("A", 1)?;

        assert!(
            report.impacted_elements.iter().any(|e| e.element_id == "B"),
            "L'impact n'a pas été propagé de A vers B."
        );

        Ok(())
    }
}
