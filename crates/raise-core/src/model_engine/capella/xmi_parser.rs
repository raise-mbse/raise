// FICHIER : src-tauri/src/model_engine/capella/xmi_parser.rs

use crate::model_engine::arcadia::ArcadiaOntology;
use crate::model_engine::types::{ArcadiaElement, NameType, ProjectModel};
use crate::utils::prelude::*;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

pub struct CapellaXmiParser;

impl CapellaXmiParser {
    /// Point d'entrée principal pour le parsing d'un fichier Capella
    pub fn parse_file(path: &Path, model: &mut ProjectModel) -> RaiseResult<()> {
        // 🎯 FIX : Utilisation d'un match explicite pour la compatibilité AppError
        let mut reader = match Reader::from_file(path) {
            Ok(r) => r,
            Err(e) => raise_error!(
                "ERR_XMI_READ_FAIL",
                error = e,
                context = json_value!({
                    "path": path.display().to_string(),
                    "format": "XMI/XML"
                })
            ),
        };
        reader.config_mut().trim_text(true);
        Self::parse_xml(&mut reader, model)
    }

    /// Boucle de parsing XML
    fn parse_xml<B: BufferedRead>(
        reader: &mut Reader<B>,
        model: &mut ProjectModel,
    ) -> RaiseResult<()> {
        let mut buf = Vec::new();

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) | Ok(Event::Empty(e)) => {
                    let mut id = String::new();
                    let mut name = String::new();
                    let mut xsi_type = String::new();
                    let mut properties = UnorderedMap::new();

                    for a in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(a.key.into_inner()).to_string();
                        let value = String::from_utf8_lossy(&a.value).to_string();

                        match key.as_str() {
                            "id" => id = value,
                            "name" => name = value,
                            "xsi:type" => xsi_type = value,
                            _ => {
                                // 🎯 PURE GRAPH : Toutes les autres propriétés XML vont dans la map
                                properties.insert(key, JsonValue::String(value));
                            }
                        }
                    }

                    if !id.is_empty() && !xsi_type.is_empty() {
                        let element = ArcadiaElement {
                            id: id.clone(),
                            name: NameType::String(if name.is_empty() {
                                "Unnamed".to_string()
                            } else {
                                name
                            }),
                            kind: xsi_type.clone(),
                            // 🎯 PURE GRAPH : Plus de champ description statique ici
                            properties,
                        };

                        Self::dispatch(model, element, &xsi_type);
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => {
                    let pos = reader.buffer_position();
                    raise_error!(
                        "ERR_XML_PARSE_FAILURE",
                        error = format!("Erreur XML à la position {} : {}", pos, e)
                    );
                }
                _ => (),
            }
            buf.clear();
        }
        Ok(())
    }

    /// 🎯 PURE GRAPH DISPATCH : Identifie la destination de l'élément et l'insère dynamiquement
    fn dispatch(model: &mut ProjectModel, mut element: ArcadiaElement, xsi_type: &str) {
        // Fonction helper pour résoudre l'URI via l'ontologie dynamique Arcadia
        let resolve = |prefix: &str, name: &str| -> String {
            ArcadiaOntology::get_uri(prefix, name).unwrap_or_else(|| xsi_type.to_string())
        };

        let layer;
        let collection;

        // --- OPERATIONAL ANALYSIS (OA) ---
        if xsi_type.contains("oa:OperationalActor") {
            element.kind = resolve("oa", "OperationalActor");
            layer = "oa";
            collection = "actors";
        } else if xsi_type.contains("oa:OperationalActivity") {
            element.kind = resolve("oa", "OperationalActivity");
            layer = "oa";
            collection = "activities";
        } else if xsi_type.contains("oa:Entity") || xsi_type.contains("oa:OperationalEntity") {
            element.kind = resolve("oa", "OperationalEntity");
            layer = "oa";
            collection = "entities";

        // --- SYSTEM ANALYSIS (SA) ---
        } else if xsi_type.contains("ctx:SystemFunction") {
            element.kind = resolve("sa", "SystemFunction");
            layer = "sa";
            collection = "functions";
        } else if xsi_type.contains("ctx:SystemComponent") || xsi_type.contains("ctx:System") {
            element.kind = resolve("sa", "SystemComponent");
            layer = "sa";
            collection = "components";

        // --- LOGICAL ARCHITECTURE (LA) ---
        } else if xsi_type.contains("la:LogicalFunction") {
            element.kind = resolve("la", "LogicalFunction");
            layer = "la";
            collection = "functions";
        } else if xsi_type.contains("la:LogicalComponent") {
            element.kind = resolve("la", "LogicalComponent");
            layer = "la";
            collection = "components";

        // --- PHYSICAL ARCHITECTURE (PA) ---
        } else if xsi_type.contains("pa:PhysicalComponent") {
            element.kind = resolve("pa", "PhysicalComponent");
            layer = "pa";
            collection = "components";
        } else {
            // Types non mappés ou transverses
            layer = "unmapped";
            collection = "elements";
        }

        // Insertion dans le graphe dynamique
        model.add_element(layer, collection, element);
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    //use crate::model_engine::arcadia::ArcadiaOntology;

    #[test]
    #[serial_test::serial]
    fn test_parse_capella_fragment_pure_graph() {
        use quick_xml::Reader;
        // Supposons que ProjectModel et CapellaXmiParser soient importés

        let xml = r#"
            <root>
                <ownedArchitectures xsi:type="org.polarsys.capella.core.data.la:LogicalArchitecture">
                    <ownedLogicalComponents xsi:type="org.polarsys.capella.core.data.la:LogicalComponent" 
                                           id="LC_1" name="EngineController" description="Main controller" />
                </ownedArchitectures>
            </root>
        "#;

        let mut reader = Reader::from_str(xml);
        reader.config_mut().trim_text(true);

        let mut model = ProjectModel::default();
        CapellaXmiParser::parse_xml(&mut reader, &mut model).expect("Le parsing a échoué");

        // Vérification de l'insertion dans la map dynamique
        let components = model.get_collection("la", "components");
        assert_eq!(
            components.len(),
            1,
            "Le composant doit être dans la collection dynamique"
        );

        let comp = &components[0];
        assert_eq!(comp.name.as_str(), "EngineController");

        let desc_value = comp.properties
            .get("description")
            .expect(&format!(
                "La propriété 'description' n'a pas été extraite par le parseur ! Propriétés extraites pour {} : {:?}",
                comp.name.as_str(),
                comp.properties
            ));

        assert_eq!(
            desc_value
                .as_str()
                .expect("La propriété description n'est pas une chaîne de caractères (String)"),
            "Main controller"
        );

        // Vérification de la résolution du type URI
        //let expected_uri = ArcadiaOntology::get_uri("la", "LogicalComponent").unwrap();
        assert_eq!(comp.kind.as_str(), "https://raise.io/la#LogicalComponent");
    }
}
