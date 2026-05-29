// FICHIER : src-tauri/src/utils/data/json.rs

// 1. Core : Gestion des erreurs
use crate::raise_error;
use crate::utils::core::error::RaiseResult;

// 2. Data : Traits de sérialisation sémantiques
use crate::utils::data::{DeserializableOwned, Serializable};

// --- RE-EXPORTS AVEC ALIAS SÉMANTIQUES (AI-Ready) ---
pub use serde_json::json as json_value;
pub use serde_json::Map as JsonObject;
pub use serde_json::Value as JsonValue;

// =========================================================================
// 🛡️ MODÉLISATION DE LA SOUVERAINETÉ (CLEARANCE)
// =========================================================================

/// Représentation Rust stricte de la propriété "clearance" définie dans `base.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Clearance {
    #[serde(rename = "C1-Public")]
    Public,
    #[serde(rename = "C2-Interne")]
    Internal,
    #[serde(rename = "C2-CA")]
    InternalCloudAct,
    #[default]
    #[serde(rename = "C3-Privé")]
    Private,
    #[serde(rename = "C3-CA")]
    PrivateCloudAct,
    #[serde(rename = "C4-Secret")]
    Secret,
    #[serde(rename = "C5-Très-Secret")]
    TopSecret,
}

impl Clearance {
    /// Méthode critique de sécurité : Détermine si la donnée a le droit absolu d'être envoyée
    /// vers une API Cloud (LLM distant, Stockage externe).
    /// Retourne `true` uniquement pour les niveaux publics ou expressément compatibles Cloud Act.
    pub fn is_cloud_authorized(&self) -> bool {
        matches!(
            self,
            Clearance::Public | Clearance::InternalCloudAct | Clearance::PrivateCloudAct
        )
    }
}

/// Tente d'extraire le niveau de confidentialité (Clearance) directement depuis un objet JSON brut.
/// Si le champ n'existe pas ou est mal formé, retourne la valeur par défaut (C3-Privé) par sécurité.
pub fn extract_clearance(value: &JsonValue) -> Clearance {
    if let JsonValue::Object(map) = value {
        if let Some(JsonValue::String(c_str)) = map.get("clearance") {
            // Désérialisation manuelle pour éviter un panic sur un JSON mal formé
            return match c_str.as_str() {
                "C1-Public" => Clearance::Public,
                "C2-Interne" => Clearance::Internal,
                "C2-CA" => Clearance::InternalCloudAct,
                "C3-Privé" => Clearance::Private,
                "C3-CA" => Clearance::PrivateCloudAct,
                "C4-Secret" => Clearance::Secret,
                "C5-Très-Secret" => Clearance::TopSecret,
                _ => Clearance::Private, // Fallback ultra-sécuritaire
            };
        }
    }
    Clearance::default()
}

/// Désérialise une chaîne JSON (`&str`) en un type fortement typé `T`.
/// 🤖 IA NOTE: Capture automatiquement un extrait du JSON en cas d'erreur de parsing pour le diagnostic.
pub fn deserialize_from_str<T: DeserializableOwned>(s: &str) -> RaiseResult<T> {
    match serde_json::from_str(s) {
        Ok(val) => Ok(val),
        Err(e) => {
            let snippet = if s.len() > 100 { &s[..100] } else { s };
            raise_error!(
                "ERR_JSON_PARSE",
                error = e,
                context = json_value!({ "snippet": snippet }) // 🎯 Utilisation de la nouvelle macro
            );
        }
    }
}

/// Sérialise un type `T` en une chaîne JSON (`String`) compacte.
/// 🤖 IA NOTE: À utiliser en priorité pour le transfert réseau ou le stockage optimisé.
pub fn serialize_to_string<T: Serializable>(v: &T) -> RaiseResult<String> {
    match serde_json::to_string(v) {
        Ok(s) => Ok(s),
        Err(e) => raise_error!("ERR_JSON_STRINGIFY", error = e),
    }
}

/// Sérialise un type `T` en une chaîne JSON (`String`) formatée avec indentation.
/// 🤖 IA NOTE: À n'utiliser QUE pour les fichiers destinés à être lus par des humains (ex: fichiers de configuration).
pub fn serialize_to_string_pretty<T: Serializable>(v: &T) -> RaiseResult<String> {
    match serde_json::to_string_pretty(v) {
        Ok(s) => Ok(s),
        Err(e) => raise_error!("ERR_JSON_STRINGIFY_PRETTY", error = e),
    }
}

/// Sérialise un type `T` en un tableau d'octets JSON (`Vec<u8>`).
/// ⚠️ Attention: Ce n'est pas du Bincode, c'est bien une représentation UTF-8 du JSON.
pub fn serialize_to_bytes<T: Serializable>(v: &T) -> RaiseResult<Vec<u8>> {
    match serde_json::to_vec(v) {
        Ok(b) => Ok(b),
        Err(e) => raise_error!("ERR_JSON_TO_BYTES", error = e),
    }
}

/// Désérialise un tableau d'octets JSON (`&[u8]`) en un type `T`.
pub fn deserialize_from_bytes<T: DeserializableOwned>(b: &[u8]) -> RaiseResult<T> {
    match serde_json::from_slice(b) {
        Ok(val) => Ok(val),
        Err(e) => raise_error!("ERR_JSON_FROM_BYTES", error = e),
    }
}

/// Convertit un `JsonValue` existant en un type structuré `T`.
pub fn deserialize_from_value<T: DeserializableOwned>(v: JsonValue) -> RaiseResult<T> {
    match serde_json::from_value(v) {
        Ok(val) => Ok(val),
        Err(e) => raise_error!("ERR_JSON_FROM_VALUE", error = e),
    }
}

