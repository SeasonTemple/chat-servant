//! `ureq` 版的 [`Http`]。**生产里真正发请求的那一层。**
//!
//! 与 [`super::transport`] 的 `HttpReasoner` 分开放，是因为：
//! transport 层只管「请求长什么样、响应怎么解析」，这一层只管「怎么发」。
//! 两层的失败原因完全不同——一个是 401，一个是 DNS 解析失败。

use std::time::Duration;

use serde_json::Value;
use ureq::Agent;

use crate::transport::Http;

/// 基于 `ureq` 的 HTTP 客户端。
///
/// `ureq` 是**阻塞**的：一次调用占一个 OS 线程直到返回。
/// 在这里是对的——推理层每条新消息最多 3 次串行调用，
/// 而引入异步运行时要付出的依赖与复杂度买不到任何东西。
pub struct UreqHttp {
    agent: Agent,
}

impl UreqHttp {
    /// `timeout` 是**单次请求**的上限。三段各一次，所以一轮的上界是三倍。
    pub fn new(timeout: Duration) -> Self {
        let agent = ureq::AgentBuilder::new()
            .timeout_connect(Duration::from_secs(5))
            .timeout(timeout)
            .build();
        Self { agent }
    }
}

impl Default for UreqHttp {
    fn default() -> Self {
        Self::new(Duration::from_secs(20))
    }
}

impl Http for UreqHttp {
    fn post_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        body: &Value,
        _timeout: Duration,
    ) -> Result<Value, String> {
        let mut req = self.agent.post(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }

        let resp = req.send_json(body.clone()).map_err(|e| {
            // `ureq` 的错误 Display **可能带上请求体**。
            // 请求体里有用户的聊天内容，绝不能原样进日志——只留状态码。
            match e {
                ureq::Error::Status(code, r) => {
                    let body = r.into_string().unwrap_or_default();
                    format!(
                        "上游返回 {code}：{}",
                        crate::transport::redact(&truncate(&body, 200))
                    )
                }
                ureq::Error::Transport(t) => format!("网络层失败：{t}"),
            }
        })?;

        resp.into_json::<Value>()
            .map_err(|e| format!("响应不是合法 JSON（{e}）——上游契约可能变了"))
    }

    fn get_json(
        &self,
        url: &str,
        headers: &[(String, String)],
        _timeout: Duration,
    ) -> Result<Value, String> {
        let mut req = self.agent.get(url);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = req.call().map_err(|e| match e {
            ureq::Error::Status(code, r) => {
                let body = r.into_string().unwrap_or_default();
                format!(
                    "上游返回 {code}：{}",
                    crate::transport::redact(&truncate(&body, 200))
                )
            }
            ureq::Error::Transport(t) => format!("网络层失败：{t}"),
        })?;
        resp.into_json::<Value>()
            .map_err(|e| format!("响应不是合法 JSON（{e}）——上游契约可能变了"))
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let t: String = s.chars().take(n).collect();
    format!("{t}…")
}
