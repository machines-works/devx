use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU16, Ordering};

use crate::events::DevxEvent;
use anyhow::Result;
use bytes::Bytes;
use http_body_util::{Either, Full};
use hyper::body::Incoming;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

pub struct ServiceProxy {
    pub service_name: String,
    pub listen_port: u16,
    pub target_port: Arc<AtomicU16>,
    pub tls_config: Option<Arc<rustls::ServerConfig>>,
}

impl ServiceProxy {
    pub async fn run(self, event_tx: mpsc::Sender<DevxEvent>) -> Result<()> {
        let listener = TcpListener::bind(("127.0.0.1", self.listen_port)).await?;

        let _ = event_tx.try_send(DevxEvent::ProxyBound {
            service: self.service_name.clone(),
            proxy_port: self.listen_port,
            target_port: self.target_port.load(Ordering::Relaxed),
        });

        let tls_acceptor = self.tls_config.map(TlsAcceptor::from);
        let target_port = self.target_port;

        loop {
            let (stream, _addr) = listener.accept().await?;
            let current_target = target_port.load(Ordering::Relaxed);
            let tls_acceptor = tls_acceptor.clone();
            let tx = event_tx.clone();

            tokio::spawn(async move {
                if let Some(acceptor) = tls_acceptor {
                    let tls_stream = match acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = tx.try_send(DevxEvent::LogLine {
                                service: "devx".to_string(),
                                line: format!("[proxy] TLS handshake error: {}", e),
                                is_stderr: true,
                            });
                            return;
                        }
                    };
                    let io = TokioIo::new(tls_stream);
                    let result = http1::Builder::new()
                        .preserve_header_case(true)
                        .serve_connection(
                            io,
                            service_fn(move |req| forward_to(req, current_target)),
                        )
                        .with_upgrades()
                        .await;
                    if let Err(e) = result {
                        let _ = tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: format!("[proxy] connection error: {}", e),
                            is_stderr: true,
                        });
                    }
                } else {
                    let io = TokioIo::new(stream);
                    let result = http1::Builder::new()
                        .preserve_header_case(true)
                        .serve_connection(
                            io,
                            service_fn(move |req| forward_to(req, current_target)),
                        )
                        .with_upgrades()
                        .await;
                    if let Err(e) = result {
                        let _ = tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: format!("[proxy] connection error: {}", e),
                            is_stderr: true,
                        });
                    }
                }
            });
        }
    }
}

pub struct VhostProxy {
    /// domain -> target port (service's actual port, updated atomically on restart)
    pub routes: HashMap<String, Arc<AtomicU16>>,
    /// (domain, service_name) pairs for event reporting
    pub domain_services: Vec<(String, String)>,
    pub tls_config: Option<Arc<rustls::ServerConfig>>,
}

