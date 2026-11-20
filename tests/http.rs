//! Updater network layer: release parsing, verified downloads and redirects.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tiny_http::{Header, Response, Server};

use lamzu_battery::updater::UpdateError;
use lamzu_battery::updater::http::{HttpMeta, HttpTransport, UreqTransport, UrlPolicy};
use lamzu_battery::updater::{UpdateInfo, check_for_update, download_update};

struct TestServer {
    base: String,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

/// Serves a JSON document at `/latest.json`, a 302 at `/redirect` (to the
/// asset), a 404 at `/missing` and the asset bytes everywhere else.
fn start_server(json: Vec<u8>, asset: Vec<u8>) -> TestServer {
    let server = Server::http("127.0.0.1:0").expect("bind local server");
    let port = server.server_addr().to_ip().expect("tcp listener").port();
    let base = format!("http://127.0.0.1:{port}");
    let redirect_target = format!("{base}/asset.exe");
    let stop = Arc::new(AtomicBool::new(false));
    let stop_thread = Arc::clone(&stop);
    let handle = std::thread::spawn(move || {
        while !stop_thread.load(Ordering::SeqCst) {
            match server.recv_timeout(Duration::from_millis(50)) {
                Ok(Some(request)) => {
                    let url = request.url().to_owned();
                    let response = if url.contains("latest.json") {
                        Response::from_data(json.clone()).with_header(
                            Header::from_bytes("Content-Type", "application/json").expect("header"),
                        )
                    } else if url.contains("redirect") {
                        Response::from_data(Vec::new())
                            .with_status_code(302)
                            .with_header(
                                Header::from_bytes("Location", redirect_target.clone())
                                    .expect("header"),
                            )
                    } else if url.contains("missing") {
                        Response::from_data(Vec::new()).with_status_code(404)
                    } else {
                        Response::from_data(asset.clone())
                    };
                    let _ = request.respond(response);
                }
                Ok(None) => {}
                Err(_) => break,
            }
        }
    });
    TestServer {
        base,
        stop,
        handle: Some(handle),
    }
}

fn digest(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn release_json(asset_url: &str, size: u64, digest_field: Option<&str>) -> Vec<u8> {
    let digest_field =
        digest_field.map_or_else(String::new, |value| format!(r#""digest":"{value}","#));
    format!(
        r#"{{"tag_name":"v1.3.0","assets":[{{"name":"LamzuBattery.exe",{digest_field}"size":{size},"browser_download_url":"{asset_url}"}}]}}"#
    )
    .into_bytes()
}

#[test]
fn download_writes_a_verified_file() {
    let data = b"MZ-payload".to_vec();
    let server = start_server(Vec::new(), data.clone());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/asset.exe", server.base),
        asset_size: data.len() as u64,
        digest: digest(&data),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let mut progress = Vec::new();
    download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |done, total| progress.push((done, total)),
    )
    .expect("download");
    assert_eq!(std::fs::read(&destination).expect("read"), data);
    assert_eq!(
        progress.first().copied(),
        Some((data.len() as u64, data.len() as u64))
    );
    assert!(
        !destination
            .with_file_name("LamzuBattery.new.exe.part")
            .exists()
    );
}

#[test]
fn download_rejects_a_bad_checksum() {
    let data = b"MZ-payload".to_vec();
    let server = start_server(Vec::new(), data.clone());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/asset.exe", server.base),
        asset_size: data.len() as u64,
        digest: "0".repeat(64),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let error = download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect_err("must fail");
    assert!(!destination.exists());
    assert!(error.message().contains("checksum"), "{}", error.message());
}

#[test]
fn download_rejects_a_size_mismatch() {
    let data = b"MZ-payload".to_vec();
    let server = start_server(Vec::new(), data.clone());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/asset.exe", server.base),
        asset_size: data.len() as u64 + 10,
        digest: digest(&data),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let error = download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect_err("must fail");
    assert!(error.message().contains("size"), "{}", error.message());
}

#[test]
fn download_rejects_a_non_executable() {
    let data = b"NOT-AN-EXE".to_vec();
    let server = start_server(Vec::new(), data.clone());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/asset.exe", server.base),
        asset_size: data.len() as u64,
        digest: digest(&data),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let error = download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect_err("must fail");
    assert!(
        error.message().contains("executable"),
        "{}",
        error.message()
    );
}

#[test]
fn download_follows_a_trusted_redirect() {
    let data = b"MZ-redirected".to_vec();
    let server = start_server(Vec::new(), data.clone());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/redirect", server.base),
        asset_size: data.len() as u64,
        digest: digest(&data),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect("redirected download");
    assert_eq!(std::fs::read(&destination).expect("read"), data);
}

#[test]
fn download_rejects_an_untrusted_redirect() {
    let data = b"MZ-redirected".to_vec();
    let server = start_server(Vec::new(), data);
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/redirect", server.base),
        asset_size: 13,
        digest: "0".repeat(64),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let error = download_update(
        &UreqTransport,
        &UrlPolicy::github(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect_err("must fail");
    assert!(
        error.message().contains("untrusted download redirect"),
        "{}",
        error.message()
    );
}

#[test]
fn download_reports_http_errors() {
    let server = start_server(Vec::new(), Vec::new());
    let info = UpdateInfo {
        version: "1.3.0".to_owned(),
        asset_url: format!("{}/missing", server.base),
        asset_size: 1,
        digest: "0".repeat(64),
    };
    let directory = tempfile::tempdir().expect("temp dir");
    let destination = directory.path().join("LamzuBattery.new.exe");
    let error = download_update(
        &UreqTransport,
        &UrlPolicy::permissive(),
        &info,
        &destination,
        &mut |_, _| {},
    )
    .expect_err("must fail");
    assert!(error.message().contains("HTTP 404"), "{}", error.message());
}

#[test]
fn check_for_update_parses_a_github_release() {
    let data = b"MZ-new".to_vec();
    let json = release_json(
        "https://github.com/lavandiI/lamzu-battery/releases/download/v1/LamzuBattery.exe",
        data.len() as u64,
        Some(&format!("sha256:{}", digest(&data))),
    );
    let transport = JsonTransport { body: json };
    let policy = UrlPolicy::github();
    let api = "https://api.github.com/repos/lavandiI/lamzu-battery/releases/latest";

    let info = check_for_update(&transport, &policy, "1.2.1", api)
        .expect("check")
        .expect("available");
    assert_eq!(info.version, "1.3.0");
    assert_eq!(info.asset_size, data.len() as u64);
    assert_eq!(info.digest, digest(&data));
    assert!(
        check_for_update(&transport, &policy, "1.3.0", api)
            .expect("check")
            .is_none()
    );
}

#[test]
fn check_for_update_requires_a_trusted_url() {
    let transport = JsonTransport {
        body: release_json(
            "https://example.com/LamzuBattery.exe",
            10,
            Some(&"a".repeat(64)),
        ),
    };
    let error = check_for_update(
        &transport,
        &UrlPolicy::github(),
        "1.2.1",
        "https://api.github.com/x",
    )
    .expect_err("untrusted");
    assert!(error.message().contains("untrusted"), "{}", error.message());
}

#[test]
fn check_for_update_requires_a_digest() {
    let transport = JsonTransport {
        body: release_json(
            "https://github.com/lavandiI/lamzu-battery/releases/download/v1/LamzuBattery.exe",
            10,
            None,
        ),
    };
    let error = check_for_update(
        &transport,
        &UrlPolicy::github(),
        "1.2.1",
        "https://api.github.com/x",
    )
    .expect_err("no digest");
    assert!(error.message().contains("digest"), "{}", error.message());
}

#[test]
fn check_for_update_rejects_a_release_without_the_asset() {
    let transport = JsonTransport {
        body: br#"{"tag_name":"v1.3.0","assets":[]}"#.to_vec(),
    };
    let error = check_for_update(
        &transport,
        &UrlPolicy::github(),
        "1.2.1",
        "https://api.github.com/x",
    )
    .expect_err("no asset");
    assert!(
        error.message().contains("LamzuBattery.exe"),
        "{}",
        error.message()
    );
}

struct JsonTransport {
    body: Vec<u8>,
}

impl HttpTransport for JsonTransport {
    fn get_stream(
        &self,
        _url: &str,
        _accept: &str,
        _timeout: Duration,
        sink: &mut dyn FnMut(&[u8]) -> Result<(), UpdateError>,
    ) -> Result<HttpMeta, UpdateError> {
        sink(&self.body)?;
        Ok(HttpMeta {
            status: 200,
            location: None,
        })
    }
}
