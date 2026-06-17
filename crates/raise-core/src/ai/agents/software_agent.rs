// FICHIER : crates/raise-core/src/ai/agents/software_agent.rs

use crate::ai::llm::client::LlmBackend;
use crate::json_db::collections::manager::CollectionsManager;
use crate::services::codegen_service;
use crate::utils::data::config::AppConfig;
use crate::utils::data::json::Clearance;
use crate::utils::prelude::*;

use crate::ai::context::memory_store::MemoryStore;
use crate::ai::context::rag::RagRetriever;
use crate::ai::graph_store::builder::SoftwareGraphBuilder;
use crate::ai::world_model::perception::encoder::HybridEncoder;

use crate::genetics::evaluators::codegen::CodeGenEvaluator;
use crate::genetics::genomes::ast_arch::{AstGenome, AstNode};
use crate::genetics::traits::Evaluator;

use super::intent_classifier::EngineeringIntent;
use super::prompt_engine::PromptEngine;
use super::{Agent, AgentContext, AgentResult, CreatedArtifact};

/// Extrait de manière robuste le code source situé entre des balises markdown
fn extract_rust_code(text: &str) -> Option<String> {
    let text = text.trim();
    let start_tag = "```rust";
    let end_tag = "```";

    if let Some(start_idx) = text.find(start_tag) {
        let code_start = start_idx + start_tag.len();
        if let Some(end_idx) = text[code_start..].find(end_tag) {
            return Some(text[code_start..code_start + end_idx].trim().to_string());
        }
    }

    // Fallback générique si le LLM a juste mis "```" sans spécifier "rust"
    if let Some(start_idx) = text.find("```") {
        let code_start = start_idx + 3;
        if let Some(end_idx) = text[code_start..].find("```") {
            return Some(text[code_start..code_start + end_idx].trim().to_string());
        }
    }

    None
}

/// 🤖 L'Agent Logiciel (L'Architecte Code)
/// Entièrement Data-Driven : charge sa configuration et son prompt depuis la base _system.
/// Pilote la mutation de l'AST en interaction avec les outils MCP, le CodegenService et le RAG.
pub struct SoftwareAgent {
    id: String,
    domain: String,
    db_name: String,
}

impl SoftwareAgent {
    pub fn new(domain: String, db_name: String) -> Self {
        Self {
            id: "ref:agents:handle:agent_software".to_string(),
            domain,
            db_name,
        }
    }
}

#[async_interface]
impl Agent for SoftwareAgent {
    fn id(&self) -> &str {
        &self.id
    }

