// FICHIER : src-tauri/src/rules_engine/evaluator.rs
use crate::rules_engine::ast::Expr;
use crate::utils::prelude::*;

/// Trait permettant aux règles d'accéder à des données externes (Lookups)
#[async_interface]
pub trait DataProvider: Send + Sync {
    async fn get_value(&self, collection: &str, id: &str, field: &str) -> Option<JsonValue>;
}

pub struct NoOpDataProvider;
#[async_interface]
impl DataProvider for NoOpDataProvider {
    async fn get_value(&self, _c: &str, _id: &str, _f: &str) -> Option<JsonValue> {
        None
    }
}

pub struct Evaluator;

impl Evaluator {
    // ========================================================================
    // 1. LE ROUTEUR HYBRIDE (DUAL-ENGINE)
    // ========================================================================

    /// Point d'entrée principal. Détecte la présence d'I/O et route l'exécution
    /// vers le moteur Synchrone (Haute Performance) ou Asynchrone.
    pub async fn evaluate<'a>(
        expr: &'a Expr,
        context: &'a JsonValue,
        provider: &dyn DataProvider,
    ) -> RaiseResult<CowData<'a, JsonValue>> {
        if !Self::requires_async(expr) {
            // 🚀 VOIE RAPIDE : 100% CPU, 0 allocation de Future sur le tas
            Self::evaluate_sync(expr, context)
        } else {
            // 🐌 VOIE LENTE : Résolution asynchrone requise pour la DB
            Box::pin(Self::evaluate_async(expr, context, provider)).await
        }
    }

    /// Détecte récursivement (et très rapidement) si un accès I/O est caché dans l'AST
    fn requires_async(expr: &Expr) -> bool {
        match expr {
            Expr::Lookup { .. } => true,
            Expr::Val(_) | Expr::Var(_) | Expr::Now | Expr::IsA(_) => false,
            Expr::And(list)
            | Expr::Or(list)
            | Expr::Eq(list)
            | Expr::Neq(list)
            | Expr::Add(list)
            | Expr::Sub(list)
            | Expr::Mul(list)
            | Expr::Div(list)
            | Expr::Concat(list) => list.iter().any(Self::requires_async),
            Expr::Not(e)
            | Expr::Abs(e)
            | Expr::Len(e)
            | Expr::Min(e)
            | Expr::Max(e)
            | Expr::Trim(e)
            | Expr::Lower(e)
            | Expr::Upper(e) => Self::requires_async(e),
            Expr::Gt(a, b)
            | Expr::Lt(a, b)
            | Expr::Gte(a, b)
            | Expr::Lte(a, b)
            | Expr::Contains { list: a, value: b }
            | Expr::RegexMatch {
                value: a,
                pattern: b,
            }
            | Expr::DateDiff { start: a, end: b }
            | Expr::DateAdd { date: a, days: b }
            | Expr::Round {
                value: a,
                precision: b,
            } => Self::requires_async(a) || Self::requires_async(b),
            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                Self::requires_async(condition)
                    || Self::requires_async(then_branch)
                    || Self::requires_async(else_branch)
            }
            Expr::Replace {
                value,
                pattern,
                replacement,
            } => {
                Self::requires_async(value)
                    || Self::requires_async(pattern)
                    || Self::requires_async(replacement)
            }
            Expr::Map {
                list,
                expr: map_expr,
                ..
            } => Self::requires_async(list) || Self::requires_async(map_expr),
            Expr::Filter {
                list, condition, ..
            } => Self::requires_async(list) || Self::requires_async(condition),
        }
    }

    // ========================================================================
    // 2. LE MOTEUR SYNCHRONE (VOIE RAPIDE - 0 GC PRESSURE)
    // ========================================================================

    pub fn evaluate_sync<'a>(
        expr: &'a Expr,
        context: &'a JsonValue,
    ) -> RaiseResult<CowData<'a, JsonValue>> {
        match expr {
            Expr::Val(v) => Ok(CowData::Borrowed(v)),
            Expr::Var(path) => resolve_path(context, path),

            Expr::And(list) => {
                for e in list {
                    let val = Self::evaluate_sync(e, context)?;
                    if !is_truthy(&val) {
                        return Ok(CowData::Owned(JsonValue::Bool(false)));
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(true)))
            }
            Expr::Or(list) => {
                for e in list {
                    let val = Self::evaluate_sync(e, context)?;
                    if is_truthy(&val) {
                        return Ok(CowData::Owned(JsonValue::Bool(true)));
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(false)))
            }
            Expr::Not(e) => {
                let res = Self::evaluate_sync(e, context)?;
                Ok(CowData::Owned(JsonValue::Bool(!is_truthy(&res))))
            }

            Expr::Eq(args) => {
                if args.len() < 2 { return Ok(CowData::Owned(JsonValue::Bool(true))); }
                let first = Self::evaluate_sync(&args[0], context)?;
                for arg in &args[1..] {
                    let next = Self::evaluate_sync(arg, context)?;
                    if first != next { return Ok(CowData::Owned(JsonValue::Bool(false))); }
                }
                Ok(CowData::Owned(JsonValue::Bool(true)))
            }
            Expr::Neq(args) => {
                if args.len() < 2 { return Ok(CowData::Owned(JsonValue::Bool(false))); }
                let a = Self::evaluate_sync(&args[0], context)?;
                let b = Self::evaluate_sync(&args[1], context)?;
                Ok(CowData::Owned(JsonValue::Bool(a != b)))
            }
            Expr::Gt(a, b) => compare_nums_sync(a, b, context, |x, y| x > y),
            Expr::Lt(a, b) => compare_nums_sync(a, b, context, |x, y| x < y),
            Expr::Gte(a, b) => compare_nums_sync(a, b, context, |x, y| x >= y),
            Expr::Lte(a, b) => compare_nums_sync(a, b, context, |x, y| x <= y),

            Expr::Add(list) => fold_nums_sync(list, context, 0.0, |acc, x| acc + x),
            Expr::Mul(list) => fold_nums_sync(list, context, 1.0, |acc, x| acc * x),
            Expr::Sub(list) => {
                if list.is_empty() { return Ok(CowData::Owned(json_value!(0))); }
                let first_val = Self::evaluate_sync(&list[0], context)?;
                let mut acc: f64 = match first_val.as_f64() {
                    Some(num) => num,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "aggregation_init", "expected": "number", "received": first_val, "item_index": 0})),
                };
                for (index, e) in list[1..].iter().enumerate() {
                    let current_val = Self::evaluate_sync(e, context)?;
                    let val: f64 = match current_val.as_f64() {
                        Some(num) => num,
                        None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "subtraction_loop", "expected": "number", "received": current_val, "item_index": index + 1})),
                    };
                    acc -= val;
                }
                Ok(CowData::Owned(smart_number(acc)))
            }
            Expr::Div(list) => {
                if list.len() < 2 { raise_error!("ERR_RULE_INVALID_ARGS", error = "L'opérateur Div requiert au moins 2 arguments"); }
                let first_val = Self::evaluate_sync(&list[0], context)?;
                let mut acc: f64 = match first_val.as_f64() {
                    Some(n) => n,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", error = "Le numérateur initial doit être un nombre"),
                };
                for e in list[1..].iter() {
                    let current_val = Self::evaluate_sync(e, context)?;
                    let val: f64 = match current_val.as_f64() {
                        Some(n) => n,
                        None => raise_error!("ERR_RULE_TYPE_MISMATCH", error = "Les dénominateurs doivent être des nombres"),
                    };
                    if val == 0.0 { raise_error!("ERR_RULE_DIV_BY_ZERO", error = "Division par zéro interdite"); }
                    acc /= val;
                }
                Ok(CowData::Owned(smart_number(acc)))
            }
            Expr::Abs(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let v: f64 = match val.as_f64() {
                    Some(num) => num,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "ABS", "expected": "number", "received": val})),
                };
                Ok(CowData::Owned(smart_number(v.abs())))
            }
            Expr::Round { value, precision } => {
                let val_res = Self::evaluate_sync(value, context)?;
                let v: f64 = match val_res.as_f64() {
                    Some(n) => n,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "ROUND", "field": "value", "expected": "number", "received": val_res})),
                };
                let prec_res = Self::evaluate_sync(precision, context)?;
                let p: i32 = prec_res.as_i64().unwrap_or(0) as i32;
                let factor = 10f64.powi(p);
                Ok(CowData::Owned(smart_number((v * factor).round() / factor)))
            }

            Expr::Min(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let arr: &Vec<JsonValue> = match val.as_array() {
                    Some(array) => array,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "MIN", "expected": "array", "received": val})),
                };
                let min = arr.iter().filter_map(|v| v.as_f64()).fold(f64::INFINITY, |a, b| a.min(b));
                if min.is_infinite() { Ok(CowData::Owned(JsonValue::Null)) } else { Ok(CowData::Owned(smart_number(min))) }
            }
            Expr::Max(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let arr: &Vec<JsonValue> = match val.as_array() {
                    Some(array) => array,
                    None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "MAX", "expected": "array", "received": val})),
                };
                let max = arr.iter().filter_map(|v| v.as_f64()).fold(f64::NEG_INFINITY, |a, b| a.max(b));
                if max.is_infinite() { Ok(CowData::Owned(JsonValue::Null)) } else { Ok(CowData::Owned(smart_number(max))) }
            }

            Expr::Contains { list, value } => {
                let list_val = Self::evaluate_sync(list, context)?;
                let search_val = Self::evaluate_sync(value, context)?;
                let found = match list_val.as_array() {
                    Some(arr) => arr.contains(&*search_val),
                    None => match list_val.as_str() {
                        Some(s) => {
                            let search_str = search_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                            s.contains(search_str)
                        }
                        None => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({ "operation": "CONTAINS" })),
                    },
                };
                Ok(CowData::Owned(JsonValue::Bool(found)))
            }

            Expr::Map { list, alias, expr: map_expr } => {
                let list_val = Self::evaluate_sync(list, context)?;
                let arr: &Vec<JsonValue> = list_val.as_array().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"expected": "array"})))?;
                let mut result_arr = Vec::new();
                for item in arr {
                    let mut local_ctx = context.clone();
                    if let Some(obj) = local_ctx.as_object_mut() {
                        obj.insert(alias.clone(), item.clone());
                    }
                    let res = Self::evaluate_sync(map_expr, &local_ctx)?;
                    result_arr.push(res.into_owned());
                }
                Ok(CowData::Owned(JsonValue::Array(result_arr)))
            }
            Expr::Filter { list, alias, condition } => {
                let list_val = Self::evaluate_sync(list, context)?;
                let arr: &Vec<JsonValue> = list_val.as_array().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"expected": "array"})))?;
                let mut result_arr = Vec::new();
                for item in arr {
                    let mut local_ctx = context.clone();
                    if let Some(obj) = local_ctx.as_object_mut() {
                        obj.insert(alias.clone(), item.clone());
                    }
                    let cond_res = Self::evaluate_sync(condition, &local_ctx)?;
                    if is_truthy(&cond_res) {
                        result_arr.push(item.clone());
                    }
                }
                Ok(CowData::Owned(JsonValue::Array(result_arr)))
            }

            Expr::RegexMatch { value, pattern } => {
                let v_str = Self::evaluate_sync(value, context)?;
                let p_str = Self::evaluate_sync(pattern, context)?;
                let v = v_str.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let p = p_str.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let re = TextRegex::new(p).map_err(|e| build_error!("ERR_RULE_INVALID_REGEX", error = e))?;
                Ok(CowData::Owned(JsonValue::Bool(re.is_match(v))))
            }
            Expr::Concat(list) => {
                let mut res = String::new();
                for e in list {
                    let v = Self::evaluate_sync(e, context)?;
                    res.push_str(v.as_str().unwrap_or(&v.to_string()));
                }
                Ok(CowData::Owned(JsonValue::String(res)))
            }
            Expr::Replace { value, pattern, replacement } => {
                let v_val = Self::evaluate_sync(value, context)?;
                let p_val = Self::evaluate_sync(pattern, context)?;
                let r_val = Self::evaluate_sync(replacement, context)?;
                let v = v_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let p = p_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let r = r_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(v.replace(p, r))))
            }

            Expr::Upper(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let s = val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.to_uppercase())))
            }
            Expr::Lower(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let s = val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.to_lowercase())))
            }
            Expr::Trim(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let s = val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.trim().to_string())))
            }
            Expr::Len(e) => {
                let val = Self::evaluate_sync(e, context)?;
                let len = match val.as_ref() {
                    JsonValue::Array(arr) => arr.len(),
                    JsonValue::String(s) => s.chars().count(),
                    JsonValue::Object(obj) => obj.len(),
                    _ => raise_error!("ERR_RULE_TYPE_MISMATCH", context = json_value!({"operation": "LEN"})),
                };
                Ok(CowData::Owned(json_value!(len)))
            }

            Expr::If { condition, then_branch, else_branch } => {
                let val_cond = Self::evaluate_sync(condition, context)?;
                if is_truthy(&val_cond) {
                    Self::evaluate_sync(then_branch, context)
                } else {
                    Self::evaluate_sync(else_branch, context)
                }
            }

            Expr::IsA(class_name) => {
                let mut is_match = false;
                if let Some(t_val) = context.get("@type") {
                    let check_match = |s: &str| -> bool {
                        s == class_name || s.ends_with(&format!(":{}", class_name)) || s.ends_with(&format!("#{}", class_name))
                    };
                    if let Some(type_str) = t_val.as_str() {
                        is_match = check_match(type_str);
                    } else if let Some(type_arr) = t_val.as_array() {
                        is_match = type_arr.iter().filter_map(|v| v.as_str()).any(check_match);
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(is_match)))
            }

            Expr::Now => Ok(CowData::Owned(json_value!(UtcClock::now().to_rfc3339()))),
            Expr::DateAdd { date, days } => {
                let d_val = Self::evaluate_sync(date, context)?;
                let days_res = Self::evaluate_sync(days, context)?;
                let days_val = days_res.as_i64().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let d_str = d_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;

                if let Ok(local_dt) = d_str.parse::<LocalTimestamp>() {
                    Ok(CowData::Owned(json_value!((local_dt + CalendarDuration::days(days_val)).to_rfc3339())))
                } else if let Ok(nd) = CalendarDate::parse_from_str(d_str, "%Y-%m-%d") {
                    Ok(CowData::Owned(json_value!((nd + CalendarDuration::days(days_val)).format("%Y-%m-%d").to_string())))
                } else {
                    raise_error!("ERR_RULE_INVALID_DATE", error = format!("Format de date invalide : {}", d_str));
                }
            }
            Expr::DateDiff { start, end } => {
                let start_val = Self::evaluate_sync(start, context)?;
                let end_val = Self::evaluate_sync(end, context)?;
                let s_str = start_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let e_str = end_val.as_str().ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;

                if let (Ok(s_dt), Ok(e_dt)) = (s_str.parse::<LocalTimestamp>(), e_str.parse::<LocalTimestamp>()) {
                    Ok(CowData::Owned(json_value!((e_dt - s_dt).num_days())))
                } else if let (Ok(s_nd), Ok(e_nd)) = (CalendarDate::parse_from_str(s_str, "%Y-%m-%d"), CalendarDate::parse_from_str(e_str, "%Y-%m-%d")) {
                    Ok(CowData::Owned(json_value!((e_nd - s_nd).num_days())))
                } else {
                    raise_error!("ERR_RULE_INVALID_DATE");
                }
            }

            Expr::Lookup { .. } => raise_error!(
                "ERR_RULE_UNEXPECTED_ASYNC",
                error = "Le nœud Lookup nécessite une évaluation asynchrone mais a été routé en synchrone."
            ),
        }
    }

    // ========================================================================
    // 3. LE MOTEUR ASYNCHRONE (VOIE LENTE - DÉLÉGATION)
    // ========================================================================

    async fn evaluate_async<'a>(
        expr: &'a Expr,
        context: &'a JsonValue,
        provider: &dyn DataProvider,
    ) -> RaiseResult<CowData<'a, JsonValue>> {
        match expr {
            Expr::Val(v) => Ok(CowData::Borrowed(v)),
            Expr::Var(path) => resolve_path(context, path),

            Expr::And(list) => {
                for e in list {
                    // 🎯 DÉLÉGATION AU ROUTEUR : Permet aux sous-branches de redevenir Sync
                    let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                    if !is_truthy(&val) {
                        return Ok(CowData::Owned(JsonValue::Bool(false)));
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(true)))
            }
            Expr::Or(list) => {
                for e in list {
                    let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                    if is_truthy(&val) {
                        return Ok(CowData::Owned(JsonValue::Bool(true)));
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(false)))
            }
            Expr::Not(e) => {
                let res = Box::pin(Self::evaluate(e, context, provider)).await?;
                Ok(CowData::Owned(JsonValue::Bool(!is_truthy(&res))))
            }

            Expr::Eq(args) => {
                if args.len() < 2 {
                    return Ok(CowData::Owned(JsonValue::Bool(true)));
                }
                let first = Box::pin(Self::evaluate(&args[0], context, provider)).await?;
                for arg in &args[1..] {
                    let next = Box::pin(Self::evaluate(arg, context, provider)).await?;
                    if first != next {
                        return Ok(CowData::Owned(JsonValue::Bool(false)));
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(true)))
            }
            Expr::Neq(args) => {
                if args.len() < 2 {
                    return Ok(CowData::Owned(JsonValue::Bool(false)));
                }
                let a = Box::pin(Self::evaluate(&args[0], context, provider)).await?;
                let b = Box::pin(Self::evaluate(&args[1], context, provider)).await?;
                Ok(CowData::Owned(JsonValue::Bool(a != b)))
            }
            Expr::Gt(a, b) => compare_nums_async(a, b, context, provider, |x, y| x > y).await,
            Expr::Lt(a, b) => compare_nums_async(a, b, context, provider, |x, y| x < y).await,
            Expr::Gte(a, b) => compare_nums_async(a, b, context, provider, |x, y| x >= y).await,
            Expr::Lte(a, b) => compare_nums_async(a, b, context, provider, |x, y| x <= y).await,

            Expr::Add(list) => {
                fold_nums_async(list, context, provider, 0.0, |acc, x| acc + x).await
            }
            Expr::Mul(list) => {
                fold_nums_async(list, context, provider, 1.0, |acc, x| acc * x).await
            }
            Expr::Sub(list) => {
                if list.is_empty() {
                    return Ok(CowData::Owned(json_value!(0)));
                }
                let first_val = Box::pin(Self::evaluate(&list[0], context, provider)).await?;
                let mut acc: f64 = match first_val.as_f64() {
                    Some(num) => num,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"operation": "aggregation_init", "expected": "number (f64)", "received": first_val, "item_index": 0})
                    ),
                };
                for (index, e) in list[1..].iter().enumerate() {
                    let current_val = Box::pin(Self::evaluate(e, context, provider)).await?;
                    let val: f64 = match current_val.as_f64() {
                        Some(num) => num,
                        None => raise_error!(
                            "ERR_RULE_TYPE_MISMATCH",
                            context = json_value!({"operation": "subtraction_loop", "expected": "number", "received": current_val, "item_index": index + 1})
                        ),
                    };
                    acc -= val;
                }
                Ok(CowData::Owned(smart_number(acc)))
            }
            Expr::Div(list) => {
                if list.len() < 2 {
                    raise_error!(
                        "ERR_RULE_INVALID_ARGS",
                        error = "L'opérateur Div requiert au moins 2 arguments"
                    );
                }
                let first_val = Box::pin(Self::evaluate(&list[0], context, provider)).await?;
                let mut acc: f64 = match first_val.as_f64() {
                    Some(n) => n,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        error = "Le numérateur initial doit être un nombre"
                    ),
                };
                for e in list[1..].iter() {
                    let current_val = Box::pin(Self::evaluate(e, context, provider)).await?;
                    let val: f64 = match current_val.as_f64() {
                        Some(n) => n,
                        None => raise_error!(
                            "ERR_RULE_TYPE_MISMATCH",
                            error = "Les dénominateurs doivent être des nombres"
                        ),
                    };
                    if val == 0.0 {
                        raise_error!(
                            "ERR_RULE_DIV_BY_ZERO",
                            error = "Division par zéro interdite"
                        );
                    }
                    acc /= val;
                }
                Ok(CowData::Owned(smart_number(acc)))
            }
            Expr::Abs(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let v: f64 = match val.as_f64() {
                    Some(num) => num,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"operation": "ABS", "expected": "number", "received": val})
                    ),
                };
                Ok(CowData::Owned(smart_number(v.abs())))
            }
            Expr::Round { value, precision } => {
                let val_res = Box::pin(Self::evaluate(value, context, provider)).await?;
                let v: f64 = match val_res.as_f64() {
                    Some(n) => n,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"operation": "ROUND", "field": "value", "expected": "number", "received": val_res})
                    ),
                };
                let prec_res = Box::pin(Self::evaluate(precision, context, provider)).await?;
                let p: i32 = prec_res.as_i64().unwrap_or(0) as i32;
                let factor = 10f64.powi(p);
                Ok(CowData::Owned(smart_number((v * factor).round() / factor)))
            }

            Expr::Min(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let arr: &Vec<JsonValue> = match val.as_array() {
                    Some(array) => array,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context =
                            json_value!({"operation": "MIN", "expected": "array", "received": val})
                    ),
                };
                let min = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::INFINITY, |a, b| a.min(b));
                if min.is_infinite() {
                    Ok(CowData::Owned(JsonValue::Null))
                } else {
                    Ok(CowData::Owned(smart_number(min)))
                }
            }
            Expr::Max(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let arr: &Vec<JsonValue> = match val.as_array() {
                    Some(array) => array,
                    None => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context =
                            json_value!({"operation": "MAX", "expected": "array", "received": val})
                    ),
                };
                let max = arr
                    .iter()
                    .filter_map(|v| v.as_f64())
                    .fold(f64::NEG_INFINITY, |a, b| a.max(b));
                if max.is_infinite() {
                    Ok(CowData::Owned(JsonValue::Null))
                } else {
                    Ok(CowData::Owned(smart_number(max)))
                }
            }

            Expr::Contains { list, value } => {
                let list_val = Box::pin(Self::evaluate(list, context, provider)).await?;
                let search_val = Box::pin(Self::evaluate(value, context, provider)).await?;
                let found = match list_val.as_array() {
                    Some(arr) => arr.contains(&*search_val),
                    None => match list_val.as_str() {
                        Some(s) => {
                            let search_str = search_val
                                .as_str()
                                .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                            s.contains(search_str)
                        }
                        None => raise_error!(
                            "ERR_RULE_TYPE_MISMATCH",
                            context = json_value!({ "operation": "CONTAINS" })
                        ),
                    },
                };
                Ok(CowData::Owned(JsonValue::Bool(found)))
            }

            Expr::Map {
                list,
                alias,
                expr: map_expr,
            } => {
                let list_val = Box::pin(Self::evaluate(list, context, provider)).await?;
                let arr: &Vec<JsonValue> = list_val.as_array().ok_or_else(|| {
                    build_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"expected": "array"})
                    )
                })?;
                let mut result_arr = Vec::new();
                for item in arr {
                    let mut local_ctx = context.clone();
                    if let Some(obj) = local_ctx.as_object_mut() {
                        obj.insert(alias.clone(), item.clone());
                    }
                    let res = Box::pin(Self::evaluate(map_expr, &local_ctx, provider)).await?;
                    result_arr.push(res.into_owned());
                }
                Ok(CowData::Owned(JsonValue::Array(result_arr)))
            }
            Expr::Filter {
                list,
                alias,
                condition,
            } => {
                let list_val = Box::pin(Self::evaluate(list, context, provider)).await?;
                let arr: &Vec<JsonValue> = list_val.as_array().ok_or_else(|| {
                    build_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"expected": "array"})
                    )
                })?;
                let mut result_arr = Vec::new();
                for item in arr {
                    let mut local_ctx = context.clone();
                    if let Some(obj) = local_ctx.as_object_mut() {
                        obj.insert(alias.clone(), item.clone());
                    }
                    let cond_res =
                        Box::pin(Self::evaluate(condition, &local_ctx, provider)).await?;
                    if is_truthy(&cond_res) {
                        result_arr.push(item.clone());
                    }
                }
                Ok(CowData::Owned(JsonValue::Array(result_arr)))
            }

            Expr::RegexMatch { value, pattern } => {
                let v_str = Box::pin(Self::evaluate(value, context, provider)).await?;
                let p_str = Box::pin(Self::evaluate(pattern, context, provider)).await?;
                let v = v_str
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let p = p_str
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let re = TextRegex::new(p)
                    .map_err(|e| build_error!("ERR_RULE_INVALID_REGEX", error = e))?;
                Ok(CowData::Owned(JsonValue::Bool(re.is_match(v))))
            }
            Expr::Concat(list) => {
                let mut res = String::new();
                for e in list {
                    let v = Box::pin(Self::evaluate(e, context, provider)).await?;
                    res.push_str(v.as_str().unwrap_or(&v.to_string()));
                }
                Ok(CowData::Owned(JsonValue::String(res)))
            }
            Expr::Replace {
                value,
                pattern,
                replacement,
            } => {
                let v_val = Box::pin(Self::evaluate(value, context, provider)).await?;
                let p_val = Box::pin(Self::evaluate(pattern, context, provider)).await?;
                let r_val = Box::pin(Self::evaluate(replacement, context, provider)).await?;
                let v = v_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let p = p_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let r = r_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(v.replace(p, r))))
            }

            Expr::Upper(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let s = val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.to_uppercase())))
            }
            Expr::Lower(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let s = val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.to_lowercase())))
            }
            Expr::Trim(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let s = val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                Ok(CowData::Owned(JsonValue::String(s.trim().to_string())))
            }
            Expr::Len(e) => {
                let val = Box::pin(Self::evaluate(e, context, provider)).await?;
                let len = match val.as_ref() {
                    JsonValue::Array(arr) => arr.len(),
                    JsonValue::String(s) => s.chars().count(),
                    JsonValue::Object(obj) => obj.len(),
                    _ => raise_error!(
                        "ERR_RULE_TYPE_MISMATCH",
                        context = json_value!({"operation": "LEN"})
                    ),
                };
                Ok(CowData::Owned(json_value!(len)))
            }

            Expr::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let val_cond = Box::pin(Self::evaluate(condition, context, provider)).await?;
                if is_truthy(&val_cond) {
                    Box::pin(Self::evaluate(then_branch, context, provider)).await
                } else {
                    Box::pin(Self::evaluate(else_branch, context, provider)).await
                }
            }

            Expr::IsA(class_name) => {
                let mut is_match = false;
                if let Some(t_val) = context.get("@type") {
                    let check_match = |s: &str| -> bool {
                        s == class_name
                            || s.ends_with(&format!(":{}", class_name))
                            || s.ends_with(&format!("#{}", class_name))
                    };
                    if let Some(type_str) = t_val.as_str() {
                        is_match = check_match(type_str);
                    } else if let Some(type_arr) = t_val.as_array() {
                        is_match = type_arr.iter().filter_map(|v| v.as_str()).any(check_match);
                    }
                }
                Ok(CowData::Owned(JsonValue::Bool(is_match)))
            }

            Expr::Now => Ok(CowData::Owned(json_value!(UtcClock::now().to_rfc3339()))),
            Expr::DateAdd { date, days } => {
                let d_val = Box::pin(Self::evaluate(date, context, provider)).await?;
                let days_res = Box::pin(Self::evaluate(days, context, provider)).await?;
                let days_val = days_res
                    .as_i64()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let d_str = d_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;

                if let Ok(local_dt) = d_str.parse::<LocalTimestamp>() {
                    Ok(CowData::Owned(json_value!((local_dt
                        + CalendarDuration::days(days_val))
                    .to_rfc3339())))
                } else if let Ok(nd) = CalendarDate::parse_from_str(d_str, "%Y-%m-%d") {
                    Ok(CowData::Owned(json_value!((nd
                        + CalendarDuration::days(days_val))
                    .format("%Y-%m-%d")
                    .to_string())))
                } else {
                    raise_error!(
                        "ERR_RULE_INVALID_DATE",
                        error = format!("Format de date invalide : {}", d_str)
                    );
                }
            }
            Expr::DateDiff { start, end } => {
                let start_val = Box::pin(Self::evaluate(start, context, provider)).await?;
                let end_val = Box::pin(Self::evaluate(end, context, provider)).await?;
                let s_str = start_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
                let e_str = end_val
                    .as_str()
                    .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;

                if let (Ok(s_dt), Ok(e_dt)) = (
                    s_str.parse::<LocalTimestamp>(),
                    e_str.parse::<LocalTimestamp>(),
                ) {
                    Ok(CowData::Owned(json_value!((e_dt - s_dt).num_days())))
                } else if let (Ok(s_nd), Ok(e_nd)) = (
                    CalendarDate::parse_from_str(s_str, "%Y-%m-%d"),
                    CalendarDate::parse_from_str(e_str, "%Y-%m-%d"),
                ) {
                    Ok(CowData::Owned(json_value!((e_nd - s_nd).num_days())))
                } else {
                    raise_error!("ERR_RULE_INVALID_DATE");
                }
            }

            Expr::Lookup {
                collection,
                id,
                field,
            } => {
                let id_v = Box::pin(Self::evaluate(id, context, provider)).await?;
                let id_s = id_v.as_str().unwrap_or("");
                let res = provider
                    .get_value(collection, id_s, field)
                    .await
                    .unwrap_or(JsonValue::Null);
                Ok(CowData::Owned(res))
            }
        }
    }
}

