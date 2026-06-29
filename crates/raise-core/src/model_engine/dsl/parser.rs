// FICHIER : crates/raise-core/src/model_engine/dsl/parser.rs

use crate::utils::prelude::*;
use pest::Parser;
use pest_derive::Parser;

#[derive(Parser)]
#[grammar = "model_engine/dsl/grammar.pest"]
pub struct DslParser;

/// Analyse une chaîne de caractères générée par l'IA (GBNF)
/// et retourne l'AST PEST brut.
pub fn parse_dsl_text(input: &str) -> RaiseResult<pest::iterators::Pairs<'_, Rule>> {
    match DslParser::parse(Rule::file, input) {
        Ok(pairs) => Ok(pairs),
        Err(e) => {
            raise_error!(
                "ERR_DSL_PARSE_FAILURE",
                error = "Échec de l'analyse syntaxique de l'intention IA.",
                context = json_value!({
                    "parsing_error": format!("{}", e),
                    "location": format!("{:?}", e.location),
                    "action": "parse_dsl_input",
                    "hint": "Le LLM a dévié de la grammaire GBNF ou le parseur Rust est désynchronisé par rapport aux règles de l'échantillonneur."
                })
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dapp_with_constraints() {
        let input = r#"
        dapp "core_engine" {
            type = "daemon"
            
            pvmt {
                frugality_score = 8
            }
            
            service "auth_service" {
                status = "enabled"
                
                resources {
                    max_memory_mb = 1024
                }
                
                module "validator" {
                    visibility = "public"
                }
            }
        }
        "#;

        let parse_result = parse_dsl_text(input);
        assert!(
            parse_result.is_ok(),
            "Le parseur doit accepter un graphe DSL complet avec contraintes."
        );
    }
}
