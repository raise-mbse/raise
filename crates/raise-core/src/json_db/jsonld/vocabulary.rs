// FICHIER : src-tauri/src/json_db/jsonld/vocabulary.rs

use crate::json_db::collections::manager::CollectionsManager;
use crate::utils::prelude::*;

// =========================================================================
// STRUCTURES MÉTIERS (Optimisées avec SharedRef pour l'Interning)
// =========================================================================

#[derive(Debug, Clone, Serializable, Deserializable, PartialEq)]
pub enum PropertyType {
    DatatypeProperty,
    ObjectProperty,
}

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct Class {
    pub iri: SharedRef<str>,
    pub label: SharedRef<str>,
    pub comment: SharedRef<str>,
    pub sub_class_of: Option<SharedRef<str>>,
}

#[derive(Debug, Clone, Serializable, Deserializable)]
pub struct Property {
    pub iri: SharedRef<str>,
    pub label: SharedRef<str>,
    pub property_type: PropertyType,
    pub domain: Option<SharedRef<str>>,
    pub range: Option<SharedRef<str>>,
}

// =========================================================================
// L'ÉTAT IMMUABLE RCU (Read-Copy-Update)
// =========================================================================

#[derive(Debug, Clone, Default)]
pub struct RegistryState {
    pub classes: UnorderedMap<SharedRef<str>, Class>,
    pub properties: UnorderedMap<SharedRef<str>, Property>,
    pub default_context: UnorderedMap<String, SharedRef<str>>,
    pub layer_contexts: UnorderedMap<String, JsonValue>,
    pub ancestry: UnorderedMap<SharedRef<str>, UniqueSet<SharedRef<str>>>,
}

// =========================================================================
// REGISTRE PRINCIPAL (RCU + Interning)
// =========================================================================

static INSTANCE: StaticCell<VocabularyRegistry> = StaticCell::new();

pub struct VocabularyRegistry {
    state: SyncRwLock<SharedRef<RegistryState>>,
    intern_pool: SyncRwLock<UnorderedMap<String, SharedRef<str>>>,
}

impl Default for VocabularyRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VocabularyRegistry {
    pub fn new() -> Self {
        Self {
            state: SyncRwLock::new(SharedRef::new(RegistryState::default())),
            intern_pool: SyncRwLock::new(UnorderedMap::new()),
        }
    }

    /// INTERNING : Déduplication atomique des chaînes en RAM (Zéro Allocation Redondante).
    pub fn intern(&self, text: &str) -> SharedRef<str> {
        if let Ok(pool) = self.intern_pool.read() {
            if let Some(s) = pool.get(text) {
                return s.clone();
            }
        }
        if let Ok(mut pool) = self.intern_pool.write() {
            if let Some(s) = pool.get(text) {
                return s.clone();
            }
            let shared: SharedRef<str> = SharedRef::from(text);
            pool.insert(text.to_string(), shared.clone());
            shared
        } else {
            SharedRef::from(text)
        }
    }

    pub fn get_state(&self) -> SharedRef<RegistryState> {
        self.state
            .read()
            .map(|s| s.clone())
            .unwrap_or_else(|_| SharedRef::new(RegistryState::default()))
    }

    fn rebuild_ancestry(state: &mut RegistryState) {
        let mut ancestry = UnorderedMap::new();
        for (iri, cls) in &state.classes {
            let mut ancestors = UniqueSet::new();
            let mut current = cls.sub_class_of.clone();
            let mut depth = 0;
            while let Some(parent) = current {
                if depth > 100 || !ancestors.insert(parent.clone()) {
                    break;
                }
                current = state
                    .classes
                    .get(parent.as_ref() as &str)
                    .and_then(|c| c.sub_class_of.clone());
                depth += 1;
            }
            ancestry.insert(iri.clone(), ancestors);
        }
        state.ancestry = ancestry;
    }

