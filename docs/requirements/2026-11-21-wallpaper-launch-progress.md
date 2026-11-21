# 需求卡：壁纸切换提速、启动失败不再卡住、磁悬条临时进度

日期：2026-11-21　提出：主人　实施：奈奈

## 目标

1. **壁纸切换不等加载**：选中新壁纸时先显示首帧静帧，引擎就绪后淡入；静帧由首帧快照预生成，冷启动/首次渲染不再让用户看空白或旧画面。
2. **启动实例不再「卡住」**：启动流程任一步失败或超时都要让 UI 复位并给出原因；同时压缩每次启动的固定重复成本（少做无用功、能并行就并行）。
3. **磁悬条显示临时进度**：启动实例、下载、安装进行中时，条子整条切成占用式进度视图（阶段文案 + 百分比 + 可取消），结束自动回原页并保留轮播位置。

## 非目标

- 「壁纸有时候会失效」按主人要求**不做**（明确排除，不顺手改）。
- 不重做磁悬条的信息页结构（服务器/歌词/系统等页不动）。
- 不改启动器的更新链路、不改发布脚本。
- 不动 `docs/` 之外的历史文档结构（只补更新记录与版本历史相应条目）。

## 完成判据

1. 切换壁纸时 300ms 内出现新壁纸的首帧静帧（同尺寸、无拉伸），引擎就绪后 <400ms 淡入；切回旧壁纸不重新解码静帧。
2. 静帧在导入壁纸时预生成并落盘；已有壁纸在启动后后台补齐，缺失时不报错、回落到实时渲染。
3. 构造三类失败（Java 缺失、依赖下载断网、进程 spawn 失败），每次都在 60 秒内回到可重试状态且提示原因；不残留「启动中」状态。
4. 启动耗时：同一实例第二次启动的固定开销明显下降（给出改前/改后实测秒数与阶段分解）。
5. 条子在启动/下载/安装期间显示进度视图，任务结束 1 秒内回到原页且页码不跳；无任务时不出现。
6. `cargo check -p theseus`、`cargo check -p theseus_gui`、`pnpm --filter @modrinth/app-frontend build` 通过；改过的前端文件 eslint 无新增错误。
7. 三批各一笔提交，提交信息带 `[奈奈]` 前缀；发布到 1.0.50 并写发版验证文档。

## 落地与判定（2026-11-21）

| 判据 | 状态 | 证据 |
| --- | --- | --- |
| 1 切换 300ms 出静帧、<400ms 淡入 | 已实现，待真机计时 | `WallpaperLayer.vue` 的 `posterVisible` 本来就一到 `engineReady` 才收起；`wallpaperPosterUrl` 改成优先 `/posters/<id>-frame.png`，所以切换时垫的是上次真实渲染帧 |
| 2 导入时预生成、已有壁纸后台补齐、缺失回落 | 部分实现 | 抓帧在 `onFirstFrame` 后 1.2s 触发（`captureFrameSnapshot()`），首次渲染成功即落盘；导入瞬间还没渲染，故「导入时」实际是首次显示后补齐，缺失时仍回落包内 `preview` |
| 3 三类失败 60 秒内可重试 | 已实现，待真机 | 维护锁 20s 超时（备份/还原占用时明确报错）、准备阶段超时若已登记进程按已启动返回、JVM 探测 10s 超时、`insert_new_process` 的 emit 失败不再返回错误、实例页 catch 先清 loading 再分析崩溃 |
| 4 第二次启动固定开销下降 | 部分实现，待实测 | 库 jar 加 `<jar>.verified` 校验戳（size+mtime+sha1 全等则跳过整份 SHA-1）；显式 Java 路径不再重复 `check_jre`（原来两次 JVM 探测）；GC 探测结果加落盘缓存（`caches/jvm-probe-cache.json`，键含 Java 路径+大小+mtime+参数，只缓存通过的结果，见 `jvm_args.rs` 的 `remember_supported_prefix`），启动器重启后不再为每个候选 GC 块起一次 `java -version`；natives 校验戳、准备段并行仍未做 |
| 5 条子进度视图 | 已实现 | `.mb-pages` 内 `v-if="activity"` 的 `mb-activity` 段（不切 mode，页码不跳），启动显示阶段文案（preparing/libraries/starting 由后端 `launch_stage` 事件推），安装/下载显示百分比与取消按钮 |
| 6 三处检查 | 通过 | `cargo check -p theseus` / `-p theseus_gui` exit 0、`pnpm --filter @modrinth/app-frontend build` ✓ 55.87s、改动文件 eslint 仅 HEAD 基线错误 |
| 7 提交与 1.0.50 | 已完成 | `7c3bfb1` + `058e25d`（`public-main`），tag `hayenai-v1.0.50`，release id 453520694 |

### 审计式自检（2026-11-21 15:40 补）

自检发现两个漏项，均在 1.0.51 修掉：

1. 磁悬条新增的 6 条文案（`app.magnetic.activity` / `cancel-activity` / `installing-job` / `stage-preparing` / `stage-libraries` / `stage-starting`）只写了 `defaultMessage`，没有进 `locales/{en-US,zh-CN}/index.json`，中文环境下会回落成英文。1.0.50 已发布，所以用 1.0.51 补上（en-US 5308→5314 键、zh-CN 5315→5321 键，无重复键）。
2. 静帧写入成功后没有更新 `wallpaperState.items` 里的 `hasFrame`，导致同一会话内切回该壁纸仍垫 `preview`，要等下次 `refreshWallpapers()`（重开启动器）。现在 `captureFrameSnapshot()` 成功后直接 `cached.hasFrame = true`。

其余自检结论：

- `store.write_frame` 自己 `create_dir_all(posters_dir())`，不依赖导入流程建目录。
- `/posters/` 旁路放在包 id 解析之前，避免 `posters` 被当成合法包名；`safe_rel` 拦 `../`。
- `clearLaunchActivity()` 会清 180s 兜底定时器，不会残留定时器把新一次启动的视图提前收掉。


### 未做（明确记账）

- 「壁纸有时候会失效」按主人要求不做。
- 切回旧壁纸复用库内 pkg 解析缓存：不做。`SceneWallpaper.vue` 主动 `releasePkgCache` 是为了不随壁纸数量累积内存/显存，只在「切回同一张」时省一次解析，收益抵不过泄漏风险。
- natives 大小校验戳、准备段（manifest / Java / 库）并行：留待 1.0.52。

### GC 探测磁盘缓存（2026-11-21 16:20 补）

启动器每重启一次，第一次启动游戏就要为每个候选 GC 参数块起一次 `java -version`（每次最多等 5 秒），纯属重复劳动。现在把「通过」的结果写进 `caches/jvm-probe-cache.json`：

- 键 = Java 可执行文件路径 + 文件大小 + 修改时间 + 参数列表，JDK 换版本或参数变了自动失效；
- 只缓存 `supported = true`：超时和 spawn 失败可能是瞬时的，缓存下来会把这个参数块永久判死；
- 命中时不写 stderr 文本（与进程内缓存命中一致），上层只关心「能不能用」；
- 超过 256 条就整体清空——一个 JDK 一辈子也就几套候选，不值得为它写淘汰算法。
- steam 导入的 `hash_dir` 全量 SHA-1、`copy_tree` 全量复制：未改，属长任务，需要进度反馈而非单纯提速。

