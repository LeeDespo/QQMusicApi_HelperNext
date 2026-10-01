//! QRC: QQ Music's word-level lyrics, and the cipher they arrive in.
//!
//! The word-level track is only served on the encrypted route
//! (`music.musichallSong.PlayLyricInfo/GetPlayLyricInfo` with `qrc=1`), where the
//! `lyric` field stops being base64 LRC and becomes **hex-encoded ciphertext** of
//! the QRC document. The plaintext route (the old fcgi one) answers only whole
//! lines, which is why the component used to have no word timing at all.
//!
//! Three layers, kept apart on purpose:
//!
//! * [`decrypt_hex`] — QQ's *private* DES (its own S/P/E boxes and key-bit order,
//!   not standard DES), applied three times as D(K3) → E(K2) → D(K1), then zlib
//!   with a UTF-8 BOM. Ported from the MIT-licensed
//!   [qrc-decoder](https://github.com/apoint123/qrc-decoder) specification; the
//!   port is pinned by a known-answer test taken from AMLL's own vector.
//! * [`extract_payload`] — the decrypted text is an XML shell whose real content
//!   is one attribute (or a CDATA block), XML-escaped.
//! * [`parse`] — `[lineStart,lineDuration]word(start,duration)…`, the QRC text
//!   format, whose words are what the word-level LRC below is built from.
//!
//! [`to_word_lrc`] is the last step and the only one that is our own choice: the
//! host renders TTML, and it builds that from an LRC whose *lines* carry several
//! timestamps — one per word. So the deliverable is an ordinary LRC with a tag
//! before every word, which every LRC reader (including ours) already accepts.

use miniz_oxide::inflate::decompress_to_vec_zlib;

/// The three DES keys, back to back. QQ's own constant.
const KEY: &[u8; 24] = b"!@#)(*$%123ZXC!@!@#)(NHL";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrcError {
    /// Not hex, or not a whole number of cipher blocks.
    NotCiphertext(String),
    /// The ciphertext decrypted to something that is not a zlib stream.
    NotCompressed(String),
    /// Decrypted and inflated, but there is no lyric content in it.
    NoContent,
}

impl std::fmt::Display for QrcError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QrcError::NotCiphertext(reason) => write!(formatter, "不是 QRC 密文：{reason}"),
            QrcError::NotCompressed(reason) => write!(formatter, "QRC 解密后不是 zlib：{reason}"),
            QrcError::NoContent => write!(formatter, "QRC 文档里没有歌词内容"),
        }
    }
}

/// One word, with the times the service gave it (milliseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrcWord {
    pub text: String,
    pub start_ms: i64,
    pub duration_ms: i64,
}

/// One lyric line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QrcLine {
    pub start_ms: i64,
    pub duration_ms: i64,
    pub words: Vec<QrcWord>,
}

// MARK: - The cipher

