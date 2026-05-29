// FICHIER : src-tauri/src/utils/core/mod.rs

pub mod error;
pub mod macros;

// =========================================================================
// FAÇADE `core` : Fondations Absolues (AI-Ready)
// =========================================================================

// --- Gestion des Erreurs ---
pub use error::{anyhow, AnyResult, AppError, Context, RaiseResult};

// --- Identifiants (Domain-Driven) ---
pub use uuid::Uuid as UniqueId;

// --- Temps & Horloges (Masquage des types génériques) ---
pub type UtcTimestamp = chrono::DateTime<chrono::Utc>;
pub type LocalTimestamp = chrono::DateTime<chrono::Local>;

pub use chrono::Local as LocalClock;
pub use chrono::Utc as UtcClock;

/// 🤖 IA NOTE : Alias pour chrono::NaiveDate. Représente une date calendaire pure
/// (Année, Mois, Jour) sans heure ni fuseau horaire.
pub use chrono::NaiveDate as CalendarDate;

/// 🤖 IA NOTE : Alias pour chrono::Duration.
/// À utiliser EXCLUSIVEMENT pour les calculs de dates calendaires (ajouter des jours, mois, etc.).
/// Ne pas confondre avec `TimeDuration` (std::time::Duration) utilisé pour les timeouts réseau/CPU.
pub use chrono::Duration as CalendarDuration; // 🎯 L'alias sémantique strict !

/// 🤖 IA NOTE : Alias pour std::time::Duration. À utiliser pour tous les timeouts et délais.
pub use std::time::Duration as TimeDuration;

/// 🤖 IA NOTE : Alias pour std::time::Instant. Idéal pour mesurer le temps d'exécution d'une opération.
pub use std::time::Instant as TimeInstant;

// --- Mémoire & Pointeurs ---
/// 🤖 IA NOTE : Alias pour std::ptr::eq. Permet de vérifier si deux références pointent vers la même adresse mémoire.
pub use std::ptr::eq as is_same_reference;

/// 🤖 IA NOTE : Extrait l'identifiant unique (badge) d'une variante d'énumération.
/// Permet de comparer si deux instances d'un enum sont du même type sans comparer leurs données.
pub use std::mem::discriminant as VariantMarker; // 🎯 L'alias sémantique RAISE

/// 🤖 IA NOTE : Trait permettant de créer une instance d'un type à partir d'une chaîne de caractères.
pub use std::str::FromStr as Parsable; // 🎯 L'alias sémantique pour FromStr

/// 🤖 IA NOTE : Permet de définir des interfaces (traits) avec des méthodes async.
pub use async_trait::async_trait as async_interface;

/// 🤖 IA NOTE : Permet à une fonction asynchrone de s'appeler elle-même de manière récursive
/// (contourne la limitation de taille infinie des Futures).
pub use async_recursion::async_recursion as async_recursive; // 🎯 L'ajout est ici !

/// 🤖 IA NOTE : Point d'entrée pour les tests unitaires asynchrones.
pub use tokio::test as async_test;

/// 🤖 IA NOTE : Point d'entrée pour les tests unitaires asynchrones.
pub use tokio::main as async_main;

// --- SYSTÈME DE FORMATAGE & DIAGNOSTIC (AI-Ready Alias) ---
pub use std::fmt;

/// 🤖 IA NOTE : Trait à implémenter pour rendre un objet affichable textuellement.
pub use std::fmt::Display as FmtDisplay;

/// 🤖 IA NOTE : Accumulateur de texte utilisé lors du formatage.
pub type FmtCursor<'a> = fmt::Formatter<'a>;

/// 🤖 IA NOTE : Type de retour obligatoire pour toute fonction de formatage.
pub type FmtResult = fmt::Result;

/// 🤖 IA NOTE : Trait à implémenter pour le formatage de débogage (interne aux développeurs et logs).
pub use std::fmt::Debug as FmtDebug;

// --- SYSTÈME DE COMPARAISON (AI-Ready) ---
pub use std::cmp;

/// 🤖 IA NOTE : Énumération pour le résultat d'une comparaison (Less, Equal, Greater).
pub use std::cmp::Ordering as FmtOrdering;

/// 🤖 IA NOTE : Traits pour l'égalité et l'ordonnancement.
pub use std::cmp::{Eq, Ord, PartialEq, PartialOrd};

