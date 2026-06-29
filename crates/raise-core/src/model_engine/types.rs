// FICHIER : crates/raise-core/src/model_engine/types.rs

use crate::utils::prelude::*;

#[derive(Debug, Serializable, Deserializable, Clone, PartialEq, Eq, Hash)]
pub struct SlugString(String);

impl SlugString {
    pub fn new(s: &str) -> RaiseResult<Self> {
        // Validation directe via regex (pattern du schéma primitive-types.schema.json)
        let re = TextRegex::new(r"^[_a-z0-9]([a-z0-9-_]*[a-z0-9])?$").unwrap();

        if re.is_match(s) {
            Ok(SlugString(s.to_string()))
        } else {
            raise_error!("ERR_INVALID_SLUG", context = json_value!({"value": s}))
        }
    }

    /// Helper de conversion sécurisé depuis un JSONValue pour éviter la répétition
    pub fn from_json(val: Option<&JsonValue>, fallback: &str) -> Self {
        let s = val.and_then(|v| v.as_str()).unwrap_or(fallback);

        Self::new(s).unwrap_or_else(|_| SlugString(fallback.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<&str> for SlugString {
    type Error = AppError;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Ok(SlugString(s.to_string()))
    }
}

// Implémentation de Default pour satisfaire le derive(Default) de ArcadiaElement
impl Default for SlugString {
    fn default() -> Self {
        SlugString("unnamed".to_string())
    }
}

/// Représentation générique d'un nœud MBSE aligné sur Arcadia v2
#[derive(Debug, Serializable, Deserializable, Clone, Default)]
pub struct ArcadiaElement {
    // 🎯 L'Identifiant Métier (base.schema.json)
    pub handle: SlugString,

    // 🎯 Attributs Standards (I18nString est fourni par le prelude)
    pub name: I18nString,

    #[serde(rename = "@type")]
    pub kind: Vec<String>,

    // 🎯 Propriétés MBSE spécifiques (mbse_node_properties du metamodel.schema.json)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub xmi_id: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<I18nString>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<I18nString>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub property_value_ids: Vec<String>,

    // 🎯 PURE GRAPH : Toutes les autres propriétés (relations top-down, constraints) finissent ici
    #[serde(flatten)]
    pub properties: UnorderedMap<String, JsonValue>,
}

/// Métadonnées du projet
#[derive(Debug, Serializable, Deserializable, Clone, Default)]
pub struct ProjectMeta {
    pub name: String,
    pub element_count: usize,
}

/// Le Modèle "Pure Graph"
#[derive(Debug, Serializable, Deserializable, Clone, Default)]
pub struct ProjectModel {
    pub meta: ProjectMeta,
    /// Structure : Layer (ex: "sa") -> Collection (ex: "components") -> Liste d'éléments
    pub layers: UnorderedMap<String, UnorderedMap<String, Vec<ArcadiaElement>>>,
}

impl ProjectModel {
    pub fn add_element(&mut self, layer: &str, collection: &str, el: ArcadiaElement) {
        self.layers
            .entry(layer.to_string())
            .or_default()
            .entry(collection.to_string())
            .or_default()
            .push(el);
    }

    pub fn get_collection(&self, layer: &str, collection: &str) -> &[ArcadiaElement] {
        self.layers
            .get(layer)
            .and_then(|cols| cols.get(collection))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn find_element(&self, handle: &str) -> Option<&ArcadiaElement> {
        self.all_elements()
            .into_iter()
            .find(|el| el.handle.as_str() == handle)
    }

    pub fn all_elements(&self) -> Vec<&ArcadiaElement> {
        self.layers
            .values()
            .flat_map(|collections| collections.values())
            .flat_map(|vec| vec.iter())
            .collect()
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_element(id: &str, name: &str) -> RaiseResult<ArcadiaElement> {
        let mut properties = UnorderedMap::new();
        properties.insert("description".to_string(), json_value!("Test content"));

        // 🎯 L'élément est bien encapsulé dans Ok()
        Ok(ArcadiaElement {
            handle: SlugString::new(id)?,
            name: I18nString::Single(name.to_string()), // 🎯 Utilisation de Single
            kind: vec!["la:LogicalComponent".to_string()],
            properties,
            ..Default::default()
        })
    }

    #[test]
    fn test_dynamic_insertion_and_retrieval() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        // 🎯 Déballage du Result lors de la création
        let el = make_test_element("comp_1", "Radar")?;

        // 🎯 Plus de ? sur add_element
        model.add_element("la", "components", el);

        let collection = model.get_collection("la", "components");
        assert_eq!(collection.len(), 1);
        assert_eq!(collection[0].name.as_str(), "Radar");

        assert_eq!(
            collection[0]
                .properties
                .get("description")
                .unwrap()
                .as_str()
                .unwrap(),
            "Test content"
        );
        Ok(())
    }

    #[test]
    fn test_global_search_find_element() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        model.add_element("sa", "functions", make_test_element("f_1", "Func1")?);
        model.add_element("oa", "actors", make_test_element("a_1", "Actor1")?);

        let found = model.find_element("a_1");
        assert!(found.is_some());
        assert_eq!(found.unwrap().name.as_str(), "Actor1");

        let not_found = model.find_element("missing");
        assert!(not_found.is_none());
        Ok(())
    }

    #[test]
    fn test_all_elements_iterator() -> RaiseResult<()> {
        let mut model = ProjectModel::default();
        model.add_element("layer1", "col1", make_test_element("1", "E1")?);
        model.add_element("layer1", "col2", make_test_element("2", "E2")?);
        model.add_element("layer2", "col1", make_test_element("3", "E3")?);

        let all = model.all_elements();
        assert_eq!(all.len(), 3);
        Ok(())
    }

    #[test]
    fn test_empty_collection_safety() {
        let model = ProjectModel::default();
        let empty = model.get_collection("non_existent", "layer");
        assert!(empty.is_empty());
    }
}
