// FICHIER : crates/raise-core/src/model_engine/capella/diagram_generator.rs

use crate::utils::prelude::*;

use quick_xml::events::Event;
use quick_xml::reader::Reader;

/// Structure simple pour stocker les coordonnées
#[derive(Debug, Clone, PartialEq)]
pub struct DiagramLayout {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub struct AirdParser;

impl AirdParser {
    /// Extrait les positions (layout) des éléments graphiques depuis un fichier .aird
    /// Retourne une Map : Target_UUID -> Layout
    pub fn extract_layout(path: &Path) -> RaiseResult<UnorderedMap<String, DiagramLayout>> {
        let mut reader = match Reader::from_file(path) {
            Ok(r) => r,
            Err(e) => raise_error!(
                "ERR_AIRD_READER_INIT",
                error = e,
                context = json_value!({
                    "action": "initialize_aird_reader",
                    "path": path.to_string_lossy(),
                    "hint": "Vérifiez que le fichier existe et que le format .aird est valide."
                })
            ),
        };
        // CORRECTION API Quick-XML
        reader.config_mut().trim_text(true);

        let mut layout_map = UnorderedMap::new();
        let mut buf = Vec::new();

        // Variable d'état très simplifiée pour l'exemple
        let mut current_element_target: Option<String> = None;

        loop {
            match reader.read_event_into(&mut buf) {
                Ok(Event::Start(e)) => {
                    // CORRECTION: as_encoded_bytes() -> into_inner()
                    let tag_name = String::from_utf8_lossy(e.name().into_inner()).to_string();

                    // Détection simplifiée d'un noeud graphique
                    // 1. Si on trouve une référence à un élément cible
                    for attr in e.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.into_inner()).to_string();
                        let val = String::from_utf8_lossy(&attr.value).to_string();

                        if key == "element" && val.contains("#") {
                            // Extraction de l'UUID après le #
                            if let Some(uuid) = val.split('#').next_back() {
                                current_element_target = Some(uuid.to_string());
                            }
                        }
                    }

                    // 2. Si on trouve un LayoutConstraint et qu'on a une cible
                    if tag_name == "layoutConstraint" {
                        if let Some(target_uuid) = &current_element_target {
                            let mut x = 0;
                            let mut y = 0;
                            let mut w = 100; // defaut
                            let mut h = 100; // defaut

                            for attr in e.attributes().flatten() {
                                let key =
                                    String::from_utf8_lossy(attr.key.into_inner()).to_string();
                                let val = String::from_utf8_lossy(&attr.value).to_string();
                                match key.as_str() {
                                    "x" => x = val.parse().unwrap_or(0),
                                    "y" => y = val.parse().unwrap_or(0),
                                    "width" => w = val.parse().unwrap_or(100),
                                    "height" => h = val.parse().unwrap_or(100),
                                    _ => {}
                                }
                            }

                            layout_map.insert(
                                target_uuid.clone(),
                                DiagramLayout {
                                    x,
                                    y,
                                    width: w,
                                    height: h,
                                },
                            );
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    // CORRECTION: as_encoded_bytes() -> into_inner()
                    let tag_name = String::from_utf8_lossy(e.name().into_inner()).to_string();
                    // Reset context si on sort d'un noeud enfant principal
                    if tag_name == "children" {
                        current_element_target = None;
                    }
                }
                Ok(Event::Eof) => break,
                _ => (),
            }
            buf.clear();
        }

        Ok(layout_map)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_layout_struct() {
        let l = DiagramLayout {
            x: 10,
            y: 20,
            width: 50,
            height: 60,
        };
        assert_eq!(l.x, 10);
    }
}