impl VhostProxy {
    pub async fn run(self, event_tx: mpsc::Sender<DevxEvent>) -> Result<()> {
        let (listener, _default_port) = if self.tls_config.is_some() {
            // Try 443 first, fall back to 8443
            match TcpListener::bind(("127.0.0.1", 443)).await {
                Ok(l) => (l, 443u16),
                Err(_) => match TcpListener::bind(("127.0.0.1", 8443)).await {
                    Ok(l) => (l, 8443),
                    Err(e) => {
                        let msg = format!(
                            "[vhost] failed to bind port 443 or 8443: {} — vhost routing disabled",
                            e
                        );
                        let _ = event_tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: msg,
                            is_stderr: true,
                        });
                        return Ok(());
                    }
                },
            }
        } else {
            match TcpListener::bind(("127.0.0.1", 80)).await {
                Ok(l) => (l, 80u16),
                Err(_) => match TcpListener::bind(("127.0.0.1", 8080)).await {
                    Ok(l) => (l, 8080),
                    Err(e) => {
                        let msg = format!(
                            "[vhost] failed to bind port 80 or 8080: {} — vhost routing disabled",
                            e
                        );
                        let _ = event_tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: msg,
                            is_stderr: true,
                        });
                        return Ok(());
                    }
                },
            }
        };

        let bound_port = listener.local_addr()?.port();

        let _ = event_tx.try_send(DevxEvent::VhostBound {
            port: bound_port,
            domains: self.domain_services,
            tls: self.tls_config.is_some(),
        });

        let routes = Arc::new(self.routes);
        let tls_acceptor = self.tls_config.map(TlsAcceptor::from);

        loop {
            let (stream, _addr) = listener.accept().await?;
            let routes = routes.clone();
            let tls_acceptor = tls_acceptor.clone();
            let tx = event_tx.clone();

            tokio::spawn(async move {
                if let Some(acceptor) = tls_acceptor {
                    let tls_stream = match acceptor.accept(stream).await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = tx.try_send(DevxEvent::LogLine {
                                service: "devx".to_string(),
                                line: format!("[vhost] TLS handshake error: {}", e),
                                is_stderr: true,
                            });
                            return;
                        }
                    };
                    let io = TokioIo::new(tls_stream);
                    let routes = routes.clone();
                    let result = http1::Builder::new()
                        .preserve_header_case(true)
                        .serve_connection(
                            io,
                            service_fn(move |req| {
                                let routes = routes.clone();
                                async move { vhost_route(req, &routes).await }
                            }),
                        )
                        .with_upgrades()
                        .await;
                    if let Err(e) = result {
                        let _ = tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: format!("[vhost] connection error: {}", e),
                            is_stderr: true,
                        });
                    }
                } else {
                    let io = TokioIo::new(stream);
                    let routes = routes.clone();
                    let result = http1::Builder::new()
                        .preserve_header_case(true)
                        .serve_connection(
                            io,
                            service_fn(move |req| {
                                let routes = routes.clone();
                                async move { vhost_route(req, &routes).await }
                            }),
                        )
                        .with_upgrades()
                        .await;
                    if let Err(e) = result {
                        let _ = tx.try_send(DevxEvent::LogLine {
                            service: "devx".to_string(),
                            line: format!("[vhost] connection error: {}", e),
                            is_stderr: true,
                        });
                    }
                }
            });
        }
    }
}

async fn vhost_route(
    req: Request<Incoming>,
    routes: &HashMap<String, Arc<AtomicU16>>,
) -> Result<Response<Either<Full<Bytes>, Incoming>>, hyper::Error> {
    let host = req
        .headers()
        .get(hyper::header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Strip port suffix if present (e.g. "arpid.localhost:8080" -> "arpid.localhost")
    let domain = host.split(':').next().unwrap_or(host);

    if let Some(target_port) = routes.get(domain) {
        let port = target_port.load(Ordering::Relaxed);
        forward_to(req, port).await
    } else {
        let body = format!("unknown domain: {}", domain);
        Ok(Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Either::Left(Full::new(Bytes::from(body))))
            .unwrap())
    }
}

fn bad_gateway() -> Response<Either<Full<Bytes>, Incoming>> {
    Response::builder()
        .status(StatusCode::BAD_GATEWAY)
        .body(Either::Left(Full::new(Bytes::from("service starting..."))))
        .unwrap()
}

async fn forward_to(
    req: Request<Incoming>,
    target_port: u16,
) -> Result<Response<Either<Full<Bytes>, Incoming>>, hyper::Error> {
    let is_upgrade = req
        .headers()
        .get(hyper::header::UPGRADE)
        .map(|v| {
            v.to_str()
                .unwrap_or("")
                .to_lowercase()
                .contains("websocket")
        })
        .unwrap_or(false);

    let target_addr = format!("127.0.0.1:{}", target_port);

    let upstream_stream = match tokio::net::TcpStream::connect(&target_addr).await {
        Ok(s) => s,
        Err(_) => return Ok(bad_gateway()),
    };

    let upstream_io = TokioIo::new(upstream_stream);
    let (mut sender, conn) = match hyper::client::conn::http1::Builder::new()
        .preserve_header_case(true)
        .handshake(upstream_io)
        .await
    {
        Ok(pair) => pair,
        Err(_) => return Ok(bad_gateway()),
    };

    tokio::spawn(async move {
        let result = if is_upgrade {
            conn.with_upgrades().await
        } else {
            conn.await
        };
        if let Err(e) = result {
            eprintln!("[proxy] upstream conn error: {}", e);
        }
    });

    let (parts, body) = req.into_parts();
    let upstream_req = Request::from_parts(parts, body);

    let upstream_resp = match sender.send_request(upstream_req).await {
        Ok(r) => r,
        Err(_) => return Ok(bad_gateway()),
    };

    let (parts, body) = upstream_resp.into_parts();
    Ok(Response::from_parts(parts, Either::Right(body)))
}
