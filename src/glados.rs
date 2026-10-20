use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use reqwest::cookie::{CookieStore, Jar};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, ORIGIN, REFERER};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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

/// 由天數（自 1970-01-01 起）換算為公曆 (年, 月, 日)
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Unix 秒換算為北京時間日期 `YYYY-MM-DD`
fn cn_date_of(unix_secs: i64) -> String {
    let (y, m, d) = civil_from_days((unix_secs + 8 * 3600).div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn today_cn() -> String {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
    cn_date_of(now)
}

/// 判斷積分歷史中是否有今天（北京時間）的正向積分記錄，即今天已簽到。
/// 時間欄位可能是日期字串或時間戳（秒／毫秒），都嘗試解析；無法識別時回傳 false，交由簽到接口判斷。
fn checked_in_today(points: &Value, today: &str) -> bool {
    let Some(history) = points["history"].as_array() else {
        return false;
    };
    history.iter().any(|entry| {
        let gained = entry["change"].as_f64().or_else(|| entry["change"].as_str()?.parse().ok()).unwrap_or(0.0) > 0.0;
        let date = ["time", "date", "createdAt", "created_at"].iter().find_map(|k| match &entry[*k] {
            Value::String(s) if s.len() >= 10 => Some(s[..10].to_string()),
            Value::Number(n) => n.as_i64().map(|t| cn_date_of(if t > 1_000_000_000_000 { t / 1000 } else { t })),
            _ => None,
        });
        gained && date.as_deref() == Some(today)
    })
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

    /// 查詢狀態 → 查詢積分歷史判斷今天是否已簽到 → 未簽到才執行簽到 → 查詢積分 → 積分足夠時兌換
    pub fn checkin(&self) -> Result<CheckinReport> {
        let status = self.get("/user/status")?;
        let left_days = status["data"]["leftDays"]
            .as_str()
            .and_then(|s| s.parse::<f64>().ok())
            .or_else(|| status["data"]["leftDays"].as_f64())
            .map(|d| format!("{} 天", d as i64))
            .unwrap_or_else(|| "None 天".into());

        let before = self.get("/user/points")?;
        let status = if checked_in_today(&before, &today_cn()) {
            "重复签到: 今日已签到（签到前检查，未重复提交）".to_string()
        } else {
            let data = self.post("/user/checkin", json!({ "token": DOMAIN }))?;
            let message = data["message"].as_str().unwrap_or("无消息字段");
            match data["code"].as_i64() {
                Some(0) => format!("签到成功 (+{} 积分): {message}", data["points"]),
                Some(1) => format!("重复签到: {message}"),
                _ => format!("签到失败: {message}"),
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cn_date_conversion() {
        assert_eq!(cn_date_of(0), "1970-01-01");
        // 2026-10-19T17:34:43Z 对应北京时间 2026-10-20 01:34
        assert_eq!(cn_date_of(1_792_431_283), "2026-10-20");
        // UTC 2026-10-19T15:59:59Z 即北京时间 2026-10-19 23:59:59，仍是 10-19
        assert_eq!(cn_date_of(1_792_425_599), "2026-10-19");
        assert_eq!(cn_date_of(1_792_425_600), "2026-10-20");
    }

    #[test]
    fn detects_checkin_today() {
        let today = "2026-10-20";
        let hit = json!({ "history": [{ "change": 1, "time": "2026-10-20 08:01:02" }] });
        assert!(checked_in_today(&hit, today));
        let ts = json!({ "history": [{ "change": "18", "time": 1_792_431_283_000i64 }] });
        assert!(checked_in_today(&ts, today));
        let old = json!({ "history": [{ "change": 1, "time": "2026-10-19 08:01:02" }] });
        assert!(!checked_in_today(&old, today));
        let spend = json!({ "history": [{ "change": -500, "time": "2026-10-20 09:00:00" }] });
        assert!(!checked_in_today(&spend, today));
        assert!(!checked_in_today(&json!({}), today));
    }
}
