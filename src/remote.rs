//! Bounded network provider for images embedded in email HTML.
//!
//! Embedded `data:` images are always available because they are already part
//! of the message. HTTP(S) loading is a separate, feature- and consent-gated
//! capability with strict byte/pixel budgets and public-address pinning.

use blitz_traits::net::{Bytes, NetHandler, NetProvider, NetWaker, Request};
use image::{ImageReader, Limits};
#[cfg(feature = "remote-content")]
use reqwest::redirect::Policy;
use std::{
    collections::HashMap,
    io::Cursor,
    sync::{Arc, Mutex},
};
#[cfg(feature = "remote-content")]
use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};
#[cfg(feature = "remote-content")]
use tokio::sync::Semaphore;

const MAX_RESOURCE_BYTES: usize = 5 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 4_096;
const MAX_IMAGE_PIXELS: u64 = 8 * 1024 * 1024;
const MAX_DOCUMENT_BYTES: u64 = 20 * 1024 * 1024;
const MAX_DOCUMENT_PIXELS: u64 = 16 * 1024 * 1024;
const MAX_DOCUMENT_REQUESTS: u8 = 32;
#[cfg(feature = "remote-content")]
const MAX_CONCURRENT_REQUESTS: usize = 4;
const MAX_TRACKED_DOCUMENTS: usize = 128;

#[derive(Default)]
struct DocumentBudget {
    requests: u8,
    bytes: u64,
    pixels: u64,
}

struct LimitedImageProvider {
    #[cfg(feature = "remote-content")]
    allow_remote: bool,
    waker: Arc<dyn NetWaker>,
    #[cfg(feature = "remote-content")]
    permits: Arc<Semaphore>,
    budgets: Arc<Mutex<HashMap<usize, DocumentBudget>>>,
}

pub fn email_image_provider(
    waker: Arc<dyn NetWaker>,
    allow_remote: bool,
) -> Result<Arc<dyn NetProvider>, String> {
    #[cfg(not(feature = "remote-content"))]
    let _ = allow_remote;
    Ok(Arc::new(LimitedImageProvider {
        #[cfg(feature = "remote-content")]
        allow_remote: allow_remote && cfg!(feature = "remote-content"),
        waker,
        #[cfg(feature = "remote-content")]
        permits: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
        budgets: Arc::new(Mutex::new(HashMap::new())),
    }))
}

impl NetProvider for LimitedImageProvider {
    fn fetch(&self, document_id: usize, request: Request, handler: Box<dyn NetHandler>) {
        if request.method != blitz_traits::net::http::Method::GET
            || !reserve_request(&self.budgets, document_id)
        {
            self.waker.wake(document_id);
            return;
        }

        let waker = Arc::clone(&self.waker);
        #[cfg(feature = "remote-content")]
        let permits = Arc::clone(&self.permits);
        let budgets = Arc::clone(&self.budgets);
        #[cfg(feature = "remote-content")]
        let allow_remote = self.allow_remote;
        tokio::spawn(async move {
            let bytes = match request.url.scheme() {
                "data" => load_data_image(request.url.as_str()),
                #[cfg(feature = "remote-content")]
                "http" | "https" if allow_remote && safe_remote_url_shape(&request.url) => {
                    let Ok(_permit) = permits.acquire().await else {
                        waker.wake(document_id);
                        return;
                    };
                    load_http_image(&request).await
                }
                _ => None,
            };

            if let Some(bytes) = bytes
                && let Some(pixels) = validated_pixel_count(&bytes)
                && reserve_image(&budgets, document_id, bytes.len() as u64, pixels)
            {
                handler.bytes(request.url.to_string(), Bytes::from(bytes));
            }
            waker.wake(document_id);
        });
    }
}

