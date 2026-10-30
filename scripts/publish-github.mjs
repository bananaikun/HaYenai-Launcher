#!/usr/bin/env node
/**
 * 把 `release/<版本>/` 里的产物发到 GitHub Releases。
 *
 * 用法（在 `HaYenai-启动器/` 下）：
 *   GITHUB_TOKEN=ghp_xxx node scripts/publish-github.mjs             # 发当前版本
 *   GITHUB_TOKEN=ghp_xxx node scripts/publish-github.mjs --archive   # 历史包，每版一个 release
 *   node scripts/publish-github.mjs --dry-run                        # 只打印要做什么
 *
 * 三条硬约束（踩过的坑）：
 * 1. **`latest.json` 必须落在"最新 release"里**：更新源是
 *    `releases/latest/download/latest.json`。GitHub 的 `latest` 默认取"最近创建的
 *    非预发布 release"，所以历史包会把当前版本挤下去 ⇒ 用 API 的 `make_latest`
 *    显式指定：历史包一律 `false`，当前版本 `true`。
 * 2. 资产名不能重复上传（409）。已存在就跳过，不报错 —— 重跑本脚本应当幂等。
 * 3. 没有 token 时只做 dry-run 提示，不做任何网络写操作。
 */
import fs from 'node:fs'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const FRONTEND = path.join(ROOT, 'apps', 'app-frontend')
const RELEASE_DIR = path.join(ROOT, 'release')
const OWNER = 'bananaikun'
const REPO = 'HaYenai-Launcher'
const API = 'https://api.github.com'
const UPLOAD = 'https://uploads.github.com'

const argv = process.argv.slice(2)
const dryRun = argv.includes('--dry-run') || !process.env.GITHUB_TOKEN
const archiveMode = argv.includes('--archive')
const token = process.env.GITHUB_TOKEN || ''

const readVersion = () => JSON.parse(fs.readFileSync(path.join(FRONTEND, 'package.json'), 'utf8')).version

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

/** 找到 tag 对应的 release，没有就建。`makeLatest` 决定它能不能被 /releases/latest 选中。 */
async function ensureRelease(version, notes, makeLatest) {
    const tag = `v${version}`
    const existing = await api(`/repos/${OWNER}/${REPO}/releases/tags/${tag}`)
    if (existing.ok) {
        console.log(`  · release ${tag} 已存在（id ${existing.body.id}）`)
        return existing.body
    }
    const created = await api(`/repos/${OWNER}/${REPO}/releases`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
            tag_name: tag,
            name: `HaYenai ${version}`,
            body: notes,
            draft: false,
            prerelease: false,
            make_latest: makeLatest ? 'true' : 'false',
        }),
    })
    if (!created.ok) throw new Error(`建 release ${tag} 失败：${created.status} ${JSON.stringify(created.body).slice(0, 300)}`)
    console.log(`  · release ${tag} 已创建（id ${created.body.id}）`)
    return created.body
}

async function uploadAsset(releaseId, file) {
    const name = path.basename(file)
    const list = await api(`/repos/${OWNER}/${REPO}/releases/${releaseId}/assets?per_page=100`)
    if (list.ok && Array.isArray(list.body) && list.body.some((a) => a.name === name)) {
        console.log(`  · 已存在，跳过：${name}`)
        return
    }
    const data = fs.readFileSync(file)
    const response = await fetch(
        `${UPLOAD}/repos/${OWNER}/${REPO}/releases/${releaseId}/assets?name=${encodeURIComponent(name)}`,
        {
            method: 'POST',
            headers: {
                Authorization: `Bearer ${token}`,
                Accept: 'application/vnd.github+json',
                'X-GitHub-Api-Version': '2022-11-28',
                'Content-Type': 'application/octet-stream',
                'Content-Length': String(data.length),
            },
            body: data,
        },
    )
    if (!response.ok) {
        const detail = await response.text()
        throw new Error(`上传 ${name} 失败：${response.status} ${detail.slice(0, 300)}`)
    }
    const mb = (data.length / 1024 / 1024).toFixed(1)
    console.log(`  · 已上传 ${name}（${mb} MB）`)
}

/** 当前版本：安装包 + .sig + latest.json + SHA256SUMS，并抢占 `latest`。 */
async function publishCurrent() {
    const version = readVersion()
    const dir = path.join(RELEASE_DIR, version)
    if (!fs.existsSync(dir)) throw new Error(`没有产物目录：${dir}`)

    const notesPath = path.join(dir, 'NOTES.md')
    const notes = fs.existsSync(notesPath)
        ? fs.readFileSync(notesPath, 'utf8').trim()
        : `HaYenai 启动器 ${version}`
    const files = fs
        .readdirSync(dir)
        .filter((name) => /\.(exe|sig|json|txt)$/.test(name))
        .map((name) => path.join(dir, name))
    const hasLatest = files.some((file) => path.basename(file) === 'latest.json')
    if (!hasLatest) throw new Error('产物里没有 latest.json：先跑 build-release.mjs')

    console.log(`[publish] 当前版本 ${version} → ${files.length} 个文件`)
    if (dryRun) {
        files.forEach((file) => console.log(`  (dry) ${path.basename(file)}`))
        return
    }
    const release = await ensureRelease(version, notes, true)
    for (const file of files) await uploadAsset(release.id, file)
}

/** 历史包：release/ 下除当前版本外，每个目录一个 release。 */
async function publishArchive() {
    const current = readVersion()
    const dirs = fs
        .readdirSync(RELEASE_DIR)
        .filter((name) => /^\d+\.\d+\.\d+$/.test(name) && name !== current)
        .sort((a, b) => a.localeCompare(b, undefined, { numeric: true }))
    console.log(`[publish] 历史版本 ${dirs.length} 个：${dirs.join(', ')}`)
    for (const version of dirs) {
        const dir = path.join(RELEASE_DIR, version)
        const files = fs
            .readdirSync(dir)
            .filter((name) => /\.(exe|txt)$/.test(name) && !name.endsWith('.sig'))
            .map((name) => path.join(dir, name))
        console.log(`- ${version}（${files.length} 个文件）`)
        if (dryRun) {
            files.forEach((file) => console.log(`  (dry) ${path.basename(file)}`))
            continue
        }
        // 历史包绝不抢 latest：更新器只能指向当前版本
        const release = await ensureRelease(version, `HaYenai 启动器 ${version}（历史版本）`, false)
        for (const file of files) await uploadAsset(release.id, file)
    }
}

async function main() {
    if (dryRun && !process.env.GITHUB_TOKEN) {
        console.log('[publish] 没有 GITHUB_TOKEN，仅演练（不发任何请求）')
    }
    if (archiveMode) await publishArchive()
    else await publishCurrent()
    console.log('[publish] 完成')
}

main().catch((error) => {
    console.error('[publish] 失败：', error.message)
    process.exit(1)
})
