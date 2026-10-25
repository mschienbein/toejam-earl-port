use crate::{Course, MAX_BODY_BYTES, MAX_CODE_BYTES, ValidationErrors, validate};
use flate2::{Compression, Decompress, FlushDecompress, Status, write::ZlibEncoder};
use sha2::{Digest, Sha256};
use std::{fmt, io::Write};

const HEADER: usize = 48;
const PREFIX: &str = "TJM1:";
// RFC 4648 character classes, computed once at compile time. This is protocol
// metadata, not an opaque encoded payload or a runtime-built lookup table.
static ALPHABET: [u8; 64] = {
    let mut alphabet = [0; 64];
    let mut index = 0;
    while index < 64 {
        alphabet[index] = match index {
            0..=25 => b'A' + index as u8,
            26..=51 => b'a' + (index - 26) as u8,
            52..=61 => b'0' + (index - 52) as u8,
            62 => b'-',
            _ => b'_',
        };
        index += 1;
    }
    alphabet
};

#[derive(Debug)]
pub enum Error {
    Json(serde_json::Error),
    Validation(ValidationErrors),
    Format(String),
}
impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(error) => write!(f, "course JSON: {error}"),
            Self::Validation(error) => error.fmt(f),
            Self::Format(error) => f.write_str(error),
        }
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(error) => Some(error),
            Self::Validation(error) => Some(error),
            Self::Format(_) => None,
        }
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}
impl From<ValidationErrors> for Error {
    fn from(value: ValidationErrors) -> Self {
        Self::Validation(value)
    }
}
fn bad(message: impl Into<String>) -> Error {
    Error::Format(message.into())
}

