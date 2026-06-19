// FICHIER : src-tauri/src/ai/protocols/ontology.rs
use crate::utils::prelude::*;

/// Énumération formelle des ontologies supportées par RAISE.
/// 🎯 ZÉRO DETTE : Aligné sur les URI JSON-LD d'Arcadia et Raise.
#[derive(Serializable, Deserializable, Debug, Clone, PartialEq, Eq)]
pub enum RaiseOntology {
    /// Arcadia Operational Analysis (OA)
    #[serde(rename = "https://raise.ai/ontology/arcadia/oa")]
    ArcadiaOA,

    /// Arcadia Logical Architecture (LA)
    #[serde(rename = "https://raise.ai/ontology/arcadia/la")]
    ArcadiaLA,

    /// Arcadia Physical Architecture (PA)
    #[serde(rename = "https://raise.ai/ontology/arcadia/pa")]
    ArcadiaPA,

    /// Raise Agent Execution Protocol
    #[serde(rename = "https://raise.ai/ontology/raise/agents")]
    RaiseAgents,

    /// Raise Assurance & Traceability
    #[serde(rename = "https://raise.ai/ontology/raise/assurance")]
    RaiseAssurance,
}

impl RaiseOntology {
    /// Retourne l'URI du JSON Schema associé pour validation via CollectionsManager.
    pub fn get_schema_uri(&self) -> String {
        let app_config = AppConfig::get();
        let domain = &app_config.mount_points.system.domain;
        let db = &app_config.mount_points.system.db;

        match self {
            Self::ArcadiaOA => format!(
                "db://{}/{}/schemas/v2/mbse/arcadia/oa.schema.json",
                domain, db
            ),
            Self::ArcadiaLA => format!(
                "db://{}/{}/schemas/v2/mbse/arcadia/la.schema.json",
                domain, db
            ),
            Self::ArcadiaPA => format!(
                "db://{}/{}/schemas/v2/mbse/arcadia/pa.schema.json",
                domain, db
            ),
            Self::RaiseAgents => format!(
                "db://{}/{}/schemas/v2/agents/protocol.schema.json",
                domain, db
            ),
            Self::RaiseAssurance => format!(
                "db://{}/{}/schemas/v2/assurance/trace.schema.json",
                domain, db
            ),
        }
    }
}

impl FmtDisplay for RaiseOntology {
    fn fmt(&self, f: &mut FmtCursor<'_>) -> FmtResult {
        let s = match self {
            Self::ArcadiaOA => "Arcadia OA",
            Self::ArcadiaLA => "Arcadia LA",
            Self::ArcadiaPA => "Arcadia PA",
            Self::RaiseAgents => "Raise Agents",
            Self::RaiseAssurance => "Raise Assurance",
        };
        write!(f, "{}", s)
    }
}

// --- Moteur de Validation Ontologique ---

pub struct OntologyRuleEngine;

impl OntologyRuleEngine {
    /// Vérifie si un lien (hiérarchique ou transversal) viole le méta-modèle.
    /// Actuellement configuré pour le Cycle en V d'Arcadia.
    pub fn is_violation(src: &str, dst: &str) -> bool {
        matches!(
            (src, dst),
            // Règles de violation (Cycle en V inversé)
            ("pa", "oa") | ("la", "pa") | ("sa", "oa") | ("sa", "pa") // System → Physical sans Logical intermédiaire
        )
    }
}

// =========================================================================
// TESTS UNITAIRES (Rigueur Ontologique)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::DbSandbox;

    /// Vérifie que la sérialisation respecte les URIs sémantiques (JSON-LD).
    #[async_test]
    #[serial_test::serial]
    async fn test_ontology_serialization() -> RaiseResult<()> {
        let ont = RaiseOntology::ArcadiaLA;

        // Test Sérialisation
        let serialized = json::serialize_to_string(&ont)?;
        assert_eq!(serialized, "\"https://raise.ai/ontology/arcadia/la\"");

        // Test Désérialisation
        let deserialized: RaiseOntology = json::deserialize_from_str(&serialized)?;
        assert_eq!(deserialized, RaiseOntology::ArcadiaLA);

        Ok(())
    }

    /// Vérifie la génération dynamique des URIs de schémas basée sur AppConfig.
    #[async_test]
    #[serial_test::serial]
    async fn test_schema_uri_generation() -> RaiseResult<()> {
        // Initialisation de la sandbox pour avoir un AppConfig valide
        let _sandbox = DbSandbox::new().await?;

        let ont = RaiseOntology::RaiseAgents;
        let uri = ont.get_schema_uri();

        // On vérifie la structure de l'URI (db://domaine/base/...)
        assert!(uri.starts_with("db://"));
        assert!(uri.contains("schemas/v2/agents/protocol.schema.json"));

        Ok(())
    }

    /// Vérifie l'affichage humain (Display).
    #[test]
    fn test_ontology_display() -> RaiseResult<()> {
        let ont = RaiseOntology::RaiseAssurance;
        let display = format!("{}", ont);
        assert_eq!(display, "Raise Assurance");
        Ok(())
    }
}
