/**
 * HaYenai brand guard.
 *
 * Two families of checks, both run on every CI build:
 *
 * 1. Forbidden patterns - things that must never reappear in the launcher
 *    sources: upstream telemetry, upstream trademark identity, the upstream
 *    site address, advertising bridges and the upstream update feed.
 * 2. Required invariants - the values that define HaYenai. These used to be
 *    phrased as upstream invariants (`Axolotl Launcher` / `red.ghs.axolotl`),
 *    which meant the guard went red as soon as the rebrand started. They are
 *    now stated positively, in terms of what this project must be.
 *
 * The guard also refuses the upstream mascot image files: the rebrand replaced
 * every one of them with `hayenai.png`, so a reappearing `assets/axolotl.*`
 * means someone restored an upstream asset by accident.
 */
import { readFile, readdir } from 'node:fs/promises'
import path from 'node:path'

const roots = ['apps/app', 'apps/app-frontend', 'packages/app-lib']
const ignoredNames = new Set(['LICENSE', 'COPYING.md'])
const ignoredDirectories = new Set(['dist', 'build', 'node_modules', 'target', '.gradle', '.sqlx', 'bin'])

const forbiddenPatterns = [
	['official product name', /Modrinth App/g],
	['official API identity', /modrinth\/theseus/gi],
	['official support identity', /support@modrinth\.com/gi],
	['official deep link', /modrinth:\/\//gi],
	[
		'official telemetry',
		/phc_9Iqi6lFs9sr5BSqh9RRNRSJ0mATS9PSgirDiX3iOYJ|posthog\.modrinth\.com|ingest\.us\.sentry\.io/gi,
	],
	['advertising bridge', /plugin:ads|api::ads|Aditude/gi],
	['official update feed', /launcher-files\.modrinth\.com\/updates\.json/gi],
	['official signing service', /DIGICERT_ONE_SIGNER_CREDENTIALS/gi],
	// Upstream identity of the project this launcher is derived from. Keeping
	// the address out of the sources is what stops the rebrand from drifting
	// back: the repository URL replaced it everywhere.
	['upstream site address', /axlmc\.org/gi],
	['upstream brand constant', /AxolotlBrandConfig/g],
	// The launcher has no analytics of its own and must not grow one by
	// accident: every network call has to be named at the call site.
	['unnamed telemetry endpoint', /telemetry\.axlmc\.org/gi],
]

/** Brand image files that the rebrand removed; `hayenai.*` replaced them. */
const forbiddenAssetNames = [/^axolotl\.(png|svg|jpg|jpeg|webp|ico|icns)$/i]
const assetDirectories = ['apps/app-frontend/src/assets']

async function* files(directory) {
	for (const entry of await readdir(directory, { withFileTypes: true })) {
		if (ignoredDirectories.has(entry.name)) continue
		const entryPath = path.join(directory, entry.name)
		if (entry.isDirectory()) yield* files(entryPath)
		else if (!ignoredNames.has(entry.name)) yield entryPath
	}
}

/** Reads a file as text; binary assets and unreadable paths are skipped. */
async function readText(file) {
	try {
		return await readFile(file, 'utf8')
	} catch {
		return undefined
	}
}

const failures = []

for (const root of roots) {
	for await (const file of files(root)) {
		const contents = await readText(file)
		if (contents === undefined) continue

		for (const [label, pattern] of forbiddenPatterns) {
			pattern.lastIndex = 0
			if (pattern.test(contents)) failures.push(`${file}: ${label}`)
		}
	}
}

for (const directory of assetDirectories) {
	try {
		for await (const file of files(directory)) {
			const name = path.basename(file)
			if (forbiddenAssetNames.some((pattern) => pattern.test(name))) {
				failures.push(`${file}: upstream mascot asset`)
			}
		}
	} catch {
		// A missing asset directory is not a brand failure; the build would fail
		// on the missing import first.
	}
}

const tauriConfig = JSON.parse(await readFile('apps/app/tauri.conf.json', 'utf8'))
const frontendConfig = await readFile('apps/app-frontend/src/config.ts', 'utf8')
const appLibBrand = await readFile('packages/app-lib/src/brand.rs', 'utf8')

const requiredInvariants = [
	['product name', tauriConfig.productName === 'HaYenai'],
	['main binary name', tauriConfig.mainBinaryName === 'HaYenai'],
	['bundle identifier', tauriConfig.identifier === 'com.hayenai.launcher'],
	[
		'deep-link scheme',
		tauriConfig.plugins?.['deep-link']?.desktop?.schemes?.includes('hayenai') === true,
	],
	[
		'deep-link scheme has no upstream alias',
		tauriConfig.plugins?.['deep-link']?.desktop?.schemes?.includes('axolotl') !== true,
	],
	[
		'User-Agent format',
		frontendConfig.includes('`hayenai/${version} (${os})`'),
	],
	[
		'frontend bundle identifier',
		frontendConfig.includes("bundleIdentifier: 'com.hayenai.launcher'"),
	],
	[
		'frontend deep-link scheme',
		frontendConfig.includes("deepLinkScheme: 'hayenai'"),
	],
	['Rust User-Agent format', appLibBrand.includes('hayenai/{version} ({os})')],
	['Rust bundle identifier', appLibBrand.includes('"com.hayenai.launcher"')],
	[
		'private Modrinth services disabled',
		frontendConfig.includes('privateModrinthServices: false'),
	],
	['GHS telemetry disabled', frontendConfig.includes('ghsTelemetry: false')],
]
for (const [label, valid] of requiredInvariants) {
	if (!valid) failures.push(`configuration: missing ${label}`)
}

if (failures.length > 0) {
	console.error(`HaYenai brand guard failed:\n${failures.join('\n')}`)
	process.exit(1)
}

console.log('HaYenai brand guard passed.')