// ============================================================================
// 4. HELPERS D'ÉVALUATION (DUAL-ENGINE)
// ============================================================================

// --- HELPERS SYNCHRONES ---
fn compare_nums_sync<'a, F>(
    a: &Expr,
    b: &Expr,
    c: &'a JsonValue,
    op: F,
) -> RaiseResult<CowData<'a, JsonValue>>
where
    F: Fn(f64, f64) -> bool,
{
    let val_a = Evaluator::evaluate_sync(a, c)?;
    let va: f64 = val_a
        .as_f64()
        .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
    let val_b = Evaluator::evaluate_sync(b, c)?;
    let vb: f64 = val_b
        .as_f64()
        .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
    Ok(CowData::Owned(JsonValue::Bool(op(va, vb))))
}

fn fold_nums_sync<'a, F>(
    list: &[Expr],
    c: &'a JsonValue,
    init: f64,
    op: F,
) -> RaiseResult<CowData<'a, JsonValue>>
where
    F: Fn(f64, f64) -> f64,
{
    let mut acc = init;
    for e in list.iter() {
        let current_val = Evaluator::evaluate_sync(e, c)?;
        let val: f64 = current_val
            .as_f64()
            .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
        acc = op(acc, val);
    }
    Ok(CowData::Owned(smart_number(acc)))
}

