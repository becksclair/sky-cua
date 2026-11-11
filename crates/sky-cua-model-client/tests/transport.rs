use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use serde_json::json;
use sky_cua_model_client::{ModelClient, ModelError};

struct Request {
    line: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    body_delay: Duration,
}

impl Reply {
    fn json(body: &str) -> Self {
        Self {
            status: 200,
            body: body.as_bytes().to_vec(),
            body_delay: Duration::ZERO,
        }
    }
}

struct Server {
    endpoint: String,
    task: JoinHandle<(Vec<Request>, usize)>,
}

impl Server {
    fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!(
            "http://{}/provider/custom?api_key=url-secret",
            listener.local_addr().unwrap()
        );
        listener.set_nonblocking(true).unwrap();
        let task = thread::spawn(move || {
            let mut connection: Option<BufReader<TcpStream>> = None;
            let mut requests = Vec::new();
            let mut accepts = 0;
            for reply in replies {
                let deadline = Instant::now() + Duration::from_secs(3);
                let request = loop {
                    if connection.is_none() {
                        let stream = loop {
                            match listener.accept() {
                                Ok((stream, _)) => break stream,
                                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                                    assert!(Instant::now() < deadline, "client never connected");
                                    thread::sleep(Duration::from_millis(5));
                                }
                                Err(error) => panic!("accept failed: {error}"),
                            }
                        };
                        // BSD-derived systems (macOS) hand back accepted sockets
                        // that inherit the listener's O_NONBLOCK; Linux does not.
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_millis(500)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(1)))
                            .unwrap();
                        accepts += 1;
                        connection = Some(BufReader::new(stream));
                    }
                    match read_request(connection.as_mut().unwrap()) {
                        Ok(request) => break request,
                        Err(_) => {
                            connection = None;
                            assert!(
                                Instant::now() < deadline,
                                "client never sent a complete request"
                            );
                        }
                    }
                };
                requests.push(request);
                let stream = connection.as_mut().unwrap().get_mut();
                write!(
                    stream,
                    "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                    reply.status,
                    reply.body.len()
                ).unwrap();
                stream.flush().unwrap();
                thread::sleep(reply.body_delay);
                // A timeout or response-size rejection may close the client side.
                let _ = stream.write_all(&reply.body);
                let _ = stream.flush();
            }
            (requests, accepts)
        });
        Self { endpoint, task }
    }

    fn finish(self) -> (Vec<Request>, usize) {
        self.task.join().unwrap()
    }
}

fn read_request(reader: &mut BufReader<TcpStream>) -> std::io::Result<Request> {
    let mut line = String::new();
    if reader.read_line(&mut line)? == 0 {
        return Err(std::io::ErrorKind::UnexpectedEof.into());
    }
    let mut headers = BTreeMap::new();
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" {
            break;
        }
        let (name, value) = header.split_once(':').expect("valid request header");
        headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
    }
    let length = headers["content-length"].parse::<usize>().unwrap();
    let mut body = vec![0; length];
    reader.read_exact(&mut body)?;
    Ok(Request {
        line,
        headers,
        body,
    })
}

fn client(max_response_bytes: usize) -> ModelClient {
    ModelClient::new(Duration::from_secs(2), max_response_bytes).unwrap()
}

