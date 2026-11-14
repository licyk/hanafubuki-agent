//! A fake release server on `127.0.0.1`, on a thread and a runtime of its
//! own (so synchronous tests, async tests and child processes can all use
//! it): GitHub's latest-release answer at [`LATEST_PATH`], assets at
//! `/download/<name>`, any path answered as a test routes it, and every
//! request kept with its headers. An unrouted path answers 404. With it,
//! release archives ([`FakeArchive`]).

mod archive;

pub use archive::FakeArchive;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;
use reqwest::Url;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use crate::client::{ReleaseClient, UpdateProxy};

/// Where GitHub answers the latest release, on the fake server too.
pub const LATEST_PATH: &str = "/repos/licyk/hanafubuki-agent/releases/latest";
/// The lookup's own time bound, as [`ReleaseClient::for_this_build`] sets it.
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(30);
/// A request head larger than this is not read further.
const MAX_HEAD_BYTES: usize = 64 * 1024;

/// One request the server received.
#[derive(Clone, Debug)]
pub struct FakeRequest {
    pub method: String,
    /// With its query, if any.
    pub path: String,
    /// By lowercase name.
    pub headers: BTreeMap<String, String>,
}

/// One answer: a status, headers and a body, sent whole unless it is
/// delayed, paused or truncated.
#[derive(Clone, Debug)]
pub struct FakeResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    delay: Duration,
    pause: Option<(usize, Duration)>,
    truncate_at: Option<usize>,
}

impl FakeResponse {
    /// 200 with `body`.
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
            pause: None,
            truncate_at: None,
        }
    }

    /// 200 with the JSON `value`.
    pub fn json(value: &Value) -> Self {
        Self::ok(value.to_string()).with_header("content-type", "application/json")
    }

    /// `status` with a body in GitHub's error form.
    pub fn status(status: u16) -> Self {
        Self {
            status,
            ..Self::json(&json!({ "message": "fake error" }))
        }
    }

    /// 302 to `location`.
    pub fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            ..Self::ok(Vec::new())
        }
        .with_header("location", location)
    }

    pub fn with_header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    /// Sent only after `delay`.
    pub fn delayed(self, delay: Duration) -> Self {
        Self { delay, ..self }
    }

    /// The first `bytes` of the body sent, then nothing for `pause`, then
    /// the rest.
    pub fn paused_after(self, bytes: usize, pause: Duration) -> Self {
        Self {
            pause: Some((bytes, pause)),
            ..self
        }
    }

    /// Its `content-length` names the whole body, but the connection closes
    /// after its first `bytes`.
    pub fn truncated_at(self, bytes: usize) -> Self {
        Self {
            truncate_at: Some(bytes),
            ..self
        }
    }
}

/// An asset of a fake release: its body, and what the release says of it.
#[derive(Clone, Debug)]
pub struct FakeAsset {
    pub name: String,
    pub body: Vec<u8>,
    /// The size the release gives.
    pub size: u64,
    /// The digest the release gives, `sha256:<hex>`, or none.
    pub digest: Option<String>,
}

impl FakeAsset {
    /// `body`, listed with its own size and SHA-256 digest.
    pub fn new(name: &str, body: impl Into<Vec<u8>>) -> Self {
        let body = body.into();
        Self {
            name: name.to_string(),
            size: body.len() as u64,
            digest: Some(sha256_digest(&body)),
            body,
        }
    }

    pub fn without_digest(self) -> Self {
        Self {
            digest: None,
            ..self
        }
    }

    pub fn with_digest(self, digest: &str) -> Self {
        Self {
            digest: Some(digest.to_string()),
            ..self
        }
    }

    /// Listed with `size` instead of its body's.
    pub fn listed_size(self, size: u64) -> Self {
        Self { size, ..self }
    }
}

/// GitHub's `sha256:<hex>` digest of `bytes`.
pub fn sha256_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