// --- HELPERS ASYNCHRONES ---
async fn compare_nums_async<'a, F>(
    a: &Expr,
    b: &Expr,
    c: &'a JsonValue,
    p: &dyn DataProvider,
    op: F,
) -> RaiseResult<CowData<'a, JsonValue>>
where
    F: Fn(f64, f64) -> bool,
{
    let val_a = Box::pin(Evaluator::evaluate(a, c, p)).await?;
    let va: f64 = val_a
        .as_f64()
        .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
    let val_b = Box::pin(Evaluator::evaluate(b, c, p)).await?;
    let vb: f64 = val_b
        .as_f64()
        .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
    Ok(CowData::Owned(JsonValue::Bool(op(va, vb))))
}

async fn fold_nums_async<'a, F>(
    list: &[Expr],
    c: &'a JsonValue,
    p: &dyn DataProvider,
    init: f64,
    op: F,
) -> RaiseResult<CowData<'a, JsonValue>>
where
    F: Fn(f64, f64) -> f64,
{
    let mut acc = init;
    for e in list.iter() {
        let current_val = Box::pin(Evaluator::evaluate(e, c, p)).await?;
        let val: f64 = current_val
            .as_f64()
            .ok_or_else(|| build_error!("ERR_RULE_TYPE_MISMATCH"))?;
        acc = op(acc, val);
    }
    Ok(CowData::Owned(smart_number(acc)))
}

