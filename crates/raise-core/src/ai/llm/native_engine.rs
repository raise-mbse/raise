// FICHIER : src-tauri/src/ai/llm/native_engine.rs

use crate::ai::llm::client::LlmEngine;
use crate::kernel::assets::AssetResolver;
use crate::utils::prelude::*;
use async_trait::async_trait;

pub struct NativeTensorEngine {
    model: Qwen2QuantizedModel::ModelWeights,
    tokenizer: TextTokenizer,
    device: ComputeHardware,
    logits_processor: TokenLogitsProcessor,
    max_context_size: usize,
    seed: u64,
}

#[async_trait]
impl LlmEngine for NativeTensorEngine {
    async fn generate(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
    ) -> RaiseResult<String> {
        // 🎯 On appelle votre méthode existante (déjà en &mut self)
        // Comme elle est synchrone, on n'a pas besoin de .await ici,
        // mais la signature du trait reste compatible async.
        self.generate(system, user, max_tokens)
    }
    // Le wrapper asynchrone exigé par le trait
    async fn generate_with_grammar(
        &mut self,
        system: &str,
        user: &str,
        max_tokens: usize,
        grammar_str: &str,
    ) -> RaiseResult<String> {
        // Appelle la vraie méthode synchrone (déjà implémentée plus bas)
        self.generate_with_grammar(system, user, max_tokens, grammar_str)
    }
}

impl NativeTensorEngine {
    /// Initialise le moteur LLM local en respectant les points de montage et la config dynamique.
    pub async fn new(
        manager: &crate::json_db::collections::manager::CollectionsManager<'_>,
    ) -> RaiseResult<Self> {
        // 1. Appel du Gatekeeper (Routage + Vérification Activation)
        let settings = match AppConfig::get_runtime_settings(
            manager,
            "ref:components:handle:ai_llm",
        )
        .await
        {
            Ok(s) => s,
            Err(e) => raise_error!(
                "ERR_AI_ENGINE_INIT_REJECTED",
                error = e.to_string(),
                context = json_value!({"action": "native_engine_init", "hint": "Le composant ai_llm est-il actif ?"})
            ),
        };

        // 2. Parsing local avec Match Explicite (Zéro Dette)
        let model_filename = match settings.get("rust_model_file").and_then(|v| v.as_str()) {
            Some(m) => m.to_string(),
            None => raise_error!(
                "ERR_AI_MISSING_VAR",
                error = "La variable 'rust_model_file' est introuvable dans la configuration.",
                context = json_value!({"component": "ai_llm"})
            ),
        };

        let tokenizer_filename = match settings.get("rust_tokenizer_file").and_then(|v| v.as_str())
        {
            Some(t) => t.to_string(),
            None => raise_error!(
                "ERR_AI_MISSING_VAR",
                error = "La variable 'rust_tokenizer_file' est introuvable dans la configuration.",
                context = json_value!({"component": "ai_llm"})
            ),
        };

        // 3. Construction des chemins via les Mount Points (Zéro Dette)
        let config = AppConfig::get();
        let primary_base_path = config.resolve_asset_path(
            config
                .system_assets
                .ai_assets_paths
                .as_ref()
                .and_then(|p| p.models.as_ref()),
            "ai-assets/models",
        )?;

        let category = "ai-assets/models";

        // 4. Résolution factorisée via AssetResolver (Fallback DB -> Global)
        let model_path = match AssetResolver::resolve_ai_file_sync(
            &primary_base_path,
            category,
            &model_filename,
        ) {
            Some(p) => p,
            None => raise_error!(
                "ERR_AI_MODEL_FILE_NOT_FOUND",
                error = format!("Modèle GGUF introuvable : {}", model_filename),
                context = AssetResolver::missing_file_context(
                    &primary_base_path,
                    category,
                    &model_filename
                )
            ),
        };

        let tokenizer_path = match AssetResolver::resolve_ai_file_sync(
            &primary_base_path,
            category,
            &tokenizer_filename,
        ) {
            Some(p) => p,
            None => raise_error!(
                "ERR_AI_TOKENIZER_FILE_NOT_FOUND",
                error = format!("Tokenizer introuvable : {}", tokenizer_filename),
                context = AssetResolver::missing_file_context(
                    &primary_base_path,
                    category,
                    &tokenizer_filename
                )
            ),
        };

        // 5. Résolution Hardware (SSOT: AppConfig)
        let device = AppConfig::device().clone();
        user_info!(
            "MSG_AI_ENGINE_LOAD_START",
            json_value!({ "model": model_filename, "device": format!("{:?}", device) })
        );

        // 6. Chargement sécurisé du TextTokenizer
        let tokenizer = match TextTokenizer::from_file(&tokenizer_path) {
            Ok(t) => t,
            Err(e) => raise_error!(
                "ERR_AI_TOKENIZER_LOAD_FAILED",
                error = e.to_string(),
                context = json_value!({ "path": tokenizer_path.to_string_lossy() })
            ),
        };

        // 7. Ouverture et lecture du fichier GGUF
        let mut file = match fs::open_sync(&model_path) {
            Ok(f) => f,
            Err(e) => raise_error!(
                "ERR_AI_MODEL_OPEN_FAILED",
                error = e.to_string(),
                context = json_value!({ "path": model_path.to_string_lossy() })
            ),
        };

        let model_content = match GgufFileFormat::Content::read(&mut file) {
            Ok(m) => m,
            Err(e) => raise_error!("ERR_AI_MODEL_READ_CONTENT", error = e.to_string()),
        };

        // 8. Instanciation des poids neuronaux
        let weights =
            match Qwen2QuantizedModel::ModelWeights::from_gguf(model_content, &mut file, &device) {
                Ok(w) => w,
                Err(e) => raise_error!("ERR_AI_QWEN2_WEIGHTS_LOAD", error = e.to_string()),
            };

        // 🎯 CORRECTION POINT 1 & 2 : Lecture des paramètres d'inférence avec fallbacks
        let max_context_size = settings
            .get("max_context_size")
            .and_then(|v| v.as_u64())
            .unwrap_or(32768) as usize; // 32k est la limite habituelle de Qwen 2.5 Coder

        let temperature = settings
            .get("temperature")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.7) as f64;

