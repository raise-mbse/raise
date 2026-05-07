import { invoke } from '@tauri-apps/api/core';
import { listen, Event, UnlistenFn } from '@tauri-apps/api/event';

/**
 * Appelle une commande Rust avec un typage strict de la réponse.
 */
export async function invokeCmd<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(cmd, args);
  } catch (error) {
    console.error(`[Rust Error] ${cmd}:`, error);
    throw error;
  }
}

/**
 * Écoute un événement émis par le backend Rust (via app.emit_all).
 */
export async function listenEvent<T>(
  eventName: string,
  callback: (payload: T) => void
): Promise<UnlistenFn> {
  return await listen<T>(eventName, (event: Event<T>) => {
    console.log(`[Tauri Event] ${eventName}:`, event.payload);
    callback(event.payload);
  });
}