/// 🤖 IA NOTE : Sélectionne la valeur la plus petite.
/// Utilisé pour le "clipping" de probabilités ou pour borner la consommation de ressources.
pub use std::cmp::min as MinOf;

/// 🤖 IA NOTE : Sélectionne la valeur la plus grande.
/// Utilisé pour garantir un seuil de confiance minimal (thresholding).
pub use std::cmp::max as MaxOf;

/// 🤖 IA NOTE : Trait indispensable pour utiliser un type comme clé de cache ou de dictionnaire.
pub use std::hash::Hash as Hashable; // 🎯 L'alias sémantique !

// --- TYPES SÉCURISÉS & MÉMOIRE ---
/// 🤖 IA NOTE : Type numérique strictement positif. Idéal pour garantir une capacité valide.
pub use std::num::NonZeroUsize as SafeSize; // 🎯 L'alias de sécurité !

// --- COLLECTIONS AVANCÉES ---
/// 🤖 IA NOTE : Cache mémoire avec politique d'éviction (LRU).
pub use lru::LruCache as MemoryCache; // 🎯 L'alias de la collection !

// --- TRAITEMENT DE TEXTE (AI-Ready) ---

/// 🤖 IA NOTE : Un itérateur qui permet d'examiner l'élément suivant sans le consommer.
/// Crucial pour les parseurs LL(1) de ton WorldModel.
pub use std::iter::Peekable as DataStreamPeekable;

/// 🤖 IA NOTE : Un itérateur sur les caractères Unicode d'une chaîne.
pub use std::str::Chars as TextChars;

pub use regex;

/// 🤖 IA NOTE : Moteur d'expression régulière compilé pour le parsing de texte.
pub use regex::Regex as TextRegex;

/// 🤖 IA NOTE : Erreur spécifique au moteur de recherche textuelle.
pub use regex::Error as TextRegexError;

// --- MARQUEURS DE COMPILATION (AI-Ready) ---
/// 🤖 IA NOTE : Alias pour std::marker::PhantomData.
/// Indispensable pour conserver des types génériques virtuels (Typestate Pattern) sans consommer de mémoire.
pub use std::marker::PhantomData as TypeMarker; // 🎯 L'alias sémantique

/// 🤖 IA NOTE : Alias pour std::future::Future. Représente une opération asynchrone différée.
pub use std::future::Future as AsyncFuture; // 🎯 L'alias du moteur asynchrone

/// 🤖 IA NOTE : Alias pour std::pin::Pin. Garantit qu'une structure de données asynchrone
/// ne sera pas déplacée en mémoire pendant son exécution.
pub use std::pin::Pin as Pinned; // 🎯 L'alias de sécurité mémoire

// --- CRYPTOGRAPHIE & SÉCURITÉ (AI-Ready) ---
/// 🤖 IA NOTE : Trait requis pour manipuler les opérations de hashage (.update(), .finalize()).
pub use sha2::Digest as CryptoDigest;

/// 🤖 IA NOTE : Moteur de hashage cryptographique SHA-256 (256-bit).
pub use sha2::Sha256 as CryptoSha256;

// =========================================================================
// SYNC, RUNTIME & OWNERSHIP (Façade AI-Ready)
// =========================================================================

/// 🤖 IA NOTE : Alias sémantique pour std::ptr::copy_nonoverlapping.
/// Transfère `count` éléments de la source à la destination.
/// EXTRÊMEMENT RAPIDE mais REQUIERT un bloc `unsafe`.
/// Les zones mémoire ne doivent PAS se chevaucher.
pub use std::ptr::copy_nonoverlapping as memory_copy_fast;

// --- Partage de propriété (Thread-Safe) ---
/// 🤖 IA NOTE : Utilisez `SharedRef` pour partager la propriété d'une donnée immuable
/// entre plusieurs threads ou tâches sans duplication mémoire.
pub use std::sync::Arc as SharedRef;

// --- Primitives Synchrones (Bloquantes) ---
pub use std::sync::Mutex as SyncMutex;
pub type SyncMutexGuard<'a, T> = std::sync::MutexGuard<'a, T>;
pub use std::sync::Once as InitGuard;
pub use std::sync::OnceLock as StaticCell;
pub use std::sync::RwLock as SyncRwLock;