#[cfg(feature = "remote-content")]
async fn load_http_image(request: &Request) -> Option<Vec<u8>> {
    if request
        .signal
        .as_ref()
        .is_some_and(|signal| signal.aborted())
    {
        return None;
    }
    let mut url = request.url.clone();
    let mut redirects = 0_u8;
    let mut response = loop {
        let client = pinned_public_client(&url).await?;
        let response = client.get(url.clone()).send().await.ok()?;
        if !response.status().is_redirection() {
            break response;
        }
        if redirects >= 3 {
            return None;
        }
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok())?;
        url = url.join(location).ok()?;
        redirects += 1;
    };
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_RESOURCE_BYTES as u64)
    {
        return None;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();
    if !content_type.starts_with("image/") || content_type.starts_with("image/svg") {
        return None;
    }

    let mut bytes = Vec::with_capacity(
        response
            .content_length()
            .unwrap_or(0)
            .min(MAX_RESOURCE_BYTES as u64) as usize,
    );
    while let Some(chunk) = response.chunk().await.ok()? {
        if request
            .signal
            .as_ref()
            .is_some_and(|signal| signal.aborted())
            || bytes.len().saturating_add(chunk.len()) > MAX_RESOURCE_BYTES
        {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some(bytes)
}

/// Resolve and pin one public address before opening the connection. This
/// prevents a hostname or redirect from reaching loopback, LAN, link-local, or
/// metadata services through DNS rebinding.
#[cfg(feature = "remote-content")]
async fn pinned_public_client(url: &reqwest::Url) -> Option<reqwest::Client> {
    if !safe_remote_url_shape(url) {
        return None;
    }
    let port = url.port_or_known_default()?;
    let (addresses, resolve_host): (Vec<SocketAddr>, Option<String>) = match url.host()? {
        url::Host::Domain(host) => (
            tokio::net::lookup_host((host, port)).await.ok()?.collect(),
            Some(host.to_owned()),
        ),
        url::Host::Ipv4(address) => {
            (vec![SocketAddr::new(address.into(), port)], None)
        }
        url::Host::Ipv6(address) => {
            (vec![SocketAddr::new(address.into(), port)], None)
        }
    };
    if addresses.is_empty()
        || addresses
            .iter()
            .any(|address| !safe_remote_ip(address.ip()))
    {
        return None;
    }
    let pinned = addresses[0];
    let mut builder = reqwest::Client::builder()
        .user_agent("Flectar Mail bounded remote image loader/0.1")
        .connect_timeout(Duration::from_secs(4))
        .timeout(Duration::from_secs(12))
        .redirect(Policy::none());
    if let Some(host) = resolve_host {
        builder = builder.resolve(&host, pinned);
    }
    builder.build().ok()
}

fn load_data_image(url: &str) -> Option<Vec<u8>> {
    // Base64 overhead is 4/3; this pre-check bounds allocation even before
    // the streaming decoder sees the data.
    if url.len() > MAX_RESOURCE_BYTES.saturating_mul(2) {
        return None;
    }
    let data = data_url::DataUrl::process(url).ok()?;
    if data.mime_type().type_ != "image" || data.mime_type().subtype == "svg+xml" {
        return None;
    }
    let (bytes, _) = data.decode_to_vec().ok()?;
    (bytes.len() <= MAX_RESOURCE_BYTES).then_some(bytes)
}

fn validated_pixel_count(bytes: &[u8]) -> Option<u64> {
    let mut reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(MAX_IMAGE_DIMENSION);
    limits.max_image_height = Some(MAX_IMAGE_DIMENSION);
    limits.max_alloc = Some(MAX_IMAGE_PIXELS * 4);
    reader.limits(limits);
    let (width, height) = reader.into_dimensions().ok()?;
    let pixels = u64::from(width).checked_mul(u64::from(height))?;
    (pixels <= MAX_IMAGE_PIXELS).then_some(pixels)
}

fn reserve_request(budgets: &Mutex<HashMap<usize, DocumentBudget>>, document_id: usize) -> bool {
    // A panic in unrelated remote-image work must not turn every later image
    // request into a second panic. The counters remain conservative if a
    // poisoned lock is recovered.
    let mut budgets = budgets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if budgets.len() >= MAX_TRACKED_DOCUMENTS && !budgets.contains_key(&document_id) {
        budgets.clear();
    }
    let budget = budgets.entry(document_id).or_default();
    if budget.requests >= MAX_DOCUMENT_REQUESTS {
        return false;
    }
    budget.requests += 1;
    true
}

fn reserve_image(
    budgets: &Mutex<HashMap<usize, DocumentBudget>>,
    document_id: usize,
    bytes: u64,
    pixels: u64,
) -> bool {
    let mut budgets = budgets
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let budget = budgets.entry(document_id).or_default();
    let Some(total_bytes) = budget.bytes.checked_add(bytes) else {
        return false;
    };
    let Some(total_pixels) = budget.pixels.checked_add(pixels) else {
        return false;
    };
    if total_bytes > MAX_DOCUMENT_BYTES || total_pixels > MAX_DOCUMENT_PIXELS {
        return false;
    }
    budget.bytes = total_bytes;
    budget.pixels = total_pixels;
    true
}

#[cfg(feature = "remote-content")]
fn safe_remote_url_shape(url: &reqwest::Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return false;
    }
    if !cfg!(test) {
        let Some(port) = url.port_or_known_default() else {
            return false;
        };
        if !matches!((url.scheme(), port), ("http", 80) | ("https", 443)) {
            return false;
        }
    }
    match url.host() {
        Some(url::Host::Ipv4(address)) => safe_remote_ip(address.into()),
        Some(url::Host::Ipv6(address)) => safe_remote_ip(address.into()),
        Some(url::Host::Domain(_)) => true,
        None => false,
    }
}

#[cfg(feature = "remote-content")]
fn safe_remote_ip(ip: IpAddr) -> bool {
    if cfg!(test) {
        return true;
    }
    match ip {
        IpAddr::V4(ip) => {
            let octets = ip.octets();
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_documentation()
                || octets[0] == 0
                || (octets[0] == 100 && (64..=127).contains(&octets[1]))
                || (octets[0] == 192 && octets[1] == 0 && octets[2] == 0)
                || (octets[0] == 198 && matches!(octets[1], 18 | 19))
                || octets[0] >= 240)
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unique_local()
                || ip.is_unicast_link_local()
                || ip.is_multicast()
                || ip.is_unspecified()
                || ip.segments()[..2] == [0x2001, 0x0db8])
                && ip
                    .to_ipv4_mapped()
                    .is_none_or(|mapped| safe_remote_ip(mapped.into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_budget_is_bounded() {
        let budgets = Mutex::new(HashMap::new());
        assert!(reserve_image(
            &budgets,
            7,
            MAX_DOCUMENT_BYTES,
            MAX_DOCUMENT_PIXELS
        ));
        assert!(!reserve_image(&budgets, 7, 1, 1));
        for _ in 0..MAX_DOCUMENT_REQUESTS {
            assert!(reserve_request(&budgets, 8));
        }
        assert!(!reserve_request(&budgets, 8));
    }

    #[test]
    #[cfg(feature = "remote-content")]
    fn remote_url_shape_rejects_credentials() {
        let url = reqwest::Url::parse("https://user:secret@example.com/pixel.png").unwrap();
        assert!(!safe_remote_url_shape(&url));
    }
}
