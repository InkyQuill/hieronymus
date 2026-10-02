//! TypeSafe SDK over the application's bounded, mockable HTTP transport.
use crate::provider_http::ProviderTransport;
use http_body_util::BodyExt;
use serde_json::Value;
use std::{
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Duration,
};
use typesafe_sdk::{Body, ClientBuilder, Questions, RawQuestion, RetryPolicy};

#[derive(Clone)]
struct Transport {
    inner: Arc<dyn ProviderTransport>,
    timeout: Duration,
}
impl tower_service::Service<http::Request<Body>> for Transport {
    type Response = http::Response<Body>;
    type Error = std::io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;
    fn poll_ready(&mut self, _: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }
    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let transport = self.clone();
        Box::pin(async move {
            let (parts, body) = request.into_parts();
            let bytes = body
                .collect()
                .await
                .map_err(std::io::Error::other)?
                .to_bytes();
            let payload = serde_json::from_slice(&bytes).map_err(std::io::Error::other)?;
            let headers = parts
                .headers
                .iter()
                // ProviderTransport owns JSON content type, accept and framing.
                .filter(|(name, _)| {
                    !matches!(
                        name.as_str(),
                        "content-type"
                            | "content-length"
                            | "accept"
                            | "host"
                            | "connection"
                            | "transfer-encoding"
                    )
                })
                .map(|(name, value)| {
                    Ok((
                        name.to_string(),
                        value.to_str().map_err(std::io::Error::other)?.to_owned(),
                    ))
                })
                .collect::<Result<Vec<_>, std::io::Error>>()?;
            let response = transport
                .inner
                .post_json(
                    &parts.uri.to_string(),
                    &headers,
                    &payload,
                    transport.timeout,
                )
                .map_err(|_| std::io::Error::other("Jev transport failed"))?;
            http::Response::builder()
                .status(response.status)
                .body(Body::from(bytes::Bytes::from(response.body)))
                .map_err(std::io::Error::other)
        })
    }
}

/// Send all named questions once, without retries or environment-selected endpoints.
/// Domain consumers still validate each answer and retain their own authority rules.
pub fn ask(
    key: &str,
    payload: &Value,
    timeout: Duration,
    transport: Arc<dyn ProviderTransport>,
) -> Result<Value, &'static str> {
    if timeout.is_zero() {
        return Err("transport_error");
    }
    // A scoped thread also permits callers already inside a Tokio runtime.
    std::thread::scope(|scope| {
        scope
            .spawn(|| {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                    .map_err(|_| "transport_error")?;
                runtime.block_on(async {
                    let client = ClientBuilder::new()
                        .api_key(key)
                        .base_url("https://api.typesafe.ai")
                        .default_model(payload["model"].as_str().ok_or("configuration_error")?)
                        .max_response_bytes(65_536)
                        .retry(RetryPolicy::default().max_retries(0))
                        .build_with_service(Transport {
                            inner: transport,
                            timeout,
                        })
                        .map_err(|_| "configuration_error")?;
                    let mut questions = Questions::new();
                    for (name, value) in
                        payload["questions"].as_object().ok_or("invalid_request")?
                    {
                        let mut question =
                            RawQuestion::new(value["type"].as_str().ok_or("invalid_request")?);
                        for (field, value) in value.as_object().ok_or("invalid_request")? {
                            if field != "type" {
                                question = question.field(field.as_str(), value);
                            }
                        }
                        questions = questions.raw(name.as_str(), question);
                    }
                    let questions = questions.prepare().map_err(|_| "invalid_request")?;
                    match client
                        .system_one(&payload["state"], &questions)
                        .timeout(timeout)
                        .send()
                        .await
                    {
                        Ok(response) => serde_json::from_slice(response.meta().raw_body())
                            .map_err(|_| "invalid_response"),
                        // One malformed answer must not discard other independently valid answers.
                        // The same 64 KiB cap applies; consumers validate every requested field.
                        Err(error) => match error.kind() {
                            typesafe_sdk::ErrorKind::ResponseValidation(error)
                                if error.body().len() <= 65_536 =>
                            {
                                serde_json::from_slice(error.body()).map_err(|_| "invalid_response")
                            }
                            typesafe_sdk::ErrorKind::Api(_) => Err("http_error"),
                            typesafe_sdk::ErrorKind::ResponseTooLarge { .. } => {
                                Err("invalid_response")
                            }
                            _ => Err("transport_error"),
                        },
                    }
                })
            })
            .join()
            .unwrap_or(Err("transport_error"))
    })
}
