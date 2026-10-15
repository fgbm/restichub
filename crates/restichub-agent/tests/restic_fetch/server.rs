//! A local HTTPS server and a local CONNECT proxy for the restic download tests. The tests make no
//! request to the real network.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use hyper_util::rt::TokioIo;
use rcgen::{
    BasicConstraints, CertificateParams, CertifiedIssuer, DnType, ExtendedKeyUsagePurpose, IsCa,
    KeyPair, PKCS_ECDSA_P256_SHA256,
};
use rustls::RootCertStore;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::io::AsyncReadExt as _;
use tokio::net::{TcpListener, TcpStream};

pub struct Reply {
    pub status: u16,
    pub headers: Vec<(&'static str, String)>,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn ok(body: impl Into<Vec<u8>>) -> Self {
        Self {
            status: 200,
            headers: Vec::new(),
            body: body.into(),
        }
    }

    pub fn status(status: u16) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: Vec::new(),
        }
    }

    pub fn redirect(location: &str) -> Self {
        Self {
            status: 302,
            headers: vec![("location", location.to_owned())],
            body: Vec::new(),
        }
    }
}

type Handler = Arc<dyn Fn(&str) -> Reply + Send + Sync>;

pub struct TestServer {
    pub port: u16,
    pub roots: RootCertStore,
    pub requests: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestServer {
    pub fn url(&self, path: &str) -> String {
        format!("https://localhost:{}{path}", self.port)
    }

    pub fn paths(&self) -> Vec<String> {
        self.requests.lock().expect("lock").clone()
    }

    pub async fn start(handler: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let (ca_der, config) = test_tls();
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let handler: Handler = Arc::new(handler);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let log = requests.clone();
        let acceptor = tokio_rustls::TlsAcceptor::from(config);
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, _)) = listener.accept().await else {
                    break;
                };
                let acceptor = acceptor.clone();
                let handler = handler.clone();
                let log = log.clone();
                tokio::spawn(async move {
                    let Ok(tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    let service = hyper::service::service_fn(
                        move |request: hyper::Request<hyper::body::Incoming>| {
                            let handler = handler.clone();
                            let log = log.clone();
                            async move {
                                let target = request
                                    .uri()
                                    .path_and_query()
                                    .map_or_else(String::new, ToString::to_string);
                                log.lock().expect("lock").push(target.clone());
                                let reply = handler(&target);
                                let mut response = hyper::Response::builder().status(reply.status);
                                for (name, value) in reply.headers {
                                    response = response.header(name, value);
                                }
                                Ok::<_, std::convert::Infallible>(
                                    response
                                        .body(http_body_util::Full::new(bytes::Bytes::from(
                                            reply.body,
                                        )))
                                        .expect("response"),
                                )
                            }
                        },
                    );
                    let _ = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service)
                        .await;
                });
            }
        });
        let mut roots = RootCertStore::empty();
        roots.add(CertificateDer::from(ca_der)).expect("root");
        Self {
            port,
            roots,
            requests,
            task,
        }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn test_tls() -> (Vec<u8>, Arc<rustls::ServerConfig>) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let ca_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("CA key");
    let mut ca_params = CertificateParams::new(Vec::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "restichub-agent-fetch-test-ca");
    let ca = CertifiedIssuer::self_signed(ca_params, ca_key).expect("CA");

    let server_key = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256).expect("server key");
    let mut server_params =
        CertificateParams::new(vec!["localhost".to_owned()]).expect("server parameters");
    server_params
        .distinguished_name
        .push(DnType::CommonName, "localhost");
    server_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    let server_cert = server_params
        .signed_by(&server_key, &ca)
        .expect("server cert");
    let key = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der()));
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![CertificateDer::from(server_cert.der().to_vec())], key)
        .expect("server TLS configuration");
    (ca.der().to_vec(), Arc::new(config))
}

/// A proxy that accepts one `CONNECT` request at a time and splices to the requested target.
pub struct TestProxy {
    pub port: u16,
    pub connects: Arc<Mutex<Vec<String>>>,
    task: tokio::task::JoinHandle<()>,
}

impl TestProxy {
    pub async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let port = listener.local_addr().expect("address").port();
        let connects = Arc::new(Mutex::new(Vec::new()));
        let log = connects.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut client, _)) = listener.accept().await else {
                    break;
                };
                let log = log.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0_u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        if client.read_exact(&mut byte).await.is_err() {
                            return;
                        }
                        head.push(byte[0]);
                    }
                    let text = String::from_utf8_lossy(&head).into_owned();
                    let target = text
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or_default()
                        .to_owned();
                    log.lock().expect("lock").push(text);
                    let address: SocketAddr = match target.rsplit_once(':') {
                        Some((_, port)) => {
                            SocketAddr::from(([127, 0, 0, 1], port.parse().unwrap_or(0)))
                        }
                        None => return,
                    };
                    let Ok(mut upstream) = TcpStream::connect(address).await else {
                        return;
                    };
                    use tokio::io::AsyncWriteExt as _;
                    if client
                        .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                });
            }
        });
        Self {
            port,
            connects,
            task,
        }
    }
}

impl Drop for TestProxy {
    fn drop(&mut self) {
        self.task.abort();
    }
}