// ============================================================================
// 5. HELPERS PURS (INCHANGÉS)
// ============================================================================

fn smart_number(n: f64) -> JsonValue {
    if n.fract() == 0.0 {
        json_value!(n as i64)
    } else {
        json_value!(n)
    }
}

pub(crate) fn resolve_path<'a>(
    context: &'a JsonValue,
    path: &str,
) -> RaiseResult<CowData<'a, JsonValue>> {
    let mut current = context;
    if path.is_empty() {
        return Ok(CowData::Borrowed(current));
    }
    for part in path.split('.') {
        current = match current {
            JsonValue::Object(map) => match map.get(part) {
                Some(val) => val,
                None => raise_error!(
                    "ERR_RULE_VAR_NOT_FOUND",
                    context = json_value!({ "path": path, "missing_part": part })
                ),
            },
            _ => raise_error!(
                "ERR_RULE_PATH_RESOLUTION_FAIL",
                context = json_value!({ "path": path, "failed_at": part })
            ),
        };
    }
    Ok(CowData::Borrowed(current))
}

fn is_truthy(v: &JsonValue) -> bool {
    match v {
        JsonValue::Bool(b) => *b,
        JsonValue::Null => false,
        JsonValue::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        JsonValue::String(s) => !s.is_empty(),
        _ => true,
    }
}

