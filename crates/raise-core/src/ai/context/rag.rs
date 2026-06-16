// FICHIER : crates/raise-core/src/ai/context/rag.rs
use crate::ai::memory::{native_store::NativeLocalStore, MemoryRecord, VectorStore};
use crate::ai::nlp::{embeddings::EmbeddingEngine, splitting};
use crate::json_db::collections::manager::CollectionsManager;

use crate::utils::prelude::*;

pub struct RagRetriever {
    backend: NativeLocalStore, // 🎯 Connexion directe et exclusive au moteur natif
    embedder: EmbeddingEngine,
    collection_name: String,
}

impl RagRetriever {
    /// Initialise le RAG en se basant EXCLUSIVEMENT sur la configuration globale
    pub async fn new(manager: &CollectionsManager<'_>) -> RaiseResult<Self> {
        let config = AppConfig::get();
        // 🎯 FIX : Utilisation des chemins domaines via la config centralisée
        let storage_path = config
            .get_path("PATH_RAISE_DOMAIN")
            .unwrap_or_else(|| PathBuf::from("./raise_default_domain"));

        Self::new_internal(storage_path, manager).await
    }

    /// Constructeur interne pour injecter le path et le manager (Idéal pour les tests)
    pub async fn new_internal(
        storage_path: PathBuf,
        manager: &CollectionsManager<'_>,
    ) -> RaiseResult<Self> {
        // 🎯 GOUVERNANCE STRICTE : Vérification de l'activation du composant RAG
        let _settings = match AppConfig::get_runtime_settings(
            manager,
            "ref:components:handle:ai_context",
        )
        .await
        {
            Ok(s) => s,
            Err(e) => raise_error!(
                "ERR_RAG_INIT_REJECTED",
                error = e.to_string(),
                context = json_value!({"action": "rag_init", "hint": "Le composant RAG est-il actif et configuré dans le catalogue système ?"})
            ),
        };

        // 🎯 FIX ZÉRO DETTE : Le nom de la collection provient désormais du catalogue de gouvernance
        let collection_name = _settings
            .get("collection_name")
            .and_then(|v| v.as_str())
            .unwrap_or("raise_knowledge_base")
            .to_string();

        // 🎯 Initialisation du moteur d'embeddings via le point de montage système
        let embedder = EmbeddingEngine::new(manager).await?;

        user_info!(
            "INF_RAG_NATIVEENGINE_INIT",
            json_value!({
                "backend": "NATIVE_EMBEDDING",
                "device": "Native",
                "collection": collection_name
            })
        );

        let device = ComputeHardware::Cpu;
        let store_dir = storage_path.join("vector_store");
        let memory = NativeLocalStore::new(manager, &device).await?;

        // 🎯 Rigueur : Passage du manager à l'infrastructure vectorielle
        memory
            .init_collection(manager, &collection_name, 384)
            .await?;

        // Chargement sécurisé de l'index
        match memory.load(manager).await {
            Ok(_) => user_trace!("INF_RAG_LOADED", json_value!({"path": store_dir})),
            Err(e) => user_warn!("WRN_RAG_EMPTY", json_value!({"error": e.to_string()})),
        }

        Ok(Self {
            backend: memory,
            embedder,
            collection_name,
        })
    }

    pub async fn index_document(
        &mut self,
        manager: &CollectionsManager<'_>,
        content: &str,
        source: &str,
    ) -> RaiseResult<usize> {
        let chunks = splitting::split_text_into_chunks(content, 512);
        if chunks.is_empty() {
            return Ok(0);
        }

        // 🎯 Match strict sur le batch d'embeddings
        let vectors = match self.embedder.embed_batch(chunks.clone()) {
            Ok(v) => v,
            Err(e) => raise_error!(
                "ERR_RAG_EMBEDDING_BATCH",
                error = e,
                context = json_value!({"source": source, "chunks_count": chunks.len()})
            ),
        };

        let ingest_time = UtcClock::now().to_rfc3339();

        let mut records = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            records.push(MemoryRecord {
                id: UniqueId::new_v4().to_string(),
                content: chunk.clone(),
                metadata: json_value!({
                    "source": source,
                    "chunk_index": i,
                    "total_chunks": chunks.len(),
                    "ingested_at": ingest_time
                }),
                vectors: Some(vectors[i].clone()),
            });
        }

