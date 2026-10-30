-- HaYenai 首次安装默认值（2026-10-25 12:10）
--
-- 主人要求：「第一次安装默认的设置，就按照我现在使用的这个设置来。」
-- 他当前的配置在 `%APPDATA%\com.hayenai.launcher\beta\app.db` 的 settings 行里，
-- 下面这些是他实际在用的值，与出厂默认不同的部分。
--
-- ⚠ 三条纪律：
-- 1. **不改已执行过的 migration**（init.sql 等）—— sqlx 会校验 checksum，改了会直接启动报错。
--    所以这里用一条新的、时间戳最晚的 migration。
-- 2. 每一条都带 `WHERE ... AND 字段 = 出厂默认`：**只升级"从没动过"的字段**，
--    用户自己改过的值一律不覆盖（否则升级会强行抹掉别人的设置）。
-- 3. 这些 SQL 全是幂等的：已经等于目标值的行不会被再次更新。

-- 界面 / 语言
UPDATE settings SET locale = 'zh-CN'
    WHERE id = 0 AND (locale IS NULL OR locale = '');

UPDATE settings SET accent_color = 'system'
    WHERE id = 0 AND accent_color = 'pink';

-- 首次安装直接开窗框（主人当前是开的）
UPDATE settings SET native_decorations = 1
    WHERE id = 0 AND native_decorations = 0;

-- 遥测 / 开发者模式（主人当前都是开的）
UPDATE settings SET telemetry = 1
    WHERE id = 0 AND telemetry = 0;

UPDATE settings SET developer_mode = 1
    WHERE id = 0 AND developer_mode = 0;

-- 默认分配内存：2048 → 4096
UPDATE settings SET mc_memory_max = 4096
    WHERE id = 0 AND mc_memory_max = 2048;

-- 国内镜像（三源全开）
UPDATE settings SET use_minecraft_mirror = 1
    WHERE id = 0 AND use_minecraft_mirror = 0;

UPDATE settings SET use_modrinth_mirror = 1
    WHERE id = 0 AND use_modrinth_mirror = 0;

-- 下载引擎：xmcl → legacy（主人当前用的 legacy）
UPDATE settings SET download_engine = 'legacy'
    WHERE id = 0 AND download_engine = 'xmcl';

-- 启动游戏时进入轻量模式（省内存，主人当前开着）
UPDATE settings SET enter_lightweight_mode_on_game_launch = 1
    WHERE id = 0 AND enter_lightweight_mode_on_game_launch = 0;
