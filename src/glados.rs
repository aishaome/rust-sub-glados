use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, ORIGIN, REFERER};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Duration;

const DOMAIN: &str = "glados.cloud";
const SITE: &str = "glados.network";
// 登入與簽到使用同一個 UA，避免被判定為「簽到裝置與登入裝置不同」
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

pub struct Glados {
    client: Client,
    jar: Arc<Jar>,
}

/// 積分達到此值時自動兌換 plan500（100 天）
const EXCHANGE_PLAN: &str = "plan500";
const EXCHANGE_POINTS: i64 = 500;

pub struct CheckinReport {
    pub left_days: String,
    pub status: String,
    pub points_total: String,
    pub exchange: String,
}

/// 網頁端會在每個請求帶 `Authorization: fp1.<48 hex>` 的裝置指紋，
/// 這裡依帳號產生固定值，讓同一帳號每次都像同一台裝置
fn fingerprint(email: &str) -> String {
    let hash = Sha256::digest(format!("v1|{USER_AGENT}|{email}"));
    let hex: String = hash.iter().map(|b| format!("{b:02x}")).collect();
    format!("fp1.{}", &hex[..48])
}

fn url(path: &str) -> String {
    format!("https://{DOMAIN}/api{path}")
}

fn base_url() -> reqwest::Url {
    format!("https://{DOMAIN}/").parse().expect("valid url")
}

impl Glados {
    /// `saved_cookie` 為之前登入保存的 cookie（`k=v; k2=v2`），有的話直接帶入
    pub fn new(email: &str, saved_cookie: Option<&str>) -> Result<Self> {
        let jar = Arc::new(Jar::default());
        if let Some(cookie) = saved_cookie {
            for pair in cookie.split(';').map(str::trim).filter(|p| !p.is_empty()) {
                jar.add_cookie_str(&format!("{pair}; Domain={DOMAIN}; Path=/"), &base_url());
            }
        }

        let mut headers = HeaderMap::new();
        headers.insert(ORIGIN, HeaderValue::from_str(&format!("https://{DOMAIN}"))?);
        headers.insert(REFERER, HeaderValue::from_str(&format!("https://{DOMAIN}/login"))?);
        headers.insert(AUTHORIZATION, HeaderValue::from_str(&fingerprint(email))?);
        let client = Client::builder()
            .user_agent(USER_AGENT)
            .default_headers(headers)
            .cookie_provider(jar.clone())
            .timeout(Duration::from_secs(60))
            .build()?;
        Ok(Self { client, jar })
    }

    /// 匯出目前 session 的 cookie，供下次直接使用
    pub fn export_cookie(&self) -> Option<String> {
        self.jar.cookies(&base_url()).and_then(|v| v.to_str().ok().map(String::from))
    }

    /// cookie 是否仍有效：能查詢到剩餘天數即視為已登入
    pub fn is_logged_in(&self) -> bool {
        self.get("/user/status")
            .is_ok_and(|s| s["code"].as_i64() == Some(0) && !s["data"]["leftDays"].is_null())
    }

    fn post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self.client.post(url(path)).json(&body).send()?;
        let status = resp.status();
        let text = resp.text()?;
        serde_json::from_str(&text).with_context(|| format!("POST {path} 响应非 JSON ({status}): {text}"))
    }

    fn get(&self, path: &str) -> Result<Value> {
        let resp = self.client.get(url(path)).send()?;
        let status = resp.status();
        let text = resp.text()?;
        serde_json::from_str(&text).with_context(|| format!("GET {path} 响应非 JSON ({status}): {text}"))
    }

    /// 要求 GLaDOS 寄送登入驗證碼
    pub fn request_code(&self, email: &str) -> Result<()> {
        let data = self.post("/authorization", json!({ "address": email.to_lowercase(), "site": SITE }))?;
        if data["captcha_required"].as_bool() == Some(true) {
            bail!("GLaDOS 要求人机验证 (Turnstile)，无法自动发送验证码，请先在浏览器手动登录一次");
        }
        if data["code"].as_i64() != Some(0) {
            bail!("发送验证码失败: {data}");
        }
        Ok(())
    }

    /// 以郵箱驗證碼登入，成功後 session cookie 會保存在 client 中
    pub fn login(&self, email: &str, mailcode: &str) -> Result<()> {
        let data = self.post(
            "/login",
            json!({ "method": "email", "site": SITE, "email": email.to_lowercase(), "mailcode": mailcode }),
        )?;
        if data["code"].as_i64() != Some(0) {
            bail!("登录失败: {data}");
        }
        Ok(())
    }

    /// 與 Glados-Railgun-checkin 相同：查詢狀態 → 簽到 → 查詢積分 → 積分足夠時兌換
    pub fn checkin(&self) -> Result<CheckinReport> {
        let status = self.get("/user/status")?;
        let left_days = status["data"]["leftDays"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| status["data"]["leftDays"].as_f64())
            .map(|d| format!("{} 天", d as i64))
            .unwrap_or_else(|| "None 天".into());

        let data = self.post("/user/checkin", json!({ "token": DOMAIN }))?;
        let message = data["message"].as_str().unwrap_or("无消息字段");
        let status = match data["code"].as_i64() {
            Some(0) => format!("签到成功 (+{} 积分): {message}", data["points"]),
            Some(1) => format!("重复签到: {message}"),
            _ => format!("签到失败: {message}"),
        };

        let points = self.get("/user/points")?;
        let points_num = points["points"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| points["points"].as_f64())
            .map(|p| p as i64);
        let points_total = points_num.map_or_else(|| "None 积分".into(), |p| format!("{p} 积分"));

        let exchange = match points_num {
            Some(p) if p >= EXCHANGE_POINTS => self.exchange(),
            Some(p) => format!("未兑换 (需 {EXCHANGE_POINTS} 积分，差 {})", EXCHANGE_POINTS - p),
            None => "未兑换 (无法获取积分)".into(),
        };

        Ok(CheckinReport { left_days, status, points_total, exchange })
    }

    /// 兌換 100 天會員；失敗只記錄在結果中，不影響簽到
    fn exchange(&self) -> String {
        match self.post("/user/exchange", json!({ "planType": EXCHANGE_PLAN })) {
            Ok(data) if data["code"].as_i64() == Some(0) => format!("兑换成功: {EXCHANGE_PLAN} (100 天)"),
            Ok(data) => format!("兑换失败: {}", data["message"].as_str().unwrap_or("未知错误")),
            Err(e) => format!("兑换失败: {e:#}"),
        }
    }
}
