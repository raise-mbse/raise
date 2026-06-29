// FICHIER : crates/raise-core/src/model_engine/dsl/mapper.rs

use super::parser::Rule;
use crate::json_db::collections::manager::CollectionsManager;
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use crate::utils::prelude::*;
use pest::iterators::Pair;

pub struct DslToArcadiaMapper;

impl Default for DslToArcadiaMapper {
    fn default() -> Self {
        Self::new()
    }
}

impl DslToArcadiaMapper {
    pub fn new() -> Self {
        Self
    }

    pub async fn transform(
        &self,
        parsed_file: Pair<'_, Rule>,
        _manager: &CollectionsManager<'_>,
    ) -> RaiseResult<ProjectModel> {
        let mut model = ProjectModel::default();

        for pair in parsed_file.into_inner() {
            if pair.as_rule() == Rule::dapp_block {
                self.traverse_dapp(pair, &mut model).await?;
            }
        }

        Ok(model)
    }

    async fn traverse_dapp(
        &self,
        pair: Pair<'_, Rule>,
        model: &mut ProjectModel,
    ) -> RaiseResult<()> {
        let mut inner = pair.into_inner();
        let handle = inner.next().unwrap().as_str().trim_matches('"').to_string();

        let mut properties = UnorderedMap::new();

        for item in inner {
            match item.as_rule() {
                Rule::attribute => self.extract_attribute(item, &mut properties),
                Rule::pvmt_block => self.extract_block(item, "pvmt_values", &mut properties),
                Rule::service_block => self.traverse_service(item, model, &handle).await?,
                _ => {}
            }
        }

        let name_val = properties
            .remove("name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| handle.clone());

        let element = ArcadiaElement {
            handle: handle.as_str().try_into()?,
            name: I18nString::Single(name_val),
            kind: vec!["raise:Dapp".to_string(), "pa:PhysicalComponent".to_string()],
            xmi_id: None,
            summary: None,
            description: None,
            property_value_ids: vec![],
            properties,
        };

        model.add_element("pa", "components", element);
        Ok(())
    }

    async fn traverse_service(
        &self,
        pair: Pair<'_, Rule>,
        model: &mut ProjectModel,
        parent_handle: &str,
    ) -> RaiseResult<()> {
        let mut inner = pair.into_inner();
        let handle = inner.next().unwrap().as_str().trim_matches('"').to_string();

        let mut properties = UnorderedMap::new();
        // 🎯 LIAISON TOP-DOWN : Préparation pour la résolution spatiale de l'ingestion
        properties.insert(
            "raise:belongsToDapp".to_string(),
            json_value!(parent_handle),
        );

        for item in inner {
            match item.as_rule() {
                Rule::attribute => self.extract_attribute(item, &mut properties),
                Rule::pvmt_block => self.extract_block(item, "pvmt_values", &mut properties),
                Rule::runtime_block => self.extract_block(item, "runtime_rules", &mut properties),
                Rule::resource_block => {
                    self.extract_block(item, "resource_constraints", &mut properties)
                }
                Rule::module_block => self.traverse_module(item, model, &handle).await?,
                _ => {}
            }
        }

        let name_val = properties
            .remove("name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| handle.clone());

        let element = ArcadiaElement {
            handle: handle.as_str().try_into()?,
            name: I18nString::Single(name_val),
            kind: vec![
                "raise:Service".to_string(),
                "sa:SystemComponent".to_string(),
            ],
            xmi_id: None,
            summary: None,
            description: None,
            property_value_ids: vec![],
            properties,
        };

        model.add_element("sa", "components", element);
        Ok(())
    }

    async fn traverse_module(
        &self,
        pair: Pair<'_, Rule>,
        model: &mut ProjectModel,
        parent_handle: &str,
    ) -> RaiseResult<()> {
        let mut inner = pair.into_inner();
        let handle = inner.next().unwrap().as_str().trim_matches('"').to_string();

        let mut properties = UnorderedMap::new();
        // 🎯 LIAISON TOP-DOWN : Préparation pour la résolution spatiale de l'ingestion
        properties.insert(
            "raise:belongsToService".to_string(),
            json_value!(parent_handle),
        );

        for item in inner {
            match item.as_rule() {
                Rule::attribute => self.extract_attribute(item, &mut properties),
                Rule::pvmt_block => self.extract_block(item, "pvmt_values", &mut properties),
                _ => {}
            }
        }

        let name_val = properties
            .remove("name")
            .and_then(|v| v.as_str().map(|s| s.to_string()))
            .unwrap_or_else(|| handle.clone());

        let element = ArcadiaElement {
            handle: handle.as_str().try_into()?,
            name: I18nString::Single(name_val),
            kind: vec![
                "raise:Module".to_string(),
                "la:LogicalComponent".to_string(),
            ],
            xmi_id: None,
            summary: None,
            description: None,
            property_value_ids: vec![],
            properties,
        };

        model.add_element("la", "components", element);
        Ok(())
    }

    fn extract_attribute(
        &self,
        pair: Pair<'_, Rule>,
        properties: &mut UnorderedMap<String, JsonValue>,
    ) {
        let mut inner = pair.into_inner();
        let key = inner.next().unwrap().as_str();
        let val_pair = inner.next().unwrap().into_inner().next().unwrap();

        let json_val = match val_pair.as_rule() {
            Rule::string_val => json_value!(val_pair.as_str().trim_matches('"')),
            Rule::number_val => json_value!(val_pair.as_str().parse::<i64>().unwrap_or(0)),
            Rule::boolean_val => json_value!(val_pair.as_str() == "true"),
            _ => json_value!(null),
        };
        properties.insert(key.to_string(), json_val);
    }

    fn extract_block(
        &self,
        pair: Pair<'_, Rule>,
        block_key: &str,
        properties: &mut UnorderedMap<String, JsonValue>,
    ) {
        let mut block_props = UnorderedMap::new();
        for attr in pair.into_inner() {
            if attr.as_rule() == Rule::attribute {
                self.extract_attribute(attr, &mut block_props);
            }
        }
        properties.insert(block_key.to_string(), json_value!(block_props));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::dsl::parser::parse_dsl_text;
    use crate::utils::testing::mock::AgentDbSandbox;

    #[tokio::test]
    async fn test_mapper_full_hierarchy_and_constraints() -> RaiseResult<()> {
        let input = r#"
        dapp "core_engine" {
            type = "daemon"
            
            pvmt {
                frugality_score = 8
            }
            
            service "auth_service" {
                status = "enabled"
                
                runtime {
                    timeout_ms = 3000
                }
                
                module "validator" {
                    visibility = "public"
                }
            }
        }
        "#;

        let parsed = parse_dsl_text(input)?;
        let file_pair = parsed.into_iter().next().unwrap();

        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test_space", "test_db");
        let mapper = DslToArcadiaMapper::new();

        let model = mapper.transform(file_pair, &manager).await?;

        // --- ASSERTIONS DAPP ---
        let dapps = model.get_collection("pa", "components");
        assert_eq!(dapps.len(), 1);
        let dapp = &dapps[0];
        assert_eq!(dapp.handle.as_str(), "core_engine");
        assert_eq!(
            dapp.kind,
            vec!["raise:Dapp".to_string(), "pa:PhysicalComponent".to_string()]
        );

        // --- ASSERTIONS SERVICE ---
        let services = model.get_collection("sa", "components");
        assert_eq!(services.len(), 1);
        let service = &services[0];
        assert_eq!(service.handle.as_str(), "auth_service");
        assert_eq!(
            service.kind,
            vec![
                "raise:Service".to_string(),
                "sa:SystemComponent".to_string()
            ]
        );
        assert_eq!(
            service
                .properties
                .get("raise:belongsToDapp")
                .and_then(|v| v.as_str()),
            Some("core_engine")
        );

        // --- ASSERTIONS MODULE ---
        let modules = model.get_collection("la", "components");
        assert_eq!(modules.len(), 1);
        let module = &modules[0];
        assert_eq!(module.handle.as_str(), "validator");
        assert_eq!(
            module.kind,
            vec![
                "raise:Module".to_string(),
                "la:LogicalComponent".to_string()
            ]
        );
        assert_eq!(
            module
                .properties
                .get("raise:belongsToService")
                .and_then(|v| v.as_str()),
            Some("auth_service")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_mapper_attribute_types() -> RaiseResult<()> {
        let input = r#"
        dapp "test_types" {
            string_attr = "valeur"
            number_attr = 42
            bool_attr = true
            name = "Super App"
        }
        "#;

        let parsed = parse_dsl_text(input)?;
        let file_pair = parsed.into_iter().next().unwrap();

        let sandbox = AgentDbSandbox::new().await?;
        let manager = CollectionsManager::new(&sandbox.db, "test_space", "test_db");
        let mapper = DslToArcadiaMapper::new();
        let model = mapper.transform(file_pair, &manager).await?;

        let dapp = &model.get_collection("pa", "components")[0];

        assert_eq!(dapp.name.as_str(), "Super App");
        assert_eq!(
            dapp.properties.get("string_attr").and_then(|v| v.as_str()),
            Some("valeur")
        );
        assert_eq!(
            dapp.properties.get("number_attr").and_then(|v| v.as_i64()),
            Some(42)
        );
        assert_eq!(
            dapp.properties.get("bool_attr").and_then(|v| v.as_bool()),
            Some(true)
        );

        Ok(())
    }
}
