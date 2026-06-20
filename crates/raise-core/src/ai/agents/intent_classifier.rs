// FICHIER : src-tauri/src/ai/agents/intent_classifier.rs

use crate::ai::llm::client::{LlmBackend, LlmClient};
use crate::utils::data::json::Clearance;
use crate::utils::prelude::*;

// Import de la Toolbox pour le parsing JSON robuste
use super::tools::extract_json_from_llm;

#[derive(Debug, Serializable, Deserializable, Clone, PartialEq)]
#[serde(tag = "intent")]
pub enum EngineeringIntent {
    #[serde(rename = "define_business_use_case")]
    DefineBusinessUseCase {
        domain: String,
        process_name: String,
        description: String,
    },
    #[serde(rename = "create_element")]
    CreateElement {
        layer: String,
        element_type: String,
        name: String,
    },
    #[serde(rename = "create_relationship")]
    CreateRelationship {
        source_name: String,
        target_name: String,
        relation_type: String,
    },
    #[serde(rename = "generate_code", alias = "create_code")]
    GenerateCode {
        language: String,
        #[serde(alias = "content", alias = "code", default)]
        context: String,
        filename: String,
    },
    #[serde(rename = "verify_quality")]
    VerifyQuality {
        #[serde(default = "default_scope")]
        scope: String, // "code", "model", "requirements"
        target: String, // Cible de la vérification
    },
    #[serde(rename = "chat")]
    Chat,
    #[serde(rename = "unknown")]
    Unknown,

    // 🎯 INJECTION : Nouvelle intention pour l'optimisation ciblée
    #[serde(rename = "mutate_code", alias = "refactor_code")]
    MutateCode {
        module_name: String,
        target_handle: String,
        instruction: String,
    },

    #[serde(rename = "ingest_normative_reference")]
    IngestNormativeReference {
        /// Le chemin ou le nom du fichier (ex: "directive_2026_288.xml")
        path: String,
    },

    // 🎯 INJECTION : Intentions d'infrastructure SRE
    #[serde(rename = "deploy_edge_artifact")]
    DeployEdgeArtifact {
        target_handle: String,
        target_architecture: String,
        payload_uri: String,
    },

    #[serde(rename = "rollback_deployment")]
    RollbackDeployment {
        target_handle: String,
        fallback_commit: String,
    },
}

fn default_scope() -> String {
    "global".to_string()
}

impl EngineeringIntent {
    /// 🎯 Retourne l'URN (Universal Resource Name) de l'agent en base de données
    pub fn recommended_agent_id(&self) -> &'static str {
        match self {
            Self::DefineBusinessUseCase { .. } => "ref:agents:handle:agent_business",
            Self::CreateElement { layer, .. } => match layer.as_str() {
                "OA" => "ref:agents:handle:agent_business",
                "SA" => "ref:agents:handle:agent_system",
                "LA" => "ref:agents:handle:agent_software",
                "PA" => "ref:agents:handle:agent_hardware",
                "EPBS" => "ref:agents:handle:agent_epbs",
                "DATA" => "ref:agents:handle:agent_data",
                "TRANSVERSE" => "ref:agents:handle:agent_quality",
                _ => "ref:agents:handle:agent_dispatcher", // Fallback
            },
            Self::CreateRelationship { .. } => "ref:agents:handle:agent_system",
            Self::VerifyQuality { .. } => "ref:agents:handle:agent_quality",
            Self::Chat | Self::Unknown => "ref:agents:handle:agent_dispatcher",
            Self::GenerateCode { .. } => "ref:agents:handle:agent_software",
            Self::MutateCode { .. } => "ref:agents:handle:agent_software",
            Self::IngestNormativeReference { .. } => "ref:agents:handle:agent_software",
            Self::DeployEdgeArtifact { .. } | Self::RollbackDeployment { .. } => {
                "ref:agents:handle:agent_devops"
            }
        }
    }

    /// Définit le scope de session par défaut pour cette intention.
    pub fn default_session_scope(&self) -> &'static str {
        match self {
            Self::Chat => "global_chat",
            _ => "main_workflow",
        }
    }
}

pub struct IntentClassifier {
    llm: LlmClient,
}

impl IntentClassifier {
    pub fn new(llm: LlmClient) -> Self {
        Self { llm }
    }

