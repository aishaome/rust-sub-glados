use anyhow::{Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::env;
use std::time::Duration;

const TOKEN_URL: &str = "https://qyapi.weixin.qq.com/cgi-bin/gettoken";
const SEND_URL: &str = "https://qyapi.weixin.qq.com/cgi-bin/message/send";
const DEFAULT_MAIL_NOTIFY_URL: &str = "https://msg.zhiyuan.win/send-email";

/// 依序推送到所有已配置的通道，單一通道失敗不影響其他通道
pub fn send_all(title: &str, content: &str) -> Result<()> {
    let mut sent = false;
    if let Some(wecom) = WeCom::from_env()? {
        sent = true;
        match wecom.send_text(title, content) {
            Ok(()) => println!("企业微信推送成功"),
            Err(e) => println!("企业微信推送失败: {e:#}"),
        }
    }
    if let Some(mail) = MailApi::from_env()? {
        sent = true;
        match mail.send_html(title, content) {
            Ok(()) => println!("邮件通知发送成功"),
            Err(e) => println!("邮件通知发送失败: {e:#}"),
        }
    }
    if !sent {
        println!("未配置任何通知通道（企业微信 / 邮件），跳过推送");
    }
    Ok(())
}

/// 企業微信應用號推送，與 Glados-Railgun-checkin 的 PushService 相同
pub struct WeCom {
    corp_id: String,
    secret: String,
    agent_id: i64,
    touser: String,
    client: Client,
}

impl WeCom {
    /// CORP_ID / SECRET / AGENT_ID 任一未設定時回傳 None（跳過推送）
    pub fn from_env() -> Result<Option<Self>> {
        let (Ok(corp_id), Ok(secret), Ok(agent_id)) = (env::var("CORP_ID"), env::var("SECRET"), env::var("AGENT_ID")) else {
            return Ok(None);
        };
        let touser = env::var("TOUSER").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "@all".into());
        let client = Client::builder().timeout(Duration::from_secs(15)).build()?;
        Ok(Some(Self { corp_id, secret, agent_id: agent_id.trim().parse()?, touser, client }))
    }

    fn token(&self) -> Result<String> {
        let data: Value = self
            .client
            .get(TOKEN_URL)
            .query(&[("corpid", &self.corp_id), ("corpsecret", &self.secret)])
            .send()?
            .json()?;
        match data["access_token"].as_str() {
            Some(t) if data["errcode"].as_i64() == Some(0) => Ok(t.to_string()),
            _ => bail!("获取 access_token 失败: {data}"),
        }
    }

    /// 純文字訊息，企業微信 App 與微信插件都能顯示
    pub fn send_text(&self, title: &str, content: &str) -> Result<()> {
        self.send(json!({
            "touser": self.touser,
            "msgtype": "text",
            "agentid": self.agent_id,
            "text": { "content": format!("{title}\n{content}") },
            "safe": 0,
        }))
    }

    fn send(&self, payload: Value) -> Result<()> {
        let token = self.token()?;
        let data: Value = self.client.post(SEND_URL).query(&[("access_token", token)]).json(&payload).send()?.json()?;
        if data["errcode"].as_i64() != Some(0) {
            bail!("企业微信推送失败: {data}");
        }
        Ok(())
    }
}

/// 郵件通知接口：POST {url}?token=xxx，body 為 {"type":"html","subject":..,"content":..}
pub struct MailApi {
    url: String,
    token: String,
    client: Client,
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl MailApi {
    /// 未設定 MAIL_NOTIFY_TOKEN 時回傳 None（跳過）
    pub fn from_env() -> Result<Option<Self>> {
        let Some(token) = env::var("MAIL_NOTIFY_TOKEN").ok().filter(|s| !s.is_empty()) else {
            return Ok(None);
        };
        let url = env::var("MAIL_NOTIFY_URL")
            .ok()
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_MAIL_NOTIFY_URL.into());
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;
        Ok(Some(Self { url, token, client }))
    }

    /// 把純文字總結轉成 HTML：每個帳號一段，狀態依結果上色
    pub fn send_html(&self, title: &str, content: &str) -> Result<()> {
        let blocks: Vec<String> = content
            .split("\n\n")
            .map(|block| {
                let lines: Vec<String> = block
                    .lines()
                    .enumerate()
                    .map(|(i, line)| {
                        let line = escape_html(line);
                        if i == 0 {
                            return format!("<b>{line}</b>");
                        }
                        let color = if line.contains("签到成功") || line.contains("兑换成功") {
                            "green"
                        } else if line.contains("失败") {
                            "red"
                        } else if line.contains("重复签到") {
                            "gray"
                        } else {
                            return line;
                        };
                        format!("<span style=\"color:{color};\">{line}</span>")
                    })
                    .collect();
                format!("<p>{}</p>", lines.join("<br>"))
            })
            .collect();
        let html = format!("<h3>{}</h3>{}", escape_html(title), blocks.join(""));

        let data: Value = self
            .client
            .post(&self.url)
            .query(&[("token", &self.token)])
            .json(&json!({ "type": "html", "subject": title, "content": html }))
            .send()?
            .json()?;
        if data["code"].as_i64() != Some(0) {
            bail!("邮件通知发送失败: {data}");
        }
        Ok(())
    }
}
