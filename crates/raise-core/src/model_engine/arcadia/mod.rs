// FICHIER : crates/raise-core/src/model_engine/arcadia/mod.rs

use crate::json_db::jsonld::VocabularyRegistry;

/// Ce module contient le référentiel sémantique d'Arcadia.
/// Il fait le pont entre le moteur et les ontologies chargées dynamiquement.
pub mod element_kind;

// --- 1. CLÉS DE PROPRIÉTÉS JSON (Vocabulaire de Structure) ---
// Ces clés correspondent à la structure de tes objets JSON dans la base.
pub const PROP_NAME: &str = "name";
pub const PROP_ID: &str = "id";
pub const PROP_DESCRIPTION: &str = "description";
pub const PROP_ALLOCATED_FUNCTIONS: &str = "allocatedFunctions";
pub const PROP_OWNED_LOGICAL_COMPONENTS: &str = "ownedLogicalComponents";
pub const PROP_OWNED_SYSTEM_COMPONENTS: &str = "ownedSystemComponents";
pub const PROP_INCOMING_EXCHANGES: &str = "incomingFunctionalExchanges";
pub const PROP_OUTGOING_EXCHANGES: &str = "outgoingFunctionalExchanges";

// --- 2. ACCÈS DYNAMIQUE AU REGISTRE (MBSE Agnostique) ---

pub struct ArcadiaOntology;

impl ArcadiaOntology {
    /// Récupère l'URI complète d'un type via le registre dynamique.
    pub fn get_uri(layer_prefix: &str, type_name: &str) -> Option<String> {
        let reg = VocabularyRegistry::global().ok()?;
        let default_ctx = reg.get_default_context();

        // On résout le préfixe (ex: "oa") via le contexte chargé depuis les fichiers .jsonld
        default_ctx
            .get(layer_prefix)
            .map(|ns| format!("{}{}", ns, type_name))
    }

    /// Vérifie si une URI est reconnue dans les ontologies chargées
    pub fn is_known_type(uri: &str) -> bool {
        VocabularyRegistry::global()
            .map(|reg| reg.has_class(uri))
            .unwrap_or(false)
    }
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_property_keys_integrity() {
        // On vérifie que les clés de structure JSON sont stables
        assert_eq!(PROP_NAME, "name");
        assert_eq!(PROP_ID, "id");
    }

    // 🎯 Note: Les tests de comparaison avec 'namespaces' ont été supprimés
    // car le code est maintenant piloté par la donnée (Data-Driven).
}
