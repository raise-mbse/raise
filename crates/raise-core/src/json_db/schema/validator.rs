// FICHIER : crates/raise-core/src/json_db/schema/validator.rs

use super::registry::SchemaRegistry;
use crate::rules_engine::compute::{execute_compute_plan, ComputeContext};
use crate::utils::prelude::*;

#[derive(Debug, Clone)]
pub struct SchemaValidator {
    root_uri: String,
    schema: JsonValue,
    reg: SchemaRegistry,
}

impl SchemaValidator {
    pub fn compile_with_registry(root_uri: &str, reg: &SchemaRegistry) -> RaiseResult<Self> {
        let Some(schema) = reg.get_by_uri(root_uri).cloned() else {
            raise_error!(
                "ERR_SCHEMA_NOT_IN_REGISTRY",
                error = format!("Le schéma sémantique est introuvable : {}", root_uri)
            );
        };

        Ok(Self {
            root_uri: root_uri.to_string(),
            schema,
            reg: reg.clone(),
        })
    }

    pub async fn compute_then_validate(
        &self,
        instance: &mut JsonValue,
        compute_ctx: &ComputeContext,
    ) -> RaiseResult<()> {
        apply_defaults(
            instance,
            &self.schema,
            &self.reg,
            &self.root_uri,
            compute_ctx,
        )
        .await?;

        self.validate(instance)
    }

    pub fn validate(&self, instance: &JsonValue) -> RaiseResult<()> {
        validate_node(instance, &self.schema, &self.reg, &self.root_uri)
    }
}

fn resolve_schema_node<'a>(
    schema: &'a JsonValue,
    reg: &'a SchemaRegistry,
    current_uri: &str,
) -> RaiseResult<&'a JsonValue> {
    if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        let (file_uri, fragment) = if ref_str.starts_with('#') {
            (current_uri.to_string(), Some(ref_str.to_string()))
        } else {
            let resolved = resolve_path_uri(current_uri, ref_str);
            let (f, frag) = split_uri_fragment(&resolved);
            (f.to_string(), frag.map(|s| s.to_string()))
        };

        if let Some(target_root) = reg.get_by_uri(&file_uri) {
            if let Some(frag) = fragment {
                let ptr = frag.replace('#', "");
                return match target_root.pointer(&ptr) {
                    Some(node) => Ok(node),
                    None => raise_error!(
                        "ERR_SCHEMA_POINTER_NOT_FOUND",
                        error = format!("Pointeur JSON '{}' introuvable dans {}", ptr, file_uri)
                    ),
                };
            }
            return Ok(target_root);
        }
    }
    Ok(schema)
}

#[async_recursive]
async fn apply_defaults(
    instance: &mut JsonValue,
    schema: &JsonValue,
    reg: &SchemaRegistry,
    current_uri: &str,
    compute_ctx: &ComputeContext,
) -> RaiseResult<()> {
    if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        let (file_uri, fragment) = if ref_str.starts_with('#') {
            (current_uri.to_string(), Some(ref_str.to_string()))
        } else {
            let resolved = resolve_path_uri(current_uri, ref_str);
            let (f, frag) = split_uri_fragment(&resolved);
            (f.to_string(), frag.map(|s| s.to_string()))
        };

        if let Some(target_root) = reg.get_by_uri(&file_uri) {
            let target_schema = if let Some(frag) = fragment {
                target_root
                    .pointer(&frag.replace('#', ""))
                    .unwrap_or(target_root)
            } else {
                target_root
            };
            return apply_defaults(instance, target_schema, reg, &file_uri, compute_ctx).await;
        }
    }

    if let Some(all_of) = schema.get("allOf").and_then(|v| v.as_array()) {
        for sub_schema in all_of {
            apply_defaults(instance, sub_schema, reg, current_uri, compute_ctx).await?;
        }
    }

    if let Some(obj) = instance.as_object_mut() {
        if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
            for (key, sub_schema) in props {
                let resolved_schema = resolve_schema_node(sub_schema, reg, current_uri)?;
                let is_missing = obj.get(key).is_none_or(|v| v.is_null());

                let compute_node = sub_schema
                    .get("x_compute")
                    .or_else(|| resolved_schema.get("x_compute"))
                    .and_then(|v| v.as_object());

                if let Some(compute) = compute_node {
                    let strategy = compute
                        .get("update")
                        .and_then(|v| v.as_str())
                        .unwrap_or("if_missing");

                    if strategy == "always" || (strategy == "if_missing" && is_missing) {
                        if let Some(plan) = compute.get("plan").and_then(|v| v.as_object()) {
                            if let Some(op) = plan.get("op").and_then(|v| v.as_str()) {
                                // 🎯 Execution avec le nouveau contexte propre
                                let computed_val =
                                    execute_compute_plan(op, plan, compute_ctx).await?;

                                if !computed_val.is_null() {
                                    obj.insert(key.clone(), computed_val);
                                }
                            }
                        }
                    }
                }

                let still_missing = obj.get(key).is_none_or(|v| v.is_null());
                if still_missing {
                    let default_val = sub_schema
                        .get("default")
                        .or_else(|| resolved_schema.get("default"));

                    if let Some(val) = default_val {
                        obj.insert(key.clone(), val.clone());
                    }
                }

                if let Some(val) = obj.get_mut(key) {
                    apply_defaults(val, sub_schema, reg, current_uri, compute_ctx).await?;
                }
            }
        }
    }

    if let Some(arr) = instance.as_array_mut() {
        if let Some(items_schema) = schema.get("items") {
            for item in arr {
                apply_defaults(item, items_schema, reg, current_uri, compute_ctx).await?;
            }
        }
    }

    Ok(())
}

