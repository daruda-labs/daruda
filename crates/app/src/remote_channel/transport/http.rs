use super::{Result, TransportError};
use serde_json::Value;
use std::{io::Read, time::Duration};

pub const MAX_BODY_BYTES: usize = 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const ACK_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RETRIES: usize = 3;
const MAX_RETRY_DELAY: f64 = 60.0;

pub fn request(
    method: &str,
    url: &str,
    authorization: Option<&str>,
    payload: &Value,
) -> Result<Value> {
    call(
        method,
        url,
        authorization,
        payload,
        REQUEST_TIMEOUT,
        MAX_RETRIES,
    )
}

pub fn acknowledge(url: &str, payload: &Value) -> Result<Value> {
    call("POST", url, None, payload, ACK_TIMEOUT, 0)
}

fn call(
    method: &str,
    url: &str,
    authorization: Option<&str>,
    payload: &Value,
    timeout: Duration,
    retries: usize,
) -> Result<Value> {
    if cfg!(test) {
        return Err(TransportError::Message(
            "Network calls are disabled in tests".into(),
        ));
    }
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(0)
        .build();
    let mut attempt = 0;
    loop {
        let mut request = agent.request(method, url);
        if let Some(auth) = authorization {
            request = request.set("Authorization", auth);
        }
        let response = if method == "GET" {
            request.call()
        } else {
            request.send_json(payload.clone())
        };
        let response = match response {
            Ok(response) => response,
            Err(ureq::Error::Status(429, response)) if attempt < retries => {
                let header = response
                    .header("Retry-After")
                    .and_then(|s| s.parse::<f64>().ok());
                let body = read(response).unwrap_or(Value::Null);
                let seconds = header
                    .or_else(|| body["retry_after"].as_f64())
                    .unwrap_or(1.0);
                if !seconds.is_finite() || !(0.0..=MAX_RETRY_DELAY).contains(&seconds) {
                    return Err(TransportError::Message(
                        "Remote API requested a long retry delay".into(),
                    ));
                }
                std::thread::sleep(Duration::from_secs_f64(seconds.max(0.1)));
                attempt += 1;
                continue;
            }
            // The budget is spent. Reported as such rather than as a bare
            // "HTTP 429", which reads as a single rejection.
            Err(ureq::Error::Status(429, _)) => {
                return Err(TransportError::Message(
                    "Remote API retry limit reached".into(),
                ));
            }
            Err(ureq::Error::Status(status, _)) => {
                return Err(TransportError::Message(format!(
                    "Remote API returned HTTP {status}"
                )));
            }
            // URLs can contain live Socket Mode tickets or interaction tokens.
            Err(ureq::Error::Transport(_)) => {
                return Err(TransportError::Message(
                    "Remote API connection failed".into(),
                ));
            }
        };
        if response.status() == 204 {
            return Ok(Value::Null);
        }
        return read(response);
    }
}

fn read(response: ureq::Response) -> Result<Value> {
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_BODY_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| TransportError::Message("Remote API response could not be read".into()))?;
    if bytes.len() > MAX_BODY_BYTES {
        return Err(TransportError::Message(
            "Remote API response exceeded the size limit".into(),
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| TransportError::Message("Remote API returned invalid JSON".into()))
}

pub fn required<'a>(value: &'a Value, key: &str) -> Result<&'a str> {
    value[key]
        .as_str()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| TransportError::Message(format!("Remote API response is missing {key}")))
}

#[cfg(test)]
mod tests {
    #[test]
    fn invalid_required_fields_do_not_expose_payloads() {
        let result = super::required(&serde_json::json!({"token": "secret"}), "url").unwrap_err();
        assert!(!result.to_string().contains("secret"));
    }
}