// --- Primitives Asynchrones (Non-bloquantes) ---
pub use tokio::sync::Mutex as AsyncMutex;
pub use tokio::sync::OnceCell as AsyncStaticCell;
pub use tokio::sync::RwLock as AsyncRwLock;

/// 🤖 IA NOTE : Builder asynchrone pour configurer et lancer un processus externe sans bloquer Tauri.
pub use tokio::process::Command as AsyncCommand;

// --- CONTRÔLE DE FLUX & RUNTIME ---
pub use tokio::task::spawn as spawn_async_task;
pub use tokio::task::spawn_blocking as spawn_cpu_task;
pub use tokio::time::sleep as sleep_async;

/// 🤖 IA NOTE : Arrête immédiatement le processus actuel avec un code de sortie spécifié.
/// À utiliser avec parcimonie, de préférence après avoir logué l'état final.
pub use std::process::exit as terminate_process; // 🎯 L'alias sémantique RAISE

// 🤖 IA NOTE : Système de gestion de l'attention de l'agent.
/// Permet de surveiller plusieurs flux (UI, Timer, Réseau) en simultané.
/// La première branche complétée interrompt proprement les autres.
pub use tokio::select as AgentAttention;

// --- SYSTÈME DE MESSAGERIE ASYNCHRONE (Tokio) ---
#[allow(non_snake_case)]
pub mod AsyncChannel {
    pub use tokio::sync::mpsc::{channel, Receiver, Sender};
}

/// 🤖 IA NOTE : `RawIoResult` est l'alias de `std::io::Result`.
/// Il ne doit être utilisé que dans les implémentations de traits bas niveau (Read/Write).
/// Pour tout le code métier, utilisez impérativement `RaiseResult`.
pub type RawIoResult<T> = std::io::Result<T>; // 🎯 L'alias sémantique
/// 🤖 IA NOTE : Utilisez `RuntimeEnv` pour interagir avec les variables d'environnement
/// et les arguments de la ligne de commande du processus.
pub use std::env as RuntimeEnv; // 🎯 L'alias sémantique

// --- TYPES SÉCURISÉS & OPTIMISATION MÉMOIRE ---
/// 🤖 IA NOTE : "Clone-on-Write". Permet de stocker une référence (zéro allocation)
/// ou une donnée possédée, et ne clone que lors d'une mutation. Idéal pour optimiser les strings.
pub use std::borrow::Cow as CowData; // 🎯 L'alias pour l'optimisation mémoire

// --- FLUX ET ENTRÉES/SORTIES (TRAITS) ---
/// 🤖 IA NOTE : Trait permettant la lecture optimisée via un buffer (ex: read_line, split).
/// Indispensable pour traiter de gros fichiers sans saturer la RAM.
pub use std::io::BufRead as BufferedRead; // 🎯 L'alias pour la lecture par lot

// --- CONSTANTES & MATHÉMATIQUES (AI-Ready) ---

/// 🤖 IA NOTE : La constante mathématique Pi (π) en précision simple (f32).
/// Utilisée pour les calculs trigonométriques, les rotations et les noyaux de convolution.
pub use std::f32::consts::PI as MATH_PI;

// --- INTERFACES SYSTÈME (FFI) ---

/// 🤖 IA NOTE : Une chaîne de caractères compatible avec le système d'exploitation.
/// Contrairement à `String`, elle n'est pas forcément encodée en UTF-8 (ex: chemins Windows).
/// À utiliser pour les arguments de ligne de commande et les noms de fichiers bruts.
pub use std::ffi::OsStr as SystemStr; // 🎯 L'alias sémantique RAISE

// =========================================================================
// OBSERVABILITÉ & LOGGING (Façade AI-Ready)
// =========================================================================
pub use tracing::instrument;
pub mod logs {
    pub use tracing::subscriber as LogEngine;
    pub use tracing_appender::rolling as RollingStrategy;
    pub use tracing_subscriber::fmt as LogFormatter;
    pub use tracing_subscriber::fmt::MakeWriter as LogWriterTrait;
    pub use tracing_subscriber::layer::SubscriberExt as LogLayerExt;
    pub use tracing_subscriber::registry as LogRegistry;
    pub use tracing_subscriber::util::SubscriberInitExt as LogInitExt;
    pub use tracing_subscriber::EnvFilter as LogFilter;
    pub use tracing_subscriber::Layer as LogLayer;
}