    async fn process(
        &self,
        ctx: &AgentContext,
        intent: &EngineeringIntent,
    ) -> RaiseResult<Option<AgentResult>> {
        // 1. Aiguillage d'Intention
        let EngineeringIntent::MutateCode {
            module_name,
            target_handle,
            instruction,
        } = intent
        else {
            return Ok(None);
        };

        crate::user_info!(
            "AI_CODER_START",
            json_value!({"target": target_handle, "instruction": instruction})
        );

        let config = AppConfig::get();
        let sys_manager = CollectionsManager::new(
            &ctx.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        let ws_manager = CollectionsManager::new(&ctx.db, &self.domain, &self.db_name);

        // 2. Récupération Dynamique du Profil de l'Agent
        let agent_doc = if let Ok(Some(doc)) =
            ws_manager.get_document("agents", "agent_software").await
        {
            doc
        } else if let Ok(Some(doc)) = sys_manager.get_document("agents", "agent_software").await {
            doc
        } else {
            raise_error!(
                "ERR_AGENT_NOT_FOUND",
                error = "La définition de l'agent logiciel est introuvable dans le graphe système."
            )
        };

        let prompt_handle = match agent_doc["base"]["neuro_profile"]["prompt_id"].as_str() {
            Some(id) => id,
            None => raise_error!(
                "ERR_AGENT_MISSING_PROMPT",
                error = "prompt_id absent du neuro_profile pour agent_software."
            ),
        };

        // 3. Compilation du Prompt Système (Injection des variables dynamiques)
        let prompt_vars = json_value!({
            "module_name": module_name,
            "target_handle": target_handle,
            "user_request": instruction
        });

        let prompt_engine_ws = PromptEngine::new(ctx.db.clone(), &self.domain, &self.db_name);
        let system_prompt = match prompt_engine_ws
            .compile(prompt_handle, Some(&prompt_vars))
            .await
        {
            Ok(p) => p,
            Err(_) => {
                let prompt_engine_sys = PromptEngine::new(
                    ctx.db.clone(),
                    &config.mount_points.system.domain,
                    &config.mount_points.system.db,
                );
                prompt_engine_sys
                    .compile(prompt_handle, Some(&prompt_vars))
                    .await?
            }
        };

        // 4. Investigation (Lecture DB Graphe via la Forteresse)
        let mut element_doc = match ws_manager
            .get_document("code_elements", target_handle)
            .await
        {
            Ok(Some(doc)) => doc,
            Ok(None) => raise_error!(
                "ERR_AI_CODER_ELEMENT_NOT_FOUND",
                error = format!(
                    "Le composant '{}' est introuvable dans le workspace actif.",
                    target_handle
                )
            ),
            Err(e) => raise_error!("ERR_DB_READ", error = e.to_string()),
        };

        let signature = element_doc["signature"].as_str().unwrap_or("").to_string();
        let body = element_doc["body"].as_str().unwrap_or("").to_string();
        let element_type = element_doc["element_type"]
            .as_str()
            .unwrap_or("")
            .to_string();

        // 5. Intégration du Contexte Global (Mémoire + RAG Textuel)
        let memory_store = MemoryStore::new(&sys_manager).await?;
        let mut session = memory_store
            .load_or_create(&sys_manager, &ctx.session_id)
            .await?;

        let rag_ctx = match RagRetriever::new(&sys_manager).await {
            Ok(mut rag) => rag
                .retrieve(&ws_manager, instruction, 3)
                .await
                .unwrap_or_default(),
            Err(e) => {
                crate::user_warn!("WARN_RAG_UNAVAILABLE", json_value!({"err": e.to_string()}));
                String::new()
            }
        };

        // 🌟 5.bis RAG Topologique (Graph RAG)
        let mut graph_ctx_str = String::new();
        let device = ComputeHardware::Cpu;

        let vb =
            NeuralWeightsBuilder::from_varmap(&ctx.world_engine.varmap, ComputeType::F32, &device);
        if let Ok(hybrid_encoder) = HybridEncoder::new(384, 16, vb) {
            if let Ok((adj, _features)) =
                SoftwareGraphBuilder::build_code_graph(&ws_manager, &hybrid_encoder, &device).await
            {
                let target_uri = format!("code_elements:{}", target_handle);

                if let Some(&target_idx) = adj.uri_to_index.get(&target_uri) {
                    let src_vec = adj.edge_src.to_vec1::<u32>().unwrap_or_default();
                    let dst_vec = adj.edge_dst.to_vec1::<u32>().unwrap_or_default();

                    let mut related_uris = Vec::new();
                    for (i, &u) in src_vec.iter().enumerate() {
                        if u as usize == target_idx {
                            let v = dst_vec[i] as usize;
                            if v != target_idx {
                                related_uris.push(adj.index_to_uri[v].clone());
                            }
                        }
                    }

                    for uri in related_uris.into_iter().take(3) {
                        let parts: Vec<&str> = uri.split(':').collect();
                        if parts.len() == 2 && parts[0] == "code_elements" {
                            if let Ok(Some(dep_doc)) =
                                ws_manager.get_document(parts[0], parts[1]).await
                            {
                                let dep_handle = dep_doc["handle"].as_str().unwrap_or("inconnu");
                                let dep_sig = dep_doc["signature"].as_str().unwrap_or("");
                                graph_ctx_str.push_str(&format!(
                                    "- Signature disponible [{}] : {}\n",
                                    dep_handle, dep_sig
                                ));
                            }
                        }
                    }
                }
            }
        }

        // =========================================================================
        // 🧬 BOUCLE ÉVOLUTIONNAIRE NEURO-SYMBOLIQUE (Validation par Compilateur)
        // =========================================================================
        let mut attempts = 0;
        let max_attempts = 3;
        let mut final_body = String::new();
        let mut current_instruction = instruction.clone();

        crate::user_info!(
            "AI_CODER_START_EVOLUTION",
            json_value!({"max_attempts": max_attempts})
        );

        while attempts < max_attempts {
            attempts += 1;

            let user_prompt = format!(
                "=== INVESTIGATION : AST DU COMPOSANT ===\nHandle: {}\nType: {}\nSignature: {}\n\n=== CORPS ACTUEL ===\n{}\n\n=== TÂCHE / MUTATION ===\n{}\n\n⚠️ INSTRUCTION CRITIQUE : Tu es un compilateur. Retourne UNIQUEMENT le nouveau code source complet et modifié dans un bloc markdown (commençant par ```rust et finissant par ```). N'ajoute AUCUN texte, AUCUNE explication, et NE RETOURNE SURTOUT PAS DE JSON.",
                target_handle, element_type, signature, body, current_instruction
            );

            session.add_user_message(&user_prompt);
            let history_str = session.to_context_string();

            let contextualized_prompt = format!(
                "{}\n\n=== CONTEXTE DOCUMENTAIRE ===\n{}\n\n=== CONTEXTE ARCHITECTURAL ===\n{}\n\n=== CORPS ACTUEL ===\n{}\n\n=== INSTRUCTION CRITIQUE ===\n1. LIS LE CORPS ACTUEL.\n2. APPLIQUE STRICTEMENT CETTE MUTATION : {}\n3. RETOURNE LE CODE COMPLET.",
                history_str, rag_ctx, graph_ctx_str, body, current_instruction
            );

            crate::user_info!(
                "AI_CODER_THINKING",
                json_value!({"action": format!("Inférence neuronale (Génération {}/{})", attempts, max_attempts)})
            );

            // Inférence via le LLM local (Moteur de Mutation)
            let response = ctx
                .llm
                .ask(
                    LlmBackend::LocalLlama,
                    &system_prompt,
                    &contextualized_prompt,
                    Clearance::Internal,
                )
                .await?;

            session.add_ai_message(&response);

            // Extraction syntaxique du bloc markdown
            let new_body = match extract_rust_code(&response) {
                Some(code) => code,
                None => {
                    crate::user_warn!(
                        "WARN_AI_CODER_NO_MARKDOWN",
                        json_value!({"attempt": attempts})
                    );
                    current_instruction = "ERREUR : Aucun bloc markdown ```rust trouvé. Tu dois absolument encapsuler ton code. Recommence.".to_string();
                    continue;
                }
            };

            // Évaluation symbolique stricte de la mutation (rustc via la façade)
            let temp_dir = match tempdir() {
                Ok(dir) => dir,
                Err(e) => raise_error!("ERR_FS_TEMP", error = e.to_string()),
            };
            let evaluator = CodeGenEvaluator::new(temp_dir.path().to_path_buf());

            let ast_candidate = AstGenome {
                root: AstNode::Function {
                    signature: signature.clone(),
                    body: new_body.clone(),
                },
            };

            crate::user_info!(
                "AI_CODER_COMPILING",
                json_value!({"action": "Vérification de l'intégrité de l'AST"})
            );
            let (objs, violation) = evaluator.evaluate(&ast_candidate).await;

            if violation == 0.0 {
                // Le code compile sans avertissements ni erreurs, objectif atteint !
                crate::user_success!("SUC_AI_CODER_VALID", json_value!({"conciseness": objs[0]}));
                final_body = new_body;
                break;
            } else {
                // Échec de compilation : injection des erreurs dans la mémoire pour auto-correction
                crate::user_warn!(
                    "WRN_AI_CODER_INVALID_SYNTAX",
                    json_value!({"violation": violation})
                );
                current_instruction = format!(
                    "ERREUR DE COMPILATION SÉMANTIQUE : Ton code a échoué à la validation stricte (Pénalité: {}). Corrige immédiatement les erreurs syntaxiques ou de cycle de vie (borrow checker) détectées et ré-émets le code complet.",
                    violation
                );
            }
        }

        // 9. Validation du Front Évolutionnaire
        if final_body.is_empty() {
            raise_error!(
                "ERR_AI_CODER_EVOLUTION_FAILED",
                error = format!("Le modèle local n'a pas réussi à converger vers un AST valide après {} générations.", max_attempts)
            );
        }

        // 10. Persistance transactionnelle de la session
        if let Err(e) = memory_store.save_session(&sys_manager, &session).await {
            crate::user_warn!("WARN_SESSION_SAVE", json_value!({"err": e.to_string()}));
        }

        // Injection du code certifié conforme
        if let Some(obj) = element_doc.as_object_mut() {
            obj.insert("body".to_string(), json_value!(final_body));
        }

        ws_manager
            .upsert_document("code_elements", element_doc.clone())
            .await?;

        crate::user_success!(
            "AI_CODER_MUTATION_SAVED",
            json_value!({"handle": target_handle})
        );

        // 11. Processus de Staging Physique Explicite
        let module_handle_ref = element_doc
            .get("module_handle")
            .and_then(|v| v.as_str())
            .or_else(|| element_doc.get("module_id").and_then(|v| v.as_str()))
            .unwrap_or("");

        let module_doc = match ws_manager.get_document("modules", module_handle_ref).await {
            Ok(Some(doc)) => doc,
            Ok(None) => raise_error!(
                "ERR_AI_CODER_MODULE_NOT_FOUND",
                error = format!(
                    "Le composant est orphelin (Module parent '{}' introuvable par handle).",
                    module_handle_ref
                )
            ),
            Err(e) => raise_error!("ERR_DB_READ", error = e.to_string()),
        };

        let module_handle = module_doc["handle"].as_str().unwrap_or("");

        crate::user_info!("AI_CODER_STAGING", json_value!({"module": module_handle}));

        let staged_path = codegen_service::stage_module(
            module_handle,
            &self.domain,
            &self.db_name,
            &ctx.db,
            true,
        )
        .await?;

        let artifacts = vec![CreatedArtifact {
            id: target_handle.to_string(),
            name: module_handle.to_string(),
            layer: "LA".to_string(),
            element_type: "Module".to_string(),
            path: staged_path,
        }];

        Ok(Some(AgentResult {
            message: format!(
                "Mutation sémantique validée par compilation croisée sur le composant {}.",
                target_handle
            ),
            artifacts,
            outgoing_message: None,
            xai_frame: None,
        }))
    }
}

// =========================================================================
// TESTS UNITAIRES (Conformité Architecture Data-Driven & Résilience)
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::llm::client::LlmClient;
    use crate::ai::world_model::NeuroSymbolicEngine;
    use crate::code_generator::models::{CodeElement, CodeElementType, Visibility};
    use crate::utils::testing::mock::MockLlmEngine;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    async fn setup_test_environment(
        sandbox: &AgentDbSandbox,
        mock_response: &str,
    ) -> RaiseResult<AgentContext> {
        let config = AppConfig::get();
        let sys_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        DbSandbox::mock_db(&sys_manager).await?;
        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        for col in &["prompts", "agents", "components", "service_configs"] {
            let _ = sys_manager.create_collection(col, &generic_schema).await;
        }

        sys_manager
            .upsert_document(
                "prompts",
                json_value!({
                    "handle": "prompt_agent_software",
                    "role": "system",
                    "identity": { "persona": "Architecte Test", "tone": "strict" },
                    "environment": "Test",
                    "input_variables": ["module_name", "target_handle", "user_request"],
                    "directives": ["Muter {{target_handle}} selon {{user_request}}"]
                }),
            )
            .await?;

        sys_manager
            .upsert_document(
                "agents",
                json_value!({
                    "handle": "agent_software",
                    "base": {
                        "name": {"fr": "Software Agent"},
                        "neuro_profile": { "prompt_id": "prompt_agent_software" }
                    }
                }),
            )
            .await?;

        let mock_engine = SharedRef::new(AsyncMutex::new(MockLlmEngine {
            response: mock_response.to_string(),
        }));

        let llm = LlmClient::new(&sys_manager, sandbox.db.clone(), Some(mock_engine))
            .await
            .unwrap();

        let world_engine =
            SharedRef::new(NeuroSymbolicEngine::bootstrap(&sys_manager).await.unwrap());

        Ok(AgentContext::new(
            "agent_software",
            "sess_test_coder",
            sandbox.db.clone(),
            llm,
            world_engine,
            sandbox.domain_root.clone(),
            sandbox.domain_root.clone(),
        )
        .await?)
    }