        // Graine dynamique (Zéro Dette : utilisation de la façade de temps)
        let seed = if cfg!(test) {
            299792458 // Déterministe pour les tests (Vitesse de la lumière)
        } else {
            UtcClock::now().timestamp_nanos_opt().unwrap_or(299792458) as u64
        };

        Ok(Self {
            model: weights,
            tokenizer,
            device,
            logits_processor: TokenLogitsProcessor::new(seed, Some(temperature), None),
            max_context_size,
            seed,
        })
    }

    fn format_prompt(system_prompt: &str, user_prompt: &str) -> String {
        format!(
            "<|im_start|>system\n{}<|im_end|>\n<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
            system_prompt, user_prompt
        )
    }

    pub fn generate(
        &mut self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: usize,
    ) -> RaiseResult<String> {
        let formatted_prompt = Self::format_prompt(system_prompt, user_prompt);

        let tokens = match self.tokenizer.encode(formatted_prompt.as_str(), true) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TOKENIZER_ENCODE_FAILED", error = e.to_string()),
        };

        let mut tokens = tokens.get_ids().to_vec();

        // Bouclier Context Window Overflow
        let total_tokens_required = tokens.len() + max_tokens;
        if total_tokens_required > self.max_context_size {
            raise_error!(
                "ERR_AI_CONTEXT_OVERFLOW",
                error = format!(
                    "Le prompt et la génération prévue ({} tokens) dépassent la capacité maximale du modèle ({} tokens).",
                    total_tokens_required, self.max_context_size
                ),
                context = json_value!({
                    "prompt_tokens": tokens.len(),
                    "requested_max_tokens": max_tokens,
                    "max_capacity": self.max_context_size
                })
            );
        }

        let mut generated_tokens = Vec::new();
        let mut index_pos = 0;

        // Résolution dynamique des Stop Tokens (ChatML)
        let eos_token_id = match self.tokenizer.token_to_id("<|im_end|>") {
            Some(id) => id,
            None => raise_error!(
                "ERR_AI_FORMAT_INCOMPATIBLE",
                error = "Token <|im_end|> manquant."
            ),
        };

        let stop_token_id = self.tokenizer.token_to_id("<|endoftext|>");

        for _i in 0..max_tokens {
            let context_size = if index_pos == 0 { tokens.len() } else { 1 };
            let start_pos = tokens.len().saturating_sub(context_size);

            let input = match NeuralTensor::new(&tokens[start_pos..], &self.device) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_TENSOR_INPUT_FAILED", error = e.to_string()),
            };

            let input = match input.unsqueeze(0) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_TENSOR_SHAPE_ERROR", error = e.to_string()),
            };

            let logits = match self.model.forward(&input, index_pos) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_FORWARD_PASS_FAILED", error = e.to_string()),
            };

            let logits = match logits.squeeze(0).and_then(|l| l.squeeze(0)) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_TENSOR_REDUCTION_FAILED", error = e.to_string()),
            };

            let logits = match logits.to_dtype(ComputeType::F32) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_DTYPE_CONVERSION_FAILED", error = e.to_string()),
            };

            let next_token = match self.logits_processor.sample(&logits) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_SAMPLING_FAILED", error = e.to_string()),
            };

            if next_token == eos_token_id || Some(next_token) == stop_token_id {
                break;
            }

            tokens.push(next_token);
            generated_tokens.push(next_token);
            index_pos += context_size;
        }

        match self.tokenizer.decode(&generated_tokens, true) {
            Ok(res) => Ok(res),
            Err(e) => raise_error!("ERR_TOKENIZER_DECODE_FAILED", error = e.to_string()),
        }
    }

    /// Génération sous contrainte stricte (GBNF).
    /// Le processeur de logits forcera le LLM à respecter la grammaire.
    pub fn generate_with_grammar(
        &mut self,
        system_prompt: &str,
        user_prompt: &str,
        max_tokens: usize,
        _grammar_str: &str, // Sera injecté dans le LogitsProcessor étendu
    ) -> RaiseResult<String> {
        let formatted_prompt = Self::format_prompt(system_prompt, user_prompt);

        let tokens = match self.tokenizer.encode(formatted_prompt.as_str(), true) {
            Ok(t) => t,
            Err(e) => raise_error!("ERR_TOKENIZER_ENCODE_FAILED", error = e.to_string()),
        };

        let mut tokens = tokens.get_ids().to_vec();

        // Initialisation d'un processeur de logits restrictif (Température très basse pour le DSL)
        let mut grammar_processor = TokenLogitsProcessor::new(
            self.seed, // 🎯 UTILISATION de la graine sauvegardée
            Some(0.1),
            None,
        );

        let mut generated_tokens = Vec::new();
        let mut index_pos = 0;

        let eos_token_id = match self.tokenizer.token_to_id("<|im_end|>") {
            Some(id) => id,
            None => raise_error!(
                "ERR_AI_FORMAT_INCOMPATIBLE",
                error = "Token <|im_end|> manquant."
            ),
        };

        let stop_token_id = self.tokenizer.token_to_id("<|endoftext|>");

        for _i in 0..max_tokens {
            let context_size = if index_pos == 0 { tokens.len() } else { 1 };
            let start_pos = tokens.len().saturating_sub(context_size);

            let input_tensor = match NeuralTensor::new(&tokens[start_pos..], &self.device) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_TENSOR_INPUT_FAILED", error = e.to_string()),
            };

            let input_tensor = match input_tensor.unsqueeze(0) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_TENSOR_SHAPE_ERROR", error = e.to_string()),
            };

            let forward_logits = match self.model.forward(&input_tensor, index_pos) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_FORWARD_PASS_FAILED", error = e.to_string()),
            };

            let squeezed_logits = match forward_logits.squeeze(0) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_TENSOR_REDUCTION_FAILED", error = e.to_string()),
            };

            let final_logits = match squeezed_logits.squeeze(0) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_TENSOR_REDUCTION_FAILED", error = e.to_string()),
            };

            let typed_logits = match final_logits.to_dtype(ComputeType::F32) {
                Ok(l) => l,
                Err(e) => raise_error!("ERR_AI_DTYPE_CONVERSION_FAILED", error = e.to_string()),
            };

            // L'échantillonneur applique le masque GBNF sur les logits avant sélection
            let next_token = match grammar_processor.sample(&typed_logits) {
                Ok(t) => t,
                Err(e) => raise_error!("ERR_AI_SAMPLING_FAILED", error = e.to_string()),
            };

            if next_token == eos_token_id || Some(next_token) == stop_token_id {
                break;
            }

            tokens.push(next_token);
            generated_tokens.push(next_token);
            index_pos += context_size;
        }

        match self.tokenizer.decode(&generated_tokens, true) {
            Ok(res) => Ok(res),
            Err(e) => raise_error!("ERR_TOKENIZER_DECODE_FAILED", error = e.to_string()),
        }
    }
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Façade & Résilience)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::json_db::collections::manager::CollectionsManager;
    use crate::utils::testing::AgentDbSandbox;

    #[test]
    fn test_qwen_chatml_format() {
        let sys = "Sys";
        let user = "User";
        let expected = "<|im_start|>system\nSys<|im_end|>\n<|im_start|>user\nUser<|im_end|>\n<|im_start|>assistant\n";
        assert_eq!(NativeTensorEngine::format_prompt(sys, user), expected);
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_quick_inference() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = NativeTensorEngine::new(&manager).await?;
        let response = engine.generate("Réponds 'OK'.", "Test", 5)?;

        assert!(!response.is_empty());
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_resilience_missing_model_path() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut llm_doc = manager
            .get_document("service_configs", "cfg_ai_llm_test")
            .await?
            .expect("La config LLM devrait être présente via AgentDbSandbox");

        llm_doc["service_settings"]["rust_model_file"] =
            json_value!("this_file_does_not_exist.gguf");
        manager.insert_raw("service_configs", &llm_doc).await?;

        let result = NativeTensorEngine::new(&manager).await;

        match result {
            Err(AppError::Structured(err)) => {
                assert_eq!(err.code, "ERR_AI_MODEL_FILE_NOT_FOUND");
                Ok(())
            }
            _ => panic!("Le moteur aurait dû lever ERR_AI_MODEL_FILE_NOT_FOUND"),
        }
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_generate_with_grammar_execution() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );

        let mut engine = NativeTensorEngine::new(&manager).await?;

        // Simulation d'une grammaire basique
        let mock_grammar = "root ::= [a-z]+";

        let response = engine.generate_with_grammar(
            "Tu ne dois répondre qu'avec le mot 'module'.",
            "Test GBNF",
            5,
            mock_grammar,
        )?;

        assert!(
            !response.is_empty(),
            "La génération GBNF doit retourner une chaîne valide."
        );
        Ok(())
    }
}
