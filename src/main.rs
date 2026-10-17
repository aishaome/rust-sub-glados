mod glados;
mod mail;
mod notify;

use anyhow::{Context, Result};
use glados::Glados;
use mail::Mailbox;
use notify::WeCom;
use std::env;
use std::time::Duration;

const CODE_TIMEOUT: Duration = Duration::from_secs(180);

fn run_account(mailbox: &mut Mailbox, email: &str) -> Result<glados::CheckinReport> {
    let glados = Glados::new(email)?;

    // 1. 記下目前最新 UID，再請 GLaDOS 寄驗證碼，只讀之後新到的信
    let after_uid = mailbox.latest_uid()?;
    println!("[{email}] 请求发送验证码…");
    glados.request_code(email)?;

    // 2. 從 QQ 郵箱讀取驗證碼
    println!("[{email}] 等待验证码邮件…");
    let code = mailbox.wait_glados_code(email, after_uid, CODE_TIMEOUT)?;
    println!("[{email}] 获取验证码 {code}，模拟登录…");

    // 3. 登入，接著用同一個 session 簽到
    glados.login(email, &code)?;
    println!("[{email}] 登录成功，执行签到…");
    glados.checkin()
}

fn main() -> Result<()> {
    let _ = dotenvy::dotenv();

    // cargo run -- --test-push：只发送一则示例通知，不请求 GLaDOS
    if env::args().any(|a| a == "--test-push") {
        let wecom = WeCom::from_env()?.context("企业微信配置不完整 (CORP_ID / SECRET / AGENT_ID)")?;
        wecom.send_text(
            "GLaDOS 签到  成功:1  失败:0  重复:0（测试推送）",
            "example@qq.com\n状态: 签到成功 (+18 积分): Checkin! Got 18 Points\n剩余: 14 天  总积分: 18 积分\n兑换: 未兑换 (需 500 积分，差 482)",
        )?;
        println!("企业微信测试推送成功");
        return Ok(());
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

    let mut mailboxes = Vec::new();
    for (user, pass) in &mail_accounts {
        mailboxes.push(Mailbox::connect(user, pass)?);
    }
    anyhow::ensure!(!mailboxes.is_empty(), "MAIL_ACCOUNTS 中没有有效的邮箱");
    let mut summary = Vec::new();
    let (mut ok, mut repeat, mut fail) = (0, 0, 0);

    // 依序處理，避免多封驗證碼同時到達時混淆
    for email in &accounts {
        // 有同名郵箱就讀它，否則讀第一個（QQ 郵箱的別名都收在同一個信箱）
        let idx = mailboxes
            .iter()
            .position(|m| m.user == email.to_lowercase())
            .unwrap_or(0);
        match run_account(&mut mailboxes[idx], email) {
            Ok(r) => {
                println!("[{email}] {} | 剩余 {} | 总 {} | {}\n", r.status, r.left_days, r.points_total, r.exchange);
                match r.status.split(' ').next().unwrap_or_default().trim_end_matches(':') {
                    "签到成功" => ok += 1,
                    "重复签到" => repeat += 1,
                    _ => fail += 1,
                }
                summary.push(format!(
                    "{email}\n状态: {}\n剩余: {}  总积分: {}\n兑换: {}",
                    r.status, r.left_days, r.points_total, r.exchange
                ));
            }
            Err(e) => {
                println!("[{email}] 失败: {e:#}\n");
                fail += 1;
                summary.push(format!("{email}\n状态: 失败: {e:#}"));
            }
        }
    }

    for mailbox in mailboxes {
        mailbox.logout()?;
    }
    let title = format!("GLaDOS 签到  成功:{ok}  失败:{fail}  重复:{repeat}");
    let content = summary.join("\n\n");
    println!("========== 签到总结 ==========\n{title}\n{content}\n");

    match WeCom::from_env()? {
        Some(wecom) => match wecom.send_text(&title, &content) {
            Ok(()) => println!("企业微信推送成功"),
            Err(e) => println!("企业微信推送失败: {e:#}"),
        },
        None => println!("企业微信配置不完整 (CORP_ID / SECRET / AGENT_ID)，跳过推送"),
    }
    Ok(())
}
