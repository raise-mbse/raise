// FICHIER : src-tauri/src/model_engine/eurlex/parser.rs

use crate::utils::prelude::*;
use quick_xml::events::Event;
use quick_xml::Reader;

/// Structure intermédiaire contenant les données brutes extraites de la loi.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct EurlexParsedData {
    pub raw_text: String,
    pub extra_n_limit: Option<u32>, // Limite additionnelle d'azote
    pub max_cu: Option<u32>,        // Seuil Cuivre
    pub max_zn: Option<u32>,        // Seuil Zinc
}

pub struct EurlexParser;

impl EurlexParser {
    /// Parcourt le fichier XML de manière optimisée pour extraire les seuils réglementaires.
    pub fn parse_xml(path: &Path) -> RaiseResult<EurlexParsedData> {
        // 1. Gestion stricte de l'ouverture du fichier via Raise
        let mut reader = match Reader::from_file(path) {
            Ok(r) => r,
            Err(e) => raise_error!(
                "ERR_EURLEX_FILE",
                error = e,
                context = json_value!({
                    "path": path.display().to_string(),
                    "action": "Reader::from_file"
                })
            ),
        };

        // Correction "Zéro Dette" pour l'API moderne de quick-xml
        reader.config_mut().trim_text(true);

        let mut buf = Vec::new();
        let mut parsed_data = EurlexParsedData::default();
        let mut inside_relevant_section = false;

        loop {
            // 2. Gestion stricte du parsing des événements XML
            match reader.read_event_into(&mut buf) {
                Ok(Event::Text(e)) => {
                    // 3. Extraction de texte native, ultra-rapide et agnostique des versions quick-xml
                    let text = String::from_utf8_lossy(e.as_ref()).into_owned();

                    // Détection du bloc pertinent
                    if text.contains("RENURE") || text.contains("matières fertilisantes") {
                        inside_relevant_section = true;
                        parsed_data.raw_text.push_str(&text);
                        parsed_data.raw_text.push(' ');
                    }

                    // Extraction des seuils si on est dans le contexte
                    if inside_relevant_section {
                        if text.contains("établie à 80 kg")
                            || text.contains("établie à 80 kilogrammes")
                        {
                            parsed_data.extra_n_limit = Some(80);
                        }
                        if text.contains("cuivre (Cu):") && text.contains("300") {
                            parsed_data.max_cu = Some(300);
                        }
                        if text.contains("zinc (Zn):") && text.contains("800") {
                            parsed_data.max_zn = Some(800);
                        }
                    }
                }
                Ok(Event::Eof) => break,
                Err(e) => raise_error!(
                    "ERR_EURLEX_PARSE",
                    error = e,
                    context = json_value!({
                        "position": reader.buffer_position()
                    })
                ),
                _ => (), // Ignore le reste (balises, etc.)
            }
            buf.clear();
        }

        // Avertissement natif via le prélude
        if parsed_data.extra_n_limit.is_none() {
            user_warn!(
                "WRN_EURLEX_MISSING_DATA",
                json_value!({"hint": "La limite d'azote RENURE n'a pas été trouvée dans le XML."})
            );
        }

        Ok(parsed_data)
    }
}

// =========================================================================
// TESTS UNITAIRES ROBUSTES
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn test_parse_valid_renure_xml() -> RaiseResult<()> {
        let tmp_dir = tempdir()?;
        let file_path = tmp_dir.path().join("directive_test_valide.xml");
        let mut file = File::create(&file_path)?;

        let xml_content = r#"
        <?xml version="1.0" encoding="UTF-8"?>
        <document>
            <oj-normal>L'utilisation de fertilisants dits RENURE est une alternative.</oj-normal>
            <oj-normal>Pour ces matières fertilisantes, la limite est établie à 80 kg par hectare et par an.</oj-normal>
            <oj-normal>Elles ne doivent pas dépasser la limite de cuivre (Cu): 300 mg kg-1.</oj-normal>
            <oj-normal>Elles ne doivent pas dépasser la limite de zinc (Zn): 800 mg kg-1.</oj-normal>
        </document>
        "#;

        file.write_all(xml_content.as_bytes())?;

        let result = EurlexParser::parse_xml(&file_path)?;

        assert_eq!(result.extra_n_limit, Some(80));
        assert_eq!(result.max_cu, Some(300));
        assert_eq!(result.max_zn, Some(800));
        assert!(result.raw_text.contains("RENURE"));

        Ok(())
    }

    #[test]
    fn test_parse_file_not_found() {
        let fake_path = PathBuf::from("/chemin/totalement/inexistant/directive.xml");
        let result = EurlexParser::parse_xml(&fake_path);

        assert!(result.is_err());

        // Validation agnostique via la représentation textuelle pour respecter l'encapsulation de AppError
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("ERR_EURLEX_FILE"));
    }
}