// QQ's DES is not standard DES: the boxes below are its own, and the key-bit
// order has a quirk. Everything here mirrors the reference implementation
// (qrc-decoder, MIT) rather than a textbook, so the tables are data, not derivable
// from first principles.
const KEY_COMPRESSION: [usize; 48] = [
    13, 16, 10, 23, 0, 4, 2, 27, 14, 5, 20, 9, 22, 18, 11, 3, 25, 7, 15, 6, 26, 19, 12, 1, 40, 51,
    30, 36, 46, 54, 29, 39, 50, 44, 32, 47, 43, 48, 38, 55, 33, 52, 45, 41, 49, 35, 28, 31,
];
const KEY_PERM_C: [usize; 28] = [
    56, 48, 40, 32, 24, 16, 8, 0, 57, 49, 41, 33, 25, 17, 9, 1, 58, 50, 42, 34, 26, 18, 10, 2, 59,
    51, 43, 35,
];
const KEY_PERM_D: [usize; 28] = [
    62, 54, 46, 38, 30, 22, 14, 6, 61, 53, 45, 37, 29, 21, 13, 5, 60, 52, 44, 36, 28, 20, 12, 4,
    27, 19, 11, 3,
];
const KEY_RND_SHIFT: [usize; 16] = [1, 1, 2, 2, 2, 2, 2, 2, 1, 2, 2, 2, 2, 2, 2, 1];
const IP_RULE: [usize; 64] = [
    34, 42, 50, 58, 2, 10, 18, 26, 36, 44, 52, 60, 4, 12, 20, 28, 38, 46, 54, 62, 6, 14, 22, 30,
    40, 48, 56, 64, 8, 16, 24, 32, 33, 41, 49, 57, 1, 9, 17, 25, 35, 43, 51, 59, 3, 11, 19, 27,
    37, 45, 53, 61, 5, 13, 21, 29, 39, 47, 55, 63, 7, 15, 23, 31,
];
const INV_IP_RULE: [usize; 64] = [
    37, 5, 45, 13, 53, 21, 61, 29, 38, 6, 46, 14, 54, 22, 62, 30, 39, 7, 47, 15, 55, 23, 63, 31,
    40, 8, 48, 16, 56, 24, 64, 32, 33, 1, 41, 9, 49, 17, 57, 25, 34, 2, 42, 10, 50, 18, 58, 26,
    35, 3, 43, 11, 51, 19, 59, 27, 36, 4, 44, 12, 52, 20, 60, 28,
];
const P_BOX: [usize; 32] = [
    16, 7, 20, 21, 29, 12, 28, 17, 1, 15, 23, 26, 5, 18, 31, 10, 2, 8, 24, 14, 32, 27, 3, 9, 19,
    13, 30, 6, 22, 11, 4, 25,
];
const E_BOX: [usize; 48] = [
    32, 1, 2, 3, 4, 5, 4, 5, 6, 7, 8, 9, 8, 9, 10, 11, 12, 13, 12, 13, 14, 15, 16, 17, 16, 17, 18,
    19, 20, 21, 20, 21, 22, 23, 24, 25, 24, 25, 26, 27, 28, 29, 28, 29, 30, 31, 32, 1,
];
const S_BOXES: [[u8; 64]; 8] = [
    [
        14, 4, 13, 1, 2, 15, 11, 8, 3, 10, 6, 12, 5, 9, 0, 7, 0, 15, 7, 4, 14, 2, 13, 1, 10, 6, 12,
        11, 9, 5, 3, 8, 4, 1, 14, 8, 13, 6, 2, 11, 15, 12, 9, 7, 3, 10, 5, 0, 15, 12, 8, 2, 4, 9,
        1, 7, 5, 11, 3, 14, 10, 0, 6, 13,
    ],
    [
        15, 1, 8, 14, 6, 11, 3, 4, 9, 7, 2, 13, 12, 0, 5, 10, 3, 13, 4, 7, 15, 2, 8, 15, 12, 0, 1,
        10, 6, 9, 11, 5, 0, 14, 7, 11, 10, 4, 13, 1, 5, 8, 12, 6, 9, 3, 2, 15, 13, 8, 10, 1, 3, 15,
        4, 2, 11, 6, 7, 12, 0, 5, 14, 9,
    ],
    [
        10, 0, 9, 14, 6, 3, 15, 5, 1, 13, 12, 7, 11, 4, 2, 8, 13, 7, 0, 9, 3, 4, 6, 10, 2, 8, 5,
        14, 12, 11, 15, 1, 13, 6, 4, 9, 8, 15, 3, 0, 11, 1, 2, 12, 5, 10, 14, 7, 1, 10, 13, 0, 6,
        9, 8, 7, 4, 15, 14, 3, 11, 5, 2, 12,
    ],
    [
        7, 13, 14, 3, 0, 6, 9, 10, 1, 2, 8, 5, 11, 12, 4, 15, 13, 8, 11, 5, 6, 15, 0, 3, 4, 7, 2,
        12, 1, 10, 14, 9, 10, 6, 9, 0, 12, 11, 7, 13, 15, 1, 3, 14, 5, 2, 8, 4, 3, 15, 0, 6, 10, 10,
        13, 8, 9, 4, 5, 11, 12, 7, 2, 14,
    ],
    [
        2, 12, 4, 1, 7, 10, 11, 6, 8, 5, 3, 15, 13, 0, 14, 9, 14, 11, 2, 12, 4, 7, 13, 1, 5, 0, 15,
        10, 3, 9, 8, 6, 4, 2, 1, 11, 10, 13, 7, 8, 15, 9, 12, 5, 6, 3, 0, 14, 11, 8, 12, 7, 1, 14,
        2, 13, 6, 15, 0, 9, 10, 4, 5, 3,
    ],
    [
        12, 1, 10, 15, 9, 2, 6, 8, 0, 13, 3, 4, 14, 7, 5, 11, 10, 15, 4, 2, 7, 12, 9, 5, 6, 1, 13,
        14, 0, 11, 3, 8, 9, 14, 15, 5, 2, 8, 12, 3, 7, 0, 4, 10, 1, 13, 11, 6, 4, 3, 2, 12, 9, 5,
        15, 10, 11, 14, 1, 7, 6, 0, 8, 13,
    ],
    [
        4, 11, 2, 14, 15, 0, 8, 13, 3, 12, 9, 7, 5, 10, 6, 1, 13, 0, 11, 7, 4, 9, 1, 10, 14, 3, 5,
        12, 2, 15, 8, 6, 1, 4, 11, 13, 12, 3, 7, 14, 10, 15, 6, 8, 0, 5, 9, 2, 6, 11, 13, 8, 1, 4,
        10, 7, 9, 5, 0, 15, 14, 2, 3, 12,
    ],
    [
        13, 2, 8, 4, 6, 15, 11, 1, 10, 9, 3, 14, 5, 0, 12, 7, 1, 15, 13, 8, 10, 3, 7, 4, 12, 5, 6,
        11, 0, 14, 9, 2, 7, 11, 4, 1, 9, 12, 14, 2, 0, 6, 10, 13, 15, 3, 5, 8, 2, 1, 14, 7, 4, 10,
        8, 13, 15, 12, 9, 0, 3, 5, 6, 11,
    ],
];

