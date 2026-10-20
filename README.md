# GLaDOS 自动登录签到

通过邮箱验证码模拟登录 GLaDOS 后签到，积分满 500 自动兑换 100 天，结果推送到企业微信和邮件。

## 流程

1. 若有保存的 cookie 且仍有效，直接用 cookie 签到（不需要验证码）
2. 否则向 GLaDOS 请求登录验证码，通过 IMAP 从邮箱读取（QQ / Foxmail / 163 / 126）
3. 用验证码登录并保存 cookie，在同一会话中签到、查询积分、按需兑换
4. 推送签到总结：企业微信（纯文本）、邮件通知接口（HTML），已配置的通道都会发送

cookie 保存在 `glados-cookies.json`（可用环境变量 `GLADOS_COOKIE_FILE` 修改路径），已加入 `.gitignore`。该文件等同于账号登录凭证，请勿提交或分享。

## 本地运行

在项目根目录创建 `.env`：

```ini
MAIL_ACCOUNTS=xxx@foxmail.com:授权码,xxx@163.com:授权码
GLADOS_EMAILS=a@qq.com,b@foxmail.com,xxx@163.com
CORP_ID=企业ID
SECRET=应用Secret
AGENT_ID=应用AgentId
TOUSER=@all
MAIL_NOTIFY_URL=https://msg.zhiyuan.win/send-email
MAIL_NOTIFY_TOKEN=邮件接口token
```

```shell
cargo run                  # 签到
cargo run -- --test-push   # 只向所有通知通道发送一条测试推送
```

- `MAIL_ACCOUNTS`：接收验证码的邮箱及授权码。GLaDOS 账号与某个邮箱同名时读取该邮箱，否则读取第一个邮箱（QQ 邮箱别名都收在同一个信箱）。
- `GLADOS_EMAILS`：要签到的 GLaDOS 账号，逗号分隔。

## GitHub Actions

在仓库 `Settings` → `Secrets and variables` → `Actions` → `New repository secret` 逐个添加以下 secret（名称须完全一致）：

| Secret 名称 | 必填 | 说明 | 示例 |
| --- | --- | --- | --- |
| `MAIL_ACCOUNTS` | 是 | 接收 GLaDOS 验证码的邮箱及其 IMAP 授权码，格式 `邮箱:授权码`，多个用英文逗号分隔 | `xxx@foxmail.com:abcdefghijklmnop,xxx@163.com:ABCDEFGHIJKLMNOP` |
| `GLADOS_EMAILS` | 是 | 要签到的 GLaDOS 账号邮箱，多个用英文逗号分隔 | `a@qq.com,b@foxmail.com,xxx@163.com` |
| `CORP_ID` | 否 | 企业微信企业 ID（我的企业 → 企业信息） | `ww0123456789abcdef` |
| `SECRET` | 否 | 企业微信自建应用的 Secret | `AbCdEf...` |
| `AGENT_ID` | 否 | 企业微信自建应用的 AgentId | `1000002` |
| `TOUSER` | 否 | 接收推送的成员 ID，多个用 `\|` 分隔，不填默认 `@all` | `@all` |
| `MAIL_NOTIFY_TOKEN` | 否 | 邮件通知接口的 token，不填则不发邮件 | `xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx` |
| `MAIL_NOTIFY_URL` | 否 | 邮件通知接口地址，不填默认 `https://msg.zhiyuan.win/send-email` | `https://msg.zhiyuan.win/send-email` |
| `GLADOS_COOKIES` | 否 | 登录后保存的 cookie（JSON），由 workflow 自动维护，首次可留空或用本地 `glados-cookies.json` 的内容 | `{"a@qq.com": "gld:sess=...; gld:sess.sig=..."}` |
| `GH_PAT` | 否 | 用于把更新后的 cookie 写回 `GLADOS_COOKIES` 的 Personal Access Token；不填则只使用现有 cookie，失效后重新登录得到的新 cookie 无法保存 | `github_pat_xxx` |

- `CORP_ID`、`SECRET`、`AGENT_ID` 任一未填则跳过企业微信推送；`MAIL_NOTIFY_TOKEN` 未填则跳过邮件通知。任一通道失败不影响其他通道和签到。
- 授权码不是邮箱登录密码：QQ 邮箱在「设置 → 账号 → POP3/IMAP/SMTP 服务」开启 IMAP 后生成；163 邮箱在「设置 → POP3/SMTP/IMAP」开启 IMAP 后生成。
- QQ 邮箱的英文别名、Foxmail、QQ 号邮箱都收在同一个信箱，`MAIL_ACCOUNTS` 填其中一个即可。
- 企业微信自建应用需在「企业可信 IP」中加入调用来源 IP；GitHub Actions 的 IP 不固定，若推送报 `60020 not allow to access from your ip`，需改用固定 IP 的代理或在本地运行。

`.github/workflows/checkin.yml` 每天北京时间 10:00 触发一次，随机延迟 0 ~ 4 小时后执行，因此签到时间在 10:00 ~ 14:00 之间随机。每个账号执行前会读取积分历史，判断今日是否已签到，已签到则不再提交签到请求。

cookie 保存在 secret `GLADOS_COOKIES`：运行前写入 `glados-cookies.json`，运行后若 cookie 有变化，用 `GH_PAT` 写回 secret。

`GH_PAT` 创建方式：GitHub `Settings` → `Developer settings` → `Personal access tokens` → `Fine-grained tokens` → `Generate new token`，`Repository access` 只选本仓库，`Permissions` → `Repository permissions` → `Secrets` 设为 `Read and write`，生成后存为本仓库的 secret `GH_PAT`。

本地已有 cookie 时，可用以下命令上传为初始值：

```shell
gh secret set GLADOS_COOKIES --repo <用户名>/<仓库名> < glados-cookies.json
```

在 Actions 页面手动运行 workflow 可立即签到。
