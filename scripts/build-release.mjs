#!/usr/bin/env node
/**
 * HaYenai 打包脚本 —— 安装包 + 便携版，产物文件名强制带版本号。
 *
 * 用法（在本目录 `HaYenai-启动器/` 下）：
 *   node scripts/build-release.mjs              # 用 package.json 里的版本
 *   node scripts/build-release.mjs --bump patch # 先把补丁号 +1 再打
 *   node scripts/build-release.mjs --bump minor
 *
 * 产出：
 *   release/<version>/HaYenai-<version>-x64-setup.exe   NSIS 安装包（只要这一个产物）
 *   release/<version>/SHA256SUMS.txt                    校验值
 *
 * 说明：
 * - 版本号唯一来源是 `apps/app-frontend/package.json`（tauri.conf.json 引用它），
 *   所以只改这一处，窗口标题/关于页/安装包名会一起变。
 * - 前端先单独 build，再用 skip-frontend 配置让 tauri 跳过 beforeBuildCommand，
 *   避免 turbo 再跑一遍（那会多花 40s 且可能污染 dist）。
 */
import { execFileSync, execSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import process from 'node:process'

const ROOT = path.resolve(import.meta.dirname, '..')
const FRONTEND = path.join(ROOT, 'apps', 'app-frontend')
const APP = path.join(ROOT, 'apps', 'app')
const PKG = path.join(FRONTEND, 'package.json')
const SKIP_CONFIG = path.join(ROOT, 'scripts', 'skip-frontend.json')

function readVersion() {
    return JSON.parse(fs.readFileSync(PKG, 'utf8')).version
}

function bump(kind) {
    const pkg = JSON.parse(fs.readFileSync(PKG, 'utf8'))
    const parts = pkg.version.split('.').map(Number)
    if (parts.length !== 3 || parts.some(Number.isNaN)) {
        throw new Error(`版本号不是 x.y.z：${pkg.version}`)
    }
    if (kind === 'major') {
        parts[0] += 1
        parts[1] = 0
        parts[2] = 0
    } else if (kind === 'minor') {
        parts[1] += 1
        parts[2] = 0
    } else {
        parts[2] += 1
    }
    pkg.version = parts.join('.')
    fs.writeFileSync(PKG, `${JSON.stringify(pkg, null, 2)}\n`)

    // Cargo.toml 的 version 必须跟着变：`tauri build` 会比对 conf 里的版本与 crate 版本，
    // 不一致直接失败（1.0.0 → 1.0.1 那次就是这么挂的，报错还藏在长日志里）。
    const cargoPath = path.join(APP, 'Cargo.toml')
    const cargo = fs.readFileSync(cargoPath, 'utf8')
    fs.writeFileSync(
        cargoPath,
        cargo.replace(/^version = ".*"$/m, `version = "${pkg.version}"`),
    )

    return pkg.version
}

const sha256 = (file) =>
    createHash('sha256').update(fs.readFileSync(file)).digest('hex')

function main() {
    const argv = process.argv.slice(2)
    const bumpIndex = argv.indexOf('--bump')
    let version = readVersion()
    if (bumpIndex >= 0) {
        const kind = argv[bumpIndex + 1] || 'patch'
        version = bump(kind)
        console.log(`[build] 版本号递增 → ${version}`)
    }
    console.log(`[build] 版本 = ${version}`)

    // 1) 前端
    console.log('[build] vite build …')
    execSync('npx vite build', { cwd: FRONTEND, stdio: 'inherit' })

    // 2) tauri：只出 NSIS 安装包（"all" 会额外拉 WiX 打 msi，Windows 上常失败）
    fs.mkdirSync(path.dirname(SKIP_CONFIG), { recursive: true })
    fs.writeFileSync(SKIP_CONFIG, JSON.stringify({ build: { beforeBuildCommand: '' } }))

    // 并行度看可用内存：theseus_gui 单个 rustc 峰值 ~3–4G。
    // 之前固定 1 是因为构建时主人在玩绝区零，内存被吃光才 OOM —— 那是并发占用，
    // 不是硬件不足。空着 10G 以上就放开到 3，只有内存紧张时才退回 1。
    // `HAYENAI_JOBS=1` 可强制串行：theseus 那个 crate 在 7G 空闲 + 2 并发时
    // 会把 rustc 干出 STATUS_STACK_BUFFER_OVERRUN(0xc0000409)。
    const freeGB = os.freemem() / 1024 ** 3
    const forced = Number(process.env.HAYENAI_JOBS || 0)
    const jobs = forced > 0 ? String(forced) : freeGB > 10 ? '3' : freeGB > 6 ? '2' : '1'

    console.log(`[build] 可用内存 ${freeGB.toFixed(1)}G → CARGO_BUILD_JOBS=${jobs}`)
    console.log('[build] tauri build --bundles nsis …')

    const env = {
        ...process.env,
        RUST_MIN_STACK: '67108864',
        CARGO_BUILD_JOBS: jobs,
        JAVA_HOME: process.env.JAVA_HOME || 'C:\\Program Files\\Java\\jdk-21',
    }
    const cargoBin = path.join(process.env.USERPROFILE || '', '.cargo', 'bin')
    if (fs.existsSync(cargoBin)) {
        env.PATH = `${cargoBin}${path.delimiter}${env.PATH ?? ''}`
    }

    // 自动更新的 minisign 私钥。**在仓库外**（不随代码走），口令为空。
    // 丢了就无法再给老版本发更新，只能用新密钥重发一版让用户手动装。
    const signingKey =
        process.env.TAURI_SIGNING_PRIVATE_KEY ||
        path.resolve(ROOT, '..', 'keys', 'hayenai-updater.key')
    if (!fs.existsSync(signingKey)) {
        throw new Error(`缺少签名私钥：${signingKey}（或设置 TAURI_SIGNING_PRIVATE_KEY）`)
    }
    env.TAURI_SIGNING_PRIVATE_KEY = signingKey
    env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD =
        process.env.TAURI_SIGNING_PRIVATE_KEY_PASSWORD ?? ''

    execFileSync(
        'npx',
        [
            'tauri',
            'build',
            '--config',
            'tauri-release.conf.json',
            '--config',
            SKIP_CONFIG,
            '--bundles',
            'nsis',
        ],
        { cwd: APP, stdio: 'inherit', env, shell: true },
    )

    // 3) 收产物 + 改名带版本号
    const outDir = path.join(ROOT, 'release', version)
    fs.mkdirSync(outDir, { recursive: true })

    // 主人 2026-10-24 23:17：只要安装包，不做便携版。

    const nsisDir = path.join(ROOT, 'target', 'release', 'bundle', 'nsis')
    const setupSrc = fs
        .readdirSync(nsisDir)
        .filter((name) => name.endsWith('.exe'))
        .map((name) => path.join(nsisDir, name))
        .sort((a, b) => fs.statSync(b).mtimeMs - fs.statSync(a).mtimeMs)[0]
    if (!setupSrc) throw new Error('没找到 NSIS 安装包，检查 tauri build 输出')
    const setupDst = path.join(outDir, `HaYenai-${version}-x64-setup.exe`)
    fs.copyFileSync(setupSrc, setupDst)

    // 自动更新的签名与清单。`createUpdaterArtifacts` 会把 `.sig` 放在安装包旁边，
    // 清单里的 signature 必须**逐字**等于这个文件内容，否则更新器签名校验失败。
    const sigSrc = `${setupSrc}.sig`
    if (!fs.existsSync(sigSrc)) {
        throw new Error(
            `缺少 ${path.basename(sigSrc)}：TAURI_SIGNING_PRIVATE_KEY 没设？没有签名就无法自动更新`,
        )
    }
    const signature = fs.readFileSync(sigSrc, 'utf8').trim()
    const sigDst = `${setupDst}.sig`
    fs.copyFileSync(sigSrc, sigDst)

    const notesPath = path.join(outDir, 'NOTES.md')
    const notes = fs.existsSync(notesPath)
        ? fs.readFileSync(notesPath, 'utf8').trim()
        : `HaYenai 启动器 ${version}`
    const latest = {
        version,
        notes,
        pub_date: new Date().toISOString(),
        platforms: {
            'windows-x86_64': {
                signature,
                // 发版后这个 tag 会一直存在，所以清单里的地址可以指向版本化 tag，
                // 不必随 latest 漂移。
                url: `https://github.com/bananaikun/HaYenai-Launcher/releases/download/v${version}/HaYenai-${version}-x64-setup.exe`,
            },
        },
    }
    fs.writeFileSync(path.join(outDir, 'latest.json'), `${JSON.stringify(latest, null, 2)}\n`)

    const sums = [setupDst, sigDst, path.join(outDir, 'latest.json')]
        .map((file) => `${sha256(file)}  ${path.basename(file)}`)
        .join('\n')
    fs.writeFileSync(path.join(outDir, 'SHA256SUMS.txt'), `${sums}\n`)

    const mb = (file) => (fs.statSync(file).size / 1024 / 1024).toFixed(1)
    console.log('\n[build] 完成：')
    console.log(`  ${path.basename(setupDst)}  ${mb(setupDst)} MB`)
    console.log(`  输出目录：${outDir}`)
}

main()
