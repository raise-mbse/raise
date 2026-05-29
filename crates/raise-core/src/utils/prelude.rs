// FICHIER : src-tauri/src/utils/prelude.rs

// =========================================================================
//  RAISE PRELUDE - L'Unique Façade Sémantique (Zéro Dette)
// =========================================================================

// --- 1. CORE, ERREURS & FONDATIONS (Synchronisés avec core/mod.rs) ---
pub use crate::utils::context::i18n::I18nString;
pub use crate::utils::core::error::{anyhow, AnyResult, AppError, Context, RaiseResult};
pub use crate::utils::core::{
    async_interface, // 🎯 Alias de async_trait::async_trait
    async_main,
    async_recursive,
    async_test,
    is_same_reference,
    memory_copy_fast,
    sleep_async,
    // Runtime & Tasks
    spawn_async_task,
    spawn_cpu_task,
    terminate_process,
    AgentAttention,
    AsyncChannel,
    AsyncCommand,
    AsyncFuture,
    AsyncMutex,
    AsyncRwLock,
    AsyncStaticCell,
    BufferedRead,
    CalendarDate,
    CalendarDuration,
    CowData,
    CryptoDigest,
    CryptoSha256,
    DataStreamPeekable, // 🎯 Pour l'anticipation (lookahead)
    Eq,
    // Formatage Sémantique
    FmtCursor, // 🎯 Remplace Formatter (plus visuel: on écrit là où est le curseur)
    FmtDebug,
    FmtDisplay,  // 🎯 Remplace Display (plus explicite pour l'IA)
    FmtOrdering, // 🎯 L'alias sémantique pour Ordering
    FmtResult,   // 🎯 Résultat de l'opération de formatage
    Hashable,
    InitGuard,      // 🎯 Alias de Once
    LocalClock,     // 🎯 Alias de chrono::Local
    LocalTimestamp, // 🎯 Alias de chrono::DateTime<Local>
    MaxOf,
    MemoryCache,
    MinOf,
    Ord,
    Parsable,
    PartialEq,
    PartialOrd,
    Pinned,
    SafeSize,
    // Concurrence & Mémoire (Alias RAISE)
    SharedRef,  // 🎯 Alias de Arc
    StaticCell, // 🎯 Alias de OnceLock
    SyncMutex,
    SyncMutexGuard,
    SyncRwLock,
    SystemStr, // 🎯 Pour la compatibilité OS native
    TextChars, // 🎯 Pour le découpage atomique du texte
    TextRegex,
    TextRegexError,
    TimeDuration, // 🎯 Alias de std::time::Duration
    TimeInstant,
    TypeMarker,
    // Identifiants & Temps (Alias RAISE)
    UniqueId,     // 🎯 Alias de uuid::Uuid
    UtcClock,     // 🎯 Alias de chrono::Utc
    UtcTimestamp, // 🎯 Alias de chrono::DateTime<Utc>
    VariantMarker,
    MATH_PI, // 🎯 La constante fondamentale
};

// --- 2. I/O, FS & SYSTÈME ---
pub use crate::utils::io::io_traits::{SyncBufRead, SyncRead, SyncSeek, SyncWrite};
pub use crate::utils::io::os_types::{
    os_temp_dir, ProcessChild, ProcessCommand, ProcessExitStatus, ProcessIoConfig, ProcessOutput,
    UnixFilePermissions,
};
pub use crate::utils::io::{
    compress, decompress, fs, os, stderr_raw, stdin_raw, stdout_raw, tempdir, Path, PathBuf,
    TempDir,
};

// --- 3. DATA, JSON & COLLECTIONS ---
pub use crate::utils::data::config::{AppConfig, CoreConfig};
pub use crate::utils::data::encoding::{decode_base64, encode_base64};
pub use crate::utils::data::json::{self, json_value, JsonObject, JsonValue};
pub use crate::utils::data::{
    Deserializable,
    DeserializableOwned,
    OrderedMap, // 🎯 BTreeMap sémantique
    Serializable,
    UniqueSet,    // 🎯 HashSet sémantique
    UnorderedMap, // 🎯 HashMap sémantique
};

