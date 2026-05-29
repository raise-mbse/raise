// FICHIER : src-tauri/src/utils/data/mod.rs

pub mod config;
pub mod encoding;
pub mod json;
// =========================================================================
// FAÇADE `data` : Re-exports Sémantiques (AI-Ready)
// =========================================================================
// 🤖 IA NOTE : La manipulation des données privilégie des verbes explicites
// (serialize/deserialize) et des types désambiguïsés (JsonValue, JsonObject)
// pour garantir un code auto-documenté et lever toute ambiguïté de typage.

pub use json::{
    deep_merge_values, deserialize_from_bytes, deserialize_from_str, deserialize_from_value,
    deserialize_from_yaml, json_value, serialize_to_bytes, serialize_to_string,
    serialize_to_string_pretty, serialize_to_value, JsonObject, JsonValue,
};

pub use config::{AppConfig, CoreConfig, CONFIG};
// Types standards et structures de données fréquemment utilisés dans le domaine métier
// --- Contrats de Sérialisation ---
pub use serde::de::DeserializeOwned as DeserializableOwned;
pub use serde::Deserializer as CustomDeserializerEngine;
pub use serde::{Deserialize as Deserializable, Serialize as Serializable};

/// 🤖 IA NOTE : Trait requis pour générer des erreurs personnalisées lors de la désérialisation.
/// À utiliser dans les implémentations de `CustomDeserializerEngine`.
pub use serde::de::Error as DeserializationErrorTrait; // 🎯 L'alias sémantique

// --- Collections Sémantiques ---
pub use std::collections::{BTreeMap as OrderedMap, HashMap as UnorderedMap, HashSet as UniqueSet};