        if let Err(e) = self
            .backend
            .add_documents(manager, &self.collection_name, records)
            .await
        {
            raise_error!(
                "ERR_RAG_ADD_DOCUMENTS",
                error = e,
                context = json_value!({"collection": self.collection_name, "source": source})
            );
        }
        if let Err(e) = self.backend.save(manager).await {
            raise_error!(
                "ERR_RAG_SAVE_BACKEND",
                error = e,
                context = json_value!({"collection": self.collection_name})
            );
        }

        Ok(chunks.len())
    }

    pub async fn retrieve(
        &mut self,
        manager: &CollectionsManager<'_>,
        query: &str,
        limit: u64,
    ) -> RaiseResult<String> {
        let query_vector = match self.embedder.embed_query(query) {
            Ok(v) => v,
            Err(e) => raise_error!(
                "ERR_RAG_EMBED_QUERY",
                error = e,
                context = json_value!({"query": query})
            ),
        };

        // Seuil Arcadia pour la pertinence sémantique
        let min_similarity = 0.65;

        let docs = match self
            .backend
            .search_similarity(
                manager,
                &self.collection_name,
                &query_vector,
                limit,
                min_similarity,
                None,
            )
            .await
        {
            Ok(d) => d,
            Err(e) => raise_error!(
                "ERR_RAG_SEARCH",
                error = e,
                context = json_value!({"query": query, "limit": limit})
            ),
        };

        if docs.is_empty() {
            return Ok(String::new());
        }

        let mut context_str = String::from("### DOCUMENTATION PERTINENTE (RAG) ###\n");
        for (i, doc) in docs.iter().enumerate() {
            let source = doc
                .metadata
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            context_str.push_str(&format!("Source [{}]: {}\n", source, doc.content));
            if i < docs.len() - 1 {
                context_str.push('\n');
            }
        }
        Ok(context_str)
    }
}
// =========================================================================
// TESTS UNITAIRES (Restauration intégrale + Nouveaux Tests)
// =========================================================================
#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::testing::{AgentDbSandbox, DbSandbox};

    fn get_hf_lock() -> &'static AsyncMutex<()> {
        static LOCK: StaticCell<AsyncMutex<()>> = StaticCell::new();
        LOCK.get_or_init(|| AsyncMutex::new(()))
    }

    /// 🎯 HELPER ZÉRO DETTE : Injecte les autorisations et configurations requises
    /// dans la base de données de test pour permettre le démarrage des moteurs IA.
    async fn inject_mock_ai_configs(manager: &CollectionsManager<'_>) -> RaiseResult<()> {
        let config = AppConfig::get();

        // 🎯 FIX : On utilise le schéma générique permissif déjà fourni par DbSandbox !
        let generic_schema_uri = format!(
            "db://{}/{}/schemas/v1/db/generic.schema.json",
            config.mount_points.system.domain, config.mount_points.system.db
        );

        // ----------------------------------------------------------------------
        // 1. CRÉATION DES COMPOSANTS (Pour satisfaire l'intégrité référentielle)
        // ----------------------------------------------------------------------
        let _ = manager
            .create_collection("components", &generic_schema_uri)
            .await;

        manager
            .upsert_document(
                "components",
                json_value!({
                    "_id": "comp_rag_id",
                    "handle": "ai_context",
                    "name": "RAG Engine"
                }),
            )
            .await?;

        manager
            .upsert_document(
                "components",
                json_value!({
                    "_id": "comp_store_id",
                    "handle": "ai_graph_store",
                    "name": "Vector Store"
                }),
            )
            .await?;

        // ----------------------------------------------------------------------
        // 2. CRÉATION DES CONFIGURATIONS (Pointant vers les composants valides)
        // ----------------------------------------------------------------------
        let _ = manager
            .create_collection("service_configs", &generic_schema_uri)
            .await;

        manager
            .upsert_document(
                "service_configs",
                json_value!({
                    "_id": "mock_rag_cfg",
                    "component_id": "ref:components:handle:ai_context",
                    "service_settings": {
                        "collection_name": "raise_knowledge_base"
                    }
                }),
            )
            .await?;

        manager
            .upsert_document(
                "service_configs",
                json_value!({
                    "_id": "mock_store_cfg",
                    "component_id": "ref:components:handle:ai_graph_store",
                    "service_settings": {
                        "embedding_dim": 384
                    }
                }),
            )
            .await?;

        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_rag_engine_end_to_end() -> RaiseResult<()> {
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;
        inject_mock_ai_configs(&manager).await?; // 🎯 FIX : Déblocage de la gouvernance

        let mut rag = RagRetriever::new_internal(sandbox.domain_root.clone(), &manager).await?;

        let content = "Le module de sécurité requiert une validation cryptographique SHA-256.";
        rag.index_document(&manager, content, "spec_secu_v2.pdf")
            .await?;

        {
            let _guard = get_hf_lock().lock().await;

            let content = "Le module de sécurité requiert une validation cryptographique SHA-256.";
            rag.index_document(&manager, content, "spec_secu_v2.pdf")
                .await?;

            let context = rag
                .retrieve(&manager, "validation cryptographique SHA-256", 1)
                .await?;

            assert!(
                context.contains("SHA-256"),
                "Le contexte doit contenir la donnée"
            );
            assert!(
                context.contains("spec_secu_v2.pdf"),
                "Le contexte doit contenir la source"
            );
        }
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_rag_engine_empty_results() -> RaiseResult<()> {
        let _guard = get_hf_lock().lock().await;
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;
        inject_mock_ai_configs(&manager).await?; // 🎯 FIX : Déblocage de la gouvernance

        let mut rag = RagRetriever::new_internal(sandbox.domain_root.clone(), &manager).await?;
        rag.index_document(&manager, "Ceci parle de cuisine.", "chef.txt")
            .await?;

        let context = rag.retrieve(&manager, "Comment coder en Rust ?", 1).await?;
        assert_eq!(context, "");
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_rag_engine_persistence() -> RaiseResult<()> {
        let _guard = get_hf_lock().lock().await;
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;
        inject_mock_ai_configs(&manager).await?; // 🎯 FIX : Déblocage de la gouvernance

        {
            let mut rag = RagRetriever::new_internal(sandbox.domain_root.clone(), &manager).await?;
            rag.index_document(&manager, "La persistance Zstd est hyper rapide.", "doc_io")
                .await?;
        }

        let mut new_rag = RagRetriever::new_internal(sandbox.domain_root.clone(), &manager).await?;
        let context = new_rag
            .retrieve(&manager, "Est-ce que Zstd est rapide ?", 1)
            .await?;
        assert!(context.contains("hyper rapide"));
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_rag_chunking_logic() -> RaiseResult<()> {
        let _guard = get_hf_lock().lock().await;
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();
        let manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&manager).await?;
        inject_mock_ai_configs(&manager).await?; // 🎯 FIX : Déblocage de la gouvernance

        let mut rag = RagRetriever::new_internal(sandbox.domain_root.clone(), &manager).await?;
        let long_text = "Data ".repeat(1000);
        let count = rag.index_document(&manager, &long_text, "stress").await?;

        assert!(
            count > 1,
            "Le texte aurait dû être découpé en plusieurs chunks"
        );
        Ok(())
    }

    #[async_test]
    #[serial_test::serial]
    #[cfg_attr(not(feature = "cuda"), ignore)]
    async fn test_rag_init_failure_handling() -> RaiseResult<()> {
        let _guard = get_hf_lock().lock().await;
        let sandbox = AgentDbSandbox::new().await?;
        let config = AppConfig::get();

        // 🎯 FIX : On sécurise la partition système (qui est ciblée par le Fallback)
        let system_manager = CollectionsManager::new(
            &sandbox.db,
            &config.mount_points.system.domain,
            &config.mount_points.system.db,
        );
        DbSandbox::mock_db(&system_manager).await?;
        inject_mock_ai_configs(&system_manager).await?;

        // On crée un manager pointant vers un domaine inexistant
        let void_manager = CollectionsManager::new(&sandbox.db, "void_space", "void_db");

        // Le Fallback Global Zéro Dette va rediriger la demande d'autorisation vers system_manager !
        let result = RagRetriever::new_internal(sandbox.domain_root.clone(), &void_manager).await;

        assert!(
            result.is_ok(),
            "L'initialisation doit réussir grâce aux fallbacks globaux qui trouvent la config dans la base système."
        );
        Ok(())
    }
}