/// The tables every block cipher operation reads, built once.
struct Tables {
    ip_left: [u32; 2048],
    ip_right: [u32; 2048],
    inv_left: [u32; 2048],
    inv_right: [u32; 2048],
    sp: [u32; 512],
    eb_high: [u32; 1024],
    eb_low: [u32; 1024],
}

impl Tables {
    fn build() -> Self {
        let mut tables = Tables {
            ip_left: [0; 2048],
            ip_right: [0; 2048],
            inv_left: [0; 2048],
            inv_right: [0; 2048],
            sp: [0; 512],
            eb_high: [0; 1024],
            eb_low: [0; 1024],
        };
        for byte_position in 0..8usize {
            for byte in 0..256usize {
                let input = (byte as u64) << (56 - byte_position * 8);
                let permuted = apply_perm64(input, &IP_RULE);
                let index = (byte_position << 8) | byte;
                tables.ip_left[index] = (permuted >> 32) as u32;
                tables.ip_right[index] = permuted as u32;
                let inverse = apply_perm64(input, &INV_IP_RULE);
                tables.inv_left[index] = (inverse >> 32) as u32;
                tables.inv_right[index] = inverse as u32;
            }
        }
        for sbox in 0..8usize {
            for input in 0..64usize {
                let value = S_BOXES[sbox][sbox_index(input)] as u32;
                tables.sp[(sbox << 6) | input] = qq_pbox(value << (28 - sbox * 4));
            }
        }
        for chunk in 0..4usize {
            let shift = (3 - chunk) * 8;
            for byte in 0..256usize {
                let input = (byte as u64) << shift;
                let mut high = 0u32;
                let mut low = 0u32;
                for i in 0..24usize {
                    if (input >> (32 - E_BOX[i])) & 1 == 1 {
                        high |= 1 << (23 - i);
                    }
                }
                for i in 24..48usize {
                    if (input >> (32 - E_BOX[i])) & 1 == 1 {
                        low |= 1 << (47 - i);
                    }
                }
                tables.eb_high[(chunk << 8) | byte] = high;
                tables.eb_low[(chunk << 8) | byte] = low;
            }
        }
        tables
    }
}

fn tables() -> &'static Tables {
    static TABLES: std::sync::OnceLock<Tables> = std::sync::OnceLock::new();
    TABLES.get_or_init(Tables::build)
}