/// Immutable normalized document. Public callers cannot bypass validation or
/// change a live course without creating a new body digest and admission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedCourse {
    course: Course,
    body: Vec<u8>,
    digest: [u8; 32],
}
impl std::ops::Deref for ValidatedCourse {
    type Target = Course;
    fn deref(&self) -> &Course {
        &self.course
    }
}
impl bincode::Encode for ValidatedCourse {
    fn encode<E: bincode::enc::Encoder>(
        &self,
        encoder: &mut E,
    ) -> Result<(), bincode::error::EncodeError> {
        bincode::Encode::encode(&self.body, encoder)
    }
}
impl<C> bincode::Decode<C> for ValidatedCourse {
    fn decode<D: bincode::de::Decoder<Context = C>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        use bincode::de::read::Reader;
        let length = <usize as bincode::Decode<C>>::decode(decoder)?;
        if length > MAX_BODY_BYTES {
            return Err(bincode::error::DecodeError::Other(
                "course configuration exceeds 2 MiB",
            ));
        }
        decoder.claim_bytes_read(length)?;
        let mut body = vec![0; length];
        decoder.reader().read(&mut body)?;
        let course = Self::from_json(&body)
            .map_err(|error| bincode::error::DecodeError::OtherString(error.to_string()))?;
        if course.canonical_json() != body {
            return Err(bincode::error::DecodeError::Other(
                "course configuration is not canonical",
            ));
        }
        Ok(course)
    }
}
impl<'de, C> bincode::BorrowDecode<'de, C> for ValidatedCourse {
    fn borrow_decode<D: bincode::de::BorrowDecoder<'de, Context = C>>(
        decoder: &mut D,
    ) -> Result<Self, bincode::error::DecodeError> {
        bincode::Decode::decode(decoder)
    }
}
impl ValidatedCourse {
    pub fn new(mut course: Course) -> Result<Self, Error> {
        validate(&course)?;
        crate::validate::normalize(&mut course);
        // All schema keys are ASCII. serde_json's sorted map ordering is JCS
        // ordering for this fixed schema; values contain integers, not floats.
        let mut value = serde_json::to_value(&course)?;
        value.sort_all_objects();
        let body = serde_json::to_vec(&value)?;
        if body.len() > MAX_BODY_BYTES {
            return Err(bad("canonical course body exceeds 2 MiB"));
        }
        let digest = Sha256::digest(&body).into();
        Ok(Self {
            course,
            body,
            digest,
        })
    }
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_BODY_BYTES {
            return Err(bad("course JSON exceeds 2 MiB"));
        }
        check_depth(bytes)?;
        Self::new(serde_json::from_slice(bytes)?)
    }
    pub fn course(&self) -> &Course {
        &self.course
    }
    pub fn canonical_json(&self) -> &[u8] {
        &self.body
    }
    pub fn digest(&self) -> [u8; 32] {
        self.digest
    }
    pub fn digest_hex(&self) -> String {
        let mut text = String::with_capacity(64);
        use fmt::Write as _;
        for byte in self.digest {
            write!(text, "{byte:02x}").expect("writing into String cannot fail");
        }
        text
    }
    pub fn to_share_code(&self) -> Result<String, Error> {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder
            .write_all(&self.body)
            .map_err(|error| bad(format!("compressing course: {error}")))?;
        let compressed = encoder
            .finish()
            .map_err(|error| bad(format!("compressing course: {error}")))?;
        if compressed.len() > MAX_BODY_BYTES {
            return Err(bad("compressed course exceeds 2 MiB"));
        }
        let mut payload = Vec::with_capacity(HEADER + compressed.len());
        payload.extend_from_slice(b"TJMC");
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&(self.body.len() as u32).to_le_bytes());
        payload.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        payload.extend_from_slice(&self.digest);
        payload.extend_from_slice(&compressed);
        let mut output = String::with_capacity(PREFIX.len() + payload.len().div_ceil(3) * 4);
        output.push_str(PREFIX);
        encode_base64(&payload, &mut output);
        if output.len() > MAX_CODE_BYTES {
            return Err(bad("share code exceeds 3 MiB"));
        }
        Ok(output)
    }
    pub fn from_share_code(code: &str) -> Result<Self, Error> {
        if code.len() > MAX_CODE_BYTES {
            return Err(bad("share code exceeds 3 MiB"));
        }
        let encoded = code
            .strip_prefix(PREFIX)
            .ok_or_else(|| bad("share code must start with TJM1:"))?;
        // Bound decoded bytes before allocating, and reject padding/whitespace.
        if encoded.len().saturating_mul(3) / 4 > MAX_BODY_BYTES + HEADER {
            return Err(bad("share payload exceeds compressed size bound"));
        }
        let payload = decode_base64(encoded)?;
        if payload.len() < HEADER || &payload[..4] != b"TJMC" {
            return Err(bad("share code has a truncated or invalid TJMC header"));
        }
        let word = |offset| u16::from_le_bytes(payload[offset..offset + 2].try_into().unwrap());
        let long =
            |offset| u32::from_le_bytes(payload[offset..offset + 4].try_into().unwrap()) as usize;
        if word(4) != 1 {
            return Err(bad("unsupported share schema major (expected 1)"));
        }
        if word(6) != 1 {
            return Err(bad("unsupported share codec flags (expected zlib=1)"));
        }
        let (plain, compressed) = (long(8), long(12));
        if plain == 0 || plain > MAX_BODY_BYTES || compressed == 0 || compressed > MAX_BODY_BYTES {
            return Err(bad("declared share lengths must each be 1 through 2 MiB"));
        }
        if payload.len() != HEADER + compressed {
            return Err(bad(
                "compressed payload length differs from its header or has trailing data",
            ));
        }
        // One extra byte detects a lying length without growing beyond the
        // bound. No read_to_end or unbounded decompressor allocation is used.
        let mut body = vec![0; plain + 1];
        let mut decoder = Decompress::new(true);
        let status = decoder
            .decompress(&payload[HEADER..], &mut body, FlushDecompress::Finish)
            .map_err(|error| bad(format!("invalid zlib stream: {error}")))?;
        if status != Status::StreamEnd
            || decoder.total_out() != plain as u64
            || decoder.total_in() != compressed as u64
        {
            return Err(bad(
                "zlib length mismatch, excessive output or trailing compressed stream",
            ));
        }
        body.truncate(plain);
        let digest: [u8; 32] = Sha256::digest(&body).into();
        if payload[16..HEADER] != digest {
            return Err(bad("course SHA-256 checksum mismatch"));
        }
        let document = Self::from_json(&body)?;
        if document.body != body {
            return Err(bad("share body is not canonical JSON/RLE/ID order"));
        }
        Ok(document)
    }
}

