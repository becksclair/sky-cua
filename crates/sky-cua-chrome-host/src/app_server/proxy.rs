use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

use anyhow::{Context, Result};
use url::Url;

const ACCEPT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const HEADER_LIMIT: usize = 64 * 1024;

pub(super) struct ProxyServer {
    shutdown_sender: mpsc::Sender<()>,
    active_connections: Arc<Mutex<HashMap<u64, TcpStream>>>,
    workers: Arc<Mutex<Vec<thread::JoinHandle<()>>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl ProxyServer {
    pub(super) fn start(
        listener: TcpListener,
        backend_url: &str,
        extension_id: Option<&str>,
    ) -> Result<Self> {
        let backend_url =
            Url::parse(backend_url).context("invalid Codex app-server backend URL")?;
        let backend_address = backend_url
            .socket_addrs(|| None)?
            .into_iter()
            .next()
            .context("Codex app-server backend URL has no socket address")?;
        listener.set_nonblocking(true)?;
        let allowed_origin =
            extension_id.map(|id| Arc::<str>::from(format!("chrome-extension://{id}")));
        let (shutdown_sender, shutdown_receiver) = mpsc::channel();
        let active_connections = Arc::new(Mutex::new(HashMap::new()));
        let workers = Arc::new(Mutex::new(Vec::new()));
        let accept_connections = Arc::clone(&active_connections);
        let accept_workers = Arc::clone(&workers);
        let thread = thread::spawn(move || {
            let mut next_connection_id = 0_u64;
            loop {
                reap_finished_workers(&accept_workers);
                if shutdown_receiver.try_recv().is_ok() {
                    break;
                }
                match listener.accept() {
                    Ok((client, _)) => {
                        let allowed_origin = allowed_origin.clone();
                        let shutdown_client = match client.try_clone() {
                            Ok(client) => client,
                            Err(error) => {
                                eprintln!(
                                    "[sky-cua-chrome-host] app-server proxy client clone error: {error}"
                                );
                                continue;
                            }
                        };
                        next_connection_id = next_connection_id.wrapping_add(1);
                        let connection_id = next_connection_id;
                        accept_connections
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .insert(connection_id, shutdown_client);
                        let worker_connections = Arc::clone(&accept_connections);
                        let worker = thread::spawn(move || {
                            if let Err(error) =
                                proxy_connection(client, backend_address, allowed_origin.as_deref())
                            {
                                eprintln!("[sky-cua-chrome-host] app-server proxy error: {error}");
                            }
                            worker_connections
                                .lock()
                                .unwrap_or_else(|poisoned| poisoned.into_inner())
                                .remove(&connection_id);
                        });
                        accept_workers
                            .lock()
                            .unwrap_or_else(|poisoned| poisoned.into_inner())
                            .push(worker);
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(ACCEPT_POLL_INTERVAL);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(error) => {
                        eprintln!("[sky-cua-chrome-host] app-server proxy accept error: {error}");
                        break;
                    }
                }
            }
        });
        Ok(Self {
            shutdown_sender,
            active_connections,
            workers,
            thread: Some(thread),
        })
    }

    pub(super) fn is_running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
}

impl Drop for ProxyServer {
    fn drop(&mut self) {
        let _ = self.shutdown_sender.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        {
            let mut active = self
                .active_connections
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for (_, connection) in active.drain() {
                let _ = connection.shutdown(Shutdown::Both);
            }
        }
        let workers = {
            let mut workers = self
                .workers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut *workers)
        };
        for worker in workers {
            let _ = worker.join();
        }
    }
}

fn proxy_connection(
    mut client: TcpStream,
    backend_address: SocketAddr,
    allowed_origin: Option<&str>,
) -> io::Result<()> {
    client.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = Vec::new();
    let header_end = loop {
        if request.len() >= HEADER_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "proxy request headers are too large",
            ));
        }
        let mut buffer = [0_u8; 4096];
        let read = client.read(&mut buffer)?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..read]);
        if request.len() > HEADER_LIMIT {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "proxy request headers are too large",
            ));
        }
        if let Some(end) = find_header_end(&request) {
            break end;
        }
    };

    let rewritten = rewrite_headers(&request[..header_end], allowed_origin)?;
    let mut backend = TcpStream::connect_timeout(&backend_address, Duration::from_secs(5))?;
    backend.write_all(&rewritten)?;
    backend.write_all(&request[header_end..])?;
    client.set_read_timeout(None)?;

    let mut client_reader = client.try_clone()?;
    let mut backend_writer = backend.try_clone()?;
    let upstream = thread::spawn(move || {
        let _ = io::copy(&mut client_reader, &mut backend_writer);
        let _ = backend_writer.shutdown(Shutdown::Write);
    });
    let _ = io::copy(&mut backend, &mut client);
    let _ = client.shutdown(Shutdown::Both);
    let _ = upstream.join();
    Ok(())
}

