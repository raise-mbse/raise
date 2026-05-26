// FICHIER : src-tauri/src/utils/io/audio.rs

use crate::utils::prelude::*;
use tokio::sync::mpsc;

// ==============================================================================
// VERSION ACTIVE : Compilée uniquement sur la station principale (feature "audio")
// ==============================================================================
#[cfg(feature = "audio")]
pub use active_impl::*;

#[cfg(feature = "audio")]
mod active_impl {
    use super::*;
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use ringbuf::{
        traits::{Consumer, Producer, Split},
        HeapRb,
    };

    const SAMPLE_RATE: u32 = 16000;
    const CHANNELS: u16 = 1;
    // Seuil d'énergie (RMS) pour détecter la voix (à calibrer selon le micro)
    const VAD_THRESHOLD: f32 = 0.015;
    // Temps de silence avant de couper la phrase (1.5 seconde)
    const SILENCE_FRAMES_MAX: usize = (SAMPLE_RATE as f32 * 1.5) as usize;

    pub struct AudioListener {
        // On conserve le stream pour qu'il ne soit pas "drop" (ce qui arrêterait l'écoute)
        _stream: cpal::Stream,
    }

    impl AudioListener {
        /// Lance l'écoute du microphone en tâche de fond.
        pub fn start() -> RaiseResult<(Self, mpsc::Receiver<Vec<f32>>)> {
            let host = cpal::default_host();

            // 1. Sélection du périphérique audio
            let device = host.default_input_device().ok_or_else(|| {
                build_error!(
                    "ERR_AUDIO_NO_DEVICE",
                    error = "Aucun microphone par défaut trouvé sur le système."
                )
            })?;

            // 2. Configuration stricte pour Whisper : 16kHz, Mono, f32
            let config = cpal::StreamConfig {
                channels: CHANNELS,
                sample_rate: SAMPLE_RATE,
                buffer_size: cpal::BufferSize::Default,
            };

            // 3. Initialisation du Ring Buffer (Lock-free)
            let rb = HeapRb::<f32>::new((SAMPLE_RATE * 5) as usize);
            let (mut prod, mut cons) = rb.split();

            // 4. Channel asynchrone pour envoyer les phrases capturées vers l'IA
            let (tx, rx) = mpsc::channel::<Vec<f32>>(10);

            // 5. Lancement du flux audio CPAL
            let stream = match device.build_input_stream(
                &config,
                move |data: &[f32], _: &cpal::InputCallbackInfo| {
                    let _ = prod.push_slice(data);
                },
                move |err| {
                    kernel_fatal!(
                        "Flux d'Entrée Audio (Callback OS)",
                        "cpal::InputStream",
                        err
                    );
                },
                None,
            ) {
                Ok(s) => s,
                Err(e) => raise_error!(
                    "ERR_AUDIO_STREAM_BUILD",
                    error = e,
                    context = json_value!({"sample_rate": SAMPLE_RATE, "channels": CHANNELS})
                ),
            };

            if let Err(e) = stream.play() {
                raise_error!("ERR_AUDIO_STREAM_PLAY", error = e);
            }

            user_info!(
                "🎤 [Audio] Microphone activé, écoute en cours...",
                json_value!({})
            );

            // 6. Thread de Détection d'Activité Vocale (VAD)
            tokio::spawn(async move {
                let mut current_phrase = Vec::new();
                let mut silence_counter = 0;
                let mut is_speaking = false;

                loop {
                    let mut temp_buf = Vec::new();
                    while let Some(sample) = cons.try_pop() {
                        temp_buf.push(sample);
                    }

                    if !temp_buf.is_empty() {
                        let sum_squares: f32 = temp_buf.iter().map(|&x| x * x).sum();
                        let rms = (sum_squares / temp_buf.len() as f32).sqrt();

                        if rms > VAD_THRESHOLD {
                            is_speaking = true;
                            silence_counter = 0;
                            current_phrase.extend_from_slice(&temp_buf);
                        } else if is_speaking {
                            silence_counter += temp_buf.len();
                            current_phrase.extend_from_slice(&temp_buf);

                            if silence_counter >= SILENCE_FRAMES_MAX {
                                if tx.send(current_phrase.clone()).await.is_err() {
                                    kernel_fatal!(
                                        "Pipeline Audio-to-AI (MPSC Stream)",
                                        "audio::voice_processor",
                                        "Le moteur IA ne reçoit plus les données vocales."
                                    );
                                    break;
                                }
                                current_phrase.clear();
                                is_speaking = false;
                                silence_counter = 0;
                                println!("🗣️ [Audio] Phrase capturée et envoyée à l'IA !");
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            });

            Ok((Self { _stream: stream }, rx))
        }
    }
}

// ==============================================================================
// VERSION MOCK : Compilée sur le noeud Edge (Pi 5) quand l'audio est désactivé
// ==============================================================================
#[cfg(not(feature = "audio"))]
pub use mock_impl::*;

#[cfg(not(feature = "audio"))]
mod mock_impl {
    use super::*;

    pub struct AudioListener {}

    impl AudioListener {
        /// Version fantôme de l'écouteur. Ne fait rien et renvoie un canal vide.
        pub fn start() -> RaiseResult<(Self, mpsc::Receiver<Vec<f32>>)> {
            // On crée un émetteur qu'on jette silencieusement, et un récepteur vide
            let (_tx, rx) = mpsc::channel::<Vec<f32>>(1);

            user_info!(
                "🔇 [Audio] Module inactif (Feature 'audio' non compilée sur ce noeud).",
                json_value!({})
            );

            // Le flux d'exécution principal continuera sans crasher,
            // mais le rx ne recevra simplement jamais de phrases.
            Ok((Self {}, rx))
        }
    }
}
