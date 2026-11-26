use crate::model::{ChatMessage, MediaRef, Member, Session};
use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::fmt::Write as _;

/// QCE HTTP API 客户端（Bearer token 鉴权，全部走 JSON envelope）。
#[derive(Clone)]
pub struct QceClient {
    base: String,
    token: String,
    http: reqwest::Client,
}

impl QceClient {
    pub fn new(host: &str, token: &str) -> Self {
        Self {
            base: host.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
        }
    }

    async fn get(&self, path: &str) -> Result<Value> {
        let resp = self
            .http
            .get(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .context("连接 QCE 服务失败")?;
        let value: Value = resp.json().await.context("响应不是合法 JSON")?;
        unwrap_envelope(value)
    }

    async fn post(&self, path: &str, body: &Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .context("连接 QCE 服务失败")?;
        let value: Value = resp.json().await.context("响应不是合法 JSON")?;
        unwrap_envelope(value)
    }

    /// 最近联系人 → 会话列表。
    pub async fn recent_contacts(&self) -> Result<Vec<Session>> {
        let data = self.get("/api/recent-contacts?includeAll=true").await?;
        Ok(extract_sessions(&data))
    }

    /// 全部好友。
    pub async fn friends(&self) -> Result<Vec<Session>> {
        let data = self.get("/api/friends").await?;
        let friends = find_array(&data, &["friends", "list", "data"]);
        Ok(friends
            .iter()
            .filter_map(|f| {
                let uid = str_of(f, &["uid", "peerUid"])?;
                let name = str_of(f, &["remark", "nick", "name"])
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                Some(Session {
                    chat_type: 1,
                    peer_uid: uid,
                    peer_uin: str_of(f, &["uin"]).unwrap_or_default(),
                    name,
                    tag: "peer".to_string(),
                })
            })
            .collect())
    }

    /// 全部群。
    pub async fn groups(&self) -> Result<Vec<Session>> {
        let data = self.get("/api/groups").await?;
        let groups = find_array(&data, &["groups", "list", "data"]);
        Ok(groups
            .iter()
            .filter_map(|g| {
                let code = str_of(g, &["groupCode", "groupId", "uin", "peerUid"])?;
                let name = str_of(g, &["groupName", "name", "remark"])
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                Some(Session {
                    chat_type: 2,
                    peer_uid: code.clone(),
                    peer_uin: code,
                    name,
                    tag: "grp".to_string(),
                })
            })
            .collect())
    }

    /// 拉取一页消息（page 1 = 最新）。
    pub async fn fetch_messages(&self, session: &Session, page: i64) -> Result<Vec<ChatMessage>> {
        let body = json!({
            "peer": { "chatType": session.chat_type, "peerUid": session.peer_uid },
            "page": page,
            "pageSize": 30,
        });
        let data = self.post("/api/messages/fetch", &body).await?;
        let messages = find_array(&data, &["messages", "list", "data"]);
        Ok(messages.iter().map(crate::model::parse_message).collect())
    }

    /// 群成员（@ 提及用）。
    pub async fn group_members(&self, group_code: &str) -> Result<Vec<Member>> {
        let path = format!("/api/groups/{group_code}/members");
        let data = self.get(&path).await?;
        let members = find_array(&data, &["members", "list", "data"]);
        Ok(members
            .iter()
            .filter_map(|m| {
                let uin = str_of(m, &["uin"])?;
                if uin.is_empty() || uin == "0" {
                    return None;
                }
                let name = str_of(m, &["cardName", "nick", "name", "card"])
                    .filter(|s| !s.is_empty())
                    .unwrap_or_default();
                Some(Member { uin, name })
            })
            .collect())
    }

    /// 发送消息：text / image base64 / mentions 至少一项非空。
    pub async fn send_message(
        &self,
        session: &Session,
        text: &str,
        image_b64: Option<&str>,
        mentions: &[String],
    ) -> Result<()> {
        let mut body = json!({
            "peer": {
                "chatType": session.chat_type,
                "peerUid": session.peer_uid,
                "peerUin": session.peer_uin,
            },
            "text": text,
        });
        if let Some(b64) = image_b64 {
            body["image"] = json!(b64);
        }
        if !mentions.is_empty() {
            body["mentions"] = json!(mentions);
        }
        self.post("/api/messages/send", &body).await?;
        Ok(())
    }

    /// 构造 `/api/media/local` URL（浏览器/粘贴用，带 token 参数）。
    pub fn media_url(&self, media: &MediaRef, session: &Session) -> Option<String> {
        let path = media.local_path.as_ref()?;
        let mut url = format!(
            "{}/api/media/local?path={}&token={}",
            self.base,
            urlenc(path),
            urlenc(&self.token)
        );
        if !media.msg_id.is_empty() {
            let _ = write!(url, "&msgId={}", urlenc(&media.msg_id));
        }
        let _ = write!(url, "&chatType={}", session.chat_type);
        if !session.peer_uid.is_empty() {
            let _ = write!(url, "&peerUid={}", urlenc(&session.peer_uid));
        }
        if !media.element_id.is_empty() {
            let _ = write!(url, "&elementId={}", urlenc(&media.element_id));
        }
        Some(url)
    }

    /// 下载媒体字节（bearer 鉴权），返回 (bytes, 扩展名)。
    pub async fn fetch_media(
        &self,
        media: &MediaRef,
        session: &Session,
    ) -> Result<(Vec<u8>, String)> {
        let mut req = self
            .http
            .get(format!("{}/api/media/local", self.base))
            .bearer_auth(&self.token)
            .query(&[("chatType", session.chat_type.to_string())]);
        if let Some(path) = &media.local_path {
            req = req.query(&[("path", path.clone())]);
        }
        if let Some(thumb) = &media.thumb_path {
            req = req.query(&[("thumbPath", thumb.clone())]);
        }
        if !media.msg_id.is_empty() {
            req = req.query(&[("msgId", media.msg_id.clone())]);
        }
        if !session.peer_uid.is_empty() {
            req = req.query(&[("peerUid", session.peer_uid.clone())]);
        }
        if !media.element_id.is_empty() {
            req = req.query(&[("elementId", media.element_id.clone())]);
        }
        let resp = req.send().await.context("下载媒体失败")?;
        if !resp.status().is_success() {
            return Err(anyhow!("媒体下载返回 {}", resp.status()));
        }
        let ext = media
            .local_path
            .as_deref()
            .and_then(|p| p.rsplit('.').next())
            .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("bin")
            .to_string();
        let bytes = resp.bytes().await.context("读取媒体内容失败")?;
        Ok((bytes.to_vec(), ext))
    }
}

/// 极简 query 参数转义。
fn urlenc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~' | b'/') {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

fn unwrap_envelope(value: Value) -> Result<Value> {
    if value.get("success").and_then(Value::as_bool) == Some(false) {
        let message = value
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("未知错误");
        return Err(anyhow!("{message}"));
    }
    Ok(value.get("data").cloned().unwrap_or(value))
}

/// 在响应 JSON 里找第一个命中键名的数组（服务端包装形态有多种）。
fn find_array(data: &Value, keys: &[&str]) -> Vec<Value> {
    if let Some(arr) = data.as_array() {
        return arr.clone();
    }
    for key in keys {
        if let Some(arr) = data.get(*key).and_then(Value::as_array) {
            return arr.clone();
        }
    }
    Vec::new()
}

fn str_of(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        value
            .get(*key)
            .and_then(|v| {
                v.as_str()
                    .map(str::to_string)
                    .or_else(|| v.as_i64().map(|n| n.to_string()))
            })
            .filter(|s| !s.is_empty())
    })
}

fn extract_sessions(data: &Value) -> Vec<Session> {
    let mut out = Vec::new();
    for item in find_array(data, &["contacts", "list", "data"]) {
        let chat_type = item
            .get("chatType")
            .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
            .or_else(|| {
                // 部分字段名兜底：群会话常见 groupCode，私聊常见 peerUid/uid。
                if item.get("groupCode").is_some() {
                    Some(2)
                } else {
                    Some(1)
                }
            })
            .unwrap_or(1);
        let peer_uid = str_of(&item, &["peerUid", "uid", "groupCode", "uin"]).unwrap_or_default();
        if peer_uid.is_empty() {
            continue;
        }
        let uin = str_of(&item, &["peerUin", "uin", "groupCode"]).unwrap_or_default();
        let name =
            str_of(&item, &["remark", "name", "nick", "groupName", "title"]).unwrap_or_default();
        out.push(Session {
            chat_type,
            peer_uid,
            peer_uin: uin,
            name,
            tag: if chat_type == 2 {
                "grp".to_string()
            } else {
                "rec".to_string()
            },
        });
    }
    out
}