fn check_depth(bytes: &[u8]) -> Result<(), Error> {
    let (mut depth, mut string, mut escape) = (0_u8, false, false);
    for &byte in bytes {
        if string {
            if escape {
                escape = false;
            } else if byte == b'\\' {
                escape = true;
            } else if byte == b'"' {
                string = false;
            }
        } else {
            match byte {
                b'"' => string = true,
                b'{' | b'[' => {
                    depth += 1;
                    if depth > 16 {
                        return Err(bad("JSON nesting exceeds sixteen levels"));
                    }
                }
                b'}' | b']' => {
                    depth = depth.saturating_sub(1);
                }
                _ => (),
            }
        }
    }
    Ok(())
}
fn encode_base64(bytes: &[u8], text: &mut String) {
    for chunk in bytes.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for shift in [18, 12, 6, 0].into_iter().take(chunk.len() + 1) {
            text.push(ALPHABET[((bits >> shift) & 63) as usize] as char);
        }
    }
}
fn decode_base64(text: &str) -> Result<Vec<u8>, Error> {
    if text.len() % 4 == 1 {
        return Err(bad("invalid unpadded Base64URL length"));
    }
    let mut output = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0_u32, 0_u8);
    for byte in text.bytes() {
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => {
                return Err(bad(
                    "share code contains a non-Base64URL character or padding",
                ));
            }
        };
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((buffer >> bits) as u8);
            buffer &= (1 << bits) - 1;
        }
    }
    if buffer != 0 {
        return Err(bad("noncanonical Base64URL unused bits"));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn base64_vectors_and_unused_bits() {
        for (raw, encoded) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foobar", "Zm9vYmFy"),
        ] {
            let mut output = String::new();
            encode_base64(raw.as_bytes(), &mut output);
            assert_eq!(output, encoded);
            assert_eq!(decode_base64(encoded).unwrap(), raw.as_bytes());
        }
        for invalid in ["a", "Zg=", "Zh", "Zm9", "Zg\n", "++//"] {
            assert!(decode_base64(invalid).is_err(), "{invalid}");
        }
    }
}

#[cfg(test)]
mod hostile_envelopes {
    use super::*;
    fn envelope(body: &[u8], declared: usize, extra_stream: bool) -> String {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::best());
        encoder.write_all(body).unwrap();
        let mut compressed = encoder.finish().unwrap();
        if extra_stream {
            compressed.extend_from_slice(&compressed.clone());
        }
        let mut payload = b"TJMC".to_vec();
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&1_u16.to_le_bytes());
        payload.extend_from_slice(&(declared as u32).to_le_bytes());
        payload.extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        payload.extend_from_slice(&Sha256::digest(body));
        payload.extend_from_slice(&compressed);
        let mut code = PREFIX.to_owned();
        encode_base64(&payload, &mut code);
        code
    }
    #[test]
    fn lying_lengths_bombs_and_extra_zlib_streams_are_bounded() {
        let document =
            ValidatedCourse::from_json(include_bytes!("../../../courses/meadow.tjemaker.json"))
                .unwrap();
        let body = document.canonical_json();
        for declared in [0, 1, body.len() - 1, body.len() + 1, MAX_BODY_BYTES + 1] {
            assert!(ValidatedCourse::from_share_code(&envelope(body, declared, false)).is_err());
        }
        assert!(ValidatedCourse::from_share_code(&envelope(body, body.len(), true)).is_err());
        let bomb = vec![b' '; MAX_BODY_BYTES + 1];
        assert!(ValidatedCourse::from_share_code(&envelope(&bomb, 16, false)).is_err());
    }
    #[test]
    fn correct_checksum_does_not_bypass_canonical_or_semantic_validation() {
        let document =
            ValidatedCourse::from_json(include_bytes!("../../../courses/meadow.tjemaker.json"))
                .unwrap();
        let pretty = serde_json::to_vec_pretty(document.course()).unwrap();
        assert!(
            ValidatedCourse::from_share_code(&envelope(&pretty, pretty.len(), false))
                .unwrap_err()
                .to_string()
                .contains("not canonical")
        );
        let invalid = b"{\"schema\":\"not-a-course\"}";
        assert!(
            ValidatedCourse::from_share_code(&envelope(invalid, invalid.len(), false)).is_err()
        );
    }
}
