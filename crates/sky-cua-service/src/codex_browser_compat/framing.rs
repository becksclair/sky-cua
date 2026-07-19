use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::MAX_FRAME_BYTES;

pub(super) async fn read_frame(
    reader: &mut (impl AsyncRead + Unpin),
) -> std::io::Result<Option<Value>> {
    let mut header = [0_u8; 4];
    match reader.read_exact(&mut header).await {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_ne_bytes(header) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Codex browser frame exceeds 100 MiB maximum",
        ));
    }
    let mut body = vec![0; length];
    reader.read_exact(&mut body).await?;
    serde_json::from_slice(&body).map(Some).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("invalid Codex browser JSON frame: {error}"),
        )
    })
}

pub(super) async fn write_frame(
    writer: &mut (impl AsyncWrite + Unpin),
    message: &Value,
) -> std::io::Result<()> {
    let body = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    if body.len() > MAX_FRAME_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "Codex browser frame exceeds 100 MiB maximum",
        ));
    }
    writer.write_all(&(body.len() as u32).to_ne_bytes()).await?;
    writer.write_all(&body).await?;
    writer.flush().await
}
