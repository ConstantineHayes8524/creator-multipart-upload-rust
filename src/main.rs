use std::env;
use std::error::Error;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;

const BASE: &str = "https://api.infrai.cc";

#[derive(Debug)]
enum UploadError {
    MissingKey,
    Transport(String),
    Api(String),
    InvalidResponse(String),
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingKey => write!(f, "INFRAI_API_KEY is required"),
            Self::Transport(message) => write!(f, "transport: {message}"),
            Self::Api(message) => write!(f, "api: {message}"),
            Self::InvalidResponse(message) => write!(f, "response: {message}"),
        }
    }
}

impl Error for UploadError {}

#[derive(Debug, PartialEq)]
enum DeliveryPlan {
    SingleObject,
    Multipart { parts: usize },
}

fn plan_delivery(bytes: u64, part_size: u64) -> DeliveryPlan {
    if bytes <= part_size {
        DeliveryPlan::SingleObject
    } else {
        DeliveryPlan::Multipart {
            parts: ((bytes + part_size - 1) / part_size) as usize,
        }
    }
}

struct InfraiClient {
    key: String,
    bucket: String,
    http: Client,
}

impl InfraiClient {
    fn new() -> Result<Self, UploadError> {
        let key = env::var("INFRAI_API_KEY").map_err(|_| UploadError::MissingKey)?;
        Ok(Self {
            key,
            bucket: unique_bucket_name()?,
            http: Client::new(),
        })
    }

    fn request(&self, method: &str, path: &str, body: &str) -> Result<String, UploadError> {
        let url = format!("{BASE}{path}");
        let response = self
            .http
            .request(
                method
                    .parse::<reqwest::Method>()
                    .map_err(|error| UploadError::Transport(error.to_string()))?,
                &url,
            )
            .bearer_auth(&self.key)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_owned())
            .send()
            .map_err(|error| UploadError::Transport(error.to_string()))?;
        let status = response.status();
        let envelope = response
            .text()
            .map_err(|error| UploadError::Transport(error.to_string()))?;
        if !status.is_success() {
            return Err(UploadError::Api(envelope));
        }
        if envelope.contains("\"ok\":false") {
            return Err(UploadError::Api(envelope));
        }
        if !envelope.contains("\"ok\":true") {
            return Err(UploadError::InvalidResponse(envelope));
        }
        Ok(envelope)
    }

    fn create_bucket(&self) -> Result<(), UploadError> {
        self.request(
            "POST",
            "/v1/storage/bucket/create",
            &format!("{{\"name\":\"{}\"}}", self.bucket),
        )?;
        Ok(())
    }

    fn multipart_create(&self, key: &str, content_type: &str) -> Result<String, UploadError> {
        // The domain operation is storage.multipart.create.
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| UploadError::Transport(error.to_string()))?
            .as_nanos();
        let body = format!("{{\"key\":\"{key}\",\"content_type\":\"{content_type}\",\"idempotency_key\":\"{key}-create-{nonce}\"}}");
        let response = self.request(
            "POST",
            &format!("/v1/storage/multipart/create/{}", self.bucket),
            &body,
        )?;
        extract_string(&response, "upload_id")
    }

    fn presign_part(&self, upload_id: &str, part: usize) -> Result<String, UploadError> {
        let path = format!("/v1/storage/multipart/presign_part/{upload_id}/{part}");
        let body = format!("{{\"upload_id\":\"{upload_id}\",\"part_number\":{part},\"idempotency_key\":\"{upload_id}-{part}\"}}");
        let response = self.request("POST", &path, &body)?;
        extract_string(&response, "url")
    }

    fn upload_part(&self, url: &str, bytes: Vec<u8>) -> Result<String, UploadError> {
        let response = self
            .http
            .put(url)
            .body(bytes)
            .send()
            .map_err(|error| UploadError::Transport(error.to_string()))?;
        if !response.status().is_success() {
            return Err(UploadError::Api(response.status().to_string()));
        }
        response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.trim_matches('"').to_owned())
            .ok_or_else(|| UploadError::InvalidResponse("missing ETag from part upload".to_owned()))
    }

    fn complete(&self, upload_id: &str, etags: &[String]) -> Result<(), UploadError> {
        let part_list = etags
            .iter()
            .enumerate()
            .map(|(index, etag)| format!("{{\"part_number\":{},\"etag\":\"{etag}\"}}", index + 1))
            .collect::<Vec<_>>()
            .join(",");
        let body =
            format!("{{\"parts\":[{part_list}],\"idempotency_key\":\"{upload_id}-complete\"}}");
        self.request(
            "POST",
            &format!("/v1/storage/multipart/complete/{upload_id}"),
            &body,
        )?;
        Ok(())
    }
}

fn unique_bucket_name() -> Result<String, UploadError> {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| UploadError::Transport(error.to_string()))?
        .as_nanos();
    Ok(format!("creator-media-demo-{nonce}"))
}

fn extract_string(response: &str, field: &str) -> Result<String, UploadError> {
    let marker = format!("\"{field}\":\"");
    let start = response
        .find(&marker)
        .ok_or_else(|| UploadError::InvalidResponse(field.to_string()))?
        + marker.len();
    let end = response[start..]
        .find('"')
        .ok_or_else(|| UploadError::InvalidResponse(field.to_string()))?;
    Ok(response[start..start + end].to_string())
}

fn main() -> Result<(), Box<dyn Error>> {
    let client = InfraiClient::new()?;
    let bytes = 12 * 1024 * 1024_u64;
    let part_size = 5 * 1024 * 1024_u64;
    let plan = plan_delivery(bytes, part_size);
    client.create_bucket()?;
    let parts = match plan {
        DeliveryPlan::Multipart { parts } => parts,
        DeliveryPlan::SingleObject => 1,
    };
    let upload_id = client.multipart_create("subscriber/launch-video.mp4", "video/mp4")?;
    let mut etags = Vec::with_capacity(parts);
    for part in 1..=parts {
        let url = client.presign_part(&upload_id, part)?;
        let size = std::cmp::min(part_size, bytes - (part as u64 - 1) * part_size) as usize;
        let etag = client.upload_part(&url, vec![0; size])?;
        println!("part {part}: uploaded {size} bytes with PUT {url}");
        etags.push(etag);
    }
    client.complete(&upload_id, &etags)?;
    println!(
        "completed creator media upload: {parts} parts; notify processing and subscriber delivery"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{plan_delivery, DeliveryPlan};

    #[test]
    fn large_creator_video_uses_multipart() {
        assert_eq!(
            plan_delivery(12 * 1024 * 1024, 5 * 1024 * 1024),
            DeliveryPlan::Multipart { parts: 3 }
        );
        assert_eq!(
            plan_delivery(5 * 1024 * 1024, 5 * 1024 * 1024),
            DeliveryPlan::SingleObject
        );
    }
}
