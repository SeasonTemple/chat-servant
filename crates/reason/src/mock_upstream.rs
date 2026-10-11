//! 测试用的假上游。**只在 `#[cfg(test)]` 下编译**，不进任何产物。
//!
//! ## 为什么是「真的发一次 HTTP」而不是一个 mock trait
//!
//! 早先的传输缝是 `trait Http`，测试给它一个 `FakeHttp` —— 不开端口、
//! 不碰 socket，于是**它只能看见我们自己拼的那个 JSON**，看不见
//! 「请求头对不对」「URL 拼对没有」「响应能不能解析」这三件事里
//! 任何一件真的发生过。
//!
//! 这不是理论风险，是**已经发生过的事故**：ureq 那次没开 TLS 后端
//! （`no TLS backend is configured`），而**全部单测都是绿的** ——
//! 因为它们压根没发过请求。
//!
//! 所以这里起一个真的 `TcpListener`，让 genai 真的走一次 HTTP，
//! 把**字节**记下来。测试断的是「请求真的长那样」，
//! 而不是「我们调用了一个我们自己的函数」。
//!
//! 为什么不用现成的 mock server crate：**为了这一个东西引依赖不值当**，
//! 而 HTTP/1.1 的请求头 + `Content-Length` 请求体一共三十行。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// 一次请求的**原始形状**。字段都是自己从字节里剥出来的，不是别人
/// 反序列化好给我们的 —— 中间那层就是最可能出错的地方。
#[derive(Debug, Clone)]
pub struct Seen {
    /// 请求行里的方法，期望是 `POST` 或 `GET`。
    pub method: String,
    /// 请求行里的路径（含 query）。**端点拼错只有这里看得出来。**
    pub path: String,
    /// 请求头，原样保留大小写之外的形式，查找时忽略大小写。
    pub headers: Vec<(String, String)>,
    /// 请求体原文。
    pub body: String,
}

impl Seen {
    /// 按名字找一个请求头。**忽略大小写** —— HTTP 头名本来就不区分。
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

/// 一个假上游。起在本机回环地址的**随机端口**上，不碰任何真实域名。
pub struct MockUpstream {
    base_url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

/// 一条预设回复。`200` + 一个 JSON 对象是最常用的那一种，
/// 但**故意做成可配的**：401（key 错）、404（模型名错）都得能演，
/// 而「上游把 key 原样回显在错误正文里」是最坏情况，必须能演出来。
pub struct Reply(pub u16, pub String);

impl MockUpstream {
    /// 起一个假上游。`replies` 按顺序发出去，发完就收工。
    pub fn new(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("回环地址上应当绑得上");
        let port = listener.local_addr().expect("刚绑的端口读得出来").port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let sink = seen.clone();

        let handle = std::thread::spawn(move || {
            // ⚠ 非阻塞 accept + 死线：测试要是根本没发请求（比如上一条断言就红了），
            // 阻塞的 accept 会把这个线程永远挂住，然后整条测试卡到 CI 超时。
            // 拿不到请求时**安静退出**比卡死好。
            listener.set_nonblocking(true).expect("能设成非阻塞");
            let deadline = Instant::now() + Duration::from_secs(20);
            for reply in replies {
                let mut stream = match accept_before(&listener, deadline) {
                    Some(s) => s,
                    None => return,
                };
                let Some(req) = read_request(&mut stream) else {
                    continue;
                };
                sink.lock().expect("锁").push(req);
                let _ = stream.write_all(&render(reply.0, &reply.1));
                let _ = stream.flush();
            }
        });

        Self {
            base_url: format!("http://127.0.0.1:{port}/v1/"),
            seen,
            handle: Some(handle),
        }
    }

    /// genai 的**端点覆盖值**。带上结尾的 `/v1/`，
    /// 所以 OpenAI 适配器会在后面拼 `chat/completions`，
    /// 断言路径时能直接和文档里的端点对上。
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// 已经收到的请求。**按到达顺序。**
    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().expect("锁").clone()
    }
}

impl Drop for MockUpstream {
    fn drop(&mut self) {
        // 不 join 的话线程会变成孤儿进程：它自己会在 20 秒死线后退出，
        // 但那 20 秒里它占着一个端口，而测试进程可能早就结束了。
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// 轮询到死线为止接一个连接。返回 `None` 表示**等超时了**。
fn accept_before(listener: &TcpListener, deadline: Instant) -> Option<std::net::TcpStream> {
    loop {
        match listener.accept() {
            Ok((s, _)) => {
                let _ = s.set_read_timeout(Some(Duration::from_secs(10)));
                return Some(s);
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if Instant::now() >= deadline {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => return None,
        }
    }
}

/// 从字节流里剥出一个完整请求。读不满就返回 `None`（**不猜**）。
fn read_request(stream: &mut std::net::TcpStream) -> Option<Seen> {
    let mut buf: Vec<u8> = Vec::new();
    let mut byte = [0u8; 1];
    // 请求头到空行为止。逐字节读是因为 HTTP/1.1 的头没有长度前缀，
    // 而这里**不值得**为了它引一个缓冲读取器。
    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
        let n = stream.read(&mut byte).ok()?;
        if n == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.len() > 64 * 1024 {
            return None;
        }
    }
    let head_end = buf.windows(4).position(|w| w == b"\r\n\r\n").unwrap_or(0);
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.lines();
    let start = lines.next()?;
    let mut parts = start.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();

    let mut headers: Vec<(String, String)> = Vec::new();
    let mut len = 0usize;
    for line in lines {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim().to_string();
        let v = v.trim().to_string();
        if k.eq_ignore_ascii_case("content-length") {
            len = v.parse().unwrap_or(0);
        }
        headers.push((k, v));
    }

    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let mut chunk = [0u8; 4096];
        let n = stream.read(&mut chunk).ok()?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    body.truncate(len);

    Some(Seen {
        method,
        path,
        headers,
        body: String::from_utf8_lossy(&body).to_string(),
    })
}

/// 组一个 HTTP/1.1 响应。
///
/// ⚠ **`Connection: close` 不是可选的**：不开的话 reqwest 会复用连接，
/// 于是「一次请求一次 accept」的模型在第二次就错位——服务端在等一个新连接，
/// 客户端在同一个连接上等第二个响应，测试挂在超时上而不是挂在断言上。
fn render(status: u16, body: &str) -> Vec<u8> {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let mut out = head.into_bytes();
    out.extend_from_slice(body.as_bytes());
    out
}
