use serde_json::Value;

/// 会话（最近联系人 / 好友 / 群 统一抽象）。
#[derive(Debug, Clone)]
pub struct Session {
    pub chat_type: i64, // 1 = 私聊, 2 = 群聊
    pub peer_uid: String,
    pub peer_uin: String,
    pub name: String,
    pub tag: String, // 列表上显示的来源标记，如 "好友" / "群"
}

impl Session {
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            self.peer_uin.clone()
        } else {
            self.name.clone()
        }
    }
}

/// 群成员（@ 选择器用）。
#[derive(Debug, Clone)]
pub struct Member {
    pub uin: String,
    pub name: String,
}

/// 消息里的可打开媒体（图片/视频/文件）。
#[derive(Debug, Clone)]
pub struct MediaRef {
    /// 展示用文件名。
    pub file_name: String,
    /// NTQQ 缓存本地路径（服务端文件系统内）。
    pub local_path: Option<String>,
    /// 缩略图本地路径（服务端按需下载用 thumbPath 参数）。
    pub thumb_path: Option<String>,
    /// QQ CDN 远程 URL（可能过期）。
    pub remote_url: Option<String>,
    /// 按需下载定位参数。
    pub msg_id: String,
    pub element_id: String,
}

/// 单条消息（已把 elements 渲染成展示文本）。
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub time: i64,
    pub sender: String,
    pub is_self: bool,
    pub body: String,
    pub media: Vec<MediaRef>,
}

fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

/// `thumbPath` 在服务端返回里可能是 {"0": path} 对象或字符串。
fn thumb_path(pic: &Value) -> Option<String> {
    let raw = pic.get("thumbPath")?;
    if let Some(s) = raw.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    raw.as_object()
        .and_then(|m| m.values().next())
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 从单个 element 提取可打开媒体（pic/video/file 的本地路径或远程 URL）。
fn extract_media(el: &Value, msg_id: &str) -> Option<MediaRef> {
    let element_id = str_field(el, "elementId").unwrap_or_default().to_string();
    let (node, url_keys) = if let Some(pic) = el.get("picElement").filter(|v| !v.is_null()) {
        (pic, vec!["originImageUrl", "sourceUrl"])
    } else if let Some(v) = el.get("videoElement").filter(|v| !v.is_null()) {
        (v, vec!["videoUrl", "sourceUrl"])
    } else if let Some(f) = el.get("fileElement").filter(|v| !v.is_null()) {
        (f, vec!["fileUrl", "sourceUrl"])
    } else {
        return None;
    };
    let thumb = thumb_path(node);
    let local_path = str_field(node, "sourcePath")
        .map(str::to_string)
        .or_else(|| thumb.clone())
        .or_else(|| str_field(node, "filePath").map(str::to_string));
    let remote_url = url_keys
        .iter()
        .find_map(|k| str_field(node, k))
        .map(str::to_string);
    if local_path.is_none() && remote_url.is_none() {
        return None;
    }
    Some(MediaRef {
        file_name: str_field(node, "fileName").unwrap_or_default().to_string(),
        local_path,
        thumb_path: thumb,
        remote_url,
        msg_id: msg_id.to_string(),
        element_id,
    })
}

fn num_field(value: &Value, key: &str) -> Option<i64> {
    value
        .get(key)
        .and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

/// 从 NTQQ element 渲染单行展示文本。
fn render_element(el: &Value) -> Option<String> {
    if let Some(text) = el
        .get("textElement")
        .and_then(|t| t.get("content"))
        .and_then(Value::as_str)
    {
        return Some(text.to_string());
    }
    if let Some(pic) = el.get("picElement").filter(|v| !v.is_null()) {
        let name = str_field(pic, "fileName").unwrap_or("");
        return Some(if name.is_empty() {
            "[图片]".to_string()
        } else {
            format!("[图片 {name}]")
        });
    }
    if let Some(face) = el.get("faceElement").filter(|v| !v.is_null()) {
        let idx = num_field(face, "faceIndex")
            .map(|i| i.to_string())
            .unwrap_or_default();
        return Some(if idx.is_empty() {
            "[表情]".to_string()
        } else {
            format!("[表情 {idx}]")
        });
    }
    if let Some(mf) = el.get("marketFaceElement").filter(|v| !v.is_null()) {
        let name = str_field(mf, "faceName").unwrap_or("贴纸");
        return Some(format!("[{name}]"));
    }
    if el.get("videoElement").is_some_and(|v| !v.is_null()) {
        return Some("[视频]".to_string());
    }
    if el.get("pttElement").is_some_and(|v| !v.is_null()) {
        return Some("[语音]".to_string());
    }
    if let Some(file) = el.get("fileElement").filter(|v| !v.is_null()) {
        let name = str_field(file, "fileName").unwrap_or("文件");
        return Some(format!("[文件 {name}]"));
    }
    if let Some(reply) = el.get("replyElement").filter(|v| !v.is_null()) {
        let text = reply
            .get("sourceMsgTextElems")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(|e| str_field(e, "textElemContent"))
                    .collect::<String>()
            })
            .filter(|s| !s.is_empty())
            .or_else(|| str_field(reply, "sourceMsgText").map(str::to_string));
        return Some(match text {
            Some(t) => format!("[回复 {t}]"),
            None => "[回复]".to_string(),
        });
    }
    if let Some(tip) = el.get("grayTipElement").filter(|v| !v.is_null()) {
        // 系统提示：xmlElement 或 jsonElement 内含可读文本的场景较杂，统一占位。
        let _ = tip;
        return Some("[系统提示]".to_string());
    }
    if el
        .get("multiForwardMsgElement")
        .is_some_and(|v| !v.is_null())
    {
        return Some("[转发消息]".to_string());
    }
    if el.get("arkElement").is_some_and(|v| !v.is_null()) {
        return Some("[卡片消息]".to_string());
    }
    if el.get("markdownElement").is_some_and(|v| !v.is_null()) {
        return Some("[Markdown 消息]".to_string());
    }
    None
}

/// 把服务端 fetch 返回的单条 message JSON 转成 `ChatMessage`。
pub fn parse_message(raw: &Value) -> ChatMessage {
    let time = num_field(raw, "msgTime").unwrap_or(0);
    let is_self = num_field(raw, "sendType") == Some(1);
    let sender = [
        str_field(raw, "sendMemberName"),
        str_field(raw, "sendNickName"),
        str_field(raw, "sendRemarkName"),
    ]
    .into_iter()
    .flatten()
    .find(|s| !s.is_empty())
    .map(str::to_string)
    .or_else(|| str_field(raw, "senderUid").map(|u| u.chars().take(12).collect()))
    .unwrap_or_else(|| "?".to_string());

    let msg_id = str_field(raw, "msgId").unwrap_or_default().to_string();
    let mut parts = Vec::new();
    let mut media = Vec::new();
    if let Some(elements) = raw.get("elements").and_then(Value::as_array) {
        for el in elements {
            if let Some(text) = render_element(el) {
                parts.push(text);
            }
            if let Some(m) = extract_media(el, &msg_id) {
                media.push(m);
            }
        }
    }
    let body = if parts.is_empty() {
        "[不支持的消息类型]".to_string()
    } else {
        parts.join(" ")
    };
    ChatMessage {
        time,
        sender,
        is_self,
        body,
        media,
    }
}
