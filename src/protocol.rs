use serde::{Deserialize, Serialize};

pub const SERVICE_TYPE: &str = "_newtowerrelay._tcp.local.";
pub const CHUNK_SIZE: usize = 64 * 1024;
/// Upper bound on encrypted wire frames (chunk JSON is base64, not byte arrays).
pub const MAX_ENCRYPTED_WIRE_FRAME: usize = 512 * 1024;
/// Upper bound on plaintext handshake frames.
pub const MAX_PLAINTEXT_WIRE_FRAME: usize = 64 * 1024;

mod base64_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine as _};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error> {
        STANDARD.encode(bytes).serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub device_id: String,
    pub name: String,
    pub public_key: String,
    pub sign_key: String,
    pub signature: String,
    pub pairing_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairingRequest {
    pub code: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileMeta {
    pub name: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub files: Vec<FileMeta>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ControlMessage {
    Hello(Hello),
    Pairing(PairingRequest),
    Offer(Offer),
    Accept,
    Decline,
    FileStart { name: String, size: u64 },
    FileComplete,
    Done,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    Control(ControlMessage),
    Chunk(#[serde(with = "base64_bytes")] Vec<u8>),
}

impl WireMessage {
    pub fn encode(&self) -> anyhow::Result<Vec<u8>> {
        let payload = serde_json::to_vec(self)?;
        let len = (payload.len() as u32).to_be_bytes();
        let mut out = Vec::with_capacity(4 + payload.len());
        out.extend_from_slice(&len);
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn decode_frame(header: &[u8; 4], payload: &[u8]) -> anyhow::Result<Self> {
        let expected = u32::from_be_bytes(*header) as usize;
        anyhow::ensure!(payload.len() == expected, "frame length mismatch");
        Ok(serde_json::from_slice(payload)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunk_roundtrip_preserves_bytes() {
        let data: Vec<u8> = (0..CHUNK_SIZE).map(|i| (i % 256) as u8).collect();
        let msg = WireMessage::Chunk(data.clone());
        let frame = msg.encode().unwrap();
        let decoded = WireMessage::decode_frame(frame[..4].try_into().unwrap(), &frame[4..]).unwrap();
        match decoded {
            WireMessage::Chunk(restored) => assert_eq!(restored, data),
            other => panic!("expected chunk, got {other:?}"),
        }
    }

    #[test]
    fn chunk_json_uses_base64_not_byte_array() {
        let data = vec![0u8; CHUNK_SIZE];
        let json = serde_json::to_string(&WireMessage::Chunk(data)).unwrap();
        assert!(
            !json.contains("[0,1,2"),
            "chunks must not serialize as JSON number arrays"
        );
        assert!(json.len() < CHUNK_SIZE + (CHUNK_SIZE / 2), "base64 chunk frame too large");
    }

    #[test]
    fn file_start_control_roundtrip() {
        let msg = WireMessage::Control(ControlMessage::FileStart {
            name: "photo.jpg".into(),
            size: 12345,
        });
        let frame = msg.encode().unwrap();
        let decoded = WireMessage::decode_frame(frame[..4].try_into().unwrap(), &frame[4..]).unwrap();
        match decoded {
            WireMessage::Control(ControlMessage::FileStart { name, size }) => {
                assert_eq!(name, "photo.jpg");
                assert_eq!(size, 12345);
            }
            other => panic!("expected FileStart, got {other:?}"),
        }
    }
}
