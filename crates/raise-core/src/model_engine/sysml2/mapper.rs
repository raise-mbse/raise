// FICHIER : src-tauri/src/model_engine/sysml2/mapper.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*;

use super::parser::{Rule, Sysml2Parser};
use crate::model_engine::types::{ArcadiaElement, ProjectModel};
use pest::Parser;

#[derive(Default)]
pub struct Sysml2ToArcadiaMapper;

impl Sysml2ToArcadiaMapper {
    pub fn new() -> Self {
        Self
    }

    pub async fn transform(
        &self,
        sysml_content: &str,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<ProjectModel> {
        let mut model = ProjectModel::default();

        // 1. Parsing initial
        let mut pairs = match Sysml2Parser::parse(Rule::file, sysml_content) {
            Ok(p) => p,
            Err(e) => raise_error!(
                "ERR_SYSML_SYNTAX_INVALID",
                error = e,
                context = json_value!({ "action": "parse_sysml_v2" })
            ),
        };

        let parsed_file = match pairs.next() {
            Some(pair) => pair,
            None => raise_error!(
                "ERR_SYSML_EMPTY_FILE",
                error = "Le parseur n'a retourné aucun contenu.",
                context = json_value!({ "action": "extract_ast_root" })
            ),
        };

        // 2. Lecture du mapping dynamique
        let mapping_doc = manager
            .get_document("configs", "ref:configs:handle:ontological_mapping")
            .await?
            .unwrap_or(json_value!({}));

        let default_sysml_mappings = json_value!({
            "part_def": {
                "SystemAnalysis": { "layer": "sa", "col": "components", "kind": "SystemComponent" },
                "LogicalArchitecture": { "layer": "la", "col": "components", "kind": "LogicalComponent" },
                "PhysicalArchitecture": { "layer": "pa", "col": "components", "kind": "PhysicalComponent" },
                "default": { "layer": "oa", "col": "entities", "kind": "OperationalEntity" }
            },
            "actor_def": {
                "SystemAnalysis": { "layer": "sa", "col": "actors", "kind": "SystemActor" },
                "default": { "layer": "oa", "col": "actors", "kind": "OperationalActor" }
            }
        });

        let sysml_mappings = if mapping_doc["sysml2_mappings"].is_null() {
            &default_sysml_mappings
        } else {
            &mapping_doc["sysml2_mappings"]
        };

        // 3. Traversal AST
        self.traverse_ast(parsed_file, &mut model, "UnknownLayer", sysml_mappings)?;

        Ok(model)
    }

    fn traverse_ast(
        &self,
        pair: pest::iterators::Pair<Rule>,
        model: &mut ProjectModel,
        current_layer: &str,
        mappings: &JsonValue,
    ) -> RaiseResult<()> {
        match pair.as_rule() {
            Rule::package_decl => {
                let mut inner_rules = pair.into_inner();
                let pkg_name = inner_rules.next().unwrap().as_str();
                for inner_pair in inner_rules {
                    self.traverse_ast(inner_pair, model, pkg_name, mappings)?;
                }
                Ok(())
            }
            Rule::requirement_def | Rule::part_def | Rule::actor_def => {
                let rule_type = format!("{:?}", pair.as_rule());
                let ident = pair
                    .clone()
                    .into_inner()
                    .find(|p| p.as_rule() == Rule::ident)
                    .map(|p| p.as_str())
                    .unwrap_or("Unknown");

                // 🎯 RÉSOLUTION DYNAMIQUE DES COORDONNÉES
                let (layer, col, kind) = if let Some(rule_map) = mappings.get(&rule_type) {
                    let config = rule_map
                        .get(current_layer)
                        .or_else(|| rule_map.get("default"))
                        .unwrap();
                    (
                        config["layer"].as_str().unwrap_or("oa"),
                        config["col"].as_str().unwrap_or("entities"),
                        config["kind"].as_str().unwrap_or("Unknown"),
                    )
                } else {
                    match pair.as_rule() {
                        Rule::requirement_def => ("transverse", "requirements", "Requirement"),
                        _ => ("oa", "others", "Unknown"),
                    }
                };

                let element = ArcadiaElement {
                    handle: format!("{}-{}-{}", layer, col, ident.to_lowercase())
                        .as_str()
                        .try_into()?,
                    name: I18nString::Single(ident.to_string()),
                    kind: vec![kind.to_string()],
                    ..Default::default()
                };

                // 🎯 FIX : On passe maintenant les 3 arguments requis par ProjectModel
                model.add_element(layer, col, element);
                Ok(())
            }
            _ => {
                for inner_pair in pair.into_inner() {
                    self.traverse_ast(inner_pair, model, current_layer, mappings)?;
                }
                Ok(())
            }
        }
    }
}