/// A fixed permutation of positions in a key's bytes.
fn permute_from_key_bytes(key: &[u8], table: &[usize]) -> u64 {
    let mut out = 0u64;
    let mut mask = 1u64 << (table.len() - 1);
    for position in table {
        let word = position >> 5;
        let bit_in_word = position & 31;
        let byte_in_word = bit_in_word >> 3;
        let bit_in_byte = bit_in_word & 7;
        let byte_index = word * 4 + 3 - byte_in_word;
        if (key[byte_index] >> (7 - bit_in_byte)) & 1 == 1 {
            out |= mask;
        }
        mask >>= 1;
    }
    out
}

fn rotate_left_28(value: u64, count: usize) -> u64 {
    const MASK: u64 = 0xFFFF_FFF0;
    let value = value & MASK;
    ((value << count) | (value >> (28 - count))) & MASK
}

/// Subkeys in the order the Feistel rounds use them.
///
/// `mode == 1` reverses them, which is how this DES spells decryption — the round
/// function itself is never inverted.
fn key_schedule(key: &[u8], mode: i32) -> [u32; 32] {
    let mut schedule = [0u32; 32];
    let mut c = permute_from_key_bytes(key, &KEY_PERM_C) << 4;
    let mut d = permute_from_key_bytes(key, &KEY_PERM_D) << 4;
    for round in 0..16usize {
        let shift = KEY_RND_SHIFT[round];
        c = rotate_left_28(c, shift);
        d = rotate_left_28(d, shift);
        let target = if mode == 1 { 15 - round } else { round };
        let mut sub = 0u64;
        for (index, position) in KEY_COMPRESSION.iter().enumerate() {
            let bit = if *position < 28 {
                (c >> (31 - position)) & 1
            } else {
                (d >> (31 - (position - 27))) & 1
            };
            if bit == 1 {
                sub |= 1 << (47 - index);
            }
        }
        let b5 = ((sub >> 40) & 0xFF) as u32;
        let b4 = ((sub >> 32) & 0xFF) as u32;
        let b3 = ((sub >> 24) & 0xFF) as u32;
        let b2 = ((sub >> 16) & 0xFF) as u32;
        let b1 = ((sub >> 8) & 0xFF) as u32;
        let b0 = (sub & 0xFF) as u32;
        schedule[target * 2] = (b5 << 16) | (b4 << 8) | b3;
        schedule[target * 2 + 1] = (b2 << 16) | (b1 << 8) | b0;
    }
    schedule
}

fn apply_perm64(input: u64, rule: &[usize]) -> u64 {
    let mut out = 0u64;
    for i in 0..64usize {
        if (input >> (64 - rule[i])) & 1 == 1 {
            out |= 1 << (63 - i);
        }
    }
    out
}

fn sbox_index(value: usize) -> usize {
    (value & 0x20) | ((value & 0x1F) >> 1) | ((value & 0x01) << 4)
}

fn qq_pbox(value: u32) -> u32 {
    let mut out = 0u32;
    for i in 0..32usize {
        let source = P_BOX[i];
        if value & (1 << (32 - source)) != 0 {
            out |= 1 << (31 - i);
        }
    }
    out
}

fn round_function(state: u32, key_high: u32, key_low: u32) -> u32 {
    let tables = tables();
    let b0 = ((state >> 24) & 0xFF) as usize;
    let b1 = ((state >> 16) & 0xFF) as usize;
    let b2 = ((state >> 8) & 0xFF) as usize;
    let b3 = (state & 0xFF) as usize;
    let expanded_high =
        tables.eb_high[b0] | tables.eb_high[256 | b1] | tables.eb_high[512 | b2] | tables.eb_high[768 | b3];
    let expanded_low =
        tables.eb_low[b0] | tables.eb_low[256 | b1] | tables.eb_low[512 | b2] | tables.eb_low[768 | b3];
    let xh = expanded_high ^ key_high;
    let xl = expanded_low ^ key_low;
    tables.sp[((xh >> 18) & 0x3F) as usize]
        | tables.sp[64 | ((xh >> 12) & 0x3F) as usize]
        | tables.sp[128 | ((xh >> 6) & 0x3F) as usize]
        | tables.sp[192 | (xh & 0x3F) as usize]
        | tables.sp[256 | ((xl >> 18) & 0x3F) as usize]
        | tables.sp[320 | ((xl >> 12) & 0x3F) as usize]
        | tables.sp[384 | ((xl >> 6) & 0x3F) as usize]
        | tables.sp[448 | (xl & 0x3F) as usize]
}

