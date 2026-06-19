// FICHIER : crates/raise-core/src/ai/tools/codegen_tool.rs

use crate::ai::protocols::mcp::{McpTool, McpToolCall, McpToolResult, ToolDefinition};
use crate::code_generator::models::{CodeElement, CodeElementType, Module, Visibility};
use crate::code_generator::CodeGeneratorService;
use crate::json_db::collections::manager::CollectionsManager;
use crate::json_db::storage::StorageEngine;
use crate::utils::data::config::AppConfig;
use crate::utils::prelude::*; // 🎯 Façade Unique

pub struct CodeGenTool {
    service: CodeGeneratorService,
    tool_def: ToolDefinition, // Cache de la définition générée dynamiquement via les schémas
    domain_root: PathBuf,
}

impl CodeGenTool {
    /// Initialise l'outil de génération de code de manière 100% stricte (Zéro Dette).
    pub async fn new(
        domain_root: PathBuf,
        db: SharedRef<StorageEngine>,
        space: &str,
        db_name: &str,
    ) -> RaiseResult<Self> {
        let manager = CollectionsManager::new(&db, space, db_name);
        let service = CodeGeneratorService::new(domain_root.clone(), &manager).await?;

        // 1. Lecture stricte
        let mcp_config =
            match AppConfig::get_runtime_settings(&manager, "ref:components:handle:codegen_engine")
                .await
            {
                Ok(doc) => doc,
                Err(e) => raise_error!(
                    "ERR_CODEGEN_CONFIG_MISSING",
                    error = e,
                    context = json_value!({ "component": "codegen_engine" })
                ),
            };

        // 2. Extraction stricte du tableau tools
        let tools = match mcp_config.get("tools").and_then(|t| t.as_array()) {
            Some(t) => t,
            None => raise_error!(
                "ERR_CODEGEN_TOOLS_MISSING",
                error = "Tableau 'tools' absent."
            ),
        };

        // 3. Identification stricte de l'outil
        let mutate_tool = match tools
            .iter()
            .find(|t| t.get("tool_id").and_then(|v| v.as_str()) == Some("mutate_ast_node"))
        {
            Some(t) => t,
            None => raise_error!(
                "ERR_CODEGEN_TOOL_NOT_FOUND",
                error = "Outil 'mutate_ast_node' introuvable dans la configuration."
            ),
        };

        let tool_name = match mutate_tool.get("tool_id").and_then(|v| v.as_str()) {
            Some(n) => n.to_string(),
            None => raise_error!("ERR_CODEGEN_ID_MISSING", error = "tool_id manquant."),
        };

        let tool_desc = match mutate_tool.get("description").and_then(|v| v.as_str()) {
            Some(d) => d.to_string(),
            None => raise_error!("ERR_CODEGEN_DESC_MISSING", error = "description manquante."),
        };

        // 4. Résolution stricte du schéma
        let schema_uri = match mutate_tool.get("input_schema_uri").and_then(|v| v.as_str()) {
            Some(u) => u,
            None => raise_error!(
                "ERR_CODEGEN_INPUT_URI_MISSING",
                error = "input_schema_uri manquante."
            ),
        };

        let mut schema_doc = match manager.get_schema_def(schema_uri).await {
            Ok(s) => s,
            Err(e) => raise_error!(
                "ERR_CODEGEN_SCHEMA_RESOLUTION",
                error = format!("Impossible de résoudre {} : {}", schema_uri, e)
            ),
        };

        // 5. Résolution des propriétés héritées (On injecte la cible sémantique et physique)
        if let Some(props) = schema_doc
            .get_mut("properties")
            .and_then(|p| p.as_object_mut())
        {
            if !props.contains_key("handle") {
                props.insert(
                    "handle".to_string(),
                    json_value!({
                        "type": "string",
                        "description": "Ancre sémantique unique (ex: fn:boot_physical_node)"
                    }),
                );
            }
            if !props.contains_key("module_name") {
                props.insert(
                    "module_name".to_string(),
                    json_value!({
                        "type": "string",
                        "description": "Nom du module d'orchestration cible (ex: mod_kernel_environment_rs)"
                    }),
                );
            }
        }

        if let Some(req) = schema_doc
            .get_mut("required")
            .and_then(|r| r.as_array_mut())
        {
            if !req.contains(&json_value!("handle")) {
                req.push(json_value!("handle"));
            }
            if !req.contains(&json_value!("module_name")) {
                req.push(json_value!("module_name"));
            }
        }

        let tool_def = ToolDefinition {
            name: tool_name,
            description: tool_desc,
            input_schema: schema_doc,
        };

        Ok(Self {
            service,
            tool_def,
            domain_root,
        })
    }
}