    async fn init_workspace_collections(ws_manager: &CollectionsManager<'_>) -> RaiseResult<()> {
        let config = AppConfig::get();
        DbSandbox::mock_db(ws_manager).await?;
        let generic_schema = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        for col in &["modules", "code_elements", "staged_contracts"] {
            let _ = ws_manager.create_collection(col, &generic_schema).await;
        }
        Ok(())
    }

    #[test]
    fn test_extract_rust_code() {
        let text1 = "Voici le code:\n```rust\nfn test() {}\n```";
        assert_eq!(extract_rust_code(text1).unwrap(), "fn test() {}");

        let text2 = "```\nlet a = 1;\n```";
        assert_eq!(extract_rust_code(text2).unwrap(), "let a = 1;");

        let text3 = "Pas de code ici.";
        assert!(extract_rust_code(text3).is_none());
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_agent_data_driven_flow() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 L'IA retourne maintenant du Markdown propre
        let llm_mock_response = "Voici le code généré :\n```rust\n{ println!(\"new\"); }\n```";
        let ctx = setup_test_environment(&sandbox, llm_mock_response).await?;

        let ws_manager =
            CollectionsManager::new(&ctx.db, &config.mount_points.modeling.domain, "master");
        init_workspace_collections(&ws_manager).await?;

        let test_file_path = sandbox.domain_root.join("test.rs");
        std::fs::write(
            &test_file_path,
            "// @raise-handle: fn_execute\nfn execute() { println!(\"old\"); }\n",
        )
        .unwrap();

        ws_manager
            .upsert_document(
                "modules",
                json_value!({
                    "_id": "mod_core_engine",
                    "handle": "mod_core_engine",
                    "element_type": "Module",
                    "path": test_file_path.to_string_lossy().to_string()
                }),
            )
            .await?;

        let code_el = CodeElement {
            module_id: Some("mod_core_engine".to_string()),
            parent_id: None,
            handle: "fn_execute".to_string(),
            element_type: CodeElementType::Function,
            visibility: Visibility::Public,
            attributes: vec![],
            docs: None,
            signature: "fn execute()".to_string(),
            body: Some("{ println!(\"old\"); }".to_string()),
            elements: vec![],
            dependencies: vec![],
            metadata: {
                let mut meta = crate::utils::data::UnorderedMap::new();
                meta.insert(
                    "file_path".to_string(),
                    test_file_path.to_string_lossy().to_string(),
                );
                meta
            },
        };

        let mut doc = crate::utils::data::json::serialize_to_value(&code_el).unwrap();

        if let Some(obj) = doc.as_object_mut() {
            obj.insert("_id".to_string(), json_value!("fn_execute"));
            obj.insert("module_handle".to_string(), json_value!("mod_core_engine"));
        }

        ws_manager.upsert_document("code_elements", doc).await?;

        let agent = SoftwareAgent::new(
            config.mount_points.modeling.domain.clone(),
            "master".to_string(),
        );

        let intent = EngineeringIntent::MutateCode {
            module_name: "mod_core_engine".to_string(),
            target_handle: "fn_execute".to_string(),
            instruction: "Update le print".to_string(),
        };

        let result_opt = agent.process(&ctx, &intent).await?;
        assert!(result_opt.is_some(), "L'agent doit retourner un résultat");
        let result = result_opt.unwrap();

        assert!(result.message.contains("fn_execute"));
        assert_eq!(result.artifacts[0].element_type, "Module");

        let updated_doc = ws_manager
            .get_document("code_elements", "fn_execute")
            .await?
            .unwrap();
        assert_eq!(
            updated_doc["body"].as_str().unwrap(),
            "{ println!(\"new\"); }"
        );

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_agent_element_not_found() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let ctx = setup_test_environment(&sandbox, "{}").await?;

        let ws_manager =
            CollectionsManager::new(&ctx.db, &config.mount_points.modeling.domain, "master");
        init_workspace_collections(&ws_manager).await?;

        let agent = SoftwareAgent::new(
            config.mount_points.modeling.domain.clone(),
            "master".to_string(),
        );
        let intent = EngineeringIntent::MutateCode {
            module_name: "mod_core_engine".to_string(),
            target_handle: "fn_missing_in_action".to_string(),
            instruction: "Refactor".to_string(),
        };

        let res = agent.process(&ctx, &intent).await;
        assert!(
            res.is_err(),
            "L'agent doit échouer si le composant n'existe pas"
        );
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("ERR_AI_CODER_ELEMENT_NOT_FOUND"));

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_agent_module_not_found() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let ctx = setup_test_environment(&sandbox, "```rust\nfn ok(){}\n```").await?;