fn des_block(input: &[u8], out: &mut [u8], schedule: &[u32; 32]) {
    let tables = tables();
    let mut left = 0u32;
    let mut right = 0u32;
    for i in 0..8usize {
        let index = (i << 8) | input[i] as usize;
        left |= tables.ip_left[index];
        right |= tables.ip_right[index];
    }
    for round in 0..15usize {
        let swapped = right;
        right = left ^ round_function(right, schedule[round * 2], schedule[round * 2 + 1]);
        left = swapped;
    }
    let l = left ^ round_function(right, schedule[30], schedule[31]);
    let r = right;
    let mut out_left = 0u32;
    let mut out_right = 0u32;
    for b in 0..4usize {
        let index_left = (b << 8) | ((l >> (24 - b * 8)) & 0xFF) as usize;
        out_left |= tables.inv_left[index_left];
        out_right |= tables.inv_right[index_left];
        let index_right = ((b + 4) << 8) | ((r >> (24 - b * 8)) & 0xFF) as usize;
        out_left |= tables.inv_left[index_right];
        out_right |= tables.inv_right[index_right];
    }
    out[0] = (out_left >> 24) as u8;
    out[1] = (out_left >> 16) as u8;
    out[2] = (out_left >> 8) as u8;
    out[3] = out_left as u8;
    out[4] = (out_right >> 24) as u8;
    out[5] = (out_right >> 16) as u8;
    out[6] = (out_right >> 8) as u8;
    out[7] = out_right as u8;
}

/// The three schedules, in the order the composition applies them.
fn schedules() -> &'static [[u32; 32]; 3] {
    static SCHEDULES: std::sync::OnceLock<[[u32; 32]; 3]> = std::sync::OnceLock::new();
    SCHEDULES.get_or_init(|| {
        [
            key_schedule(&KEY[16..24], 1), // D(K3)
            key_schedule(&KEY[8..16], 0),  // E(K2)
            key_schedule(&KEY[0..8], 1),   // D(K1)
        ]
    })
}

/// Decrypt the hex form of a QRC payload and inflate it.
///
/// The host never sees the cipher: this returns the QRC document's text.
pub fn decrypt_hex(hex: &str) -> Result<String, QrcError> {
    let hex = hex.trim();
    if hex.len() % 2 != 0 {
        return Err(QrcError::NotCiphertext("hex 长度为奇数".into()));
    }
    let mut encrypted = Vec::with_capacity(hex.len() / 2);
    let bytes = hex.as_bytes();
    for pair in bytes.chunks(2) {
        let text = std::str::from_utf8(pair).map_err(|_| QrcError::NotCiphertext("含非 ASCII".into()))?;
        let byte = u8::from_str_radix(text, 16)
            .map_err(|_| QrcError::NotCiphertext(format!("不是十六进制：{text}")))?;
        encrypted.push(byte);
    }
    if encrypted.is_empty() || encrypted.len() % 8 != 0 {
        return Err(QrcError::NotCiphertext(format!("长度 {} 不是 8 的倍数", encrypted.len())));
    }

    let schedules = schedules();
    let mut plain = vec![0u8; encrypted.len()];
    for offset in (0..encrypted.len()).step_by(8) {
        let mut block = [0u8; 8];
        block.copy_from_slice(&encrypted[offset..offset + 8]);
        for schedule in schedules.iter() {
            des_block(&block, &mut plain[offset..offset + 8], schedule);
            block.copy_from_slice(&plain[offset..offset + 8]);
        }
    }

    let inflated = decompress_to_vec_zlib(&plain)
        .map_err(|error| QrcError::NotCompressed(format!("{error:?}")))?;
    // The stream carries a UTF-8 BOM.
    let inflated = match inflated.as_slice() {
        [0xEF, 0xBB, 0xBF, rest @ ..] => rest.to_vec(),
        _ => inflated,
    };
    String::from_utf8(inflated).map_err(|error| QrcError::NotCompressed(error.to_string()))
}