#[async_interface]
impl McpTool for CodeGenTool {
    fn definition(&self) -> ToolDefinition {
        self.tool_def.clone()
    }

    async fn execute(&self, call: McpToolCall) -> McpToolResult {
        let args = &call.arguments;

        // 🎯 EXTRACTION STRICTE ZÉRO DETTE (On remonte les erreurs au LLM au lieu de deviner)
        let module_name = match args.get("module_name").and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v,
            _ => return McpToolResult::error(call.id, "Argument 'module_name' manquant ou vide."),
        };

        let handle = match args.get("handle").and_then(|v| v.as_str()) {
            Some(v) if !v.is_empty() => v,
            _ => return McpToolResult::error(call.id, "Argument 'handle' manquant ou vide."),
        };

        let el_type_str = match args.get("element_type").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return McpToolResult::error(call.id, "Argument 'element_type' manquant."),
        };

        let element_type = match el_type_str {
            "struct" => CodeElementType::Struct,
            "impl_block" => CodeElementType::ImplBlock,
            "enum" => CodeElementType::Enum,
            "trait" => CodeElementType::Trait,
            "test_function" => CodeElementType::TestFunction,
            "macro" => CodeElementType::Macro,
            "constant" => CodeElementType::Macro,
            "type_alias" => CodeElementType::Macro,
            "function" => CodeElementType::Function,
            _ => {
                return McpToolResult::error(
                    call.id,
                    &format!("Type d'élément '{}' non supporté.", el_type_str),
                )
            }
        };

        let vis_str = match args.get("visibility").and_then(|v| v.as_str()) {
            Some(v) => v,
            None => return McpToolResult::error(call.id, "Argument 'visibility' manquant."),
        };

        let visibility = match vis_str {
            "public" => Visibility::Public,
            "crate" => Visibility::Crate,
            "protected" => Visibility::Protected,
            "private" => Visibility::Private,
            _ => {
                return McpToolResult::error(
                    call.id,
                    &format!("Visibilité '{}' non supportée.", vis_str),
                )
            }
        };

        let signature = match args.get("signature").and_then(|v| v.as_str()) {
            Some(v) => v.to_string(),
            None => return McpToolResult::error(call.id, "Argument 'signature' manquant."),
        };

        let body = args
            .get("body")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let schema_uri = match self.service.get_schema_uri("software") {
            Ok(uri) => uri,
            Err(e) => {
                return McpToolResult::error(
                    call.id,
                    &format!("Erreur de résolution du schéma sémantique software : {}", e),
                )
            }
        };

        let mut meta = UnorderedMap::new();
        if !schema_uri.is_empty() {
            meta.insert("$schema".to_string(), schema_uri);
        }

        // Création de l'élément muté
        let mutated_element = CodeElement {
            module_id: None,
            parent_id: args
                .get("parent_id")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string()),
            element_type,
            handle: handle.to_string(),
            visibility,
            attributes: args
                .get("attributes")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            docs: Some("🤖 Muté par l'Agent IA (Zéro Dette)".to_string()),
            signature,
            body,
            elements: vec![],
            dependencies: args
                .get("dependencies")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default(),
            metadata: meta,
        };

        let module_path = self.domain_root.join(format!("{}_staged.rs", module_name));

        let mut target_module = match Module::new(
            module_name,
            module_path, // Utilisation du chemin absolu
        ) {
            Ok(m) => m,
            Err(e) => return McpToolResult::error(call.id, &e.to_string()),
        };
        target_module.elements.push(mutated_element);

        match self.service.stage_module(target_module).await {
            Ok(staged) => McpToolResult::success(
                call.id,
                json_value!({
                    "status": "Contrat de mutation généré avec succès",
                    "temp_path": staged.temp_path.to_string_lossy(),
                    "action_required": "Exécute 'raise-cli code-gen stage' pour appliquer la mutation physique."
                }),
            ),
            Err(e) => McpToolResult::error(call.id, &format!("Échec du Staging : {}", e)),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Validation Stricte Zéro Dette)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::data::config::AppConfig;
    use crate::utils::data::json::json_value;
    use crate::utils::testing::AgentDbSandbox;

    async fn inject_mock_codegen_config(
        manager: &CollectionsManager<'_>,
        with_schema: bool,
    ) -> RaiseResult<()> {
        let config = AppConfig::get();
        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        let input_uri = "v2/dapps/services/elements/code_element.schema.json";

        if with_schema {
            manager
                .create_schema_def(
                    input_uri,
                    json_value!({
                        "type": "object",
                        "properties": {}
                    }),
                )
                .await?;
        }

        let full_uri = manager.build_schema_uri(input_uri).await;

        // 🎯 FIX CRITIQUE : On écrase l'ID généré par AgentDbSandbox pour fusionner les configs
        manager.upsert_document("service_configs", json_value!({
            "_id": "cfg_codegen_engine_test",
            "component_id": "ref:components:handle:codegen_engine",
            "service_settings": {
                "format_on_save": false,
                "strict_mode": true,
                "semantic_routing": {
                    "software": { "aliases": ["rust"], "collection": "code_elements", "schema_uri": generic_schema.clone() }
                },
                "tools": [{
                    "tool_id": "mutate_ast_node",
                    "description": "Mutate AST",
                    "input_schema_uri": full_uri
                }]
            }
        })).await?;

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_codegen_init_fails_if_schema_missing() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // SANS SCHÉMA
        inject_mock_codegen_config(&manager, false).await?;

        let result = CodeGenTool::new(
            sandbox.domain_root.clone(),
            sandbox.db.clone(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )
        .await;

        match result {
            Err(e) if e.to_string().contains("ERR_CODEGEN_SCHEMA_RESOLUTION") => Ok(()),
            _ => panic!("L'initialisation aurait dû échouer par manque de schéma."),
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_codegen_tool_dynamic_definition() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        // AVEC SCHÉMA
        inject_mock_codegen_config(&manager, true).await?;

        let tool = CodeGenTool::new(
            sandbox.domain_root.clone(),
            sandbox.db.clone(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )
        .await?;

        let def = tool.definition();
        assert_eq!(def.name, "mutate_ast_node");

        let props = def
            .input_schema
            .get("properties")
            .expect("Le schéma MCP doit avoir 'properties'");
        assert!(props.get("module_name").is_some());
        assert!(props.get("handle").is_some());

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_codegen_tool_rejects_missing_arguments() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        inject_mock_codegen_config(&manager, true).await?;

        let tool = CodeGenTool::new(
            sandbox.domain_root.clone(),
            sandbox.db.clone(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )
        .await?;

        let call = McpToolCall::new(
            "mutate_ast_node",
            json_value!({
                "module_name": "mod_kernel_environment_rs",
                "body": "{ println!(\"test\"); }"
            }),
        );

        let result = tool.execute(call).await;

        assert!(
            result.is_error,
            "L'outil aurait dû rejeter la requête incomplète"
        );
        let error_msg = result.content.to_string();
        // Le premier argument manquant sera signalé (handle)
        assert!(error_msg.contains("manquant"));

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_codegen_tool_successful_mutation_staging() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        inject_mock_codegen_config(&manager, true).await?;

        let tool = CodeGenTool::new(
            sandbox.domain_root.clone(),
            sandbox.db.clone(),
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        )
        .await?;

        let call = McpToolCall::new(
            "mutate_ast_node",
            json_value!({
                "module_name": "mod_kernel_math_rs",
                "handle": "fn:calculate_entropy",
                "element_type": "function",
                "visibility": "public",
                "signature": "pub fn calculate_entropy(data: &[u8]) -> f64",
                "body": "{\n    0.42 // Implémentation factice pour le test\n}",
                "dependencies": ["ref:code_elements:handle:sys:imports"],
                "attributes": ["#[inline]", "#[allow(dead_code)]"]
            }),
        );

        let result = tool.execute(call).await;

        assert!(!result.is_error, "Le staging a échoué: {}", result.content);

        let json_str = result.content.to_string();
        assert!(json_str.contains("Contrat de mutation généré avec succès"));
        assert!(json_str.contains("mod_kernel_math_rs"));

        Ok(())
    }
}
