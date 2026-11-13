//! TCP 监听器：同一端口既跑 Nostr 中继（WebSocket）又跑 HTTP 短码 API。
//!
//! 一个连接进来后，先只读握手头（到 `\r\n\r\n` 为止，不消耗 body），据此分流：
//! - 带 `Upgrade: websocket` 的是中继握手，把已读走的头包回 `PrefixStream` 交给 tungstenite 继续握手/读帧；
//! - 否则按 HTTP/1.1 处理短码 API，读完 body、写回响应后关闭。
//!
//! Zeabur 在前面做 TLS 终止，所以这里只接明文的 ws:// 与 http://，对外就是 wss/https。

use std::io::{self, Cursor};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;

use crate::relay::RelayServer;
use crate::store::Store;

/// 普通 API 的 body 上限。更新包单独放宽（见 MAX_UPLOAD_BODY）。
/// 不设上限的话，一个伪造的 Content-Length 就能把内存吃光。
const MAX_BODY: usize = 64 * 1024;
/// 更新包（几十 MB 的安装包）走独立的额度。
const MAX_UPLOAD_BODY: usize = 256 * 1024 * 1024;
/// 同时在线的连接数上限：有人拿一堆连接占着不放，服务器也扛不住。
const MAX_CONNS: usize = 512;

/// 把已被读走的握手头包回去的流：先吐 prefix 里的字节，再转交内层 TcpStream。
pub struct PrefixStream {
    prefix: Cursor<Vec<u8>>,
    inner: TcpStream,
}

impl PrefixStream {
    pub fn new(prefix: Vec<u8>, inner: TcpStream) -> Self {
        PrefixStream {
            prefix: Cursor::new(prefix),
            inner,
        }
    }
}

impl AsyncRead for PrefixStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let me = self.get_mut();
        if me.prefix.position() < me.prefix.get_ref().len() as u64 {
            let start = me.prefix.position() as usize;
            let rest = &me.prefix.get_ref()[start..];
            let n = std::cmp::min(rest.len(), buf.remaining());
            buf.put_slice(&rest[..n]);
            me.prefix.set_position((start + n) as u64);
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut me.inner).poll_read(cx, buf)
        }
    }
}

impl AsyncWrite for PrefixStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        data: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, data)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

pub async fn serve(
    listener: tokio::net::TcpListener,
    server: Arc<RelayServer>,
    store: Arc<Store>,
) -> io::Result<()> {
    // 连接数闸门：超出上限的新连接直接丢掉，不给它分配任务。
    let gate = Arc::new(tokio::sync::Semaphore::new(MAX_CONNS));
    loop {
        let (stream, peer) = listener.accept().await?;
        let server = server.clone();
        let store = store.clone();
        // 限流按 IP 算，不带端口：带端口的话每次连接都是新桶，等于没限。
        let peer = peer.ip().to_string();
        let gate = gate.clone();
        tokio::spawn(async move {
            let Ok(permit) = gate.acquire_owned().await else {
                return;
            };
            if let Err(e) = handle_conn(stream, server, store, peer).await {
                eprintln!("连接处理出错: {e}");
            }
            drop(permit);
        });
    }
}

async fn handle_conn(
    mut stream: TcpStream,
    server: Arc<RelayServer>,
    store: Arc<Store>,
    peer: String,
) -> io::Result<()> {
    // 只读握手头，不消耗 body
    let mut head = Vec::with_capacity(2048);
    let mut byte = [0u8; 1];
    let mut head_done = false;
    loop {
        let n = stream.read(&mut byte).await?;
        if n == 0 {
            return Ok(());
        }
        head.push(byte[0]);
        if head.len() >= 4 && &head[head.len() - 4..] == b"\r\n\r\n" {
            head_done = true;
            break;
        }
        if head.len() > 16384 {
            break;
        }
    }
    if !head_done {
        // 头都没写完就断了（或超长）：这是畸形请求，别拿半截头去解析。
        let _ = stream
            .write_all(
                b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
            )
            .await;
        return Ok(());
    }
    let head_str = String::from_utf8_lossy(&head);
    let first_line = head_str.lines().next().unwrap_or("");
    let mut it = first_line.split_whitespace();
    let method = it.next().unwrap_or("");
    let path = it.next().unwrap_or("");
    let is_ws = head_str.contains("Upgrade: websocket")
        && head_str
            .to_ascii_lowercase()
            .contains("connection: upgrade");

    if is_ws {
        let prefixed = PrefixStream::new(head, stream);
        match tokio_tungstenite::accept_async(prefixed).await {
            Ok(ws) => server.run_ws(ws).await,
            Err(e) => eprintln!("WS 握手失败: {e}"),
        }
        return Ok(());
    }

    // 更新包是几十 MB 的二进制，不走返回字符串的 JSON 路由，直接分流到静态文件响应
    if method == "GET" && path.starts_with("/update/") {
        let resp = crate::api::serve_update_file(path).await;
        stream.write_all(&resp).await?;
        return Ok(());
    }

    let content_length = head_str
        .lines()
        .find_map(|l| {
            let l = l.to_ascii_lowercase();
            l.starts_with("content-length:")
                .then(|| l["content-length:".len()..].trim().to_string())
        })
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(0);
    // body 大小要有上限：伪造一个巨大的 Content-Length 就能把内存吃光。
    // 更新包是几十 MB 的安装包，单独放宽额度。
    let max_body = if path.starts_with("/api/admin/update") {
        MAX_UPLOAD_BODY
    } else {
        MAX_BODY
    };
    if content_length > max_body {
        let msg = "{\"error\":\"请求体太大\"}";
        let resp = format!(
            "HTTP/1.1 413 Payload Too Large\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{msg}",
            msg.len()
        );
        let _ = stream.write_all(resp.as_bytes()).await;
        return Ok(());
    }
    let mut body = vec![0u8; content_length];
    if content_length > 0 {
        stream.read_exact(&mut body).await?;
    }
    let resp = crate::api::handle(method, path, &body, &server, &store, &peer).await;
    let _ = stream.write_all(resp.as_bytes()).await;
    Ok(())
}
