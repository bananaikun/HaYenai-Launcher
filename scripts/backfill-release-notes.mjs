#!/usr/bin/env node
/**
 * 用仓库里的 `CHANGELOG.md` 补齐既有 GitHub Releases 的说明。
 *
 * 用法（在 `HaYenai-启动器/` 下，token 从 git 凭据或环境变量取）：
 *   node scripts/backfill-release-notes.mjs --dry-run
 *   GITHUB_TOKEN=ghp_xxx node scripts/backfill-release-notes.mjs
 *
 * 为什么单独一个脚本：`publish-github.mjs` 只负责「发当前版本 / 补历史包资产」，
 * 它写历史包正文时只有一句模板。这里只做一件事 —— 把正文按 CHANGELOG 更新一遍，
 * 幂等（正文相同就跳过），不动任何资产。
 */
import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const CHANGELOG = path.join(ROOT, 'CHANGELOG.md')
const OWNER = 'bananaikun'
const REPO = 'HaYenai-Launcher'
const API = 'https://api.github.com'
const REPO_URL = `https://github.com/${OWNER}/${REPO}`

const dryRun = process.argv.includes('--dry-run') || !process.env.GITHUB_TOKEN
const token = process.env.GITHUB_TOKEN || ''

function readChangelog() {
    if (!fs.existsSync(CHANGELOG)) throw new Error('缺少 CHANGELOG.md')
    return fs.readFileSync(CHANGELOG, 'utf8')
}

/** 表格里的一行：`| 1.0.19 | 10-25 | 壁纸属性面板落地… |` */
function tableRows(markdown) {
    const rows = new Map()
    for (const line of markdown.split('\n')) {
        const match = /^\|\s*\**(\d+\.\d+\.\d+)\**\s*\|\s*(\d{2}-\d{2})\s*\|\s*(.+?)\s*\|\s*$/.exec(
            line.trim(),
        )
        if (match) {
            rows.set(match[1], { date: match[2].replace(/\*\*/g, ''), summary: match[3] })
        }
    }
    return rows
}

/** 详细段落：`## 1.0.29（2026-10-31）` 到下一个 `## ` 之前 */
function detailSections(markdown) {
    const sections = new Map()
    const parts = markdown.split(/^## /m)
    for (const part of parts.slice(1)) {
        const match = /^(\d+\.\d+\.\d+)/.exec(part)
        if (!match) continue
        const body = part.slice(part.indexOf('\n') + 1).trim()
        // 「附：发布纪律」这类没有版本号的段落会被上面的正则挡掉
        sections.set(match[1], body.replace(/^---\s*$/gm, '').trim())
    }
    return sections
}

function buildBody(version, details, rows) {
    const detail = details.get(version)
    const row = rows.get(version)
    const footer = [
        '',
        '---',
        '',
        `- 安装包：\`HaYenai-${version}-x64-setup.exe\`（同目录有 \`SHA256SUMS.txt\`）`,
        `- 完整更新记录：[CHANGELOG.md](${REPO_URL}/blob/main/CHANGELOG.md)`,
        `- 自动更新：应用内「设置 → 更新」即可（1.0.29 起，已下载的包会留在本地，不会重复下载）。`,
    ].join('\n')

    if (detail) return `${detail}${footer}`
    if (!row) return null

    return `### ${version} · 2026-${row.date}\n\n- ${row.summary}\n${footer}`
}

async function api(endpoint, init = {}) {
    const response = await fetch(`${API}${endpoint}`, {
        ...init,
        headers: {
            Authorization: `Bearer ${token}`,
            Accept: 'application/vnd.github+json',
            'X-GitHub-Api-Version': '2022-11-28',
            ...(init.headers ?? {}),
        },
    })
    const text = await response.text()
    let body = null
    try {
        body = text ? JSON.parse(text) : null
    } catch {
        body = text
    }
    return { status: response.status, ok: response.ok, body }
}

async function listReleases() {
    const out = []
    for (let page = 1; page <= 5; page += 1) {
        const { ok, body } = await api(`/repos/${OWNER}/${REPO}/releases?per_page=100&page=${page}`)
        if (!ok || !Array.isArray(body) || body.length === 0) break
        out.push(...body)
    }
    return out
}

async function ensureLabel(name, color, description) {
    const existing = await api(`/repos/${OWNER}/${REPO}/labels/${encodeURIComponent(name)}`)
    if (existing.ok) return `存在 ${name}`
    const created = await api(`/repos/${OWNER}/${REPO}/labels`, {
        method: 'POST',
        body: JSON.stringify({ name, color, description }),
    })
    return created.ok ? `新建 ${name}` : `失败 ${name}: ${created.status}`
}

async function main() {
    const markdown = readChangelog()
    const rows = tableRows(markdown)
    const details = detailSections(markdown)
    console.log(`[notes] 表格 ${rows.size} 行，详细段落 ${details.size} 个`)

    if (dryRun) {
        for (const version of [...rows.keys()].sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))) {
            const body = buildBody(version, details, rows)
            console.log(`\n===== ${version} =====\n${body}`)
        }
        console.log('\n[notes] 没有 GITHUB_TOKEN，仅演练')
        return
    }

    // 反馈入口用的标签（Issue 模板里 labels=feedback；不存在时 GitHub 会静默忽略）
    for (const [name, color, description] of [
        ['feedback', '0e8a16', '来自启动器内反馈入口'],
        ['crash', 'd73a4a', '启动器崩溃'],
        ['suggestion', 'a2eeef', '功能建议'],
    ]) {
        console.log('[label]', await ensureLabel(name, color, description))
    }

    const releases = await listReleases()
    console.log(`[notes] 远端 release ${releases.length} 个`)

    for (const release of releases) {
        const version = String(release.tag_name || '').replace(/^v/, '')
        const body = buildBody(version, details, rows)
        if (!body) {
            console.log(`- 跳过 ${release.tag_name}（CHANGELOG 里没有这个版本）`)
            continue
        }
        if ((release.body || '').trim() === body.trim()) {
            console.log(`- 跳过 ${release.tag_name}（正文已一致）`)
            continue
        }
        const updated = await api(`/repos/${OWNER}/${REPO}/releases/${release.id}`, {
            method: 'PATCH',
            body: JSON.stringify({ body }),
        })
        console.log(
            updated.ok ? `- 更新 ${release.tag_name} ✔` : `- 更新 ${release.tag_name} 失败 ${updated.status}`,
        )
    }
}

main().catch((error) => {
    console.error('[notes] 失败：', error.message)
    process.exit(1)
})
