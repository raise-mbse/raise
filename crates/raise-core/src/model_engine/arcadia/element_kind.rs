// FICHIER : crates/raise-core/src/model_engine/arcadia/element_kind.rs

use crate::model_engine::types::ArcadiaElement;
use crate::utils::prelude::*;

/// Les couches principales de la méthodologie Arcadia + Data + Transverse
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serializable)]
pub enum Layer {
    OperationalAnalysis,
    SystemAnalysis,
    LogicalArchitecture,
    PhysicalArchitecture,
    EPBS,
    Data,
    Transverse,
    Unknown,
}

/// Catégorisation fonctionnelle simplifiée des éléments
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serializable)]
pub enum ElementCategory {
    Component,
    Function,
    Actor,
    Exchange,
    Interface,
    Data,
    Capability,
    Other,
}

impl Layer {
    pub const COUNT: usize = 8;
    pub fn index(&self) -> usize {
        *self as usize
    }
}

impl ElementCategory {
    pub const COUNT: usize = 8;
    pub fn index(&self) -> usize {
        *self as usize
    }
}

/// Trait d'extension pour ajouter de l'intelligence sémantique à ArcadiaElement
pub trait ArcadiaSemantics {
    fn get_layer(&self) -> Layer;
    fn get_category(&self) -> ElementCategory;
    fn is_behavioral(&self) -> bool;
    fn is_structural(&self) -> bool;
}

impl ArcadiaSemantics for ArcadiaElement {
    /// Déduit la couche d'appartenance à partir de l'URI du type
    fn get_layer(&self) -> Layer {
        let k = &self.kind;

        // Déduction agnostique par segment d'URI ou préfixe de type sur le vecteur
        if k.iter().any(|s| s.contains("/oa#") || s.starts_with("oa:")) {
            Layer::OperationalAnalysis
        } else if k.iter().any(|s| s.contains("/sa#") || s.starts_with("sa:")) {
            Layer::SystemAnalysis
        } else if k.iter().any(|s| s.contains("/la#") || s.starts_with("la:")) {
            Layer::LogicalArchitecture
        } else if k.iter().any(|s| s.contains("/pa#") || s.starts_with("pa:")) {
            Layer::PhysicalArchitecture
        } else if k
            .iter()
            .any(|s| s.contains("/epbs#") || s.starts_with("epbs:"))
        {
            Layer::EPBS
        } else if k
            .iter()
            .any(|s| s.contains("/data#") || s.starts_with("data:"))
        {
            Layer::Data
        } else if k.iter().any(|s| {
            s.contains("/transverse")
                || s.contains("/common")
                || s.contains("/libraries")
                || s.starts_with("transverse:")
        }) {
            Layer::Transverse
        } else {
            Layer::Unknown
        }
    }

    /// Déduit la catégorie fonctionnelle à partir du suffixe de l'URI
    fn get_category(&self) -> ElementCategory {
        let k = &self.kind;

        // Déduction agnostique par suffixe d'URI sur le vecteur
        if k.iter().any(|s| {
            s.ends_with("Component") || s.ends_with("System") || s.ends_with("ConfigurationItem")
        }) {
            ElementCategory::Component
        } else if k
            .iter()
            .any(|s| s.ends_with("Function") || s.ends_with("Activity"))
        {
            ElementCategory::Function
        } else if k.iter().any(|s| s.ends_with("Actor")) {
            ElementCategory::Actor
        } else if k
            .iter()
            .any(|s| s.ends_with("Exchange") || s.ends_with("Flow") || s.ends_with("Link"))
        {
            ElementCategory::Exchange
        } else if k
            .iter()
            .any(|s| s.ends_with("Interface") || s.ends_with("Port"))
        {
            ElementCategory::Interface
        } else if k
            .iter()
            .any(|s| s.ends_with("Class") || s.ends_with("DataType") || s.ends_with("ExchangeItem"))
        {
            ElementCategory::Data
        } else if k
            .iter()
            .any(|s| s.ends_with("Capability") || s.ends_with("Scenario"))
        {
            ElementCategory::Capability
        } else {
            ElementCategory::Other
        }
    }

    fn is_behavioral(&self) -> bool {
        matches!(
            self.get_category(),
            ElementCategory::Function | ElementCategory::Exchange | ElementCategory::Capability
        )
    }

    fn is_structural(&self) -> bool {
        matches!(
            self.get_category(),
            ElementCategory::Component | ElementCategory::Interface | ElementCategory::Actor
        )
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model_engine::types::ArcadiaElement;

    /// Helper pour créer un élément de test compatible Pure Graph
    fn make_el(kind: &str) -> RaiseResult<ArcadiaElement> {
        Ok(ArcadiaElement {
            handle: "test_id".try_into()?,
            name: I18nString::default(),
            kind: vec![kind.to_string()],
            properties: UnorderedMap::new(),
            ..Default::default()
        })
    }

    #[test]
    fn test_layer_detection_agnostic() -> RaiseResult<()> {
        let el_oa = make_el("oa:OperationalActor")?;
        assert_eq!(el_oa.get_layer(), Layer::OperationalAnalysis);

        let el_sa = make_el("sa:SystemFunction")?;
        assert_eq!(el_sa.get_layer(), Layer::SystemAnalysis);

        let el_data = make_el("data:Class")?;
        assert_eq!(el_data.get_layer(), Layer::Data);

        let el_trans = make_el("transverse:Requirement")?;
        assert_eq!(el_trans.get_layer(), Layer::Transverse);
        Ok(())
    }

    #[test]
    fn test_category_detection_agnostic() -> RaiseResult<()> {
        let comp = make_el("pa:PhysicalComponent")?;
        assert_eq!(comp.get_category(), ElementCategory::Component);
        assert!(comp.is_structural());

        let func = make_el("sa:SystemFunction")?;
        assert_eq!(func.get_category(), ElementCategory::Function);
        assert!(func.is_behavioral());

        let data = make_el("data:DataType")?;
        assert_eq!(data.get_category(), ElementCategory::Data);
        Ok(())
    }

    #[test]
    fn test_unknown_type_handling() -> RaiseResult<()> {
        let unknown = make_el("http://external.org/UnknownThing")?;
        assert_eq!(unknown.get_layer(), Layer::Unknown);
        assert_eq!(unknown.get_category(), ElementCategory::Other);
        Ok(())
    }
}