        let ws_manager =
            CollectionsManager::new(&ctx.db, &config.mount_points.modeling.domain, "master");
        init_workspace_collections(&ws_manager).await?;

        ws_manager
            .upsert_document(
                "code_elements",
                json_value!({
                    "_id": "fn_orphan",
                    "handle": "fn_orphan",
                    "module_handle": "mod_ghost",
                    "element_type": "Function",
                    "visibility": "Public",
                    "attributes": [],
                    "elements": [],
                    "dependencies": [],
                    "metadata": {}
                }),
            )
            .await?;

        let agent = SoftwareAgent::new(
            config.mount_points.modeling.domain.clone(),
            "master".to_string(),
        );
        let intent = EngineeringIntent::MutateCode {
            module_name: "mod_ghost".to_string(),
            target_handle: "fn_orphan".to_string(),
            instruction: "Update".to_string(),
        };

        let res = agent.process(&ctx, &intent).await;
        assert!(
            res.is_err(),
            "L'agent doit bloquer au staging si le module parent est manquant"
        );
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("ERR_AI_CODER_MODULE_NOT_FOUND"));

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_software_agent_invalid_markdown_response() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let ctx =
            setup_test_environment(&sandbox, "Désolé, je ne peux pas générer ce code.").await?;
        let ws_manager =
            CollectionsManager::new(&ctx.db, &config.mount_points.modeling.domain, "master");
        init_workspace_collections(&ws_manager).await?;

        ws_manager
            .upsert_document(
                "code_elements",
                json_value!({
                    "_id": "fn_execute",
                    "handle": "fn_execute",
                    "module_handle": "mod_core_engine",
                    "element_type": "Function",
                    "visibility": "Public",
                    "attributes": [],
                    "elements": [],
                    "dependencies": [],
                    "metadata": {}
                }),
            )
            .await?;

        let agent = SoftwareAgent::new(
            config.mount_points.modeling.domain.clone(),
            "master".to_string(),
        );
        let intent = EngineeringIntent::MutateCode {
            module_name: "mod_core_engine".to_string(),
            target_handle: "fn_execute".to_string(),
            instruction: "Fais n'importe quoi".to_string(),
        };

        let res = agent.process(&ctx, &intent).await;
        assert!(res.is_err());
        assert!(res
            .unwrap_err()
            .to_string()
            .contains("ERR_AI_CODER_EVOLUTION_FAILED"));

        Ok(())
    }
}
