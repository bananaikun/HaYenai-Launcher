import teamData from './team.json'

export interface TeamMember {
	name: string
	avatar: string
	avatarRemote?: string
	url?: string
}

const teamAvatarModules = import.meta.glob('./avatars/*', {
	eager: true,
	import: 'default',
	query: '?url',
}) as Record<string, string>

/**
 * 上游（Mystic Stars / Axolotl）团队成员数据。
 *
 * 关于页现在不展示它 —— 贡献者列表按主人要求整块去掉了。
 * 数据保留是因为它属于**上游归属信息**（GPL 项目要能说清来路），
 * 以后想恢复列表不必去翻仓库历史。
 *
 * `contributors.json` 已删：43KB、100+ 条，和新版关于页无关。
 */
export const teamMembers: (TeamMember & { avatarUrl: string })[] = teamData.map((member) => ({
	...member,
	avatarUrl: teamAvatarModules[`./avatars/${member.avatar}`],
}))
