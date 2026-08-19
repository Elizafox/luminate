// SPDX-License-Identifier: GPL-3.0-or-later
// SPDX-FileCopyrightText: 2026 Elizabeth Kiara Regina Ashford

//! Hyper serving path with bounded HTTP parsing and idle I/O.

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::task::{Context, Poll};
use std::time::Duration;

use axum::extract::ConnectInfo;
use axum::{Extension, Router};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::server::graceful::GracefulShutdown;
use hyper_util::service::TowerToHyperService;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::task::JoinSet;
use tokio::time::{Instant, Sleep, sleep};

use crate::PeerAddress;
use crate::resource::HttpLimits;
use crate::transport::BoundedListener;

pub(crate) async fn serve(
    mut listener: BoundedListener,
    app: Router,
    limits: HttpLimits,
    shutdown: impl Future<Output = ()>,
) -> anyhow::Result<()> {
    tokio::pin!(shutdown);
    let mut connections = JoinSet::new();
    let graceful = GracefulShutdown::new();
    loop {
        tokio::select! {
            biased;
            () = &mut shutdown => break,
            (connection, peer) = listener.accept_connection() => {
                let upgraded = UpgradeFlag::default();
                let service = TowerToHyperService::new(
                    app.clone()
                        .layer(Extension(upgraded.clone()))
                        .layer(Extension(ConnectInfo(PeerAddress(peer)))),
                );
                let watcher = graceful.watcher();
                connections.spawn(async move {
                    let io = TokioIo::new(IdleIo::new(
                        connection,
                        limits.keep_alive_idle,
                        upgraded.0,
                    ));
                    let mut builder = Builder::new(TokioExecutor::new());
                    builder
                        .http1()
                        .timer(TokioTimer::new())
                        .header_read_timeout(limits.request_header_timeout)
                        .max_headers(limits.maximum_headers)
                        .max_buf_size(limits.maximum_header_bytes);
                    builder
                        .http2()
                        .max_header_list_size(u32::try_from(limits.maximum_header_bytes).unwrap_or(u32::MAX));
                    let connection = builder.serve_connection_with_upgrades(io, service);
                    if let Err(error) = watcher.watch(connection).await {
                        tracing::debug!(%peer, %error, "HTTP connection ended with an error");
                    }
                });
            }
        }
    }
    graceful.shutdown().await;
    while connections.join_next().await.is_some() {}
    Ok(())
}

struct IdleIo<T> {
    inner: T,
    timeout: Duration,
    sleep: Pin<Box<Sleep>>,
    upgraded: Arc<AtomicBool>,
}

impl<T> IdleIo<T> {
    fn new(inner: T, timeout: Duration, upgraded: Arc<AtomicBool>) -> Self {
        Self {
            inner,
            timeout,
            sleep: Box::pin(sleep(timeout)),
            upgraded,
        }
    }

    fn poll_timeout(&mut self, cx: &mut Context<'_>) -> io::Result<()> {
        if !self.upgraded.load(Ordering::Relaxed) && self.sleep.as_mut().poll(cx).is_ready() {
            Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "HTTP connection was idle",
            ))
        } else {
            Ok(())
        }
    }

    fn reset(&mut self) {
        self.sleep.as_mut().reset(Instant::now() + self.timeout);
    }
}

#[derive(Clone, Default)]
pub(crate) struct UpgradeFlag(Arc<AtomicBool>);

impl UpgradeFlag {
    pub(crate) fn mark(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for IdleIo<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if let Err(error) = self.poll_timeout(cx) {
            return Poll::Ready(Err(error));
        }
        let before = buffer.filled().len();
        let result = Pin::new(&mut self.inner).poll_read(cx, buffer);
        if matches!(result, Poll::Ready(Ok(()))) && buffer.filled().len() > before {
            self.reset();
        }
        result
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for IdleIo<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        if let Err(error) = self.poll_timeout(cx) {
            return Poll::Ready(Err(error));
        }
        let result = Pin::new(&mut self.inner).poll_write(cx, buffer);
        if matches!(result, Poll::Ready(Ok(written)) if written > 0) {
            self.reset();
        }
        result
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}
