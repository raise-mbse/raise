// FICHIER : crates/raise-core/src/utils/data/encoding.rs

use crate::utils::core::error::RaiseResult;
use crate::utils::data::json::json_value;

/// Décode une chaîne Base64 standard en un vecteur d'octets de manière 100% native (Zéro Dépendance).
pub fn decode_base64(input: &str) -> RaiseResult<Vec<u8>> {
    let mut buffer = Vec::new();
    let mut accumulated = 0u32;
    let mut bit_count = 0;

    for c in input.chars() {
        if c == '=' {
            break; // Fin du padding
        }
        if c.is_whitespace() {
            continue; // Ignorer les sauts de ligne ou espaces
        }

        let val = match c {
            'A'..='Z' => c as u32 - 'A' as u32,
            'a'..='z' => c as u32 - 'a' as u32 + 26,
            '0'..='9' => c as u32 - '0' as u32 + 52,
            '+' => 62,
            '/' => 63,
            _ => {
                return Err(crate::build_error!(
                    "ERR_ENCODING_BASE64",
                    error = "Caractère invalide détecté lors du décodage",
                    context = json_value!({"char": c})
                ))
            }
        };

        accumulated = (accumulated << 6) | val;
        bit_count += 6;

        if bit_count >= 8 {
            bit_count -= 8;
            buffer.push((accumulated >> bit_count) as u8);
        }
    }
    Ok(buffer)
}

/// Encode des octets en chaîne Base64 standard (RFC 4648).
/// Algorithme natif sans dépendance externe.
pub fn encode_base64(input: &[u8]) -> String {
    let charset = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::new();
    let mut i = 0;

    while i < input.len() {
        let b1 = input[i];
        let b2 = if i + 1 < input.len() { input[i + 1] } else { 0 };
        let b3 = if i + 2 < input.len() { input[i + 2] } else { 0 };

        result.push(charset[(b1 >> 2) as usize] as char);
        result.push(charset[(((b1 & 0x03) << 4) | (b2 >> 4)) as usize] as char);

        if i + 1 < input.len() {
            result.push(charset[(((b2 & 0x0f) << 2) | (b3 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < input.len() {
            result.push(charset[(b3 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }
    result
}
