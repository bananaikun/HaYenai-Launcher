-- AI 方案（Preset）：人格 + 规则 + 绑定的厂商/模型，打包成一个可选用的成品。
--
-- 为什么是一张表而不是「人格库 + 方案引用」：这一阶段的消费者只有 AI 机器人和
-- Agent，它们要的是「一选就完整」的东西。人格复用等真的出现「同一段人格配多个模型」
-- 的需求再加拆分，现在拆只会让 UI 多一层选择。
CREATE TABLE ai_presets (
	id TEXT NOT NULL PRIMARY KEY,
	name TEXT NOT NULL,
	-- 空串表示「跟随当前启用情况」：不给方案绑死模型，留给调用方自己挑。
	provider_id TEXT NOT NULL DEFAULT '',
	model_id TEXT NOT NULL DEFAULT '',
	persona TEXT NOT NULL DEFAULT '',
	rules TEXT NOT NULL DEFAULT '',
	-- 毫秒时间戳，列表按它倒序（最近编辑的在上），与项目其它表保持同一口径。
	created_at INTEGER NOT NULL DEFAULT 0,
	updated_at INTEGER NOT NULL DEFAULT 0
);

-- 当前选中的方案。机器人 / Agent 读这一个值，不必自己记。
ALTER TABLE ai_settings ADD COLUMN active_preset_id TEXT NOT NULL DEFAULT '';