#[test]
fn provider_payload_and_headers_round_trip_without_schema_assumptions() {
    let server = Server::start(vec![Reply::json(
        r#"{"vendor_result":{"items":[null,3,"ok"]}}"#,
    )]);
    let payload = json!({"engine": "arbitrary-provider", "input": [{"image": [1, 2, 3]}], "options": {"flag": true}});
    let response = client(1024)
        .post_json(
            &server.endpoint,
            &[
                ("Authorization", "Bearer header-secret"),
                ("X-Provider-Version", "2026-11"),
            ],
            &payload,
        )
        .unwrap();
    assert_eq!(
        response.body,
        json!({"vendor_result": {"items": [null, 3, "ok"]}})
    );
    assert!(response.elapsed > Duration::ZERO);
    let (requests, _) = server.finish();
    assert_eq!(requests.len(), 1);
    assert_eq!(
        requests[0].line,
        "POST /provider/custom?api_key=url-secret HTTP/1.1\r\n"
    );
    assert_eq!(requests[0].headers["authorization"], "Bearer header-secret");
    assert_eq!(requests[0].headers["x-provider-version"], "2026-11");
    assert_eq!(requests[0].headers["content-type"], "application/json");
    assert_eq!(requests[0].headers["accept"], "application/json");
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&requests[0].body).unwrap(),
        payload
    );
}

#[test]
fn http_error_exposes_only_status() {
    let mut reply = Reply::json(r#"{"error":"response-secret"}"#);
    reply.status = 401;
    let server = Server::start(vec![reply]);
    let error = client(1024)
        .post_json(
            &server.endpoint,
            &[("Authorization", "Bearer header-secret")],
            &json!({"input": "request-secret"}),
        )
        .unwrap_err();
    assert_eq!(error, ModelError::Http(401));
    assert_eq!(error.to_string(), "model HTTP 401");
    assert_eq!(format!("{error:?}"), "Http(401)");
    server.finish();
}

#[test]
fn invalid_json_exposes_no_response_contents() {
    let server = Server::start(vec![Reply::json("response-secret is not JSON")]);
    let error = client(1024)
        .post_json(&server.endpoint, &[], &json!({}))
        .unwrap_err();
    assert_eq!(error, ModelError::InvalidResponse);
    assert_eq!(error.to_string(), "model request failed: InvalidResponse");
    server.finish();
}

#[test]
fn response_at_byte_limit_is_accepted() {
    let server = Server::start(vec![Reply::json(r#"{"ok":true}"#)]);
    let response = client(11)
        .post_json(&server.endpoint, &[], &json!({}))
        .unwrap();
    assert_eq!(response.body, json!({"ok": true}));
    server.finish();
}

#[test]
fn response_beyond_byte_limit_is_rejected_before_json_parsing() {
    let server = Server::start(vec![Reply::json(r#"{"ok":true}"#)]);
    let error = client(10)
        .post_json(&server.endpoint, &[], &json!({}))
        .unwrap_err();
    assert_eq!(error, ModelError::ResponseTooLarge);
    server.finish();
}

#[test]
fn global_timeout_also_bounds_reading_the_response_body() {
    let mut reply = Reply::json(r#"{"ok":true}"#);
    reply.body_delay = Duration::from_millis(300);
    let server = Server::start(vec![reply]);
    let client = ModelClient::new(Duration::from_millis(80), 1024).unwrap();
    let started = Instant::now();
    let error = client
        .post_json(&server.endpoint, &[], &json!({}))
        .unwrap_err();
    assert_eq!(error, ModelError::Transport);
    assert!(started.elapsed() < Duration::from_secs(1));
    server.finish();
}

#[test]
fn consecutive_calls_reuse_the_same_http_connection() {
    let server = Server::start(vec![Reply::json("1"), Reply::json("2")]);
    let client = client(1024);
    for expected in [1, 2] {
        let response = client
            .post_json(&server.endpoint, &[], &json!({"call": expected}))
            .unwrap();
        assert_eq!(response.body, json!(expected));
    }
    let (requests, accepts) = server.finish();
    assert_eq!(requests.len(), 2);
    assert_eq!(
        accepts, 1,
        "both calls should use the client's connection pool"
    );
}

#[test]
fn zero_timeout_or_response_limit_is_invalid_configuration() {
    assert!(matches!(
        ModelClient::new(Duration::ZERO, 1024),
        Err(ModelError::InvalidConfiguration)
    ));
    assert!(matches!(
        ModelClient::new(Duration::from_secs(1), 0),
        Err(ModelError::InvalidConfiguration)
    ));
}