    pub async fn classify(&self, user_input: &str) -> EngineeringIntent {
        let lower_input = user_input.to_lowercase();

        // 1. COURT-CIRCUIT (Heuristiques rapides)
        if lower_input.contains("vérifi")
            || lower_input.contains("verify")
            || lower_input.contains("qualité")
        {
            return EngineeringIntent::VerifyQuality {
                scope: if lower_input.contains("code") {
                    "code".into()
                } else {
                    "model".into()
                },
                target: extract_target_heuristics(user_input),
            };
        }

        // 2. 🔄 BOUCLE DE RÉFLEXION POUR LA CLASSIFICATION LLM
        let system_prompt = "Tu es le Dispatcher IA de RAISE. Tu convertis les demandes utilisateur en JSON STRICT.\n\
                             SCHÉMAS :\n\
                             - Création : { \"intent\": \"create_element\", \"layer\": \"SA|LA|PA|DATA|OA|TRANSVERSE\", \"element_type\": \"str\", \"name\": \"str\" }\n\
                             - Code : { \"intent\": \"generate_code\", \"language\": \"str\", \"filename\": \"str\" }\n\
                             - Chat : { \"intent\": \"chat\" }";

        let mut current_feedback = String::new();
        let max_retries = 2;

        for attempt in 1..=max_retries {
            let user_prompt = if attempt == 1 {
                user_input.to_string()
            } else {
                format!(
                    "La tentative précédente a échoué. {}\nDemande initiale : {}",
                    current_feedback, user_input
                )
            };

            // 🎯 FIX : On trace l'erreur matérielle avant le fallback
            let response = match self
                .llm
                .ask(
                    LlmBackend::LocalLlama,
                    system_prompt,
                    &user_prompt,
                    Clearance::Internal,
                )
                .await
            {
                Ok(res) => res,
                Err(e) => {
                    user_warn!(
                        "WARN_INTENT_LLM_FAIL",
                        json_value!({
                            "component": "IntentClassifier",
                            "error": e.to_string(),
                            "action": "Fallback to heuristics"
                        })
                    );
                    break;
                }
            };

            let clean_json = extract_json_from_llm(&response);

            // 🎯 FIX : Boucle de feedback intelligente
            let mut val: JsonValue = match json::deserialize_from_str(&clean_json) {
                Ok(v) => v,
                Err(e) => {
                    current_feedback = format!("ERREUR : Le format JSON est invalide ({}). Veille à ne répondre QUE par un objet JSON strict.", e);
                    continue;
                }
            };

            // --- VALIDATION ET AUTO-CORRECTION DES CHAMPS CRITIQUES ---
            if let Some(intent_name) = val["intent"].as_str() {
                if intent_name == "create_element" {
                    if val.get("layer").is_none() || val["layer"].is_null() {
                        let h = heuristic_fallback(user_input);
                        val["layer"] = h["layer"].clone();
                        val["element_type"] = h["element_type"].clone();
                    }

                    if val["layer"].is_null() {
                        current_feedback =
                            "ERREUR : Le champ 'layer' est obligatoire pour 'create_element'."
                                .to_string();
                        continue;
                    }
                }

                match json::deserialize_from_value::<EngineeringIntent>(val) {
                    Ok(intent) => return intent,
                    Err(e) => {
                        current_feedback = format!(
                            "ERREUR JSON : {}. Assure-toi de fournir tous les champs requis pour l'intent.",
                            e
                        );
                        continue;
                    }
                }
            } else {
                current_feedback = "ERREUR : La clé 'intent' est manquante.".to_string();
            }
        }

        // 3. FALLBACK ULTIME (Si le LLM échoue)
        let fallback_val = heuristic_fallback(user_input);
        json::deserialize_from_value::<EngineeringIntent>(fallback_val)
            .unwrap_or(EngineeringIntent::Unknown)
    }
}

// --- HELPER FUNCTIONS ---

fn extract_target_heuristics(input: &str) -> String {
    let lower = input.to_lowercase();
    for kw in [
        "pour ",
        "sur ",
        "check ",
        "verify ",
        "vérifie ",
        "verification ",
    ] {
        if let Some(idx) = lower.find(kw) {
            // 🎯 FIX UTF-8 : Comptage strict par caractères pour éviter le Panic
            let char_offset = lower[..idx + kw.len()].chars().count();
            return input
                .chars()
                .skip(char_offset)
                .collect::<String>()
                .trim()
                .to_string();
        }
    }
    input.to_string()
}

