use anyhow::Result;
use serde_json::{Map, Value};
use std::env;
use std::fs;

/// 各帳號登入後的 GLaDOS cookie，存成 `{ "email": "k=v; k2=v2" }`
pub struct CookieFile {
    path: String,
    map: Map<String, Value>,
}

impl CookieFile {
    pub fn load() -> Self {
        let path = env::var("GLADOS_COOKIE_FILE").unwrap_or_else(|_| "glados-cookies.json".into());
        let map = fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok())
            .unwrap_or_default();
        Self { path, map }
    }

    pub fn get(&self, email: &str) -> Option<&str> {
        self.map.get(&email.to_lowercase()).and_then(Value::as_str)
    }

    /// 每次更新都立即寫檔，中途失敗也不會丟失已取得的 cookie
    pub fn set(&mut self, email: &str, cookie: Option<String>) -> Result<()> {
        match cookie {
            Some(c) => self.map.insert(email.to_lowercase(), Value::String(c)),
            None => self.map.remove(&email.to_lowercase()),
        };
        fs::write(&self.path, serde_json::to_string_pretty(&self.map)?)?;
        Ok(())
    }
}
