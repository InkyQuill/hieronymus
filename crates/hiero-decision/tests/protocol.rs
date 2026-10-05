//! Decision protocol regression tests use synthetic, local-only evidence.
use hiero_decision::{Error, MAX_RESPONSE_BYTES, Request};
use serde_json::{Value, json};

fn request() -> Request {
    Request::new(&json!({"model":"audit","state":{"text":"private fixture"},"questions":{
        "yes":{"type":"noul","instructions":"Is it true?"},
        "pick":{"type":"choice","instructions":{"question":"Pick"},"criteria":{"a":null,"b":"Other"}},
        "rate":{"type":"score","instructions":"Rate","criteria":["low","high"]}
    }})).unwrap()
}
fn response() -> Value {
    json!({"answers":{
        "yes":{"type":"noul","noul":0.8},
        "pick":{"type":"choice","choice":"a","confidence":0.9,"probabilities":{"a":0.9,"b":0.1}},
        "rate":{"type":"score","score":0.25,"confidence":0.8,"legend":{"0":"low","1":"high"},"probabilities":{"0":0.75,"1":0.25}}
    }})
}
fn decode(value: &Value) -> Value {
    request()
        .decode(&serde_json::to_vec(value).unwrap())
        .unwrap()
}

#[test]
fn named_answers_and_structured_requests_round_trip() {
    let request = request();
    assert!(request.payload()["questions"]["pick"]["instructions"].is_object());
    assert_eq!(decode(&response()), response());
}
#[test]
fn malformed_sibling_is_omitted_without_discarding_valid_answers() {
    let mut body = response();
    body["answers"]["yes"]["noul"] = json!(1.5);
    let decoded = decode(&body);
    assert!(decoded["answers"].get("yes").is_none());
    assert_eq!(decoded["answers"]["pick"], body["answers"]["pick"]);
    assert_eq!(decoded["answers"]["rate"], body["answers"]["rate"]);
}
#[test]
fn invalid_distributions_winners_and_types_cannot_authorize_decisions() {
    for invalid in [
        json!({"type":"noul","noul":true}),
        json!({"type":"choice","choice":"a","confidence":0.9,"probabilities":{"a":0.9,"b":0.1}}),
        json!({"type":"noul","noul":-0.1}),
        json!({"type":"noul","noul":1.5}),
    ] {
        let mut body = response();
        body["answers"]["yes"] = invalid;
        assert!(decode(&body)["answers"].get("yes").is_none());
    }
    for invalid in [
        json!({"type":"choice","choice":"c","confidence":0.9,"probabilities":{"a":0.9,"b":0.1}}),
        json!({"type":"choice","choice":"b","confidence":0.9,"probabilities":{"a":0.9,"b":0.1}}),
        json!({"type":"choice","choice":"a","confidence":1.5,"probabilities":{"a":0.9,"b":0.1}}),
        json!({"type":"choice","choice":"a","confidence":0.9,"probabilities":{"a":0.9}}),
        json!({"type":"choice","choice":"a","confidence":0.9,"probabilities":{"a":0.9,"b":0.9}}),
    ] {
        let mut body = response();
        body["answers"]["pick"] = invalid;
        assert!(decode(&body)["answers"].get("pick").is_none());
    }
}
#[test]
fn score_requires_matching_legend_and_weighted_value() {
    for field in ["score", "confidence", "legend", "probabilities"] {
        let mut body = response();
        body["answers"]["rate"]
            .as_object_mut()
            .unwrap()
            .remove(field);
        assert!(decode(&body)["answers"].get("rate").is_none());
    }
    let mut body = response();
    body["answers"]["rate"]["score"] = json!(0.9);
    assert!(decode(&body)["answers"].get("rate").is_none());
    body = response();
    body["answers"]["rate"]["legend"]["0"] = json!("unexpected");
    assert!(decode(&body)["answers"].get("rate").is_none());
}
#[test]
fn duplicate_keys_malformed_json_trailing_data_and_scalar_envelopes_fail_closed() {
    for body in [
        r#"{"answers":{"yes":{"type":"noul","noul":0.1,"noul":0.9}}}"#,
        r#"{"answers":{},"answers":{"yes":{"type":"noul","noul":0.9}}}"#,
        r#"{"answers":{"yes":{"type":"noul","noul":1e999}}}"#,
        r#"{"answers":{}} true"#,
        "null",
        "[]",
        "{broken",
    ] {
        assert_eq!(
            request().decode(body.as_bytes()),
            Err(Error::InvalidResponse)
        );
    }
}
#[test]
fn exact_byte_limit_is_enforced_before_decoding() {
    let mut body = serde_json::to_vec(&response()).unwrap();
    body.resize(MAX_RESPONSE_BYTES, b' ');
    assert!(request().decode(&body).is_ok());
    body.push(b' ');
    assert_eq!(request().decode(&body), Err(Error::InvalidResponse));
}
#[test]
fn unexpected_names_are_ignored_and_missing_answers_are_not_fabricated() {
    let body = json!({"answers":{"unrequested":{"type":"noul","noul":0.9}}});
    assert_eq!(decode(&body)["answers"], json!({}));
}
#[test]
fn requests_reject_invalid_content_and_preserve_extension_fields() {
    let base = request().payload().clone();
    for field in ["model", "state", "questions"] {
        let mut value = base.clone();
        value.as_object_mut().unwrap().remove(field);
        assert!(Request::new(&value).is_err());
    }
    for value in [json!(null), json!(true), json!(12)] {
        let mut payload = base.clone();
        payload["state"] = value;
        assert!(Request::new(&payload).is_err());
    }
    let mut value = base;
    value["questions"]["yes"]["extension"] = json!({"version":1});
    assert_eq!(
        Request::new(&value).unwrap().payload()["questions"]["yes"]["extension"],
        json!({"version":1})
    );
}
#[test]
fn diagnostics_never_render_private_state() {
    assert!(!format!("{:?}", request()).contains("private fixture"));
    assert_eq!(
        Error::InvalidResponse.to_string(),
        "invalid decision response"
    );
}