// =========================================================================
// TESTS UNITAIRES
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    async fn test_eq_async() -> RaiseResult<()> {
        let provider = NoOpDataProvider;
        let ctx = json_value!({});
        let expr = Expr::Eq(vec![Expr::Val(json_value!(10)), Expr::Val(json_value!(10))]);

        let res = match Evaluator::evaluate(&expr, &ctx, &provider).await {
            Ok(val) => val,
            Err(e) => raise_error!(
                "ERR_TEST_EVALUATION_FAILED",
                error = e.to_string(),
                context = json_value!({ "test": "test_eq_async" })
            ),
        };

        assert_eq!(res.as_bool(), Some(true));
        Ok(())
    }

    #[async_test]
    async fn test_lookup_mock() -> RaiseResult<()> {
        struct MockProvider;
        #[async_interface]
        impl DataProvider for MockProvider {
            async fn get_value(&self, _c: &str, _id: &str, _f: &str) -> Option<JsonValue> {
                Some(json_value!("Alice"))
            }
        }

        let expr = Expr::Lookup {
            collection: "users".into(),
            id: Box::new(Expr::Val(json_value!("u1"))),
            field: "name".into(),
        };
        let context_data = json_value!({});

        let res = match Evaluator::evaluate(&expr, &context_data, &MockProvider).await {
            Ok(val) => val,
            Err(e) => raise_error!(
                "ERR_TEST_EVALUATION_FAILED",
                error = e.to_string(),
                context = json_value!({ "test": "test_lookup_mock" })
            ),
        };

        assert_eq!(res.as_str(), Some("Alice"));
        Ok(())
    }

    #[async_test]
    async fn test_datediff_evaluation() -> RaiseResult<()> {
        let provider = NoOpDataProvider;
        let ctx = json_value!({});

        let expr = Expr::DateDiff {
            start: Box::new(Expr::Val(json_value!("2026-04-28"))),
            end: Box::new(Expr::Val(json_value!("2026-04-30"))),
        };

        let res = Evaluator::evaluate(&expr, &ctx, &provider).await?;
        assert_eq!(res.as_i64(), Some(2));
        Ok(())
    }

    #[async_test]
    async fn test_isa_ontological_evaluation() -> RaiseResult<()> {
        let provider = NoOpDataProvider;
        let ctx = json_value!({
            "@context": "db://_system/master/_ontologies/handle/onto-raise-core",
            "@type": ["raise:Database", "pa:PhysicalComponent"],
            "handle": "master"
        });

        let expr_true = Expr::IsA("Database".to_string());
        let res_true = Evaluator::evaluate(&expr_true, &ctx, &provider).await?;
        assert_eq!(res_true.as_bool(), Some(true));

        let expr_false = Expr::IsA("User".to_string());
        let res_false = Evaluator::evaluate(&expr_false, &ctx, &provider).await?;
        assert_eq!(res_false.as_bool(), Some(false));

        Ok(())
    }
}
