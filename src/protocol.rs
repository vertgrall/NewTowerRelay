use serde::{Deserialize, Serialize};

pub const SERVICE_TYPE: &str = "_newtowerrelay._tcp.local.";
pub const CHUNK_SIZE: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hello {
    pub device_id: String,
    pub name: String,
    pub public_key: String,
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
    FileComplete,
    Done,
    Error { message: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WireMessage {
    Control(ControlMessage),
    Chunk(Vec<u8>),
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