/// The QRC document inside the XML shell.
///
/// The real content is one XML-escaped attribute (`LyricContent="…"`), or a CDATA
/// block in older payloads; both spell newlines as entities.
pub fn extract_payload(xml: &str) -> Option<String> {
    if let Some(start) = xml.find("<![CDATA[") {
        if let Some(end) = xml[start..].find("]]>") {
            let content = &xml[start + "<![CDATA[".len()..start + end];
            return Some(content.trim().to_string());
        }
    }
    let marker = "LyricContent=\"";
    let start = xml.find(marker)? + marker.len();
    let end = xml[start..].find('"')? + start;
    Some(decode_entities(&xml[start..end]).trim().to_string())
}

fn decode_entities(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#10;", "\n")
        .replace("&#13;", "\r")
        .replace("&amp;", "&")
}

/// Parse the QRC text: `[lineStart,lineDuration]word(start,duration)…`.
pub fn parse(payload: &str) -> Vec<QrcLine> {
    let mut lines = Vec::new();
    for raw in payload.split(['\n', '\r']) {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let Some(header_end) = line.find(']') else { continue };
        if !line.starts_with('[') {
            continue;
        }
        let header = &line[1..header_end];
        let mut header_parts = header.split(',');
        let Some(start) = header_parts.next().and_then(|v| v.trim().parse::<i64>().ok()) else {
            continue;
        };
        let duration = header_parts.next().and_then(|v| v.trim().parse::<i64>().ok()).unwrap_or(0);

        let content = line[header_end + 1..].trim();
        let mut words = Vec::new();
        let mut rest = content;
        while let Some(open) = rest.find('(') {
            let text = rest[..open].to_string();
            let Some(close) = rest[open..].find(')') else { break };
            let inside = &rest[open + 1..open + close];
            let mut parts = inside.split(',');
            let word_start = parts.next().and_then(|v| v.trim().parse::<i64>().ok());
            let word_duration = parts.next().and_then(|v| v.trim().parse::<i64>().ok());
            if let (Some(word_start), Some(word_duration)) = (word_start, word_duration) {
                if !text.is_empty() {
                    words.push(QrcWord {
                        text,
                        start_ms: word_start,
                        duration_ms: word_duration,
                    });
                }
            }
            rest = &rest[open + close + 1..];
        }
        if words.is_empty() {
            continue;
        }
        lines.push(QrcLine {
            start_ms: start,
            duration_ms: duration,
            words,
        });
    }
    lines
}

/// Format one timestamp the way LRC spells it.
fn lrc_timestamp(milliseconds: i64) -> String {
    let centiseconds = milliseconds.max(0) / 10;
    format!(
        "{:02}:{:02}.{:02}",
        centiseconds / 6000,
        (centiseconds / 100) % 60,
        centiseconds % 100
    )
}

