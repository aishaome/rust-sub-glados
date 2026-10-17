# GLaDOS 自动登录签到

通过邮箱验证码模拟登录 GLaDOS 后签到，积分满 500 自动兑换 100 天，结果推送到企业微信。

## 流程

1. 向 GLaDOS 请求登录验证码
2. 通过 IMAP 从邮箱读取验证码（QQ / Foxmail / 163 / 126）
3. 用验证码登录，并在同一会话中签到、查询积分、按需兑换
4. 企业微信推送签到总结（纯文本）

## 本地运行

在项目根目录创建 `.env`：

```ini
MAIL_ACCOUNTS=xxx@foxmail.com:授权码,xxx@163.com:授权码
GLADOS_EMAILS=a@qq.com,b@foxmail.com,xxx@163.com
CORP_ID=企业ID
SECRET=应用Secret
AGENT_ID=应用AgentId
TOUSER=@all
```

```shell
cargo run                  # 签到
cargo run -- --test-push   # 只发送一条测试推送
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

- `CORP_ID`、`SECRET`、`AGENT_ID` 任一未填则跳过推送，不影响签到。
- 授权码不是邮箱登录密码：QQ 邮箱在「设置 → 账号 → POP3/IMAP/SMTP 服务」开启 IMAP 后生成；163 邮箱在「设置 → POP3/SMTP/IMAP」开启 IMAP 后生成。
- QQ 邮箱的英文别名、Foxmail、QQ 号邮箱都收在同一个信箱，`MAIL_ACCOUNTS` 填其中一个即可。
- 企业微信自建应用需在「企业可信 IP」中加入调用来源 IP；GitHub Actions 的 IP 不固定，若推送报 `60020 not allow to access from your ip`，需改用固定 IP 的代理或在本地运行。

`.github/workflows/checkin.yml` 在北京时间 8 ~ 19 点每小时触发一次，每次以「1 / 剩余次数」的概率执行，最后一次必定执行，再随机延迟 0 ~ 30 分钟，因此签到时间在 8 ~ 20 点之间随机；当天签到后以 cache 标记，后续触发直接跳过。

在 Actions 页面手动运行 workflow 可立即签到。
