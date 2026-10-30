<div align="center">

<img src="https://raw.githubusercontent.com/bananaikun/HaYenai-Launcher/main/apps/app/icons/128x128.png" width="120" height="120" alt="HaYenai" />

# HaYenai

**Minecraft Java 版桌面客户端 —— 液态玻璃界面，壁纸即皮肤。**

<p>
  <a href="https://github.com/bananaikun/HaYenai-Launcher/releases"><img src="https://img.shields.io/github/v/release/bananaikun/HaYenai-Launcher?style=for-the-badge&logo=github" alt="Release" /></a>
  <a href="https://github.com/bananaikun/HaYenai-Launcher/stargazers"><img src="https://img.shields.io/github/stars/bananaikun/HaYenai-Launcher?style=for-the-badge&logo=github&color=ffb800" alt="Stars" /></a>
  <img src="https://img.shields.io/badge/License-GPL--3.0-blue.svg?style=for-the-badge" alt="License" />
</p>

<p>
  <a href="#下载">下载</a> ｜
  <a href="#特性">特性</a> ｜
  <a href="#本地构建">构建</a> ｜
  <a href="CONTRIBUTING.md">参与贡献</a>
</p>

</div>

---

## 这是什么

HaYenai 是一个开源的 Minecraft Java 版第三方启动器：在一个客户端里搜索、安装、更新来自
Modrinth 与 CurseForge 的模组、整合包、资源包和光影，并管理实例、账户与个性化外观。

**本项目是对 [Axolotl](https://github.com/Mystic-Stars/Axolotl) 的二次修改（fork）**，
而 Axolotl 自身是 [Modrinth App](https://github.com/modrinth/code) 的衍生。按上游授权，
本仓库同样以 **GPL-3.0-only** 发布，并保留全部上游署名与许可声明（见 [COPYING.md](COPYING.md)）。

本仓库与上游 Axolotl、Modrinth 官方团队**没有隶属关系**。上游不对本仓库的内容负责，
本仓库也不代表上游立场。

## 在上游基础上做了什么

fork 后的改动集中在**界面材质**和**壁纸引擎**两块，功能与结构与上游保持高度一致：

**界面**

- 统一的玻璃材质系统：侧栏、标题栏、卡片、面板共用一套配方（压暗 / 模糊 / 饱和三档可调），壁纸能真正透出来
- 主题色跟随壁纸取色，深浅色可独立跟随或手动锁定；提供强调色的对比度钳位，浅壁纸下文字不会糊
- 自绘标题栏，无系统边框；路由切换无黑屏闪烁

**壁纸引擎**

- 导入本地 Wallpaper Engine 包（场景 / 视频 / 网页），或从 Steam 库扫描导入
- 壁纸轮播：关闭 / 按顺序 / 随机，间隔 5–90 秒滑块可调，可多选参与轮播的壁纸
- **可调项面板**：读取包内 `project.json` 的 `general.properties`，按类型渲染滑块 / 开关 / 颜色 / 分组，
  支持 `condition` 条件显隐（例如「只在媒体播放器开启时才显示音量位置」），改动实时热更
- **画质四档**（低 / 中 / 高 / 超高）：帧率、抗锯齿（关闭 / FXAA / MSAA ×2 / MSAA ×4）、
  后处理、粒子、纹理分辨率
- 音乐频谱可视化：宿主把分析器的左右各 64 段频谱，以 `setAudio({ snapshot })` 拉模型喂给壁纸

**其他**

- AI 助手（可配置多家供应商）
- 音乐播放器、歌词与频谱条
- 服务器状态探测，显示在线人数与服务器图标
- 中文界面，国内镜像与 DoH 默认可用

## 下载

前往 [Releases](https://github.com/bananaikun/HaYenai-Launcher/releases) 下载对应平台的安装包。

| 平台 | 文件 |
| --- | --- |
| Windows 10/11 (x64) | `HaYenai-*-x64-setup.exe` |
| macOS | 暂未提供 |
| Linux | 暂未提供 |

更新检查已指向本仓库的 Releases，但**自动更新尚未接入**，目前需要手动下载安装包覆盖安装。

## 本地构建

**环境要求**

- Node.js **≥ 24.15.0**（仓库锁定 `pnpm@10.x`，用 corepack 即可）
- pnpm（`corepack enable` 后自动可用）
- Rust 工具链（stable）
- JDK **21**（Rust 构建脚本会调 Gradle 编译 Java 侧组件）

**步骤**

```bash
corepack pnpm install

# 开发模式
corepack pnpm app:dev

# 打安装包
corepack pnpm app:build
```

**已知构建注意**

- 打包前确认 `JAVA_HOME` 指向 JDK 21
- 内存不足的机器加 `CARGO_BUILD_JOBS=1`，否则链接 Rust 主程序时可能触发 LLVM OOM
- Rust 依赖走 crates.io 索引，国内网络不佳时可用 `CARGO_NET_OFFLINE=true`（依赖已缓存的前提下）

## 许可

**GPL-3.0-only**。

派生自 Axolotl（GPL-3.0-only），因此本项目必须同样以 GPL-3.0 发布。
许可证全文见 [LICENSE](LICENSE)，上游与第三方组件的归属见 [COPYING.md](COPYING.md) 与 `third-party/licenses/`。

Modrinth 的商标、Logo、封面图等受限品牌资源不随本项目分发。

## 致谢

本项目的绝大部分工作来自上游与这些项目：

- [Axolotl](https://github.com/Mystic-Stars/Axolotl) —— 直接基座
- [Modrinth App](https://github.com/modrinth/code) —— 上游的上游
- [webwallgl](https://github.com/oneincase/webwallgl) —— Wallpaper Engine 壁纸渲染
- [Plain Craft Launcher](https://github.com/Meloong-Git/PCL) —— 中文项目名快照
- [LogShare-Web-UI](https://github.com/NingZeStudio/LogShare-Web-UI) —— 日志查看器
