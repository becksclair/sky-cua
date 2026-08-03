use std::io::{self, ErrorKind, Read, Write};

pub fn read_frame(reader: &mut impl Read) -> io::Result<Option<serde_json::Value>> {
    loop {
        let mut header = [0_u8; 4];
        match reader.read_exact(&mut header) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(error),
        }

        const MAX_FRAME_SIZE: usize = 100 * 1024 * 1024;
        let length = u32::from_ne_bytes(header) as usize;
        if length > MAX_FRAME_SIZE {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "frame exceeds maximum size",
            ));
        }
        let mut body = vec![0_u8; length];
        reader.read_exact(&mut body)?;

        match serde_json::from_slice(&body) {
            Ok(message) => return Ok(Some(message)),
            Err(error) => eprintln!("[sky-cua-chrome-host] dropping invalid JSON frame: {error}"),
        }
    }
}

pub fn write_frame(writer: &mut impl Write, message: &serde_json::Value) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    if body.len() > u32::MAX as usize {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "message too large for 4-byte length prefix",
        ));
    }

    writer.write_all(&(body.len() as u32).to_ne_bytes())?;
    writer.write_all(&body)?;
    writer.flush()
}

/// Write a full frame (length prefix + JSON body encoded into one contiguous
/// buffer) under a total wall-clock deadline. Manual loop rather than
/// `write_all`, because the caller cannot re-check a deadline inside
/// `write_all`; each syscall gets the remaining budget as its socket write
/// timeout, and the prior timeout is restored on every exit path so the final
/// tiny remaining timeout cannot poison later writes.
#[cfg(unix)]
pub fn write_frame_until(
    stream: &mut std::os::unix::net::UnixStream,
    message: &serde_json::Value,
    deadline: std::time::Instant,
) -> io::Result<()> {
    let body = serde_json::to_vec(message).map_err(io::Error::other)?;
    if body.len() > u32::MAX as usize {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "message too large for 4-byte length prefix",
        ));
    }

    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_ne_bytes());
    frame.extend_from_slice(&body);

    let previous_timeout = stream.write_timeout()?;
    let result = (|| {
        let mut offset = 0;
        while offset < frame.len() {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .ok_or_else(write_deadline_exceeded)?;
            if remaining.is_zero() {
                return Err(write_deadline_exceeded());
            }
            stream.set_write_timeout(Some(remaining))?;
            match stream.write(&frame[offset..]) {
                Ok(0) => {
                    return Err(io::Error::new(
                        ErrorKind::WriteZero,
                        "write to native-host client returned zero bytes",
                    ));
                }
                Ok(written) => offset += written,
                Err(error) if error.kind() == ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        stream.flush()
    })();
    match stream.set_write_timeout(previous_timeout) {
        Ok(()) => result,
        Err(error) => Err(error),
    }
}

fn write_deadline_exceeded() -> io::Error {
    io::Error::new(
        ErrorKind::TimedOut,
        "control-plane frame write deadline exceeded",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_round_trip_uses_native_length_prefix() {
        let message = json!({ "jsonrpc": "2.0", "id": "1", "method": "ping" });
        let mut encoded = Vec::new();
        write_frame(&mut encoded, &message).unwrap();

        let length = u32::from_ne_bytes(encoded[..4].try_into().unwrap()) as usize;
        assert_eq!(length, encoded.len() - 4);

        let mut cursor = std::io::Cursor::new(encoded);
        assert_eq!(read_frame(&mut cursor).unwrap(), Some(message));
    }
}
