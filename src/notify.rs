use anyhow::{Result, bail};
use reqwest::blocking::Client;
use serde_json::{Value, json};
use std::env;
use std::time::Duration;

const TOKEN_URL: &str = "https://qyapi.weixin.qq.com/cgi-bin/gettoken";
const SEND_URL: &str = "https://qyapi.weixin.qq.com/cgi-bin/message/send";

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