/// Word-level LRC: one `[mm:ss.cc]` tag per word.
///
/// This is the shape the host already understands — its LRC reader treats several
/// timestamps on one line as several words and turns them into word-level TTML.
/// A word's text may not contain `[` or `]`, because that is what ends a tag.
pub fn to_word_lrc(lines: &[QrcLine]) -> String {
    let mut out = String::new();
    for line in lines {
        for word in &line.words {
            // The word's own spacing is kept: for English lyrics the space belongs
            // to the word, and the host puts it back exactly as it found it.
            let text = word.text.replace(['[', ']'], "");
            if text.trim().is_empty() {
                continue;
            }
            out.push('[');
            out.push_str(&lrc_timestamp(word.start_ms));
            out.push(']');
            out.push_str(&text);
        }
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Fetch → decrypt → parse → word-level LRC, in one call.
pub fn word_level_lrc(hex: &str) -> Result<String, QrcError> {
    let document = decrypt_hex(hex)?;
    let payload = extract_payload(&document).ok_or(QrcError::NoContent)?;
    let lines = parse(&payload);
    if lines.is_empty() {
        return Err(QrcError::NoContent);
    }
    Ok(to_word_lrc(&lines))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first cipher block of AMLL's own QRC vector
    /// (`packages/lyric/test/eqrc.hex` in the AMLL submodule), and the bytes this
    /// DES must produce for it. A known-answer test rather than a round trip,
    /// because a self-consistent implementation of the *wrong* cipher would pass a
    /// round trip happily.
    #[test]
    fn the_first_block_matches_the_reference_vector() {
        let block: Vec<u8> = (0..16)
            .step_by(2)
            .map(|i| u8::from_str_radix(&"76782DD2D2D02305BAD47017F2618CC5"[i..i + 2], 16).unwrap())
            .collect();
        let mut out = [0u8; 8];
        let schedules = schedules();
        let mut block_array = [0u8; 8];
        block_array.copy_from_slice(&block);
        for schedule in schedules.iter() {
            des_block(&block_array, &mut out, schedule);
            block_array.copy_from_slice(&out);
        }
        // What the reference implementation produces for this block: the head of a
        // zlib stream (`78 9c …`).
        assert_eq!(out[0], 0x78, "first byte must be a zlib header");
        assert_eq!(out[1], 0x9c, "and the second the zlib flag byte");
    }

    #[test]
    fn a_hex_oddity_is_refused_rather_than_guessed() {
        assert!(matches!(decrypt_hex("abc"), Err(QrcError::NotCiphertext(_))));
        assert!(matches!(decrypt_hex("zz"), Err(QrcError::NotCiphertext(_))));
        assert!(matches!(decrypt_hex(""), Err(QrcError::NotCiphertext(_))));
        // Right shape, but not a zlib stream once decrypted.
        assert!(matches!(decrypt_hex("0011223344556677"), Err(QrcError::NotCompressed(_))));
    }

    #[test]
    fn the_payload_is_found_in_either_shell() {
        let attribute = r#"<QrcInfos><LyricInfo><Lyric_1 LyricContent="[0,10]a(0,5)b(5,5)&#10;[10,10]c(10,10)"/></LyricInfo></QrcInfos>"#;
        assert_eq!(
            extract_payload(attribute).unwrap(),
            "[0,10]a(0,5)b(5,5)\n[10,10]c(10,10)"
        );
        let cdata = "<QrcInfos><![CDATA[[0,10]a(0,10)]]></QrcInfos>";
        assert_eq!(extract_payload(cdata).unwrap(), "[0,10]a(0,10)");
        assert!(extract_payload("<QrcInfos/>").is_none());
    }

    #[test]
    fn qrc_lines_come_back_with_their_words() {
        let payload = "[190871,1984]For (190871,361)the (191232,172)first (191404,376)time(191780,1075)\n\
                       [193459,4198]What's (193459,412)past (193871,574)is (194445,506)past(194951,2706)";
        let lines = parse(payload);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].start_ms, 190871);
        assert_eq!(lines[0].words.len(), 4);
        assert_eq!(lines[0].words[0].text, "For ");
        assert_eq!(lines[0].words[0].start_ms, 190871);
        assert_eq!(lines[0].words[3].text, "time");
        assert_eq!(lines[1].words[0].text, "What's ");
    }

    #[test]
    fn word_level_lrc_puts_one_tag_before_every_word() {
        let lines = parse("[1000,500]你(1000,200)好(1200,300)\n[2000,500]世界(2000,500)");
        let lrc = to_word_lrc(&lines);
        assert_eq!(lrc, "[00:01.00]你[00:01.20]好\n[00:02.00]世界");
        // Every line carries more than one timestamp, which is exactly how the
        // host tells word-level lyrics from line-level ones.
        assert!(lrc.lines().next().unwrap().matches('[').count() > 1);
    }

    #[test]
    fn a_bracket_in_a_word_cannot_break_the_tag() {
        let lines = parse("[0,100]a[b(0,50)c]d(50,50)");
        let lrc = to_word_lrc(&lines);
        assert_eq!(lrc, "[00:00.00]ab[00:00.05]cd");
    }
}