    /// INITIALISATION DEPUIS LA DB (Zéro Fichier)
    pub async fn init_from_db(db_mgr: &CollectionsManager<'_>) -> RaiseResult<()> {
        // 1. On garantit que l'instance globale existe avant de la populer
        if INSTANCE.get().is_none() {
            let _ = INSTANCE.set(Self::new());
        }
        let registry = Self::global()?;

        // 2. On lit le catalogue système
        let sys_path = db_mgr
            .storage
            .config
            .db_root(&db_mgr.space, &db_mgr.db)
            .join("_system.json");

        if fs::exists_async(&sys_path).await {
            let content = fs::read_to_string_async(&sys_path).await?;
            let sys_doc: JsonValue = json::deserialize_from_str(&content)?;

            // 3. Hydratation de l'état RCU via le registre global
            if let Some(ontologies) = sys_doc.get("ontologies").and_then(|o| o.as_object()) {
                for (ns, _) in ontologies {
                    if let Ok(Some(doc)) = db_mgr
                        .get_document("_ontologies", &format!("ontology_{}", ns))
                        .await
                    {
                        // On injecte les données DB dans le singleton actif !
                        let _ = registry.load_layer_from_json(ns, &doc).await;
                    }
                }
            }
        }
        Ok(())
    }

    pub fn set_global_instance(registry: Self) {
        let _ = INSTANCE.set(registry);
    }

    pub fn global() -> RaiseResult<&'static Self> {
        if let Some(registry) = INSTANCE.get() {
            Ok(registry)
        } else {
            raise_error!(
                "ERR_DB_REGISTRY_NOT_INITIALIZED",
                error = "Le registre sémantique (VocabularyRegistry) est inaccessible.",
                context = json_value!({
                    "help": "Vérifiez que VocabularyRegistry::init_from_db() a été appelé durant l'amorçage."
                })
            )
        }
    }

    /// CŒUR RCU : Parsing JSON-LD 100% en RAM
    pub async fn load_layer_from_json(&self, layer: &str, json: &JsonValue) -> RaiseResult<()> {
        let Some(ctx) = json.get("@context") else {
            raise_error!(
                "ERR_JSONLD_CONTEXT_MISSING",
                context = json_value!({"layer": layer})
            );
        };

        let mut new_state = (*self.get_state()).clone();
        new_state
            .layer_contexts
            .insert(layer.to_string(), ctx.clone());

        if let Some(obj) = ctx.as_object() {
            for (k, v) in obj {
                if let Some(iri) = v.as_str() {
                    new_state
                        .default_context
                        .insert(k.clone(), self.intern(iri));
                }
            }
        }

        let expand = |t: &str, s: &RegistryState| -> SharedRef<str> {
            if Self::is_iri(t) {
                return self.intern(t);
            }
            if let Some((p, suf)) = t.split_once(':') {
                if let Some(b) = s.default_context.get(p) {
                    return self.intern(&format!("{}{}", b, suf));
                }
            }
            self.intern(t)
        };

        if let Some(graph) = json.get("@graph").and_then(|v| v.as_array()) {
            for node in graph {
                if let Some(id) = node.get("@id").and_then(|v| v.as_str()) {
                    let full_id = expand(id, &new_state);
                    let types = extract_types(node);
                    if types.contains(&"owl:Class".to_string()) {
                        new_state.classes.insert(
                            full_id.clone(),
                            Class {
                                iri: full_id.clone(),
                                label: self.intern(&get_str(node, "rdfs:label")),
                                comment: self.intern(&get_str(node, "rdfs:comment")),
                                sub_class_of: get_str_opt(node, "rdfs:subClassOf")
                                    .map(|s| expand(&s, &new_state)),
                            },
                        );
                    }
                    if types.contains(&"owl:ObjectProperty".to_string())
                        || types.contains(&"owl:DatatypeProperty".to_string())
                    {
                        new_state.properties.insert(
                            full_id.clone(),
                            Property {
                                iri: full_id.clone(),
                                label: self.intern(&get_str(node, "rdfs:label")),
                                property_type: if types.contains(&"owl:ObjectProperty".to_string())
                                {
                                    PropertyType::ObjectProperty
                                } else {
                                    PropertyType::DatatypeProperty
                                },
                                domain: get_str_opt(node, "rdfs:domain")
                                    .map(|s| expand(&s, &new_state)),
                                range: get_str_opt(node, "rdfs:range")
                                    .map(|s| expand(&s, &new_state)),
                            },
                        );
                    }
                }
            }
        }
        Self::rebuild_ancestry(&mut new_state);
        if let Ok(mut g) = self.state.write() {
            *g = SharedRef::new(new_state);
            Ok(())
        } else {
            raise_error!("ERR_LOCK_POISONED")
        }
    }

    // --- ACCESSEURS (Rétablit les capacités du Cerveau) ---

    pub fn get_class(&self, iri: &str) -> Option<Class> {
        self.get_state().classes.get(iri).cloned()
    }
    pub fn get_property(&self, iri: &str) -> Option<Property> {
        self.get_state().properties.get(iri).cloned()
    }
    pub fn has_class(&self, iri: &str) -> bool {
        self.get_state().classes.contains_key(iri)
    }

    pub fn get_context_for_layer(&self, layer: &str) -> Option<JsonValue> {
        self.get_state().layer_contexts.get(layer).cloned()
    }

    pub fn get_default_context(&self) -> UnorderedMap<String, SharedRef<str>> {
        self.get_state().default_context.clone()
    }

    pub fn is_subtype_of(&self, children: &[String], parent: &str) -> bool {
        children.iter().any(|child| {
            if child == parent {
                return true;
            }
            self.get_state()
                .ancestry
                .get(child.as_str())
                .is_some_and(|a| a.contains(parent))
        })
    }

    pub fn is_iri(t: &str) -> bool {
        t.starts_with("http") || t.starts_with("urn:")
    }
}

