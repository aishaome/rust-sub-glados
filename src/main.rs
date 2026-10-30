mod cookies;
mod glados;
mod mail;
mod notify;

use anyhow::{Context, Result};
use cookies::CookieFile;
use glados::Glados;
use mail::Mailbox;
use std::env;
use std::time::Duration;

const CODE_TIMEOUT: Duration = Duration::from_secs(180);

/// 收驗證碼用的郵箱，需要郵箱登入時才連線
struct MailAccount {
    user: String,
    pass: String,
    conn: Option<Mailbox>,
}

impl MailAccount {
    fn mailbox(&mut self) -> Result<&mut Mailbox> {
        if self.conn.is_none() {
            self.conn = Some(Mailbox::connect(&self.user, &self.pass)?);
        }
        Ok(self.conn.as_mut().expect("connected"))
    }
}

fn run_account(mail: &mut MailAccount, cookies: &mut CookieFile, email: &str) -> Result<glados::CheckinReport> {
    // 1. 先用保存的 cookie，仍有效就直接簽到
    if let Some(saved) = cookies.get(email) {
        let glados = Glados::new(email, Some(saved))?;
        if glados.is_logged_in() {
            println!("[{email}] cookie 有效，执行签到…");
            let report = glados.checkin()?;
            cookies.set(email, glados.export_cookie())?;
            return Ok(report);
        }
        println!("[{email}] cookie 已失效，改用邮箱登录");
    }

    // 2. 記下目前最新 UID，再請 GLaDOS 寄驗證碼，只讀之後新到的信
    let glados = Glados::new(email, None)?;
    let mailbox = mail.mailbox()?;
    let after_uid = mailbox.latest_uid()?;
    println!("[{email}] 请求发送验证码…");
    glados.request_code(email)?;

    // 3. 從郵箱讀取驗證碼
    println!("[{email}] 等待验证码邮件…");
    let code = mailbox.wait_glados_code(email, after_uid, CODE_TIMEOUT)?;
    println!("[{email}] 已获取验证码，模拟登录…");

    // 4. 登入後保存 cookie，接著用同一個 session 簽到
    glados.login(email, &code)?;
    cookies.set(email, glados.export_cookie())?;
    println!("[{email}] 登录成功，已保存 cookie，执行签到…");
    glados.checkin()
}

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    // cargo run -- --test-push：只向所有通知通道发送一则示例，不请求 GLaDOS
    if env::args().any(|a| a == "--test-push") {
        return notify::send_all(
            "GLaDOS 签到  成功:1  失败:1  重复:1（测试推送）",
            "example@qq.com\n状态: 签到成功 (+18 积分): Checkin! Got 18 Points\n剩余: 14 天  总积分: 18 积分\n兑换: 未兑换 (需 500 积分，差 482)\n\n\
             example@foxmail.com\n状态: 重复签到: Today's observation logged.\n剩余: 118 天  总积分: 195 积分\n兑换: 未兑换 (需 500 积分，差 305)\n\n\
             example@163.com\n状态: 失败: GLaDOS 要求人机验证 (Turnstile)",
        );
    }
    // MAIL_ACCOUNTS=帳號:授權碼,帳號:授權碼
    let mail_accounts: Vec<(String, String)> = env::var("MAIL_ACCOUNTS")
        .context("缺少环境变量 MAIL_ACCOUNTS（格式: 账号:授权码,账号:授权码）")?
        .split(',')
        .filter_map(|s| s.trim().split_once(':'))
        .map(|(u, p)| (u.trim().to_string(), p.trim().to_string()))
        .collect();
    let accounts: Vec<String> = env::var("GLADOS_EMAILS")
        .context("缺少环境变量 GLADOS_EMAILS（以逗号分隔）")?
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    let mut mailboxes: Vec<MailAccount> = mail_accounts
        .into_iter()
        .map(|(user, pass)| MailAccount { user: user.to_lowercase(), pass, conn: None })
        .collect();
    anyhow::ensure!(!mailboxes.is_empty(), "MAIL_ACCOUNTS 中没有有效的邮箱");
    let mut cookies = CookieFile::load();
    let mut summary = Vec::new();
    let mut notify_summary = Vec::new();
    let (mut ok, mut repeat, mut fail) = (0, 0, 0);

    // 依序處理，避免多封驗證碼同時到達時混淆
    for email in &accounts {
        // 有同名郵箱就讀它，否則讀第一個（QQ 郵箱的別名都收在同一個信箱）
        let idx = mailboxes
            .iter()
            .position(|m| m.user == email.to_lowercase())
            .unwrap_or(0);
        match run_account(&mut mailboxes[idx], &mut cookies, email) {
            Ok(r) => {
                println!("[{email}] {} | 剩余 {} | 总 {} | {}\n", r.status, r.left_days, r.points_total, r.exchange);
                match r.status.split(' ').next().unwrap_or_default().trim_end_matches(':') {
                    "签到成功" => ok += 1,
                    "重复签到" => repeat += 1,
                    _ => fail += 1,
                }
                let base = format!(
                    "{email}\n状态: {}\n剩余: {}  总积分: {}\n兑换: {}",
                    r.status, r.left_days, r.points_total, r.exchange
                );
                summary.push(base.clone());
                // 订阅地址含个人配置，只放进通知（企业微信 / 邮件），不写入标准输出（公开仓库的 Actions 日志）
                let mut with_subs = base;
                if !r.subscriptions.is_empty() {
                    with_subs.push_str("\n订阅地址:");
                    for (name, url) in &r.subscriptions {
                        with_subs.push_str(&format!("\n{name}: {url}"));
                    }
                }
                notify_summary.push(with_subs);
            }
            Err(e) => {
                println!("[{email}] 失败: {e:#}\n");
                fail += 1;
                summary.push(format!("{email}\n状态: 失败: {e:#}"));
                notify_summary.push(format!("{email}\n状态: 失败: {e:#}"));
            }
        }
    }

    for mailbox in mailboxes.into_iter().filter_map(|m| m.conn) {
        mailbox.logout()?;
    }
    let title = format!("GLaDOS 签到  成功:{ok}  失败:{fail}  重复:{repeat}");
    let content = summary.join("\n\n");
    println!("========== 签到总结 ==========\n{title}\n{content}\n");

    notify::send_all(&title, &notify_summary.join("\n\n"))
}