fn reap_finished_workers(workers: &Mutex<Vec<thread::JoinHandle<()>>>) {
    let mut workers = workers
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut active = Vec::with_capacity(workers.len());
    for worker in workers.drain(..) {
        if worker.is_finished() {
            let _ = worker.join();
        } else {
            active.push(worker);
        }
    }
    *workers = active;
}

fn find_header_end(request: &[u8]) -> Option<usize> {
    request
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|position| position + 4)
}

fn rewrite_headers(headers: &[u8], allowed_origin: Option<&str>) -> io::Result<Vec<u8>> {
    let headers = std::str::from_utf8(headers)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let mut lines = headers.split("\r\n");
    let request_line = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
    let mut rewritten = String::with_capacity(headers.len());
    rewritten.push_str(request_line);
    rewritten.push_str("\r\n");
    let mut origin = None;
    let mut websocket_upgrade = false;
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let Some((name, value)) = line.split_once(':') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed proxy request header",
            ));
        };
        if name.eq_ignore_ascii_case("origin") {
            origin = Some(value.trim());
            continue;
        }
        if name.eq_ignore_ascii_case("upgrade") && value.trim().eq_ignore_ascii_case("websocket") {
            websocket_upgrade = true;
        }
        rewritten.push_str(line);
        rewritten.push_str("\r\n");
    }
    if websocket_upgrade
        && let Some(allowed_origin) = allowed_origin
        && origin != Some(allowed_origin)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "WebSocket origin is not authorized",
        ));
    }
    rewritten.push_str("\r\n");
    Ok(rewritten.into_bytes())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn authorizes_and_strips_the_extension_origin() {
        let request = b"GET /?clientId=sidepanel-window-1 HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nOrigin: chrome-extension://extension-1\r\n\r\n";
        let rewritten = rewrite_headers(request, Some("chrome-extension://extension-1")).unwrap();
        let rewritten = String::from_utf8(rewritten).unwrap();
        assert!(rewritten.starts_with("GET /?clientId=sidepanel-window-1 HTTP/1.1\r\n"));
        assert!(!rewritten.to_ascii_lowercase().contains("origin:"));
    }

    #[test]
    fn rejects_a_foreign_websocket_origin() {
        let request =
            b"GET / HTTP/1.1\r\nUpgrade: websocket\r\nOrigin: chrome-extension://foreign\r\n\r\n";
        let error = rewrite_headers(request, Some("chrome-extension://extension-1")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn worker_exits_when_the_backend_closes() {
        let backend_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let backend_address = backend_listener.local_addr().unwrap();
        let backend = thread::spawn(move || {
            let (mut stream, _) = backend_listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let read = stream.read(&mut request).unwrap();
            let request = std::str::from_utf8(&request[..read]).unwrap();
            assert!(request.starts_with("GET / HTTP/1.1\r\n"));
            assert!(!request.to_ascii_lowercase().contains("origin:"));
        });

        let proxy_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let proxy_address = proxy_listener.local_addr().unwrap();
        let proxy = ProxyServer::start(
            proxy_listener,
            &format!("ws://{backend_address}"),
            Some("extension-1"),
        )
        .unwrap();
        let mut client = TcpStream::connect(proxy_address).unwrap();
        client
            .write_all(
                b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nUpgrade: websocket\r\nOrigin: chrome-extension://extension-1\r\n\r\n",
            )
            .unwrap();

        let deadline = Instant::now() + Duration::from_secs(1);
        while proxy
            .workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        while !proxy
            .active_connections
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }
        while !proxy
            .workers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_empty()
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(10));
        }

        assert!(
            proxy
                .active_connections
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "proxy worker remained active after backend shutdown"
        );
        assert!(
            proxy
                .workers
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .is_empty(),
            "completed proxy workers were not reaped"
        );
        backend.join().unwrap();
        drop(proxy);
    }
}