fn heuristic_fallback(input: &str) -> JsonValue {
    let lower = input.to_lowercase();

    if lower.contains("vérif") || lower.contains("check") {
        return json_value!({
            "intent": "verify_quality",
            "scope": "code",
            "target": input
        });
    }

    if lower.contains("code") || lower.contains("génère") || lower.contains("generate") {
        return json_value!({
            "intent": "generate_code",
            "language": "rust",
            "filename": "generated_component.rs",
            "context": input
        });
    }

    let (layer, etype) = if lower.contains("système") {
        ("SA", "System")
    } else if lower.contains("exigence") {
        ("TRANSVERSE", "Requirement")
    } else if lower.contains("classe") {
        ("DATA", "Class")
    } else if lower.contains("logiciel") {
        ("LA", "Component")
    } else if lower.contains("matériel") {
        ("PA", "PhysicalNode")
    } else if lower.contains("acteur") {
        ("OA", "OperationalActor")
    } else {
        ("SA", "Function")
    };

    if lower.contains("optimise")
        || lower.contains("refactor")
        || lower.contains("corrige le module")
    {
        return json_value!({
            "intent": "mutate_code",
            "module_name": "unknown_module", // L'Agent devra le résoudre
            "target_handle": "unknown_handle",
            "instruction": input
        });
    }

    json_value!({ "intent": "create_element", "layer": layer, "element_type": etype, "name": input })
}

// --- TESTS UNITAIRES ---
#[cfg(test)]
mod tests {
    use super::*;

    fn extract_name(input: &str, keyword: &str) -> String {
        let lower = input.to_lowercase();
        if let Some(idx) = lower.find(keyword) {
            // 🎯 FIX UTF-8 : Dans les tests également
            let char_offset = lower[..idx + keyword.len()].chars().count();
            let raw: String = input.chars().skip(char_offset).collect();
            let clean = raw
                .trim()
                .trim_start_matches("de ")
                .trim_start_matches("du ")
                .trim_start_matches("la ")
                .trim_start_matches("le ")
                .trim_start_matches("l'")
                .trim_start_matches("une ")
                .trim_start_matches("un ")
                .trim();
            return clean.to_string();
        }
        input.to_string()
    }

    #[test]
    fn test_recommended_agent_routing() {
        let intent_sa = EngineeringIntent::CreateElement {
            layer: "SA".to_string(),
            element_type: "System".to_string(),
            name: "Test".to_string(),
        };
        assert_eq!(
            intent_sa.recommended_agent_id(),
            "ref:agents:handle:agent_system"
        );

        let intent_la = EngineeringIntent::CreateElement {
            layer: "LA".to_string(),
            element_type: "Component".to_string(),
            name: "Test".to_string(),
        };
        assert_eq!(
            intent_la.recommended_agent_id(),
            "ref:agents:handle:agent_software"
        );

        let intent_code = EngineeringIntent::GenerateCode {
            language: "rust".into(),
            context: "".into(),
            filename: "".into(),
        };
        assert_eq!(
            intent_code.recommended_agent_id(),
            "ref:agents:handle:agent_software"
        );

        let intent_qa = EngineeringIntent::VerifyQuality {
            scope: "code".into(),
            target: "MyComp".into(),
        };
        assert_eq!(
            intent_qa.recommended_agent_id(),
            "ref:agents:handle:agent_quality"
        );
    }

    #[test]
    fn test_extract_name() {
        assert_eq!(
            extract_name("Crée une exigence de performance", "exigence"),
            "performance"
        );
        assert_eq!(
            extract_name("Crée l'exigence l'autonomie", "exigence"),
            "autonomie"
        );
        assert_eq!(
            extract_name("Nouvelle classe utilisateur", "classe"),
            "utilisateur"
        );
    }

    #[test]
    fn test_heuristic_fallback_code() {
        let val = heuristic_fallback("Génère code python");
        assert_eq!(val["intent"], "generate_code");
        assert_eq!(val["language"], "rust");
    }

    #[test]
    fn test_heuristic_fallback_create() {
        let val = heuristic_fallback("Ajoute un composant logiciel");
        assert_eq!(val["intent"], "create_element");
        assert_eq!(val["layer"], "LA");
        assert_eq!(val["element_type"], "Component");
    }

    #[test]
    fn test_heuristic_fallback_verify() {
        let val = heuristic_fallback("Check le module X");
        assert_eq!(val["intent"], "verify_quality");
        assert_eq!(val["scope"], "code");
    }

    #[test]
    fn test_extract_target() {
        let t = extract_target_heuristics("Vérifie sur le Jetson");
        assert_eq!(t, "le Jetson");
    }
}