fn validate_node(
    instance: &JsonValue,
    schema: &JsonValue,
    reg: &SchemaRegistry,
    current_uri: &str,
) -> RaiseResult<()> {
    if let Some(ref_str) = schema.get("$ref").and_then(|v| v.as_str()) {
        let (file_uri, fragment) = if ref_str.starts_with('#') {
            (current_uri.to_string(), Some(ref_str.to_string()))
        } else {
            let resolved = resolve_path_uri(current_uri, ref_str);
            let (f, frag) = split_uri_fragment(&resolved);
            (f.to_string(), frag.map(|s| s.to_string()))
        };

        let Some(target_root) = reg.get_by_uri(&file_uri) else {
            raise_error!(
                "ERR_SCHEMA_REF_NOT_FOUND",
                error = format!("Référence de schéma introuvable : {}", file_uri)
            );
        };

        let target_schema = if let Some(frag) = fragment {
            let pointer = frag.replace('#', "");
            let Some(s) = target_root.pointer(&pointer) else {
                raise_error!(
                    "ERR_SCHEMA_POINTER_NOT_FOUND",
                    error = format!("Pointeur JSON '{}' introuvable dans {}", pointer, file_uri)
                );
            };
            s
        } else {
            target_root
        };

        return validate_node(instance, target_schema, reg, &file_uri);
    }

    if let Some(t) = schema.get("type").and_then(|v| v.as_str()) {
        match t {
            "object" => {
                if instance.is_object() {
                    validate_object(instance, schema, reg, current_uri)?;
                } else {
                    raise_type_error("object", instance)?;
                }
            }
            "string" => {
                if instance.is_string() {
                    validate_string(instance, schema)?;
                } else {
                    raise_type_error("string", instance)?;
                }
            }
            "number" => {
                if instance.is_number() {
                    validate_number(instance, schema)?;
                } else {
                    raise_type_error("number", instance)?;
                }
            }
            "integer" => {
                if instance.is_i64() || instance.is_u64() {
                    validate_number(instance, schema)?;
                } else {
                    raise_type_error("integer", instance)?;
                }
            }
            "boolean" if !instance.is_boolean() => {
                raise_type_error("boolean", instance)?;
            }
            "array" => {
                if instance.is_array() {
                    validate_array(instance, schema, reg, current_uri)?;
                } else {
                    raise_type_error("array", instance)?;
                }
            }
            "null" if !instance.is_null() => {
                raise_type_error("null", instance)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn raise_type_error(expected: &str, actual: &JsonValue) -> RaiseResult<()> {
    // On extrait un extrait de la valeur réelle (50 caractères max) pour ne pas inonder les logs
    let actual_str = actual.to_string();
    let snippet = if actual_str.len() > 50 {
        format!("{}...", &actual_str[..50])
    } else {
        actual_str
    };

    raise_error!(
        "ERR_VALIDATION_TYPE_MISMATCH",
        error = format!("Échec de conformité : type '{}' attendu.", expected),
        context = json_value!({
            "expected_type": expected,
            "actual_value": snippet,
            "hint": "Le document contient un type de donnée non conforme au schéma sémantique."
        })
    );
}

fn validate_object(
    instance: &JsonValue,
    schema: &JsonValue,
    reg: &SchemaRegistry,
    current_uri: &str,
) -> RaiseResult<()> {
    let Some(obj) = instance.as_object() else {
        return Ok(());
    };

    if let Some(req) = schema.get("required").and_then(|v| v.as_array()) {
        for r in req {
            if let Some(key) = r.as_str() {
                if !obj.contains_key(key) {
                    raise_error!(
                        "ERR_VALIDATION_REQUIRED_FIELD_MISSING",
                        error = format!("Propriété obligatoire manquante : '{}'", key)
                    );
                }
            }
        }
    }

    if let Some(props) = schema.get("properties").and_then(|v| v.as_object()) {
        for (key, sub_schema) in props {
            if let Some(val) = obj.get(key) {
                if let Err(e) = validate_node(val, sub_schema, reg, current_uri) {
                    raise_error!(
                        "ERR_VALIDATION_NESTED_PROPERTY_FAIL",
                        error = format!(
                            "Échec de validation sur la propriété '{}': {}",
                            key,
                            e.to_string()
                        )
                    );
                }
            }
        }
    }

    let mut compiled_patterns = Vec::new();
    if let Some(patterns) = schema.get("patternProperties").and_then(|v| v.as_object()) {
        for (pattern, sub_schema) in patterns {
            let re = match TextRegex::new(pattern) {
                Ok(r) => r,
                Err(_) => {
                    raise_error!(
                        "ERR_SCHEMA_INVALID_REGEX_PATTERN",
                        error = format!("Regex invalide dans 'patternProperties' : {}", pattern)
                    );
                }
            };

            for (key, val) in obj {
                if re.is_match(key) {
                    if let Err(e) = validate_node(val, sub_schema, reg, current_uri) {
                        raise_error!(
                            "ERR_VALIDATION_PATTERN_PROPERTY_FAIL",
                            error = format!(
                                "Échec de validation pour la clé dynamique '{}': {}",
                                key,
                                e.to_string()
                            )
                        );
                    }
                }
            }
            compiled_patterns.push(re);
        }
    }

    if let Some(ap) = schema.get("additionalProperties") {
        let is_allowed = if ap.is_boolean() {
            ap.as_bool().unwrap_or(true)
        } else {
            true
        };

        if !is_allowed {
            let defined_props: Vec<&String> = schema
                .get("properties")
                .and_then(|v| v.as_object())
                .map(|m| m.keys().collect())
                .unwrap_or_default();

            for k in obj.keys() {
                let is_defined = defined_props.contains(&k);
                let matches_pattern = compiled_patterns.iter().any(|re| re.is_match(k));

                if !is_defined
                    && !matches_pattern
                    && !k.starts_with('_')
                    && k != "$schema"
                    && k != "@context"
                {
                    raise_error!(
                        "ERR_VALIDATION_ADDITIONAL_PROPERTY_FORBIDDEN",
                        error = format!("Propriété non autorisée : '{}'", k)
                    );
                }
            }
        }
    }
    Ok(())
}

fn validate_string(instance: &JsonValue, schema: &JsonValue) -> RaiseResult<()> {
    let Some(s) = instance.as_str() else {
        return Ok(());
    };

    if let Some(min) = schema.get("minLength").and_then(|v| v.as_u64()) {
        if s.chars().count() < min as usize {
            raise_error!(
                "ERR_VALIDATION_STRING_TOO_SHORT",
                error = format!("La chaîne est trop courte (minimum: {} caractères).", min)
            );
        }
    }

    if let Some(max) = schema.get("maxLength").and_then(|v| v.as_u64()) {
        if s.chars().count() > max as usize {
            raise_error!(
                "ERR_VALIDATION_STRING_TOO_LONG",
                error = format!("La chaîne est trop longue (maximum: {} caractères).", max)
            );
        }
    }

    if let Some(pattern) = schema.get("pattern").and_then(|v| v.as_str()) {
        let re = match TextRegex::new(pattern) {
            Ok(r) => r,
            Err(_) => {
                raise_error!(
                    "ERR_SCHEMA_INVALID_REGEX",
                    error = format!("Regex invalide dans le schéma : {}", pattern)
                );
            }
        };
        if !re.is_match(s) {
            raise_error!(
                "ERR_VALIDATION_PATTERN_MISMATCH",
                error = "Le format de la chaîne ne correspond pas au motif exigé."
            );
        }
    }
    Ok(())
}

fn validate_number(instance: &JsonValue, schema: &JsonValue) -> RaiseResult<()> {
    let Some(n) = instance.as_f64() else {
        return Ok(());
    };

    if let Some(min) = schema.get("minimum").and_then(|v| v.as_f64()) {
        if n < min {
            raise_error!(
                "ERR_VALIDATION_NUMBER_TOO_SMALL",
                error = format!("La valeur est inférieure au minimum autorisé ({}).", min)
            );
        }
    }

    if let Some(max) = schema.get("maximum").and_then(|v| v.as_f64()) {
        if n > max {
            raise_error!(
                "ERR_VALIDATION_NUMBER_TOO_LARGE",
                error = format!("La valeur est supérieure au maximum autorisé ({}).", max)
            );
        }
    }
    Ok(())
}

fn validate_array(
    instance: &JsonValue,
    schema: &JsonValue,
    reg: &SchemaRegistry,
    current_uri: &str,
) -> RaiseResult<()> {
    let Some(arr) = instance.as_array() else {
        return Ok(());
    };

    if let Some(min) = schema.get("minItems").and_then(|v| v.as_u64()) {
        if arr.len() < min as usize {
            raise_error!(
                "ERR_VALIDATION_ARRAY_TOO_SMALL",
                error = format!(
                    "Le tableau contient trop peu d'éléments (minimum: {}).",
                    min
                )
            );
        }
    }

    if let Some(max) = schema.get("maxItems").and_then(|v| v.as_u64()) {
        if arr.len() > max as usize {
            raise_error!(
                "ERR_VALIDATION_ARRAY_TOO_LARGE",
                error = format!("Le tableau contient trop d'éléments (maximum: {}).", max)
            );
        }
    }

    if let Some(items_schema) = schema.get("items") {
        if items_schema.is_object() {
            for (index, item) in arr.iter().enumerate() {
                if let Err(e) = validate_node(item, items_schema, reg, current_uri) {
                    raise_error!(
                        "ERR_VALIDATION_ARRAY_ITEM_FAIL",
                        error = format!(
                            "Échec de validation à l'index [{}]: {}",
                            index,
                            e.to_string()
                        )
                    );
                }
            }
        }
    }
    Ok(())
}

fn split_uri_fragment(uri: &str) -> (&str, Option<&str>) {
    if let Some(idx) = uri.find('#') {
        (&uri[0..idx], Some(&uri[idx..]))
    } else {
        (uri, None)
    }
}

fn resolve_path_uri(base: &str, target_path: &str) -> String {
    if target_path.starts_with("db://") {
        return target_path.to_string();
    }
    if target_path.is_empty() {
        return base.to_string();
    }

    let (prefix, base_path_str) = if let Some(stripped) = base.strip_prefix("db://") {
        ("db://", stripped)
    } else {
        ("", base)
    };

    let base_path = Path::new(base_path_str);
    let parent = base_path.parent().unwrap_or(Path::new(""));
    let joined = parent.join(target_path);
    let normalized = normalize_path(&joined);

    format!(
        "{}{}",
        prefix,
        normalized.to_string_lossy().replace('\\', "/")
    )
}

fn normalize_path(path: &Path) -> PathBuf {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            fs::Component::CurDir => {}
            fs::Component::ParentDir => {
                components.pop();
            }
            fs::Component::Normal(c) => components.push(c),
            fs::Component::RootDir | fs::Component::Prefix(_) => {}
        }
    }
    let mut result = PathBuf::new();
    for c in components {
        result.push(c);
    }
    result
}

// ============================================================================
// TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_validator(schema: JsonValue) -> SchemaValidator {
        let mut reg = SchemaRegistry::new();
        reg.register("db://test/schema".to_string(), schema);
        SchemaValidator::compile_with_registry("db://test/schema", &reg).unwrap()
    }

    #[test]
    fn test_string_constraints() -> RaiseResult<()> {
        let v = setup_validator(json_value!({
            "type": "string",
            "minLength": 3,
            "maxLength": 5,
            "pattern": "^[A-Z]+$"
        }));

        assert!(v.validate(&json_value!("TEST")).is_ok());
        assert!(v.validate(&json_value!("TE")).is_err());
        assert!(v.validate(&json_value!("TESTING")).is_err());
        assert!(v.validate(&json_value!("test")).is_err());
        Ok(())
    }

    #[async_test]
    async fn test_apply_defaults_basic() -> RaiseResult<()> {
        let v = setup_validator(json_value!({
            "type": "object",
            "properties": {
                "active": { "type": "boolean", "default": true },
                "version": { "type": "integer", "default": 1 }
            }
        }));

        let mut data = json_value!({});
        let ctx = ComputeContext {
            document: data.clone(),
            ..Default::default()
        };
        v.compute_then_validate(&mut data, &ctx).await?;

        assert_eq!(data["active"], true);
        assert_eq!(data["version"], 1);
        Ok(())
    }

    #[async_test]
    async fn test_dual_schema_x_compute() -> RaiseResult<()> {
        let mut reg = SchemaRegistry::new();

        reg.register("db://test/v1".to_string(), json_value!({
            "type": "object",
            "properties": {
                "Id": { "type": "string", "x_compute": { "update": "if_missing", "plan": { "op": "uuid_v4" } } }
            }
        }));

        reg.register("db://test/v2".to_string(), json_value!({
            "type": "object",
            "properties": {
                "_id": { "type": "string", "x_compute": { "update": "if_missing", "plan": { "op": "uuid_v4" } } }
            }
        }));

        let v1 = SchemaValidator::compile_with_registry("db://test/v1", &reg).unwrap();
        let v2 = SchemaValidator::compile_with_registry("db://test/v2", &reg).unwrap();

        let mut data_v1 = json_value!({});
        let mut data_v2 = json_value!({});

        let ctx_v1 = ComputeContext {
            document: data_v1.clone(),
            ..Default::default()
        };
        let ctx_v2 = ComputeContext {
            document: data_v2.clone(),
            ..Default::default()
        };

        v1.compute_then_validate(&mut data_v1, &ctx_v1).await?;
        v2.compute_then_validate(&mut data_v2, &ctx_v2).await?;

        assert!(
            data_v1.get("Id").is_some(),
            "Le schéma v1 n'a pas injecté 'Id'"
        );
        assert!(
            data_v2.get("_id").is_some(),
            "Le schéma v2 n'a pas injecté '_id'"
        );
        Ok(())
    }

    #[async_test]
    async fn test_x_compute_all_of_inheritance() -> RaiseResult<()> {
        let mut reg = SchemaRegistry::new();

        // 1. Le schéma Base (Contient le calcul de l'@id)
        let base_schema = json_value!({
            "type": "object",
            "properties": {
                "@id": {
                    "type": "string",
                    "x_compute": {
                        "update": "if_missing",
                        "plan": {
                            "op": "concat",
                            "args": [
                                { "op": "get_context", "path": "database_name" },
                                "/",
                                { "op": "get_context", "path": "collection_name" },
                                "/handle/",
                                { "op": "get", "path": "handle" }
                            ]
                        }
                    }
                }
            },
            "required": ["@id"]
        });

        // 2. Le schéma User (Hérite de Base)
        let user_schema = json_value!({
            "type": "object",
            "allOf": [
                { "$ref": "db://test/base.schema.json" }
            ],
            "properties": {
                "handle": { "type": "string" }
            }
        });

        reg.register("db://test/base.schema.json".to_string(), base_schema);
        reg.register("db://test/user.schema.json".to_string(), user_schema);

        let validator = SchemaValidator::compile_with_registry("db://test/user.schema.json", &reg)?;

        // 3. Le document cible
        let mut doc = json_value!({ "handle": "testUser" });

        let ctx = ComputeContext {
            document: doc.clone(),
            collection_name: "users".to_string(),
            db_name: "master".to_string(),
            space_name: "_system".to_string(),
        };

        // 4. L'exécution
        validator.compute_then_validate(&mut doc, &ctx).await?;

        // 5. L'assertion fatale
        assert_eq!(
            doc["@id"], "master/users/handle/testUser",
            "❌ FATAL: L'héritage allOf a échoué, l'@id n'a pas été calculé !"
        );
        Ok(())
    }
}
