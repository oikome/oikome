// Copyright 2025 The Oikome Authors
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Recovery keys (architecture v3 §13.4): a node's identity seed written so
//! a person can copy it down and type it back.
//!
//! The key is 35 bytes, a version (`1`), the 32-byte seed and the first two
//! bytes of the seed's BLAKE3 hash, in Crockford Base32: 56 characters in 14
//! groups of 4 (`7KQ2-M9XD-…`). Reading it ignores case, spaces and dashes,
//! takes `I` and `L` for `1` and `O` for `0`, and rejects a key whose check
//! bytes do not match, so a typo is caught instead of restoring another
//! identity.

use zeroize::Zeroizing;

use crate::error::IdentityError;

const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const VERSION: u8 = 1;
/// Version, seed and two check bytes.
const PAYLOAD_BYTES: usize = 35;
/// Characters in a key, without dashes (35 bytes × 8 = 56 × 5 bits).
pub const RECOVERY_KEY_CHARS: usize = 56;
/// Characters per group.
const GROUP: usize = 4;

fn check_bytes(seed: &[u8; 32]) -> [u8; 2] {
    let hash = blake3::hash(seed);
    [hash.as_bytes()[0], hash.as_bytes()[1]]
}

fn invalid(msg: &str) -> IdentityError {
    IdentityError::InvalidKey(format!("recovery key: {msg}"))
}

/// The recovery key of `seed`, grouped as `XXXX-XXXX-…`.
pub fn encode_recovery_key(seed: &[u8; 32]) -> Zeroizing<String> {
    let mut payload = Zeroizing::new([0u8; PAYLOAD_BYTES]);
    payload[0] = VERSION;
    payload[1..33].copy_from_slice(seed);
    payload[33..].copy_from_slice(&check_bytes(seed));
    let mut out = Zeroizing::new(String::with_capacity(
        RECOVERY_KEY_CHARS + RECOVERY_KEY_CHARS / GROUP,
    ));
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut written = 0;
    for &byte in payload.iter() {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            if written > 0 && written % GROUP == 0 {
                out.push('-');
            }
            out.push(char::from(ALPHABET[((acc >> bits) & 31) as usize]));
            written += 1;
            acc &= (1 << bits) - 1;
        }
    }
    out
}

/// The seed in a recovery key typed by a person.
pub fn decode_recovery_key(text: &str) -> Result<Zeroizing<[u8; 32]>, IdentityError> {
    let mut payload = Zeroizing::new([0u8; PAYLOAD_BYTES]);
    let mut acc: u32 = 0;
    let mut bits = 0;
    let mut symbols = 0;
    let mut bytes = 0;
    for c in text.chars() {
        let value = match c.to_ascii_uppercase() {
            '-' | ' ' | '\t' | '\r' | '\n' => continue,
            'O' => 0,
            'I' | 'L' => 1,
            c => ALPHABET
                .iter()
                .position(|&a| char::from(a) == c)
                .ok_or_else(|| invalid("it contains a character that is not part of a key"))?
                as u32,
        };
        symbols += 1;
        if symbols > RECOVERY_KEY_CHARS {
            return Err(invalid("it is longer than 56 characters"));
        }
        acc = (acc << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            payload[bytes] = ((acc >> bits) & 0xff) as u8;
            bytes += 1;
            acc &= (1 << bits) - 1;
        }
    }
    if symbols != RECOVERY_KEY_CHARS {
        return Err(invalid("it has 56 characters (14 groups of 4)"));
    }
    if payload[0] != VERSION {
        return Err(invalid("this version of Oikome cannot read it"));
    }
    let mut seed = Zeroizing::new([0u8; 32]);
    seed.copy_from_slice(&payload[1..33]);
    if payload[33..] != check_bytes(&seed) {
        return Err(invalid("it has a typo; check each group"));
    }
    Ok(seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7; 32];

    #[test]
    fn round_trips_in_fourteen_groups() {
        let key = encode_recovery_key(&SEED);
        let groups: Vec<&str> = key.split('-').collect();
        assert_eq!(groups.len(), 14);
        assert!(groups.iter().all(|g| g.len() == 4));
        assert_eq!(*decode_recovery_key(&key).unwrap(), SEED);
        // Every seed byte value survives.
        let seed: [u8; 32] = std::array::from_fn(|i| (i * 8) as u8);
        assert_eq!(
            *decode_recovery_key(&encode_recovery_key(&seed)).unwrap(),
            seed
        );
    }

    #[test]
    fn reading_forgives_case_spacing_and_lookalikes() {
        let key = encode_recovery_key(&SEED).replace('-', " ").to_lowercase();
        assert_eq!(*decode_recovery_key(&key).unwrap(), SEED);
        let lookalikes = encode_recovery_key(&SEED)
            .replace('1', "l")
            .replace('0', "o");
        assert_eq!(*decode_recovery_key(&lookalikes).unwrap(), SEED);
    }

    #[test]
    fn typos_and_wrong_lengths_are_refused() {
        let key = encode_recovery_key(&SEED).to_string();
        // Change one character.
        let mut typo: Vec<char> = key.chars().collect();
        typo[6] = if typo[6] == 'A' { 'B' } else { 'A' };
        let typo: String = typo.into_iter().collect();
        assert!(decode_recovery_key(&typo).is_err());
        assert!(decode_recovery_key(&key[..key.len() - 1]).is_err());
        assert!(decode_recovery_key(&format!("{key}A")).is_err());
        assert!(decode_recovery_key(&key.replace(&key[..1], "U")).is_err());
        assert!(decode_recovery_key("").is_err());
    }
}
