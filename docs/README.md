# HaYenai 启动器 · 文档索引

## 一句话定位

基于 [HaYenai](https://github.com/Mystic-Stars/Axolotl) 的个人二次开发项目：先跑通上游、再按自己的口味改界面与交互，**不推翻上游的功能与架构**。

## 当前状态（2026-10-17）

| 项 | 状态 |
|---|---|
| 上游基线 | `Mystic-Stars/Axolotl` @ `27b1488`（release 1.10.0-beta.1） |
| 依赖安装 | 已完成（`pnpm install`，14 个 workspace 项目） |
| 首次构建 | 进行中 / 见构建日志 |
| 源码改动 | **零**（这一阶段只装不改） |
| 品牌替换 | 未开始 |

## 事实基线（改任何决策前先看这张表）

| 事实 | 值 |
|---|---|
| 形态 | monorepo：`apps/` 6 个 + `packages/` 11 个，turbo + pnpm workspace |
| 桌面端 | `apps/app`（Rust + Tauri 2）、`apps/app-frontend`（Vue 3 + Vite） |
| 包管理器 | `pnpm@10.33.2`（`packageManager` 字段钉死） |
| Node | `>=24.15.0`（`.nvmrc` 为 24.15.0） |
| Rust | `1.95.0`（`rust-toolchain.toml` 钉死） |
| 子模块 | `apps/app/vendor/cubiomes`、`third-party/blockbench`，缺了会影响构建 |
| 许可 | 桌面包 **GPL-3.0-only**；各子包各自声明；细则见 `COPYING.md` |
| 产品标识 | `productName: HaYenai`、`identifier: red.ghs.axolotl` |

## 三份文档

| 文档 | 什么时候读 |
|---|---|
| [协作规范.md](协作规范.md) | 开工前：分支怎么开、提交怎么写、依赖能不能加、自审怎么做 |
| [交付流程.md](交付流程.md) | 每个需求：五个阶段、每阶段产出物与放行判据 |
| 本文件 | 想知道「现在到哪一步了」 |

## `docs/` 目录约定

```
docs/
├── README.md              本索引（状态与基线，改状态时更新）
├── 协作规范.md            长期有效的工程约束
├── 交付流程.md            流水线定义
├── requirements/          需求卡，YYYY-MM-DD-<slug>.md
├── canvas/                画布评审留档，按需求 slug 分目录
└── releases/              验证报告与发版记录，v<X.Y.Z>.md
```

规则：`docs/` 只放长期有用的东西。一次性的调试日志、临时截图、构建日志不进仓库（构建日志留在终端或本地临时目录）。

## 三条不长眼睛就会踩的红线

1. **pnpm 版本必须 10.33.2**。本机全局 pnpm 是 12.x，会盖住 corepack 的版本，导致 `packages/api-client` 的 `prepare` 里嵌套 `pnpm run build` 直接报 `ERR_PNPM_BAD_PM_VERSION`。用垫片或显式 `corepack pnpm@10.33.2`，别去改 `package.json`。
2. **子模块没拉 = 构建必挂**。`apps/app/vendor/cubiomes` 是 C 源码，`third-party/blockbench` 是皮肤编辑器。先 `git submodule update --init --depth 1`。
3. **`COPYING.md` 的商标条款是硬约束**：Modrinth 的商标、logo、封面图不得随改版重新引入。外观可以换，来源不能抹。

## 变更记录指针

- 工程约定变更 → 改 `协作规范.md` 并在文末「修订记录」加一行
- 流程变更 → 改 `交付流程.md`，同步更新闸门总表
- 版本变更 → `CHANGELOG.md`（发版时从提交摘要提炼）