// --- 4. INFÉRENCE & MACHINE LEARNING (Forteresse RAISE) ---
// Ces exports masquent totalement l'écosystème ML (Candle, FastEmbed, Tokenizers)
// au reste du projet pour garantir un couplage faible et une haute résilience.
pub use crate::utils::inference::{
    compute_cross_entropy, // Fonction de coût (Loss)

    configure_parallel_pool, // Limiteur de ressources CPU multi-threads
    execute_parallel_map,    // Itération parallèle ultra-rapide
    init_embedding_layer,    // Constructeur de couche d'embedding

    init_linear_layer,   // Constructeur de couche linéaire
    init_lstm_layer,     // Constructeur de couche LSTM
    load_neural_weights, // Charge les fichiers .safetensors sans risque de crash (mmap)

    resolve_compute_device, // Alloue le meilleur GPU disponible avec fallback CPU
    // 🧬 1. Types Fondamentaux (Calcul Matriciel)
    ComputeHardware,           // Matériel physique cible (CUDA, Metal, CPU)
    ComputeType,               // Précision mathématique requise (ex: F32, F16)
    DimIndex,                  // Outil d'indexation pour manipuler les dimensions (D)
    GgufFileFormat,            // Module I/O pour modèles quantifiés
    LightweightEmbeddingModel, // Modèles CPU supportés (BGE, MiniLM)
    LightweightInitOptions,    // Paramètres d'exécution légers

    // ⚡ 8. Moteur d'Embeddings Léger (ONNX / CPU)
    LightweightTextEmbedding, // Moteur d'inférence léger (FastEmbed)
    NeuralActivation,         // Fonctions d'activation (ReLU, etc.)
    NeuralBertConfig,         // Configuration du modèle BERT

    NeuralBertModel, // Moteur natif de vectorisation (BERT)
    NeuralCoreError, // Erreur native du moteur Tensoriel

    NeuralEmbeddingLayer, // Couche de plongement (Embedding)
    NeuralInitStrategy,   // Stratégies d'initialisation (Xavier, etc.)
    NeuralLinearLayer,    // Couche Dense / Linéaire
    NeuralLstmLayer,      // Couche Récurrente LSTM
    // 🏗️ 3. Architecture et Couches Neuronales
    NeuralModule,         // Trait de Forward Pass
    NeuralOptimizerAdamW, // Optimiseur AdamW
    // 🎓 4. Apprentissage et Optimisation
    NeuralOptimizerTrait, // Trait de base pour l'optimisation
    NeuralRnnTrait,       // Trait spécifique aux RNN (ex: LSTM)
    NeuralShape,          // Dimensions des matrices/tenseurs
    NeuralTensor,         // Structure de données cœur pour les calculs d'IA
    NeuralVar,            // Variable mutable pour l'optimiseur
    // ⚖️ 2. Gestion des Modèles et Poids (NN)
    NeuralWeightsBuilder, // Constructeur pour charger les paramètres d'un modèle
    NeuralWeightsMap,     // Espace mémoire hébergeant les poids du réseau
    NvidiaMonitor,
    OptimizerConfigAdamW, // Hyperparamètres de l'optimiseur
    Qwen2QuantizedModel,  // Moteur LLM natif (ex: Qwen2)

    SafeTensorsIO, // Module I/O pour poids natifs
    // ⚙️ Utilitaires d'Inférence & Infrastructure
    TextEmbedder, // Pipeline RAG complet
    // 📝 5. Tokenisation et Traitement du Texte (NLP)
    TextTokenizer, // Moteur de tokenisation universel
    // 🤖 6. Génération Textuelle & LLM (Transformers)
    TokenLogitsProcessor, // Gestionnaire de température / Top-P / Top-K
    WhisperAudio,         // Traitement du signal (Mel, MFCC)
    WhisperConfig,        // Paramétrage audio
    // 👁️ 7. Multimodalité : Audio & Vision
    WhisperModel,        // Architecture de transcription audio
    DEFAULT_EMBED_MODEL, // Modèle par défaut
};

// --- 5. RÉSEAU & CONNECTIVITÉ ---
pub use crate::utils::network::http_types::{
    run_http_server, HttpClient, HttpClientBuilder, HttpJsonPayload, HttpRouter, HttpStatusCode,
    HttpTcpListener,
};

pub use crate::utils::network::p2p_types::{
    connection_limits,
    gossipsub,
    identity,
    kad,
    request_response,
    P2pBehaviour,
    P2pConnectionLimits,
    P2pGossipSub,
    P2pIdentity,
    P2pKademlia,
    P2pMultiaddr,
    P2pNetworkBehaviourTrait, // 🎯 Requis pour les implémentations de protocoles[cite: 11]
    P2pNoise,
    P2pPeerId,
    P2pRequestResponse, // 🎯 Pour les échanges directs de messages[cite: 11]
    P2pStreamProtocol,  // 🎯 Pour l'identification des flux[cite: 11]
    P2pSwarm,
    P2pSwarmBuilder, // 🎯 Pour la configuration fine du Swarm[cite: 11]
    P2pSwarmEvent,
    P2pTransportTrait,
    P2pYamux,
    StreamProtocol,
};

pub use crate::utils::network::{
    build_p2p_node_async, get_client, get_string_async, post_authenticated_async,
    post_json_with_retry_async, start_local_api_async,
};

// --- 6. MACROS & OBSERVABILITÉ (Exports Racine) ---
pub use crate::{
    build_error, kernel_fatal, kernel_trace, raise_error, require_session, user_debug, user_error,
    user_info, user_success, user_trace, user_warn,
};

// On expose les logs de core pour le paramétrage du moteur
pub use crate::utils::core::logs;
