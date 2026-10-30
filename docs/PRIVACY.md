# HaYenai 启动器 · 隐私政策

> 最后更新：2026-10-17
> 适用范围：HaYenai 启动器桌面客户端（`com.hayenai.launcher`）

## 一句话

启动器默认只在你自己的电脑上工作：**不收集、不上传任何个人信息**。只有你主动开启的功能（遥测、Discord 状态）才会向外发送数据，且随时可关。

## 1. 数据存在哪里

全部在本机，默认目录：

| 平台 | 路径 |
|---|---|
| Windows | `%APPDATA%\com.hayenai.launcher\` |
| macOS | `~/Library/Application Support/com.hayenai.launcher/` |
| Linux | `~/.config/com.hayenai.launcher/` |

其中包含：设置（SQLite `app.db`）、日志（`logs/`）、实例与账户索引、缓存。

便携模式下（可执行文件同目录存在 `.HaYenai` 文件夹）改为把数据写在可执行文件旁。

删除该目录即等于清除全部本地数据；卸载启动器不会自动删除它。

## 2. 账户凭据

- 微软账号登录走官方设备码流程，**启动器不接触你的密码**。
- 登录后的令牌存放在系统凭据库（Windows Credential Manager / macOS Keychain / Secret Service），不写入明文配置。
- 令牌只用于向 Mojang / Microsoft 换取游戏会话，不发送给本项目作者或任何第三方。

## 3. 网络请求都发往哪里

| 目的 | 目标 |
|---|---|
| 游戏文件、版本清单、模组/整合包元数据 | Mojang、Fabric、Forge、NeoForge、Modrinth、CurseForge 等公开 API |
| 启动器更新检查、公告、服务器状态 | 本项目自建更新服务（DNS SRV `_hayenai-update._tcp.v.bananaikun.dynv6.net`） |
| 玩家身份与服务器查询（仅在你使用时） | 你指定的 Minecraft 服务器 |

更新检查会带上 `X-HaYenai-Channel` / `X-HaYenai-Platform` / `X-HaYenai-Version` 三个头（仅版本、平台、通道），**不含用户名、账号 ID、硬件指纹或 IP 之外的任何标识**。

更新服务不可达时，启动器照常离线运行，不会因此限制任何本地功能。

## 4. 遥测（默认关闭）

- 首次启动会询问是否同意匿名遥测，**默认关闭**，拒绝不影响任何功能。
- 即使同意，本构建也**没有内置收集端点**：只有环境变量 `THESEUS_TELEMETRY_ENDPOINT` 被显式设置时才会发送，否则事件只写入本地队列并自然过期。
- 可在「设置 → 隐私」随时关闭。

## 5. Discord 状态（默认关闭）

开启后会把「正在启动某实例」之类的状态显示到你的 Discord 个人资料，由 Discord 客户端处理。关闭后立即停止。

## 6. 日志

日志只写本地 `logs/`，默认保留最近若干会话。导出日志由你手动触发，内容可能包含本地路径；**发送给他人前请自行检查**。

## 7. 第三方内容

启动器使用 Modrinth、CurseForge 等平台的内容索引；这些平台各自有隐私政策，与本项目无关。本项目与 Mojang、Microsoft、Modrinth 均无从属关系。

## 8. 变更与联系

本政策变更会随版本发布更新本文件。问题与数据删除请求：在项目仓库提 issue。

```plaintext
项目仓库：https://github.com/Bananaikun/HaYenai
上游来源：https://github.com/Mystic-Stars/Axolotl（GPL-3.0-only）
```
