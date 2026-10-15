//! Redirects, size bounds, certificate checks and the proxy tunnel of the download client (ADR-0038).

use restichub_agent::http_get::{HttpsClient, Proxy};
use rustls::RootCertStore;

use crate::server::{Reply, TestProxy, TestServer};

#[tokio::test]
async fn nfr_sec_06_a_get_returns_the_body() {
    let server = TestServer::start(|_| Reply::ok("hello")).await;
    let client = HttpsClient::new(server.roots.clone(), None);
    let response = client
        .get(&server.url("/x"), "*/*", 1024)
        .await
        .expect("get");
    assert_eq!(
        (response.status, response.body.as_slice()),
        (200, b"hello".as_slice())
    );
}

#[tokio::test]
async fn nfr_sec_06_a_redirect_to_https_is_followed() {
    let server = TestServer::start(|path| match path {
        "/start" => Reply::redirect("/final"),
        _ => Reply::ok("done"),
    })
    .await;
    let client = HttpsClient::new(server.roots.clone(), None);
    let response = client
        .get(&server.url("/start"), "*/*", 1024)
        .await
        .expect("get");
    assert_eq!(response.body, b"done");
    assert_eq!(server.paths(), ["/start", "/final"]);
}

#[tokio::test]
async fn nfr_sec_06_a_redirect_to_plain_http_is_refused() {
    let server = TestServer::start(|_| Reply::redirect("http://localhost/x")).await;
    let client = HttpsClient::new(server.roots.clone(), None);
    let error = client
        .get(&server.url("/start"), "*/*", 1024)
        .await
        .expect_err("plain http");
    assert!(error.to_string().contains("not an https://"), "{error:#}");
}

#[tokio::test]
async fn nfr_sec_06_a_redirect_loop_ends() {
    let server = TestServer::start(|_| Reply::redirect("/again")).await;
    let client = HttpsClient::new(server.roots.clone(), None);
    let error = client
        .get(&server.url("/again"), "*/*", 1024)
        .await
        .expect_err("loop");
    assert!(
        error.to_string().contains("redirected too many times"),
        "{error:#}"
    );
}

#[tokio::test]
async fn nfr_sec_06_a_body_above_the_bound_is_refused() {
    let server = TestServer::start(|_| Reply::ok(vec![0_u8; 4096])).await;
    let client = HttpsClient::new(server.roots.clone(), None);
    let error = client
        .get(&server.url("/big"), "*/*", 1024)
        .await
        .expect_err("too large");
    assert!(error.to_string().contains("too large"), "{error:#}");
}

#[tokio::test]
async fn nfr_sec_06_an_untrusted_certificate_is_refused() {
    let server = TestServer::start(|_| Reply::ok("x")).await;
    let client = HttpsClient::new(RootCertStore::empty(), None);
    let error = client
        .get(&server.url("/x"), "*/*", 1024)
        .await
        .expect_err("untrusted");
    assert!(format!("{error:#}").contains("TLS handshake"), "{error:#}");
}

#[tokio::test]
async fn fr_ag_11_the_download_goes_through_an_https_proxy_tunnel() {
    let server = TestServer::start(|_| Reply::ok("through")).await;
    let proxy = TestProxy::start().await;
    let parsed = Proxy::parse(
        &format!("http://user:pw@127.0.0.1:{}", proxy.port),
        Vec::new(),
    )
    .expect("proxy");
    let client = HttpsClient::new(server.roots.clone(), Some(parsed));
    let response = client
        .get(&server.url("/x"), "*/*", 1024)
        .await
        .expect("get");
    assert_eq!(response.body, b"through");
    let connects = proxy.connects.lock().expect("lock").clone();
    assert_eq!(connects.len(), 1);
    assert!(connects[0].starts_with(&format!("CONNECT localhost:{} HTTP/1.1", server.port)));
    assert!(connects[0].contains("Proxy-Authorization: Basic "));
}

#[tokio::test]
async fn fr_ag_11_a_refused_tunnel_is_an_error() {
    // The proxy listens, but the target port is closed, so the proxy drops the tunnel.
    let proxy = TestProxy::start().await;
    let parsed =
        Proxy::parse(&format!("http://127.0.0.1:{}", proxy.port), Vec::new()).expect("proxy");
    let client = HttpsClient::new(RootCertStore::empty(), Some(parsed));
    assert!(
        client
            .get("https://localhost:9/x", "*/*", 1024)
            .await
            .is_err()
    );
}