// Helpers
fn extract_types(n: &JsonValue) -> Vec<String> {
    n.get("@type")
        .map(|t| {
            if let Some(s) = t.as_str() {
                vec![s.to_string()]
            } else {
                t.as_array()
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default()
            }
        })
        .unwrap_or_default()
}
fn get_str(n: &JsonValue, k: &str) -> String {
    get_str_opt(n, k).unwrap_or_default()
}
fn get_str_opt(n: &JsonValue, k: &str) -> Option<String> {
    n.get(k).and_then(|v| v.as_str().map(|s| s.to_string()))
}

// ============================================================================
// TESTS UNITAIRES ROBUSTES
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[async_test]
    #[serial_test::serial]
    async fn test_vocabulary_integrity_and_rcu() -> RaiseResult<()> {
        let reg = VocabularyRegistry::new();

        // 1. Test de l'interning (Égalité des pointeurs)
        let uri = "https://raise.io/oa#OperationalActivity";
        let p1 = reg.intern(uri);
        let p2 = reg.intern(uri);
        assert!(
            SharedRef::ptr_eq(&p1, &p2),
            "Dette détectée : Doublon mémoire pour une même URI."
        );

        // 2. Test RCU (Isolation pendant erreur)
        let valid_json = json_value!({
            "@context": { "oa": "https://raise.io/oa#" },
            "@graph": [
                { "@id": "oa:Actor", "@type": "owl:Class", "rdfs:label": "Acteur" }
            ]
        });
        reg.load_layer_from_json("oa", &valid_json).await?;
        let state_v1 = reg.get_state();

        // Tentative de chargement d'un JSON corrompu
        let corrupt_json = json_value!({ "invalid": true });
        let result = reg.load_layer_from_json("fail", &corrupt_json).await;

        assert!(
            result.is_err(),
            "Le registre aurait dû rejeter le JSON malformé."
        );
        // L'état ne doit pas avoir bougé
        assert!(
            SharedRef::ptr_eq(&state_v1, &reg.get_state()),
            "L'état a été corrompu après un échec."
        );

        Ok(())
    }

    #[test]
    fn test_ancestry_inference() {
        // ✅ FIX : On n'a pas besoin d'instancier 'reg' pour appeler la méthode statique.
        let mut s = RegistryState::default();

        let root: SharedRef<str> = SharedRef::from("CoreNode");
        let child: SharedRef<str> = SharedRef::from("Actor");

        s.classes.insert(
            child.clone(),
            Class {
                iri: child.clone(),
                label: SharedRef::from(""),
                comment: SharedRef::from(""),
                sub_class_of: Some(root.clone()),
            },
        );

        // Appel direct via le type.
        VocabularyRegistry::rebuild_ancestry(&mut s);

        let ancestors = s
            .ancestry
            .get(&child)
            .expect("L'entrée d'ancêtre devrait exister.");
        assert!(ancestors.contains(&root));
    }
}
