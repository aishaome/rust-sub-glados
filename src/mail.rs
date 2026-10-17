use anyhow::{Context, Result, bail, ensure};
use mail_parser::MessageParser;
use native_tls::TlsStream;
use regex::Regex;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const IMAP_PORT: u16 = 993;

/// 依郵箱網域推斷 IMAP 伺服器
fn imap_host(user: &str) -> Result<&'static str> {
    let domain = user.rsplit('@').next().unwrap_or_default().to_lowercase();
    Ok(match domain.as_str() {
        "qq.com" | "foxmail.com" => "imap.qq.com",
        "163.com" => "imap.163.com",
        "126.com" => "imap.126.com",
        "yeah.net" => "imap.yeah.net",
        _ => bail!("不支持的邮箱域名: {domain}"),
    })
}

/// 逐位元組讀一行，避免多讀走屬於 imap crate 的資料
fn read_line(stream: &mut impl Read) -> Result<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    while !buf.ends_with(b"\r\n") {
        ensure!(stream.read(&mut byte)? == 1, "IMAP 连接中断");
        buf.push(byte[0]);
    }
    Ok(String::from_utf8_lossy(&buf).trim_end().to_string())
}

pub struct Mailbox {
    session: imap::Session<TlsStream<TcpStream>>,
}

impl Mailbox {
    pub fn connect(user: &str, pass: &str) -> Result<Self> {
        let host = imap_host(user)?;
        let tls = native_tls::TlsConnector::builder().build()?;
        let tcp = TcpStream::connect((host, IMAP_PORT)).with_context(|| format!("连接 {host} 失败"))?;
        let mut stream = tls.connect(host, tcp)?;

        // 自行讀取 greeting，並在交給 imap crate 前送出 ID 指令：
        // 網易郵箱沒收到 ID 會在 SELECT/EXAMINE 回 "Unsafe Login"，
        // 而 imap-proto 0.10 無法解析 "* ID (...)" 回應，所以不能透過 Session 送
        read_line(&mut stream)?;
        if host != "imap.qq.com" {
            stream.write_all(b"a0 ID (\"name\" \"glados-checkin\" \"version\" \"1.0\" \"vendor\" \"rust-imap\")\r\n")?;
            loop {
                let line = read_line(&mut stream)?;
                if line.starts_with("a0 ") {
                    ensure!(line.starts_with("a0 OK"), "{host} ID 指令失败: {line}");
                    break;
                }
            }
        }

        let mut client = imap::Client::new(stream);
        client.debug = std::env::var("IMAP_DEBUG").is_ok();
        let session = client
            .login(user, pass)
            .map_err(|(e, _)| e)
            .with_context(|| format!("{user} IMAP 登录失败：请确认已开启 IMAP 服务并使用授权码"))?;
        Ok(Self { session })
    }

    /// 目前 INBOX 中最大的 UID，用來只看之後新到的郵件
    pub fn latest_uid(&mut self) -> Result<u32> {
        // EXAMINE 唯讀開啟，不會把郵件標記為已讀
        let mailbox = self.session.examine("INBOX")?;
        if let Some(next) = mailbox.uid_next {
            return Ok(next.saturating_sub(1));
        }
        // 163 等郵箱不一定回 UIDNEXT，改為直接取最後一封的 UID
        if mailbox.exists == 0 {
            return Ok(0);
        }
        let fetches = self.session.fetch("*", "UID")?;
        Ok(fetches.iter().filter_map(|f| f.uid).max().unwrap_or(0))
    }

    /// 等待寄給 `email` 的 GLaDOS 驗證碼（UID 必須大於 `after_uid`）
    pub fn wait_glados_code(&mut self, email: &str, after_uid: u32, timeout: Duration) -> Result<String> {
        let code_re = Regex::new(r"Verification Code:\s*(\d{4,8})")?;
        let email = email.to_lowercase();
        let deadline = Instant::now() + timeout;
        // 只接受請求之後寄出的信（容許 2 分鐘時鐘誤差），避免拿到舊驗證碼
        let not_before = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64 - 120;

        while Instant::now() < deadline {
            self.session.examine("INBOX")?;
            let uids = self.session.uid_search(format!("UID {}:*", after_uid + 1))?;
            let mut uids: Vec<u32> = uids.into_iter().filter(|&u| u > after_uid).collect();
            uids.sort_unstable_by(|a, b| b.cmp(a));

            for uid in uids {
                let fetches = self.session.uid_fetch(uid.to_string(), "BODY.PEEK[]")?;
                let Some(raw) = fetches.iter().next().and_then(|f| f.body()) else { continue };
                let Some(msg) = MessageParser::default().parse(raw) else { continue };

                let from_glados = msg
                    .from()
                    .and_then(|a| a.first())
                    .and_then(|a| a.address())
                    .is_some_and(|a| a.to_lowercase().contains("glados"));
                let fresh = msg.date().is_some_and(|d| d.to_timestamp() >= not_before);
                if !from_glados || !fresh {
                    continue;
                }

                // 三個郵箱是同一個 QQ 信箱的別名，需以收件人 / 正文確認是哪個帳號的驗證碼
                let to_match = msg
                    .to()
                    .map(|to| to.iter().any(|a| a.address().is_some_and(|x| x.to_lowercase() == email)))
                    .unwrap_or(false);
                let body = msg.body_text(0).unwrap_or_default();
                if !to_match && !body.to_lowercase().contains(&email) {
                    continue;
                }

                if let Some(c) = code_re.captures(&body) {
                    return Ok(c[1].to_string());
                }
            }
            thread::sleep(Duration::from_secs(5));
        }
        bail!("等待 {email} 的 GLaDOS 验证码超时")
    }

    pub fn logout(mut self) -> Result<()> {
        self.session.logout()?;
        Ok(())
    }
}
