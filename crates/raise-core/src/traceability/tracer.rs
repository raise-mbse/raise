// FICHIER : src-tauri/src/traceability/tracer.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::jsonld::vocabulary::PropertyType;
use crate::json_db::jsonld::{ContextManager, VocabularyRegistry};
use crate::model_engine::types::ProjectModel;
use crate::utils::prelude::*;

/// Service de traçabilité basé sur un Graphe d'identifiants.
pub struct Tracer {
    downstream_links: UnorderedMap<String, Vec<String>>,
    upstream_links: UnorderedMap<String, Vec<String>>,
}

impl Tracer {
    /// Initialisation depuis le JsonDb (Architecture cible)
    pub async fn from_db(manager: &CollectionsManager<'_>) -> RaiseResult<Self> {
        let mut docs = Vec::new();
        if let Ok(collections) = manager.list_collections().await {
            for col in collections {
                if let Ok(col_docs) = manager.list_all(&col).await {
                    docs.extend(col_docs);
                }
            }
        }
        Self::build_graph(docs)
    }

    /// 🎯 PURE GRAPH : Initialisation via l'itérateur universel
    pub fn from_legacy_model(model: &ProjectModel) -> RaiseResult<Self> {
        let mut docs = Vec::new();

        // On itère sur absolument tout le modèle de manière dynamique
        for e in model.all_elements() {
            if let Ok(val) = crate::utils::json::serialize_to_value(e) {
                docs.push(val);
            }
        }

        Self::build_graph(docs)
    }

    pub fn from_json_list(documents: Vec<JsonValue>) -> RaiseResult<Self> {
        Self::build_graph(documents)
    }

    fn build_graph(documents: Vec<JsonValue>) -> RaiseResult<Self> {
        let mut downstream: UnorderedMap<String, Vec<String>> = UnorderedMap::new();
        let mut upstream: UnorderedMap<String, Vec<String>> = UnorderedMap::new();
        let ctx = ContextManager::new()?;
        let registry = VocabularyRegistry::global()?;

        for doc in documents {
            let id = match doc.get("handle").and_then(|v| v.as_str()) {
                Some(id) => id.to_string(),
                None => continue,
            };

            let properties = doc
                .get("properties")
                .and_then(|p| p.as_object())
                .or(doc.as_object());

            if let Some(props) = properties {
                for (key, value) in props {
                    if is_link_property(key, &ctx, registry) {
                        let mut targets = Vec::new();
                        if let Some(target_id) = value.as_str() {
                            targets.push(target_id.to_string());
                        } else if let Some(arr) = value.as_array() {
                            for t in arr {
                                if let Some(target_id) = t.as_str() {
                                    targets.push(target_id.to_string());
                                }
                            }
                        }

                        for target_id in &targets {
                            upstream
                                .entry(target_id.clone())
                                .or_default()
                                .push(id.clone());
                        }
                        downstream.entry(id.clone()).or_default().extend(targets);
                    }
                }
            }
        }

        Ok(Self {
            downstream_links: downstream,
            upstream_links: upstream,
        })
    }

    pub fn get_downstream_ids(&self, element_id: &str) -> Vec<String> {
        self.downstream_links
            .get(element_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_upstream_ids(&self, element_id: &str) -> Vec<String> {
        self.upstream_links
            .get(element_id)
            .cloned()
            .unwrap_or_default()
    }
}

fn is_link_property(key: &str, ctx: &ContextManager, registry: &VocabularyRegistry) -> bool {
    if matches!(
        key,
        "allocatedTo" | "realizedBy" | "satisfiedBy" | "verifiedBy" | "model_id"
    ) {
        return true;
    }
    let expanded_uri = ctx.expand_term(key);
    if let Some(prop) = registry.get_property(&expanded_uri) {
        return prop.property_type == PropertyType::ObjectProperty;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::json_db::jsonld::VocabularyRegistry;
    use crate::model_engine::types::ArcadiaElement;
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
    async fn test_reverse_indexing_ai_model() -> RaiseResult<()> {
        let _sandbox = init_test_env().await?;

        let mut model = ProjectModel::default();

        // 1. Cible
        model.add_element(
            "pa",
            "components",
            ArcadiaElement {
                handle: "ai_1".try_into()?,
                kind: vec!["AIModel".into()],
                ..Default::default()
            },
        );

        // 2. Source
        let mut props = UnorderedMap::new();
        props.insert("model_id".to_string(), json_value!("ai_1"));

        model.add_element(
            "pa",
            "components",
            ArcadiaElement {
                handle: "rep_1".try_into()?,
                kind: vec!["QualityReport".into()],
                properties: props, // Ceci sera sérialisé sous {"properties": {"model_id": ...}}
                ..Default::default()
            },
        );

        // 🎯 DEBUG : Force l'affichage pour vérifier la structure sérialisée
        let docs: Vec<JsonValue> = model
            .all_elements()
            .iter()
            .map(|e| crate::utils::json::serialize_to_value((*e).clone()).unwrap())
            .collect();
        println!("DEBUG JSON: {:?}", docs);

        let tracer = Tracer::from_legacy_model(&model)?;

        let upstream = tracer.get_upstream_ids("ai_1");

        assert_eq!(
            upstream.len(),
            1,
            "Le lien inverse (upstream) n'a pas été détecté."
        );
        assert_eq!(upstream[0], "rep_1");

        Ok(())
    }
}