/// Convertit un type structuré `T` en un `JsonValue` dynamique.
pub fn serialize_to_value<T: Serializable>(v: T) -> RaiseResult<JsonValue> {
    match serde_json::to_value(v) {
        Ok(val) => Ok(val),
        Err(e) => raise_error!("ERR_JSON_TO_VALUE", error = e),
    }
}

/// Fusionne récursivement deux objets JSON (Deep Merge).
/// 🤖 IA NOTE: Les propriétés de l'objet `b` écrasent celles de `a` en cas de conflit. Les sous-objets sont fusionnés.
pub fn deep_merge_values(a: &mut JsonValue, b: JsonValue) {
    match (a, b) {
        (JsonValue::Object(a_map), JsonValue::Object(b_map)) => {
            for (k, v) in b_map {
                deep_merge_values(a_map.entry(k).or_insert(JsonValue::Null), v);
            }
        }
        (a_val, b_val) => *a_val = b_val,
    }
}

/// 🤖 IA NOTE : Désérialise une chaîne YAML en un type `T`.
/// Utilise le moteur `serde_yaml` avec la gestion d'erreur structurée de RAISE.
pub fn deserialize_from_yaml<T: DeserializableOwned>(content: &str) -> RaiseResult<T> {
    match serde_yaml::from_str(content) {
        Ok(val) => Ok(val),
        Err(e) => {
            // On capture l'erreur YAML dans notre système d'observabilité
            crate::raise_error!(
                "ERR_YAML_DESERIALIZATION",
                error = e.to_string(),
                context = crate::utils::data::json::json_value!({
                    "engine": "serde_yaml",
                    "content_preview": content.chars().take(100).collect::<String>()
                })
            )
        }
    }
}
/// Parcourt récursivement un JsonValue et remplace une sous-chaîne par une autre
/// dans toutes les valeurs de type String (idéal pour réécrire les URI absolues).
pub fn replace_uri_in_json(value: &mut JsonValue, search_str: &str, replace_str: &str) {
    match value {
        // Si c'est une chaîne, on fait le remplacement
        JsonValue::String(s)
            if s.contains(search_str) => {
                *s = s.replace(search_str, replace_str);
            }
                |

        // Si c'est un tableau, on plonge dans chaque élément
        JsonValue::Array(arr) => {
            for item in arr.iter_mut() {
                replace_uri_in_json(item, search_str, replace_str);
            }
        }
        // Si c'est un objet, on plonge dans chaque valeur
        JsonValue::Object(obj) => {
            for (_, val) in obj.iter_mut() {
                replace_uri_in_json(val, search_str, replace_str);
            }
        }
        // Pour les nombres, booléens et null, on ne fait rien
        _ => {}
    }
}
// --- TESTS UNITAIRES ---
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::core::error::{AppError, RaiseResult};
    use crate::utils::data::Deserializable;

    #[derive(Serializable, Deserializable, Debug, PartialEq)]
    struct User {
        id: u32,
        role: String,
        #[serde(default)]
        clearance: Clearance,
    }

    #[test]
    fn test_deserialize_success() {
        let raw = r#"{"id": 1, "role": "admin", "clearance": "C4-Secret"}"#;
        let user: User = deserialize_from_str(raw).unwrap();
        assert_eq!(user.id, 1);
        assert_eq!(user.clearance, Clearance::Secret);
    }

    #[test]
    fn test_clearance_default_fallback() {
        // Un utilisateur sans clearance définie doit tomber sur le fallback par défaut (Privé)
        let raw = r#"{"id": 2, "role": "user"}"#;
        let user: User = deserialize_from_str(raw).unwrap();
        assert_eq!(user.clearance, Clearance::Private);
    }

    #[test]
    fn test_extract_clearance_from_json_value() {
        let doc = json_value!({
            "name": "Mission Apollo",
            "clearance": "C1-Public"
        });

        let clearance = extract_clearance(&doc);
        assert_eq!(clearance, Clearance::Public);
        assert!(clearance.is_cloud_authorized());
    }

    #[test]
    fn test_extract_clearance_security_fallback() {
        // En cas d'erreur de frappe, le système doit verrouiller (Private)
        let doc = json_value!({
            "name": "Projet Top Secret",
            "clearance": "C5-Tres-Secret" // Manque l'accent
        });

        let clearance = extract_clearance(&doc);
        assert_eq!(clearance, Clearance::Private); // Verrouillage de sécurité
        assert!(!clearance.is_cloud_authorized());
    }

    #[test]
    fn test_deserialize_error_structured() {
        let bad_raw = r#"{"id": "not_a_number"}"#;
        let res: RaiseResult<User> = deserialize_from_str(bad_raw);

        assert!(res.is_err());
        if let Err(AppError::Structured(data)) = res {
            assert_eq!(data.code, "ERR_JSON_PARSE");
            assert!(data.context.get("snippet").is_some());
        } else {
            panic!("Devrait retourner une erreur structurée");
        }
    }

    #[test]
    fn test_deep_merge() {
        let mut base = json_value!({ "api": { "port": 8080, "host": "localhost" }, "db": "prod" });
        let update = json_value!({ "api": { "port": 9000 }, "db": "staging" });

        deep_merge_values(&mut base, update);

        assert_eq!(base["api"]["port"], 9000);
        assert_eq!(base["api"]["host"], "localhost");
        assert_eq!(base["db"], "staging");
    }
}