/// The server; it stops when dropped.
pub struct FakeRelease {
    address: SocketAddr,
    routes: Arc<Mutex<BTreeMap<String, FakeResponse>>>,
    requests: Arc<Mutex<Vec<FakeRequest>>>,
    shutdown: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl FakeRelease {
    /// A server with no route yet.
    pub fn start() -> Self {
        let listener =
            std::net::TcpListener::bind(("127.0.0.1", 0)).expect("bind the fake release");
        listener
            .set_nonblocking(true)
            .expect("a non-blocking listener");
        let address = listener.local_addr().expect("the fake release's address");
        let routes = Arc::new(Mutex::new(BTreeMap::new()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let (shutdown, stopped) = oneshot::channel();
        let served = (routes.clone(), requests.clone());
        let thread = std::thread::Builder::new()
            .name("fake-release".to_string())
            .spawn(move || serve(listener, served.0, served.1, stopped))
            .expect("start the fake release");
        Self {
            address,
            routes,
            requests,
            shutdown: Some(shutdown),
            thread: Some(thread),
        }
    }

    /// `path` on this server, as a URL.
    pub fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.address)
    }

    /// The latest-release endpoint, for `HANAFUBUKI_AGENT_TEST_RELEASE_URL`.
    pub fn api_url(&self) -> String {
        self.url(LATEST_PATH)
    }

    /// Where [`Self::publish`] serves the asset `name`.
    pub fn asset_url(&self, name: &str) -> String {
        self.url(&format!("/download/{name}"))
    }

    /// Answers `path` (without its query) with `response`, from now on.
    pub fn route(&self, path: &str, response: FakeResponse) {
        self.routes.lock().insert(path.to_string(), response);
    }

    /// Answers the latest-release endpoint with `response`.
    pub fn answer_latest(&self, response: FakeResponse) {
        self.route(LATEST_PATH, response);
    }

    /// Publishes the release `tag` with `assets`: the latest-release answer
    /// ([`Self::release_json`]) and each asset's body at its URL.
    pub fn publish(&self, tag: &str, assets: &[FakeAsset]) {
        self.answer_latest(FakeResponse::json(&self.release_json(tag, assets)));
        for asset in assets {
            self.route(
                &format!("/download/{}", asset.name),
                FakeResponse::ok(asset.body.clone())
                    .with_header("content-type", "application/octet-stream"),
            );
        }
    }

    /// The latest-release answer for `tag` and `assets`, in GitHub's form
    /// (with some of the fields the lookup does not read), to serve as it is
    /// or altered.
    pub fn release_json(&self, tag: &str, assets: &[FakeAsset]) -> Value {
        let assets: Vec<Value> = assets
            .iter()
            .enumerate()
            .map(|(index, asset)| {
                json!({
                    "id": index + 1,
                    "name": asset.name,
                    "label": "",
                    "content_type": "application/octet-stream",
                    "state": "uploaded",
                    "size": asset.size,
                    "digest": asset.digest,
                    "download_count": 0,
                    "browser_download_url": self.asset_url(&asset.name),
                })
            })
            .collect();
        json!({
            "id": 1,
            "tag_name": tag,
            "target_commitish": "main",
            "name": format!("hanafubuki-agent {}", tag.trim_start_matches('v')),
            "draft": false,
            "immutable": false,
            "prerelease": false,
            "html_url": format!("https://github.com/licyk/hanafubuki-agent/releases/tag/{tag}"),
            "assets": assets,
            "body": "Notes.",
        })
    }

    /// Every request so far, in order.
    pub fn requests(&self) -> Vec<FakeRequest> {
        self.requests.lock().clone()
    }

    /// A client of this server's latest-release endpoint (no proxy), with
    /// `token` and the lookup's usual time bound.
    pub fn client(&self, token: Option<String>) -> ReleaseClient {
        self.client_with_timeout(token, LOOKUP_TIMEOUT)
    }

    /// [`Self::client`], the lookup bounded by `timeout`.
    pub fn client_with_timeout(&self, token: Option<String>, timeout: Duration) -> ReleaseClient {
        client_of(&self.api_url(), token, timeout)
    }
}

/// A client of the fake release whose latest-release endpoint is `api_url`
/// (from [`FakeRelease::api_url`], in another process), without a proxy:
/// a release build of a test reaches it too.
pub fn client_of(api_url: &str, token: Option<String>, timeout: Duration) -> ReleaseClient {
    let endpoint = Url::parse(api_url).expect("the fake release's URL");
    ReleaseClient::new(endpoint, true, &UpdateProxy::Disabled, token, timeout)
        .expect("a client of the fake release")
}

impl Drop for FakeRelease {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

type Routes = Arc<Mutex<BTreeMap<String, FakeResponse>>>;
type Requests = Arc<Mutex<Vec<FakeRequest>>>;

fn serve(
    listener: std::net::TcpListener,
    routes: Routes,
    requests: Requests,
    stopped: oneshot::Receiver<()>,
) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("the fake release's runtime");
    runtime.block_on(async move {
        let listener = TcpListener::from_std(listener).expect("the fake release's listener");
        tokio::pin!(stopped);
        loop {
            tokio::select! {
                _ = &mut stopped => return,
                accepted = listener.accept() => {
                    let Ok((stream, _)) = accepted else { continue };
                    tokio::spawn(answer(stream, routes.clone(), requests.clone()));
                }
            }
        }
    });
}

async fn answer(mut stream: TcpStream, routes: Routes, requests: Requests) {
    let Some(request) = read_request(&mut stream).await else {
        return;
    };
    let route = request
        .path
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    requests.lock().push(request);
    let response = routes
        .lock()
        .get(&route)
        .cloned()
        .unwrap_or_else(|| FakeResponse::status(404));
    tokio::time::sleep(response.delay).await;
    let mut head = format!(
        "HTTP/1.1 {} Fake\r\ncontent-length: {}\r\nconnection: close\r\n",
        response.status,
        response.body.len()
    );
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    if stream.write_all(head.as_bytes()).await.is_err() {
        return;
    }
    let body = &response.body[..response
        .truncate_at
        .unwrap_or(usize::MAX)
        .min(response.body.len())];
    let (first, rest) = match response.pause {
        Some((bytes, _)) => body.split_at(bytes.min(body.len())),
        None => (body, &[][..]),
    };
    let _ = stream.write_all(first).await;
    let _ = stream.flush().await;
    if let Some((_, pause)) = response.pause {
        tokio::time::sleep(pause).await;
    }
    let _ = stream.write_all(rest).await;
    let _ = stream.shutdown().await;
}

async fn read_request(stream: &mut TcpStream) -> Option<FakeRequest> {
    let mut bytes = Vec::new();
    let end = loop {
        let mut buffer = [0; 4096];
        let count = stream.read(&mut buffer).await.ok()?;
        if count == 0 || bytes.len() > MAX_HEAD_BYTES {
            return None;
        }
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(position) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break position;
        }
    };
    let head = String::from_utf8_lossy(&bytes[..end]);
    let mut lines = head.split("\r\n");
    let mut request_line = lines.next()?.split(' ');
    let method = request_line.next()?.to_string();
    let path = request_line.next()?.to_string();
    let headers = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(name, value)| (name.trim().to_ascii_lowercase(), value.trim().to_string()))
        .collect();
    Some(FakeRequest {
        method,
        path,
        headers,
    })
}
