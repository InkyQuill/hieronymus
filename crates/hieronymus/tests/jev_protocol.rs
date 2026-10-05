//! Local Jev integration contracts; no service keys or external requests.
use hieronymus::{
    jev,
    provider_http::{BlockingHttpTransport, HttpError, HttpResponse, ProviderTransport},
};
use serde_json::{Value, json};
use std::{
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

struct Wire {
    status: u16,
    body: String,
    calls: Mutex<Vec<(String, Value, Duration)>>,
}
impl ProviderTransport for Wire {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        payload: &Value,
        timeout: Duration,
    ) -> Result<HttpResponse, HttpError> {
        assert_eq!(
            headers,
            &[("authorization".into(), "Bearer fixture-key".into())]
        );
        self.calls
            .lock()
            .unwrap()
            .push((url.into(), payload.clone(), timeout));
        Ok(HttpResponse {
            status: self.status,
            body: self.body.clone(),
        })
    }
    fn get_json(
        &self,
        _: &str,
        _: &[(String, String)],
        _: Duration,
    ) -> Result<HttpResponse, HttpError> {
        panic!("Jev must not perform metadata requests")
    }
}
fn payload() -> Value {
    json!({"model":"audit","state":"fixture","questions":{
        "good":{"type":"noul","instructions":"Good?"},
        "bad":{"type":"noul","instructions":"Bad?"}
    }})
}
fn wire(status: u16, body: Value) -> Arc<Wire> {
    Arc::new(Wire {
        status,
        body: body.to_string(),
        calls: Mutex::new(Vec::new()),
    })
}
#[test]
fn partial_failure_preserves_names_and_uses_exact_deadline_and_endpoint_once() {
    let wire = wire(
        200,
        json!({"answers":{
            "bad":{"type":"noul","noul":1.5},"good":{"type":"noul","noul":0.9}
        }}),
    );
    let result = jev::ask(
        "fixture-key",
        &payload(),
        Duration::from_millis(123),
        wire.clone(),
    )
    .unwrap();
    assert_eq!(
        result["answers"],
        json!({"good":{"type":"noul","noul":0.9}})
    );
    let calls = wire.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0],
        (
            "https://api.typesafe.ai/v1/systemone".into(),
            payload(),
            Duration::from_millis(123)
        )
    );
}
#[test]
fn redirects_and_retryable_statuses_are_errors_with_no_second_request() {
    for status in [307, 429, 500] {
        let wire = wire(status, json!({"error":"fixture"}));
        assert_eq!(
            jev::ask(
                "fixture-key",
                &payload(),
                Duration::from_secs(1),
                wire.clone()
            ),
            Err("http_error")
        );
        assert_eq!(wire.calls.lock().unwrap().len(), 1);
    }
}
#[test]
fn invalid_configuration_and_header_injection_fail_before_transport() {
    let wire = wire(200, json!({"answers":{}}));
    for key in ["", "   ", "fixture-key\r\nx-injected: yes"] {
        assert_eq!(
            jev::ask(key, &payload(), Duration::from_secs(1), wire.clone()),
            Err("configuration_error")
        );
    }
    assert_eq!(
        jev::ask("fixture-key", &payload(), Duration::ZERO, wire.clone()),
        Err("transport_error")
    );
    let mut invalid = payload();
    invalid["questions"] = json!({});
    assert_eq!(
        jev::ask(
            "fixture-key",
            &invalid,
            Duration::from_secs(1),
            wire.clone()
        ),
        Err("invalid_request")
    );
    assert!(wire.calls.lock().unwrap().is_empty());
}
#[test]
fn oversized_injected_transport_is_rejected_before_json_parse() {
    let wire = wire(200, json!({"answers":{},"padding":"x".repeat(65536)}));
    assert_eq!(
        jev::ask("fixture-key", &payload(), Duration::from_secs(1), wire),
        Err("invalid_response")
    );
}

fn server(body: String, delay: Duration) -> (String, std::thread::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let worker = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut request = Vec::new();
        let mut buf = [0; 4096];
        loop {
            let n = stream.read(&mut buf).unwrap();
            if n == 0 {
                break;
            }
            request.extend_from_slice(&buf[..n]);
            if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..pos]);
                let len = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                if request.len() >= pos + 4 + len {
                    break;
                }
            }
        }
        std::thread::sleep(delay);
        let reply = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = stream.write_all(reply.as_bytes());
    });
    (url, worker)
}
#[test]
fn production_body_cap_accepts_boundary_and_rejects_one_byte_more() {
    for size in [65536, 65537] {
        let body = " ".repeat(size);
        let (url, worker) = server(body, Duration::ZERO);
        let result = BlockingHttpTransport::default().post_json_bounded(
            &url,
            &[],
            &json!({}),
            Duration::from_secs(2),
            65536,
        );
        worker.join().unwrap();
        if size == 65536 {
            assert_eq!(result.unwrap().body.len(), size);
        } else {
            assert!(matches!(result, Err(HttpError::TooLarge { limit: 65536 })));
        }
    }
}
#[test]
fn production_transport_honors_the_whole_request_timeout() {
    let (url, worker) = server("{}".into(), Duration::from_millis(100));
    let result = BlockingHttpTransport::default().post_json_bounded(
        &url,
        &[],
        &json!({}),
        Duration::from_millis(20),
        65536,
    );
    worker.join().unwrap();
    assert!(matches!(result, Err(HttpError::Timeout { .. })));
}
